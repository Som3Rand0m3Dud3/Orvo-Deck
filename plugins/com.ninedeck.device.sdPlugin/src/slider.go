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
)

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
)

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
	case actApp:
		if app, _ := a.settings["app"].(string); app != "" {
			return cleanLabel(strings.TrimSuffix(strings.ToLower(app), ".exe")) + " volume"
		}
		return "App volume"
	}
	return "Slider"
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
	slMu.Unlock()
	if a != nil {
		applyLevel(a, float32(v)/1000)
		return
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
		setVolume(volMaster, "", level)
	case actMic:
		setVolume(volMic, "", level)
	case actApp:
		app, _ := a.settings["app"].(string)
		setVolume(volApp, app, level)
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
	switch ev.Event {
	case "willAppear", "didReceiveSettings":
		a := &sliderAction{action: ev.Action, context: ev.Context, settings: ev.Payload.Settings}
		slMu.Lock()
		slActions[ev.Device] = a
		pos, ok := slLast[ev.Device]
		slMu.Unlock()
		updateSliderName(ev.Device)
		if ok {
			applyLevel(a, float32(pos)/1000) // match the level to where the slider is now
		}
		send(map[string]any{"event": "setTitle", "context": ev.Context, "payload": map[string]any{"title": sliderName(a)}})
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
