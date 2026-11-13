// NineDeck device plugin for OpenDeck (OpenAction API).
// Finds NineDeck pads on USB serial, registers each one with OpenDeck as a 3x4 deck,
// forwards key presses, and shows the name of each key's assigned action on the pad's screen.
package main

import (
	"bufio"
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
		var ev struct {
			Event string `json:"event"`
		}
		json.Unmarshal(msg, &ev)
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
	for time.Now().Before(deadline) {
		line, _ := rd.ReadString('\n')
		if strings.HasPrefix(strings.TrimSpace(line), "NINEDECK") {
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
	p := &pad{id: namespace + "-" + serialNo, port: portName, sp: sp}
	padsMu.Lock()
	pads[portName] = p
	padsMu.Unlock()

	migrateLayout(p.id)
	p.write("H 0") // OpenDeck handles the keys now, stop the F13-F23 presses
	send(map[string]any{
		"event": "registerDevice",
		"payload": map[string]any{
			"id": p.id, "name": "NineDeck", "rows": rows, "columns": cols, "encoders": 0, "type": 0,
		},
	})
	log.Println("registered", p.id, "on", portName)
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
		// "K<n> 1" / "K<n> 0"
		if len(line) >= 4 && line[0] == 'K' {
			parts := strings.Fields(line[1:])
			if len(parts) != 2 {
				continue
			}
			k, _ := strconv.Atoi(parts[0])
			if (k == 10 || k == 11) && parts[1] == "1" {
				flipPage(p.id, map[int]int{10: -1, 11: 1}[k])
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
	t := time.NewTicker(1500 * time.Millisecond)
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

func flipPage(device string, step int) {
	pages := listPages(device)
	if len(pages) < 2 {
		return
	}
	cur := selectedProfile(device)
	i := 0
	for n, p := range pages {
		if p == cur {
			i = n
		}
	}
	next := pages[(i+step+len(pages))%len(pages)]
	send(map[string]any{"event": "switchProfile", "device": device, "profile": next})
	go func() {
		time.Sleep(400 * time.Millisecond)
		select {
		case kick <- struct{}{}:
		default:
		}
	}()
}

// migrateLayout moves profiles saved with the old 3x4 layout (page keys in a 4th column)
// to the 3x3 layout, so keys 4-9 keep their actions.
func migrateLayout(device string) {
	root := filepath.Join(configDir(), "profiles", device)
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
		if !ok || len(keys) != 12 {
			return nil
		}
		nk := make([]any, 9)
		for r := 0; r < 3; r++ {
			for c := 0; c < 3; c++ {
				oldPos, newPos := r*4+c, r*3+c
				inst := keys[oldPos]
				renumber(inst, oldPos, newPos)
				nk[newPos] = inst
			}
		}
		prof["keys"] = nk
		if out, err := json.Marshal(prof); err == nil {
			os.WriteFile(path+".bak-3x4", b, 0o644)
			os.WriteFile(path, out, 0o644)
			log.Println("migrated", path, "to 3x3")
		}
		return nil
	})
}

// renumber rewrites "<device>.<profile>.Keypad.<old>.<n>" contexts inside an instance.
func renumber(v any, oldPos, newPos int) {
	switch t := v.(type) {
	case map[string]any:
		for k, x := range t {
			if s, ok := x.(string); ok && k == "context" {
				t[k] = strings.Replace(s, fmt.Sprintf(".Keypad.%d.", oldPos), fmt.Sprintf(".Keypad.%d.", newPos), 1)
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
