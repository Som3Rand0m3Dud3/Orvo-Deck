//go:build windows

package main

// Windows audio levels through Core Audio (COM). All calls run on one locked OS thread;
// while the slider moves only the newest level is applied.

import (
	"log"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
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
	level float32
}

var (
	volOnce sync.Once
	volMu   sync.Mutex
	volNext *volReq
	volWake = make(chan struct{}, 1)
)

func setVolume(kind int, app string, level float32) {
	volOnce.Do(func() { go volWorker() })
	volMu.Lock()
	volNext = &volReq{kind, app, level}
	volMu.Unlock()
	select {
	case volWake <- struct{}{}:
	default:
	}
}

func volWorker() {
	runtime.LockOSThread()
	if err := ole.CoInitializeEx(0, ole.COINIT_APARTMENTTHREADED); err != nil {
		log.Println("audio: CoInitialize:", err)
	}
	for range volWake {
		volMu.Lock()
		r := volNext
		volNext = nil
		volMu.Unlock()
		if r == nil {
			continue
		}
		if err := applyVolume(r); err != nil {
			log.Println("audio:", err)
		}
	}
}

func defaultDevice(flow uint32) (*wca.IMMDevice, error) {
	var de *wca.IMMDeviceEnumerator
	if err := wca.CoCreateInstance(wca.CLSID_MMDeviceEnumerator, 0, wca.CLSCTX_ALL, wca.IID_IMMDeviceEnumerator, &de); err != nil {
		return nil, err
	}
	defer de.Release()
	var dev *wca.IMMDevice
	if err := de.GetDefaultAudioEndpoint(flow, wca.EConsole, &dev); err != nil {
		return nil, err
	}
	return dev, nil
}

func applyVolume(r *volReq) error {
	switch r.kind {
	case volMaster, volMic:
		flow := uint32(wca.ERender)
		if r.kind == volMic {
			flow = wca.ECapture
		}
		dev, err := defaultDevice(flow)
		if err != nil {
			return err
		}
		defer dev.Release()
		var aev *wca.IAudioEndpointVolume
		if err := dev.Activate(wca.IID_IAudioEndpointVolume, wca.CLSCTX_ALL, nil, &aev); err != nil {
			return err
		}
		defer aev.Release()
		return aev.SetMasterVolumeLevelScalar(r.level, nil)
	case volApp:
		want := strings.TrimSuffix(strings.ToLower(strings.TrimSpace(r.app)), ".exe")
		if want == "" {
			return nil
		}
		dev, err := defaultDevice(wca.ERender)
		if err != nil {
			return err
		}
		defer dev.Release()
		var sm *wca.IAudioSessionManager2
		if err := dev.Activate(wca.IID_IAudioSessionManager2, wca.CLSCTX_ALL, nil, &sm); err != nil {
			return err
		}
		defer sm.Release()
		var se *wca.IAudioSessionEnumerator
		if err := sm.GetSessionEnumerator(&se); err != nil {
			return err
		}
		defer se.Release()
		var n int
		if err := se.GetCount(&n); err != nil {
			return err
		}
		for i := 0; i < n; i++ {
			var sc *wca.IAudioSessionControl
			if se.GetSession(i, &sc) != nil || sc == nil {
				continue
			}
			disp, err := sc.QueryInterface(wca.IID_IAudioSessionControl2)
			if err != nil {
				sc.Release()
				continue
			}
			sc2 := (*wca.IAudioSessionControl2)(unsafe.Pointer(disp))
			var pid uint32
			sc2.GetProcessId(&pid)
			if pid != 0 && strings.Contains(processName(pid), want) {
				if d, err := sc.QueryInterface(wca.IID_ISimpleAudioVolume); err == nil {
					sav := (*wca.ISimpleAudioVolume)(unsafe.Pointer(d))
					sav.SetMasterVolume(r.level, nil)
					sav.Release()
				}
			}
			sc2.Release()
			sc.Release()
		}
	}
	return nil
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
