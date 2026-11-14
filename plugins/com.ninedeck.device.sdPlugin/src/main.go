// NineDeck device plugin for OpenDeck (OpenAction API).
// Finds NineDeck pads on USB serial, registers each one with OpenDeck as a 3x4 deck,
// forwards key presses, and shows the name of each key's assigned action on the pad's screen.
package main

import (
	"bufio"
	"bytes"
	"encoding/base64"
	"image"
	_ "image/jpeg"
	_ "image/png"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"io/fs"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/websocket"
	"go.bug.st/serial"
	"go.bug.st/serial/enumerator"
)

const (
	namespace = "nd"
	rows      = 3
	cols      = 3
	numKeys   = 11
	labelMax  = 11
)

// pad key number (1..11) -> OpenDeck position in a 3x3 grid.
// Keys 1-9 are the grid; 10 and 11 are the page keys, handled here (they flip pages), so -1.
func keyToPos(k int) int {
	if k >= 1 && k <= 9 {
		return k - 1
	}
	return -1
}

type pad struct {
	id      string
	port    string
	sp      serial.Port
	wmu     sync.Mutex
	fw      string // firmware version the pad reports, e.g. "0.2"
	box     int    // box size in pixels when the firmware can show pictures (0.3+), else 0
	pmu     sync.Mutex
	pending map[int][]byte // box index -> RGB565 picture (nil = empty), waiting to be sent
	pageSet string
	timer   *time.Timer
	page    string    // page (profile id) the pad shows now
	flipped time.Time // when the pad's own page keys last changed page
	shown   [9][]byte // what each box shows now (to skip resending the same picture)
	smu     sync.Mutex // one sender at a time (pad keys and app updates)
	labels  [numKeys]string
	sent    [numKeys]bool
	closed  bool
}

var (
	ws     *websocket.Conn
	wsMu   sync.Mutex
	padsMu sync.Mutex
	pads   = map[string]*pad{} // by port name
	kick   = make(chan struct{}, 1)
)

func send(v any) {
	wsMu.Lock()
	defer wsMu.Unlock()
	if err := ws.WriteJSON(v); err != nil {
		log.Println("websocket write:", err)
	}
}

func (p *pad) writeRaw(b []byte) {
	p.wmu.Lock()
	defer p.wmu.Unlock()
	if p.closed {
		return
	}
	p.sp.Write(b)
}

func (p *pad) write(line string) {
	p.wmu.Lock()
	defer p.wmu.Unlock()
	if p.closed {
		return
	}
	p.sp.Write([]byte(line + "\n"))
}

func main() {
	port := flag.String("port", "", "")
	uuid := flag.String("pluginUUID", "", "")
	regEvent := flag.String("registerEvent", "", "")
	flag.String("info", "", "")
	flag.Parse()

	if lf, err := os.OpenFile(filepath.Join(exeDir(), "plugin.log"), os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0644); err == nil {
		log.SetOutput(lf)
	}
	log.Println("NineDeck plugin starting")

	var err error
	ws, _, err = websocket.DefaultDialer.Dial("ws://localhost:"+*port, nil)
	if err != nil {
		log.Fatal("connect to OpenDeck: ", err)
	}
	send(map[string]any{"event": *regEvent, "uuid": *uuid})

	go scanner()
	go labelLoop()

	for {
		_, msg, err := ws.ReadMessage()
		if err != nil {
			log.Println("OpenDeck connection closed:", err)
			break
		}
		handleActionEvent(msg)
		var ev struct {
			Event      string  `json:"event"`
			Device     string  `json:"device"`
			Controller *string `json:"controller"`
			Position   *int    `json:"position"`
			Image      *string `json:"image"`
			Profile    string  `json:"profile"`
			Images     []*string `json:"images"`
			SlideMs    *int      `json:"slideMs"`
			Taper      *int      `json:"taper"`
			Flip       *int      `json:"flip"`
			Recal      bool      `json:"recalibrate"`
		}
		json.Unmarshal(msg, &ev)
		switch ev.Event {
		case "setImage":
			if ev.Position != nil && (ev.Controller == nil || *ev.Controller == "Keypad") {
				img := ""
				if ev.Image != nil {
					img = *ev.Image
				}
				queuePicture(ev.Device, ev.Profile, *ev.Position, img)
			}
		case "orvoPadSettings":
			if ev.Taper != nil {
				sliderTaper.Store(int32(*ev.Taper))
			}
			if ev.Flip != nil {
				sliderFlip.Store(int32(*ev.Flip))
			}
			if ev.Recal {
				padsMu.Lock()
				for _, p := range pads {
					if !newer("0.7", p.fw) {
						p.write("C")
					}
				}
				padsMu.Unlock()
			}
			if ev.SlideMs != nil || ev.Taper != nil || ev.Flip != nil {
				if ev.SlideMs != nil {
					slideMs.Store(int32(*ev.SlideMs))
				}
				padsMu.Lock()
				for _, p := range pads {
					applySettings(p)
				}
				padsMu.Unlock()
			}
		case "orvoPageImages":
			go storePage(ev.Device, ev.Profile, ev.Images)
		case "orvoPadStatusRequest":
			padsMu.Lock()
			for _, p := range pads {
				sendStatus(p.id, p.fw, "ready", "")
			}
			padsMu.Unlock()
		case "orvoUpdateFirmware":
			go updateFirmware(ev.Device)
		}
		if ev.Event == "setImage" || ev.Event == "deviceDidConnect" {
			// a key changed in OpenDeck: refresh labels (profile file is written shortly after)
			go func() {
				time.Sleep(400 * time.Millisecond)
				select {
				case kick <- struct{}{}:
				default:
				}
			}()
		}
	}

	// give the pad back its normal behaviour
	padsMu.Lock()
	for _, p := range pads {
		for k := 1; k <= numKeys; k++ {
			p.write(fmt.Sprintf("L%d %d", k, k))
		}
		p.write("H 1")
	}
	padsMu.Unlock()
	time.Sleep(200 * time.Millisecond)
}

func exeDir() string {
	e, err := os.Executable()
	if err != nil {
		return "."
	}
	return filepath.Dir(e)
}

// ---------- finding and talking to pads ----------

func scanner() {
	// test hook: NINEDECK_PORT forces a specific serial port
	if fp := os.Getenv("NINEDECK_PORT"); fp != "" {
		for {
			padsMu.Lock()
			_, known := pads[fp]
			padsMu.Unlock()
			if !known {
				tryOpen(fp, "TEST")
			}
			time.Sleep(2 * time.Second)
		}
	}
	for {
		list, err := enumerator.GetDetailedPortsList()
		if err == nil {
			for _, d := range list {
				if !d.IsUSB || !strings.EqualFold(d.VID, "2E8A") {
					continue
				}
				padsMu.Lock()
				_, known := pads[d.Name]
				padsMu.Unlock()
				if !known {
					tryOpen(d.Name, d.SerialNumber)
				}
			}
		}
		time.Sleep(2 * time.Second)
	}
}

func tryOpen(portName, serialNo string) {
	sp, err := serial.Open(portName, &serial.Mode{BaudRate: 115200})
	if err != nil {
		return
	}
	sp.SetDTR(true)
	sp.SetReadTimeout(300 * time.Millisecond)
	sp.Write([]byte("?\n"))

	rd := bufio.NewReader(sp)
	deadline := time.Now().Add(2 * time.Second)
	ok := false
	fw := ""
	box := 0
	for time.Now().Before(deadline) {
		line, _ := rd.ReadString('\n')
		if f := strings.Fields(strings.TrimSpace(line)); len(f) >= 2 && f[0] == "NINEDECK" {
			fw = f[1]
			for i := 2; i+1 < len(f); i++ {
				if f[i] == "BOX" {
					box, _ = strconv.Atoi(f[i+1])
				}
			}
			ok = true
			break
		}
	}
	if !ok {
		sp.Close()
		return
	}
	sp.SetReadTimeout(serial.NoTimeout)

	if serialNo == "" {
		serialNo = strings.ReplaceAll(portName, "\\", "")
	}
	p := &pad{id: namespace + "-" + serialNo, port: portName, sp: sp, fw: fw, box: box, pending: map[int][]byte{}}
	padsMu.Lock()
	pads[portName] = p
	padsMu.Unlock()

	migrateLayout(p.id)
	slMu.Lock()
	delete(slNameSent, p.id) // send the slider name / icon again
	slMu.Unlock()
	if !newer("0.9.2", p.fw) {
		// "I'm here" once a second: without it the pad shows its idle logo (app closed or crashed)
		go func() {
			for {
				time.Sleep(time.Second)
				p.wmu.Lock()
				gone := p.closed
				p.wmu.Unlock()
				if gone {
					return
				}
				p.write("h")
			}
		}()
	}
	p.write("H 0")
	if p.box > 0 {
		p.page = selectedProfile(p.id)
		p.pageSet = pageLabel(p.id)
		applySettings(p)
		p.write("P " + p.pageSet)
		p.write("F")
	} // OpenDeck handles the keys now, so the F13-F23 presses are off
	send(map[string]any{
		"event": "registerDevice",
		"payload": map[string]any{
			"id": p.id, "name": "NineDeck", "rows": rows, "columns": cols, "encoders": sliderCount(p), "type": 0,
		},
	})
	log.Println("registered", p.id, "on", portName, "firmware", fw)
	updMu.Lock()
	wasUpdating := updating[p.id]
	delete(updating, p.id)
	updMu.Unlock()
	if wasUpdating {
		sendStatus(p.id, fw, "done", "Pad updated to firmware v"+fw)
	} else {
		sendStatus(p.id, fw, "ready", "")
	}
	select {
	case kick <- struct{}{}:
	default:
	}

	go reader(p, rd)
}

func reader(p *pad, rd *bufio.Reader) {
	for {
		line, err := rd.ReadString('\n')
		if err != nil {
			break
		}
		line = strings.TrimSpace(line)
		// "S1 <0-1000>": the slider moved
		if strings.HasPrefix(line, "S1 ") {
			if v, err := strconv.Atoi(strings.TrimSpace(line[3:])); err == nil {
				sliderMoved(p, v)
			}
			continue
		}
		// "K<n> 1" / "K<n> 0"
		if len(line) >= 4 && line[0] == 'K' {
			parts := strings.Fields(line[1:])
			if len(parts) != 2 {
				continue
			}
			k, _ := strconv.Atoi(parts[0])
			if (k == 10 || k == 11) && parts[1] == "1" {
				flipPage(p, map[int]int{10: -1, 11: 1}[k])
				continue
			}
			pos := keyToPos(k)
			if pos < 0 {
				continue
			}
			ev := "keyUp"
			if parts[1] == "1" {
				ev = "keyDown"
			}
			send(map[string]any{"event": ev, "payload": map[string]any{"device": p.id, "position": pos}})
		}
	}
	log.Println("pad disconnected:", p.id)
	p.wmu.Lock()
	p.closed = true
	p.sp.Close()
	p.wmu.Unlock()
	send(map[string]any{"event": "deregisterDevice", "payload": p.id})
	padsMu.Lock()
	delete(pads, p.port)
	padsMu.Unlock()
}

// ---------- key labels from OpenDeck's profile files ----------

type diskInstance struct {
	Action struct {
		Name string `json:"name"`
	} `json:"action"`
	States []struct {
		Name string `json:"name"`
		Text         string  `json:"text"`
		TextOverride *string `json:"text_override"`
	} `json:"states"`
	CurrentState int `json:"current_state"`
}

type diskProfile struct {
	Keys []*diskInstance `json:"keys"`
}

func configDir() string {
	// the plugin lives in <config>/plugins/<uuid>/, so walk up two folders
	if d := filepath.Dir(filepath.Dir(exeDir())); fileExists(filepath.Join(d, "profiles")) {
		return d
	}
	if a := os.Getenv("APPDATA"); a != "" {
		if d := filepath.Join(a, "com.orvo.studio"); fileExists(d) {
			return d
		}
		return filepath.Join(a, "opendeck")
	}
	h, _ := os.UserHomeDir()
	return filepath.Join(h, ".config", "opendeck")
}

func fileExists(p string) bool {
	_, err := os.Stat(p)
	return err == nil
}

func selectedProfile(device string) string {
	b, err := os.ReadFile(filepath.Join(configDir(), "profiles", device+".json"))
	if err == nil {
		var c struct {
			Selected string `json:"selected_profile"`
		}
		if json.Unmarshal(b, &c) == nil && c.Selected != "" {
			return c.Selected
		}
	}
	return "Default"
}

func readProfile(device string) *diskProfile {
	name := selectedProfile(device)
	path := filepath.Join(append([]string{configDir(), "profiles", device}, strings.Split(name, "/")...)...) + ".json"
	b, err := os.ReadFile(path)
	if err != nil {
		return nil
	}
	var p diskProfile
	if json.Unmarshal(b, &p) != nil {
		return nil
	}
	return &p
}

func cleanLabel(s string) string {
	s = strings.Join(strings.Fields(s), " ") // newlines and double spaces -> single space
	out := make([]rune, 0, len(s))
	for _, r := range s {
		if r >= 32 && r < 127 { // the pad's font is ASCII only
			out = append(out, r)
		}
	}
	s = strings.TrimSpace(string(out))
	if len(s) > labelMax {
		cut := strings.LastIndex(s[:labelMax+1], " ")
		if cut > 2 {
			s = s[:cut] // whole words only
		} else {
			s = s[:labelMax]
		}
	}
	return s
}

func labelFor(in *diskInstance) string {
	if in == nil {
		return ""
	}
	if in.CurrentState >= 0 && in.CurrentState < len(in.States) {
		st := in.States[in.CurrentState]
		// your own title (set in OpenDeck) wins, then the action's own title, then the action name
		if st.TextOverride != nil && cleanLabel(*st.TextOverride) != "" {
			return cleanLabel(*st.TextOverride)
		}
		if cleanLabel(st.Text) != "" {
			return cleanLabel(st.Text)
		}
	}
	return cleanLabel(in.Action.Name)
}

func labelLoop() {
	t := time.NewTicker(400 * time.Millisecond)
	for {
		select {
		case <-t.C:
		case <-kick:
		}
		padsMu.Lock()
		list := make([]*pad, 0, len(pads))
		for _, p := range pads {
			list = append(list, p)
		}
		padsMu.Unlock()

		for _, p := range list {
			if p.box > 0 {
				continue // this pad shows the app's pictures instead
			}
			prof := readProfile(p.id)
			for k := 1; k <= numKeys; k++ {
				pos := keyToPos(k)
				lbl := ""
				if pos >= 0 && prof != nil && pos < len(prof.Keys) {
					lbl = labelFor(prof.Keys[pos])
				}
				if k == 10 {
					lbl = "<"
				} else if k == 11 {
					lbl = ">"
				}
				if !p.sent[k-1] || p.labels[k-1] != lbl {
					p.labels[k-1] = lbl
					p.sent[k-1] = true
					p.write(fmt.Sprintf("L%d %s", k, lbl))
					time.Sleep(15 * time.Millisecond)
				}
			}
		}
	}
}

// ---------- pages (OpenDeck profiles) ----------

// listPages returns the device's profiles ("Name" or "Folder/Name"), sorted the same way as the app.
func listPages(device string) []string {
	root := filepath.Join(configDir(), "profiles", device)
	var out []string
	filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() || !strings.HasSuffix(d.Name(), ".json") {
			return nil
		}
		rel, _ := filepath.Rel(root, path)
		out = append(out, strings.TrimSuffix(filepath.ToSlash(rel), ".json"))
		return nil
	})
	sort.Slice(out, func(i, j int) bool { return strings.ToLower(out[i]) < strings.ToLower(out[j]) })
	return out
}

func flipPage(p *pad, step int) {
	device := p.id
	pages := listPages(device)
	if len(pages) < 2 {
		return
	}
	p.pmu.Lock()
	cur := p.page
	p.pmu.Unlock()
	if cur == "" {
		cur = selectedProfile(device)
	}
	i := 0
	for n, x := range pages {
		if x == cur {
			i = n
		}
	}
	next := pages[(i+step+len(pages))%len(pages)]

	if p.box > 0 {
		// Show the page right away from the pictures Orvo Studio prepared, then tell the app.
		p.pmu.Lock()
		p.page = next
		p.flipped = time.Now()
		p.pending = map[int][]byte{}
		cached, ok := pageCache(device, next)
		p.pmu.Unlock()
		if ok && !newer("0.5", p.fw) {
			p.write(fmt.Sprintf("A %d", step)) // slide the new page in from the side of the key pressed
		}
		if ok {
			for pos := 0; pos < 9; pos++ {
				sendBox(p, pos, cached[pos])
			}
		}
		p.pageSet = pageName(next)
		p.write("P " + p.pageSet)
		p.write("F")
	}
	send(map[string]any{"event": "switchProfile", "device": device, "profile": next})
	if p.box == 0 {
		go func() {
			time.Sleep(400 * time.Millisecond)
			kickLabels()
		}()
	}
}

// slide speed chosen in Orvo Studio (ms, 0 = no slide)
var slideMs atomic.Int32

// slider curve chosen in Orvo Studio (0 = linear B pot, 1 = audio A pot, -1 = not set)
var sliderTaper atomic.Int32

// slider direction chosen in Orvo Studio (1 = flipped, -1 = not set)
var sliderFlip atomic.Int32

func init() {
	slideMs.Store(180)
	sliderTaper.Store(-1)
	sliderFlip.Store(-1)
}

func applySettings(p *pad) {
	if p.box > 0 && !newer("0.5", p.fw) {
		p.write(fmt.Sprintf("V %d", slideMs.Load()))
	}
	if t := sliderTaper.Load(); t >= 0 && !newer("0.7", p.fw) {
		p.write(fmt.Sprintf("T %d", t))
	}
	if f := sliderFlip.Load(); f >= 0 && !newer("0.8", p.fw) {
		p.write(fmt.Sprintf("R %d", f))
	}
}

// ---------- page pictures kept on the PC (as many pages as you like) ----------

var (
	cacheMu sync.Mutex
	cache   = map[string]map[string][9][]byte{} // device -> page -> box pictures (pad format)
)

func pageCache(device, page string) ([9][]byte, bool) {
	cacheMu.Lock()
	defer cacheMu.Unlock()
	c, ok := cache[device][page]
	return c, ok
}

func setCacheBox(device, page string, pos int, pix []byte) {
	cacheMu.Lock()
	defer cacheMu.Unlock()
	if cache[device] == nil {
		cache[device] = map[string][9][]byte{}
	}
	c := cache[device][page]
	c[pos] = pix
	cache[device][page] = c
}

func storePage(device, page string, images []*string) {
	p := padByID(device)
	if p == nil || p.box == 0 {
		return
	}
	var c [9][]byte
	for i := 0; i < 9 && i < len(images); i++ {
		if images[i] != nil && *images[i] != "" {
			c[i] = toRGB565(*images[i], p.box)
		}
	}
	cacheMu.Lock()
	if cache[device] == nil {
		cache[device] = map[string][9][]byte{}
	}
	cache[device][page] = c
	cacheMu.Unlock()
}

func pageName(id string) string {
	if i := strings.LastIndex(id, "/"); i >= 0 {
		id = id[i+1:]
	}
	return cleanLabel(id)
}

// sendBox sends one box unless the pad already shows exactly that picture.
func sendBox(p *pad, pos int, pix []byte) bool {
	p.smu.Lock()
	defer p.smu.Unlock()
	if p.shown[pos] != nil && pix != nil && bytes.Equal(p.shown[pos], pix) {
		return false
	}
	if p.shown[pos] == nil && pix == nil && p.sent[pos] {
		return false
	}
	p.shown[pos] = pix
	p.sent[pos] = true
	if pix == nil {
		p.write(fmt.Sprintf("E%d", pos+1))
		return true
	}
	var b bytes.Buffer
	if !newer("0.4", p.fw) {
		if z := rle(pix); len(z) < len(pix) {
			fmt.Fprintf(&b, "Z%d %d %d %d\n", pos+1, p.box, p.box, len(z))
			b.Write(z)
			p.writeRaw(b.Bytes())
			return true
		}
	}
	fmt.Fprintf(&b, "I%d %d %d\n", pos+1, p.box, p.box)
	b.Write(pix)
	p.writeRaw(b.Bytes())
	return true
}

// rle packs RGB565 pixels as runs: [count 1-255][low byte][high byte].
func rle(pix []byte) []byte {
	out := make([]byte, 0, len(pix)/2)
	for i := 0; i < len(pix); {
		lo, hi := pix[i], pix[i+1]
		n := 1
		for i+2*n+1 < len(pix) && n < 255 && pix[i+2*n] == lo && pix[i+2*n+1] == hi {
			n++
		}
		out = append(out, byte(n), lo, hi)
		i += 2 * n
	}
	return out
}

// migrateLayout moves profiles saved with the old 3x4 layout (page keys in a 4th column)
// to the 3x3 layout, so keys 4-9 keep their actions, then repairs any key whose saved
// position no longer matches its slot (contexts are "Keypad.<pos>.<idx>" on disk and the
// key's custom images live in images/<device>/<profile>/Keypad.<pos>.<idx>/).
func migrateLayout(device string) {
	cfg := configDir()
	root := filepath.Join(cfg, "profiles", device)
	filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
		if err != nil || d.IsDir() || !strings.HasSuffix(d.Name(), ".json") {
			return nil
		}
		b, err := os.ReadFile(path)
		if err != nil {
			return nil
		}
		var prof map[string]any
		if json.Unmarshal(b, &prof) != nil {
			return nil
		}
		keys, ok := prof["keys"].([]any)
		if !ok {
			return nil
		}
		changed := false
		if len(keys) == 12 {
			nk := make([]any, 9)
			for r := 0; r < 3; r++ {
				for c := 0; c < 3; c++ {
					nk[r*3+c] = keys[r*4+c]
				}
			}
			keys = nk
			changed = true
		}
		rel, _ := filepath.Rel(root, path)
		profileID := strings.TrimSuffix(filepath.ToSlash(rel), ".json")
		imgDir := filepath.Join(append([]string{cfg, "images", device}, strings.Split(profileID, "/")...)...)
		type move struct{ from, to string }
		var moves []move
		for i, k := range keys {
			inst, ok := k.(map[string]any)
			if !ok {
				continue
			}
			old := keyPos(inst["context"])
			if old < 0 || old == i {
				continue
			}
			// rename this key's image folders (Keypad.<old>.<n> -> Keypad.<i>.<n>)
			entries, _ := os.ReadDir(imgDir)
			prefix := fmt.Sprintf("Keypad.%d.", old)
			for _, e := range entries {
				if e.IsDir() && strings.HasPrefix(e.Name(), prefix) {
					moves = append(moves, move{e.Name(), fmt.Sprintf("Keypad.%d.%s", i, strings.TrimPrefix(e.Name(), prefix))})
				}
			}
			renumber(inst, old, i)
			changed = true
		}
		// two steps through temporary names so swaps never overwrite each other
		for n, m := range moves {
			os.Rename(filepath.Join(imgDir, m.from), filepath.Join(imgDir, fmt.Sprintf("orvo-tmp-%d", n)))
		}
		for n, m := range moves {
			dst := filepath.Join(imgDir, m.to)
			os.RemoveAll(dst)
			os.Rename(filepath.Join(imgDir, fmt.Sprintf("orvo-tmp-%d", n)), dst)
		}
		if !changed {
			return nil
		}
		prof["keys"] = keys
		if out, err := json.Marshal(prof); err == nil {
			if !fileExists(path + ".bak-orvo") {
				os.WriteFile(path+".bak-orvo", b, 0o644)
			}
			os.WriteFile(path, out, 0o644)
			log.Println("fixed layout of", path)
		}
		return nil
	})
}

// keyPos reads the position from a context string ("Keypad.4.0" or "dev.profile.Keypad.4.0").
func keyPos(v any) int {
	s, ok := v.(string)
	if !ok {
		return -1
	}
	parts := strings.Split(s, ".")
	for i := 0; i+1 < len(parts); i++ {
		if parts[i] == "Keypad" {
			if n, err := strconv.Atoi(parts[i+1]); err == nil {
				return n
			}
		}
	}
	return -1
}

// renumber rewrites the position in every "context" inside an instance (children included).
func renumber(v any, oldPos, newPos int) {
	switch t := v.(type) {
	case map[string]any:
		for k, x := range t {
			if s, ok := x.(string); ok && k == "context" {
				parts := strings.Split(s, ".")
				for i := 0; i+1 < len(parts); i++ {
					if parts[i] == "Keypad" && parts[i+1] == strconv.Itoa(oldPos) {
						parts[i+1] = strconv.Itoa(newPos)
						break
					}
				}
				t[k] = strings.Join(parts, ".")
			} else {
				renumber(x, oldPos, newPos)
			}
		}
	case []any:
		for _, x := range t {
			renumber(x, oldPos, newPos)
		}
	}
}

// ---------- firmware updates (asked for by Orvo Studio) ----------

// The newest firmware ships next to this plugin as firmware/ninedeck-fw.uf2.
const latestFirmware = "0.9.3"

var (
	updMu    sync.Mutex
	updating = map[string]bool{}
)

func newer(a, b string) bool { // is version a newer than b?
	pa, pb := strings.Split(a, "."), strings.Split(b, ".")
	for i := 0; i < len(pa) || i < len(pb); i++ {
		var x, y int
		if i < len(pa) {
			x, _ = strconv.Atoi(pa[i])
		}
		if i < len(pb) {
			y, _ = strconv.Atoi(pb[i])
		}
		if x != y {
			return x > y
		}
	}
	return false
}

func sendStatus(device, fw, state, message string) {
	send(map[string]any{"event": "orvoPadStatus", "payload": map[string]any{
		"device": device, "name": "NineDeck", "firmware": fw, "latest": latestFirmware,
		"updateAvailable": fw != "" && newer(latestFirmware, fw), "state": state, "message": message,
	}})
}

func updateFirmware(device string) {
	fail := func(fw, msg string) {
		updMu.Lock()
		delete(updating, device)
		updMu.Unlock()
		log.Println("firmware update failed:", msg)
		sendStatus(device, fw, "error", msg)
	}
	var p *pad
	padsMu.Lock()
	for _, x := range pads {
		if x.id == device {
			p = x
		}
	}
	padsMu.Unlock()
	if p == nil {
		fail("", "The pad is not connected.")
		return
	}
	image, err := os.ReadFile(filepath.Join(exeDir(), "firmware", "ninedeck-fw.uf2"))
	if err != nil {
		fail(p.fw, "The firmware file is missing from the plugin.")
		return
	}
	updMu.Lock()
	updating[device] = true
	updMu.Unlock()
	sendStatus(device, p.fw, "updating", "Restarting the pad into update mode…")

	// Firmware 0.2+ understands "B"; older firmware restarts into the bootloader
	// when its serial port is opened at 1200 baud and closed again.
	if !newer("0.2", p.fw) {
		p.write("B")
		time.Sleep(300 * time.Millisecond)
	} else {
		p.wmu.Lock()
		p.closed = true
		p.sp.Close()
		p.wmu.Unlock()
		time.Sleep(300 * time.Millisecond)
		if sp, err := serial.Open(p.port, &serial.Mode{BaudRate: 1200}); err == nil {
			sp.SetDTR(false)
			time.Sleep(100 * time.Millisecond)
			sp.Close()
		}
	}

	drive := ""
	for i := 0; i < 40 && drive == ""; i++ {
		time.Sleep(500 * time.Millisecond)
		drive = findUF2Drive()
	}
	if drive == "" {
		fail(p.fw, "The pad did not show up as an update drive. Hold BOOTSEL while plugging it in, then try again.")
		return
	}
	sendStatus(device, p.fw, "updating", "Installing firmware v"+latestFirmware+"…")
	if err := os.WriteFile(filepath.Join(drive, "NINEDECK.UF2"), image, 0o644); err != nil {
		fail(p.fw, "Could not copy the firmware to the pad: "+err.Error())
		return
	}
	sendStatus(device, p.fw, "updating", "Restarting the pad…")
	// The pad restarts by itself; the scanner finds it again and reports "done".
	go func() {
		time.Sleep(30 * time.Second)
		updMu.Lock()
		still := updating[device]
		updMu.Unlock()
		if still {
			fail(p.fw, "The pad did not come back after the update. Unplug it and plug it back in.")
		}
	}()
}

// findUF2Drive looks for the Raspberry Pi update drive (it holds INFO_UF2.TXT).
func findUF2Drive() string {
	var candidates []string
	if os.PathSeparator == '\\' {
		for c := 'D'; c <= 'Z'; c++ {
			candidates = append(candidates, string(c)+":\\")
		}
	} else {
		candidates = append(candidates, "/Volumes/RPI-RP2", "/Volumes/RP2350")
		for _, pattern := range []string{"/media/*/*", "/run/media/*/*", "/mnt/*"} {
			m, _ := filepath.Glob(pattern)
			candidates = append(candidates, m...)
		}
	}
	for _, c := range candidates {
		b, err := os.ReadFile(filepath.Join(c, "INFO_UF2.TXT"))
		if err == nil && (strings.Contains(string(b), "RP2") || strings.Contains(string(b), "UF2 Bootloader")) {
			return c
		}
	}
	return ""
}

// ---------- pictures: the pad shows exactly what the app's boxes show ----------

func padByID(device string) *pad {
	padsMu.Lock()
	defer padsMu.Unlock()
	for _, p := range pads {
		if p.id == device {
			return p
		}
	}
	return nil
}

// queuePicture converts the app's rendered box (a data URL) for the pad and sends it a moment
// later together with any other boxes that change at the same time, so a page change shows at once.
func queuePicture(device, profile string, pos int, dataURL string) {
	p := padByID(device)
	if p == nil || p.box == 0 || pos < 0 || pos > 8 {
		if p != nil && p.box == 0 {
			kickLabels()
		}
		return
	}
	var pix []byte
	if dataURL != "" {
		pix = toRGB565(dataURL, p.box)
	}
	p.pmu.Lock()
	defer p.pmu.Unlock()
	if profile != "" {
		setCacheBox(device, profile, pos, pix)
		if profile != p.page {
			if time.Since(p.flipped) < 1500*time.Millisecond {
				return // a late picture from the page we just left: the pad already moved on
			}
			p.page = profile // the page was changed in the app
		}
	}
	p.pending[pos] = pix
	if p.timer == nil {
		p.timer = time.AfterFunc(12*time.Millisecond, func() { flushPictures(p) })
	}
}

func flushPictures(p *pad) {
	p.pmu.Lock()
	pending := p.pending
	p.pending = map[int][]byte{}
	p.timer = nil
	p.pmu.Unlock()

	if len(pending) == 0 {
		return
	}
	changed := false
	for pos := 0; pos < 9; pos++ {
		if pix, ok := pending[pos]; ok && sendBox(p, pos, pix) {
			changed = true
		}
	}
	p.pmu.Lock()
	page := p.page
	p.pmu.Unlock()
	name := pageName(page)
	if page == "" {
		name = pageLabel(p.id)
	}
	if name != p.pageSet {
		changed = true
		p.pageSet = name
		p.write("P " + name)
	}
	if changed {
		p.write("F")
	}
}

// pageLabel is the page's name for the pad's side panel, e.g. "Streaming".
func pageLabel(device string) string {
	name := selectedProfile(device)
	if i := strings.LastIndex(name, "/"); i >= 0 {
		name = name[i+1:]
	}
	return cleanLabel(name)
}

func kickLabels() {
	select {
	case kick <- struct{}{}:
	default:
	}
}

// toRGB565 decodes a data URL picture and scales it to size x size (area average),
// returning little-endian RGB565 pixels.
func toRGB565(dataURL string, size int) []byte {
	comma := strings.IndexByte(dataURL, ',')
	if comma < 0 {
		return nil
	}
	raw, err := base64.StdEncoding.DecodeString(dataURL[comma+1:])
	if err != nil {
		return nil
	}
	src, _, err := image.Decode(bytes.NewReader(raw))
	if err != nil {
		return nil
	}
	sb := src.Bounds()
	sw, sh := sb.Dx(), sb.Dy()
	out := make([]byte, size*size*2)
	for y := 0; y < size; y++ {
		y0, y1 := y*sh/size, (y+1)*sh/size
		if y1 <= y0 {
			y1 = y0 + 1
		}
		for x := 0; x < size; x++ {
			x0, x1 := x*sw/size, (x+1)*sw/size
			if x1 <= x0 {
				x1 = x0 + 1
			}
			var r, g, bl, n uint32
			for yy := y0; yy < y1; yy++ {
				for xx := x0; xx < x1; xx++ {
					// see-through parts show the pad's box colour (like the app), not black
					cr, cg, cb, ca := src.At(sb.Min.X+xx, sb.Min.Y+yy).RGBA()
					t := 255 - ca>>8
					r += cr>>8 + t*24/255
					g += cg>>8 + t*28/255
					bl += cb>>8 + t*24/255
					n++
				}
			}
			r, g, bl = r/n, g/n, bl/n
			c := uint16((r>>3)<<11 | (g>>2)<<5 | bl>>3)
			out[(y*size+x)*2] = byte(c)
			out[(y*size+x)*2+1] = byte(c >> 8)
		}
	}
	return out
}
