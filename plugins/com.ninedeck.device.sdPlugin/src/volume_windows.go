//go:build windows

package main

// Windows audio levels through Core Audio (COM). Everything runs on one locked OS thread.
// The volume controls are opened once and kept (re-opened only after an error, e.g. when the
// default speakers change), so setting a level is nearly instant. While the slider moves only the
// newest level is applied.

import (
	"fmt"
	"log"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"
	"unsafe"

	ole "github.com/go-ole/go-ole"
	"github.com/moutend/go-wca/pkg/wca"
	"golang.org/x/sys/windows"
)

const (
	volMaster = iota
	volApp
	volMic
)

type volReq struct {
	kind  int
	app   string
	dev   string // chosen speakers / mic (endpoint ID); "" = the Windows default
	level float32
	get   chan float32 // set: read the level instead of setting it
	list  chan []string
	devs  chan []audioDev // set: list the speakers (mic == false) or microphones
	mic   bool
}

var (
	volOnce sync.Once
	volMu   sync.Mutex
	volNext *volReq
	volWake = make(chan struct{}, 1)
	volCall = make(chan *volReq, 4)
)

func startVol() { volOnce.Do(func() { go volWorker() }) }

func setVolume(kind int, app, dev string, level float32) {
	startVol()
	volMu.Lock()
	volNext = &volReq{kind: kind, app: app, dev: dev, level: level}
	volMu.Unlock()
	select {
	case volWake <- struct{}{}:
	default:
	}
}

// getVolume reads a level (0..1); ok is false if it could not be read.
func getVolume(kind int, app, dev string) (float32, bool) {
	startVol()
	r := &volReq{kind: kind, app: app, dev: dev, get: make(chan float32, 1)}
	volCall <- r
	select {
	case v := <-r.get:
		return v, v >= 0
	case <-time.After(500 * time.Millisecond):
		return 0, false
	}
}

// listApps returns the programs that currently have sound sessions (e.g. "discord", "chrome").
func listApps() []string {
	startVol()
	r := &volReq{list: make(chan []string, 1)}
	volCall <- r
	select {
	case l := <-r.list:
		return l
	case <-time.After(800 * time.Millisecond):
		return nil
	}
}

// listAudioDevices returns the speakers (mic == false) or microphones that are plugged in and on.
func listAudioDevices(mic bool) []audioDev {
	startVol()
	r := &volReq{devs: make(chan []audioDev, 1), mic: mic}
	volCall <- r
	select {
	case l := <-r.devs:
		return l
	case <-time.After(800 * time.Millisecond):
		return nil
	}
}

type endpoint struct {
	flow uint32
	dev  *wca.IMMDevice
	aev  *wca.IAudioEndpointVolume
}

func epKey(flow uint32, dev string) string { return fmt.Sprintf("%d|%s", flow, dev) }

var (
	endpoints = map[string]*endpoint{} // flow (render / capture) + chosen device -> opened volume control
	sessions  []*session                // app sessions on the default speakers
	sessAt    time.Time
)

type session struct {
	name string
	vol  *wca.ISimpleAudioVolume
}

func volWorker() {
	runtime.LockOSThread()
	if err := ole.CoInitializeEx(0, ole.COINIT_APARTMENTTHREADED); err != nil {
		log.Println("audio: CoInitialize:", err)
	}
	tick := time.NewTicker(3 * time.Second)
	for {
		select {
		case <-tick.C:
			// the default speakers / mic may have changed (headphones plugged in): re-open the
			// controls, but only while the slider is still, so moving it never stutters
			if time.Since(lastApply) > 1500*time.Millisecond {
				dropEndpoint(wca.ERender)
				dropEndpoint(wca.ECapture)
			}
		case r := <-volCall:
			if r.devs != nil {
				flow := uint32(wca.ERender)
				if r.mic {
					flow = wca.ECapture
				}
				r.devs <- enumDevices(flow)
			} else if r.list != nil {
				refreshSessions(true)
				seen := map[string]bool{}
				var names []string
				for _, s := range sessions {
					if s.name != "" && !seen[s.name] {
						seen[s.name] = true
						names = append(names, s.name)
					}
				}
				sort.Strings(names)
				r.list <- names
			} else {
				r.get <- readLevel(r)
			}
		case <-volWake:
			volMu.Lock()
			r := volNext
			volNext = nil
			volMu.Unlock()
			if r != nil {
				if err := applyVolume(r); err != nil {
					log.Println("audio:", err)
				}
			}
		}
	}
}

// openEndpoint opens the volume control of the chosen device, or of the Windows default when none
// is chosen or the chosen one is unplugged (it is picked up again once it is back).
func openEndpoint(flow uint32, devID string) (*endpoint, error) {
	key := epKey(flow, devID)
	if e := endpoints[key]; e != nil {
		return e, nil
	}
	var de *wca.IMMDeviceEnumerator
	if err := wca.CoCreateInstance(wca.CLSID_MMDeviceEnumerator, 0, wca.CLSCTX_ALL, wca.IID_IMMDeviceEnumerator, &de); err != nil {
		return nil, err
	}
	defer de.Release()
	var dev *wca.IMMDevice
	if devID != "" {
		dev = findDevice(de, flow, devID)
	}
	if dev == nil {
		if err := de.GetDefaultAudioEndpoint(flow, wca.EConsole, &dev); err != nil {
			return nil, err
		}
	}
	var aev *wca.IAudioEndpointVolume
	if err := dev.Activate(wca.IID_IAudioEndpointVolume, wca.CLSCTX_ALL, nil, &aev); err != nil {
		dev.Release()
		return nil, err
	}
	e := &endpoint{flow, dev, aev}
	endpoints[key] = e
	return e, nil
}

// eachDevice walks the active devices of one kind; visit returns true to keep the device (and stop).
func eachDevice(de *wca.IMMDeviceEnumerator, flow uint32, visit func(dev *wca.IMMDevice, id string) bool) {
	var dc *wca.IMMDeviceCollection
	if err := de.EnumAudioEndpoints(flow, wca.DEVICE_STATE_ACTIVE, &dc); err != nil || dc == nil {
		return
	}
	defer dc.Release()
	var n uint32
	if dc.GetCount(&n) != nil {
		return
	}
	for i := uint32(0); i < n; i++ {
		var dev *wca.IMMDevice
		if dc.Item(i, &dev) != nil || dev == nil {
			continue
		}
		var id string
		if dev.GetId(&id) == nil && visit(dev, id) {
			return
		}
		dev.Release()
	}
}

func findDevice(de *wca.IMMDeviceEnumerator, flow uint32, devID string) *wca.IMMDevice {
	var found *wca.IMMDevice
	eachDevice(de, flow, func(dev *wca.IMMDevice, id string) bool {
		if id == devID {
			found = dev
			return true
		}
		return false
	})
	return found
}

func enumDevices(flow uint32) []audioDev {
	list := []audioDev{}
	var de *wca.IMMDeviceEnumerator
	if err := wca.CoCreateInstance(wca.CLSID_MMDeviceEnumerator, 0, wca.CLSCTX_ALL, wca.IID_IMMDeviceEnumerator, &de); err != nil {
		return list
	}
	defer de.Release()
	eachDevice(de, flow, func(dev *wca.IMMDevice, id string) bool {
		name := ""
		var ps *wca.IPropertyStore
		if dev.OpenPropertyStore(wca.STGM_READ, &ps) == nil && ps != nil {
			var pv wca.PROPVARIANT
			if ps.GetValue(&wca.PKEY_Device_FriendlyName, &pv) == nil {
				name = pv.String()
			}
			ps.Release()
		}
		if name == "" {
			name = "Unnamed device"
		}
		list = append(list, audioDev{ID: id, Name: name})
		return false
	})
	sort.Slice(list, func(i, j int) bool { return list[i].Name < list[j].Name })
	return list
}

// dropEndpoint closes every opened control of one kind (speakers or microphones).
func dropEndpoint(flow uint32) {
	for key, e := range endpoints {
		if e.flow == flow {
			e.aev.Release()
			e.dev.Release()
			delete(endpoints, key)
		}
	}
	if flow == wca.ERender {
		dropSessions()
	}
}

func flowOf(kind int) uint32 {
	if kind == volMic {
		return wca.ECapture
	}
	return wca.ERender
}

var lastApply time.Time

func applyVolume(r *volReq) error {
	lastApply = time.Now()
	switch r.kind {
	case volMaster, volMic:
		for try := 0; try < 2; try++ {
			e, err := openEndpoint(flowOf(r.kind), r.dev)
			if err != nil {
				return err
			}
			if err = e.aev.SetMasterVolumeLevelScalar(r.level, nil); err == nil {
				return nil
			}
			dropEndpoint(flowOf(r.kind))
		}
	case volApp:
		want := appKey(r.app)
		if want == "" {
			return nil
		}
		refreshSessions(false)
		for _, s := range sessions {
			if strings.Contains(s.name, want) {
				if s.vol.SetMasterVolume(r.level, nil) != nil {
					dropSessions()
				}
			}
		}
	}
	return nil
}

func readLevel(r *volReq) float32 {
	var level float32 = -1
	switch r.kind {
	case volMaster, volMic:
		if e, err := openEndpoint(flowOf(r.kind), r.dev); err == nil {
			if e.aev.GetMasterVolumeLevelScalar(&level) != nil {
				level = -1
			}
		}
	case volApp:
		want := appKey(r.app)
		refreshSessions(false)
		for _, s := range sessions {
			if want != "" && strings.Contains(s.name, want) {
				if s.vol.GetMasterVolume(&level) == nil {
					break
				}
			}
		}
	}
	return level
}

func appKey(app string) string {
	return strings.TrimSuffix(strings.ToLower(strings.TrimSpace(app)), ".exe")
}

func dropSessions() {
	for _, s := range sessions {
		s.vol.Release()
	}
	sessions = nil
}

// refreshSessions lists the app sound sessions; kept for 3 s so moving the slider stays fast.
func refreshSessions(force bool) {
	if !force && sessions != nil && time.Since(sessAt) < 3*time.Second {
		return
	}
	dropSessions()
	sessAt = time.Now()
	e, err := openEndpoint(wca.ERender, "")
	if err != nil {
		return
	}
	var sm *wca.IAudioSessionManager2
	if err := e.dev.Activate(wca.IID_IAudioSessionManager2, wca.CLSCTX_ALL, nil, &sm); err != nil {
		return
	}
	defer sm.Release()
	var se *wca.IAudioSessionEnumerator
	if err := sm.GetSessionEnumerator(&se); err != nil {
		return
	}
	defer se.Release()
	var n int
	if se.GetCount(&n) != nil {
		return
	}
	for i := 0; i < n; i++ {
		var sc *wca.IAudioSessionControl
		if se.GetSession(i, &sc) != nil || sc == nil {
			continue
		}
		name := ""
		if disp, err := sc.QueryInterface(wca.IID_IAudioSessionControl2); err == nil {
			sc2 := (*wca.IAudioSessionControl2)(unsafe.Pointer(disp))
			var pid uint32
			sc2.GetProcessId(&pid)
			if pid != 0 {
				name = processName(pid)
			}
			sc2.Release()
		}
		if d, err := sc.QueryInterface(wca.IID_ISimpleAudioVolume); err == nil && name != "" {
			sessions = append(sessions, &session{name, (*wca.ISimpleAudioVolume)(unsafe.Pointer(d))})
		}
		sc.Release()
	}
}

func processName(pid uint32) string {
	h, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
	if err != nil {
		return ""
	}
	defer windows.CloseHandle(h)
	buf := make([]uint16, windows.MAX_PATH)
	size := uint32(len(buf))
	if windows.QueryFullProcessImageName(h, 0, &buf[0], &size) != nil {
		return ""
	}
	return strings.ToLower(strings.TrimSuffix(filepath.Base(windows.UTF16ToString(buf[:size])), ".exe"))
}
