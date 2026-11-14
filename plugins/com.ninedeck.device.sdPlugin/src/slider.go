package main

// The slider: the pad reports its position ("S1 0-1000"). If one of NineDeck's own slider
// actions (master / app / mic volume) is on it, the plugin sets that level directly, so the
// slider position and the volume always match. Any other plugin's dial action gets the movement
// as dial turns through OpenDeck.

import (
	"encoding/json"
	"fmt"
	"strings"
	"sync"
	"time"
)

const (
	actMaster = "com.ninedeck.device.mastervolume"
	actApp    = "com.ninedeck.device.appvolume"
	actMic    = "com.ninedeck.device.miclevel"
	// Orvo Soundboard levels: Orvo Studio owns these, the plugin just passes the slider on.
	actSbMic   = "com.ninedeck.device.sbmic"
	actSbSound = "com.ninedeck.device.sbsound"
)

// Soundboard levels as Orvo Studio last reported them (0-1, 1 = 100 %).
var (
	sbMu     sync.Mutex
	sbLevels = map[string]float32{"mic": 1, "sound": 1}
)

func sbWhich(action string) string {
	if action == actSbMic {
		return "mic"
	}
	return "sound"
}

func setSoundboardLevels(mic, sound *float64) {
	sbMu.Lock()
	defer sbMu.Unlock()
	if mic != nil {
		sbLevels["mic"] = float32(*mic)
	}
	if sound != nil {
		sbLevels["sound"] = float32(*sound)
	}
}

type sliderAction struct {
	action   string
	context  string
	settings map[string]any
}

var (
	slMu       sync.Mutex
	slActions  = map[string]*sliderAction{} // device -> our action on its slider
	slLast     = map[string]int{}           // device -> last position (0-1000)
	slNameSent = map[string]string{}
	slKey      = map[string]string{} // device -> what the slider controlled last ("action|app")
	slPickup   = map[string]int{}    // device -> level (0-1000) the slider must reach to take over
)

// audioDev is one set of speakers or one microphone, as offered in the action's settings.
type audioDev struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

// devOf: the speakers / mic chosen for a master volume or mic level action ("" = Windows default).
func devOf(a *sliderAction) string {
	if a.action == actApp {
		return ""
	}
	dev, _ := a.settings["device"].(string)
	return dev
}

func targetKey(a *sliderAction) string {
	app, _ := a.settings["app"].(string)
	if dev := devOf(a); dev != "" {
		return a.action + "|" + appKey(app) + "|" + dev
	}
	return a.action + "|" + appKey(app)
}

func levelOf(a *sliderAction) (float32, bool) {
	switch a.action {
	case actMaster:
		return getVolume(volMaster, "", devOf(a))
	case actMic:
		return getVolume(volMic, "", devOf(a))
	case actApp:
		app, _ := a.settings["app"].(string)
		return getVolume(volApp, app, "")
	case actSbMic, actSbSound:
		sbMu.Lock()
		defer sbMu.Unlock()
		v := sbLevels[sbWhich(a.action)]
		if v > 1 {
			v = 1
		}
		return v, true
	}
	return 0, false
}

// startPickup: the slider now controls something else (another page, or a new action). Instead of
// making the level jump to the slider, the slider only takes over once it passes the current level.
// The pad shows that level as an orange mark on its bar.
func startPickup(p *pad, a *sliderAction) {
	lvl, ok := levelOf(a)
	slMu.Lock()
	pos, have := slLast[p.id]
	if !ok || !have {
		delete(slPickup, p.id)
		slMu.Unlock()
		p.write("G-1")
		return
	}
	target := int(lvl*1000 + 0.5)
	if d := target - pos; d >= -15 && d <= 15 {
		delete(slPickup, p.id)
		slMu.Unlock()
		p.write("G-1")
		return
	}
	slPickup[p.id] = target
	slMu.Unlock()
	p.write(fmt.Sprintf("G%d", target))
}

// Firmware 0.6+ has the slider.
func sliderCount(p *pad) int {
	if p.box > 0 && !newer("0.6", p.fw) {
		return 1
	}
	return 0
}

func sliderName(a *sliderAction) string {
	if a == nil {
		return "Slider"
	}
	switch a.action {
	case actMaster:
		return "Master volume"
	case actMic:
		return "Mic level"
	case actSbMic:
		return "Voice volume"
	case actSbSound:
		return "Sounds volume"
	case actApp:
		if app, _ := a.settings["app"].(string); app != "" {
			return cleanLabel(strings.TrimSuffix(strings.ToLower(app), ".exe")) + " volume"
		}
		return "App volume"
	}
	return "Slider"
}

func slActionFor(device string) *sliderAction {
	slMu.Lock()
	defer slMu.Unlock()
	return slActions[device]
}

// Icon the pad draws at the bottom of its level bar: 1 speakers, 2 mic, 3 an app.
func sliderIconOf(a *sliderAction) int {
	if a == nil {
		return 0
	}
	switch a.action {
	case actMaster, actSbSound:
		return 1
	case actMic, actSbMic:
		return 2
	case actApp:
		return 3
	}
	return 0
}

// The level each thing had before the slider first changed it, so removing the slider action
// puts it back (an app turned down doesn't stay quiet forever).
var origLevel = map[string]float32{} // targetKey -> level

func rememberOriginal(a *sliderAction) {
	key := targetKey(a)
	slMu.Lock()
	_, have := origLevel[key]
	slMu.Unlock()
	if have {
		return
	}
	if lvl, ok := levelOf(a); ok {
		slMu.Lock()
		if _, have := origLevel[key]; !have {
			origLevel[key] = lvl
		}
		slMu.Unlock()
	}
}

// actionRemoved: the user deleted a slider action (not just a page change): restore the level.
func actionRemoved(device, action string, settings map[string]any) {
	a := &sliderAction{action: action, settings: settings}
	key := targetKey(a)
	slMu.Lock()
	lvl, have := origLevel[key]
	delete(origLevel, key)
	if slKey[device] == key {
		delete(slKey, device)
	}
	slMu.Unlock()
	if have {
		applyLevel(a, lvl)
	}
}

func updateSliderName(device string) {
	p := padByID(device)
	if p == nil || sliderCount(p) == 0 {
		return
	}
	slMu.Lock()
	name := sliderName(slActions[device])
	same := slNameSent[device] == name
	slNameSent[device] = name
	slMu.Unlock()
	if !same {
		p.write("N1 " + name)
		if !newer("0.9.1", p.fw) {
			p.write(fmt.Sprintf("Y%d", sliderIconOf(slActionFor(device))))
		}
	}
}

// The app's slider bar follows the real one: send the newest position, at most ~30 times a second.
var (
	uiMu    sync.Mutex
	uiLast  = map[string]time.Time{}
	uiTimer = map[string]*time.Timer{}
	uiValue = map[string]int{}
)

func showInApp(device string, v int) {
	uiMu.Lock()
	defer uiMu.Unlock()
	uiValue[device] = v
	sendNow := func() {
		uiMu.Lock()
		val := uiValue[device]
		uiLast[device] = time.Now()
		uiTimer[device] = nil
		uiMu.Unlock()
		send(map[string]any{"event": "orvoSlider", "payload": map[string]any{"device": device, "value": val}})
	}
	if wait := 33*time.Millisecond - time.Since(uiLast[device]); wait > 0 {
		if uiTimer[device] == nil {
			uiTimer[device] = time.AfterFunc(wait, sendNow)
		}
		return
	}
	uiLast[device] = time.Now()
	go send(map[string]any{"event": "orvoSlider", "payload": map[string]any{"device": device, "value": v}})
}

func sliderMoved(p *pad, v int) {
	showInApp(p.id, v)
	slMu.Lock()
	last, had := slLast[p.id]
	slLast[p.id] = v
	a := slActions[p.id]
	target, picking := slPickup[p.id]
	if picking && had && ((last-target)*(v-target) <= 0 || abs(v-target) <= 15) {
		delete(slPickup, p.id) // passed the current level: take over
		picking = false
		go p.write("G-1")
	}
	slMu.Unlock()
	if picking {
		return
	}
	if a != nil {
		rememberOriginal(a)
		applyLevel(a, float32(v)/1000)
		return
	}
	if a == nil {
		slMu.Lock()
		delete(slKey, p.id) // moved with nothing on it: the next action picks up again
		slMu.Unlock()
	}
	// someone else's dial action: send the movement as dial turns (1 turn = 1 %)
	if had {
		if ticks := v/10 - last/10; ticks != 0 {
			send(map[string]any{"event": "encoderChange", "payload": map[string]any{"device": p.id, "position": 0, "ticks": ticks}})
		}
	}
}

func applyLevel(a *sliderAction, level float32) {
	switch a.action {
	case actMaster:
		setVolume(volMaster, "", devOf(a), level)
	case actMic:
		setVolume(volMic, "", devOf(a), level)
	case actApp:
		app, _ := a.settings["app"].(string)
		setVolume(volApp, app, "", level)
	case actSbMic, actSbSound:
		which := sbWhich(a.action)
		sbMu.Lock()
		sbLevels[which] = level
		sbMu.Unlock()
		send(map[string]any{"event": "orvoSoundboardLevel", "payload": map[string]any{"which": which, "value": level}})
	}
}

// handleActionEvent tracks NineDeck's own slider actions as OpenDeck places and removes them.
func handleActionEvent(msg []byte) {
	var ev struct {
		Event   string `json:"event"`
		Action  string `json:"action"`
		Context string `json:"context"`
		Device  string `json:"device"`
		Payload struct {
			Settings   map[string]any `json:"settings"`
			Controller string         `json:"controller"`
		} `json:"payload"`
	}
	if json.Unmarshal(msg, &ev) != nil || !strings.HasPrefix(ev.Action, "com.ninedeck.device.") {
		return
	}
	// Soundboard keys are played by Orvo Studio itself; they are not slider actions.
	if ev.Action == "com.ninedeck.device.sound" {
		return
	}
	switch ev.Event {
	case "willAppear", "didReceiveSettings":
		a := &sliderAction{action: ev.Action, context: ev.Context, settings: ev.Payload.Settings}
		key := targetKey(a)
		slMu.Lock()
		slActions[ev.Device] = a
		changed := slKey[ev.Device] != key
		slKey[ev.Device] = key
		slMu.Unlock()
		updateSliderName(ev.Device)
		if changed && ev.Payload.Controller == "Encoder" {
			if p := padByID(ev.Device); p != nil && sliderCount(p) > 0 {
				go startPickup(p, a)
			}
		}
		send(map[string]any{"event": "setTitle", "context": ev.Context, "payload": map[string]any{"title": ""}}) // no words unless the user adds them
	case "orvoActionRemoved":
		go actionRemoved(ev.Device, ev.Action, ev.Payload.Settings)
		return
	case "propertyInspectorDidAppear", "sendToPlugin":
		if ev.Action == actApp {
			go func() {
				send(map[string]any{"event": "sendToPropertyInspector", "action": ev.Action, "context": ev.Context,
					"payload": map[string]any{"apps": listApps()}})
			}()
		} else if ev.Action == actMaster || ev.Action == actMic {
			go func() {
				devices := listAudioDevices(ev.Action == actMic)
				if devices == nil {
					devices = []audioDev{}
				}
				send(map[string]any{"event": "sendToPropertyInspector", "action": ev.Action, "context": ev.Context,
					"payload": map[string]any{"devices": devices}})
			}()
		}
		return
	case "willDisappear":
		slMu.Lock()
		if cur := slActions[ev.Device]; cur != nil && cur.context == ev.Context {
			delete(slActions, ev.Device)
		}
		slMu.Unlock()
		updateSliderName(ev.Device)
	}
	_ = fmt.Sprint
}

func abs(x int) int {
	if x < 0 {
		return -x
	}
	return x
}
