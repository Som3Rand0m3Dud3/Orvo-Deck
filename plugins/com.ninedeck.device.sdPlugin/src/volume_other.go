//go:build !windows

package main

const (
	volMaster = iota
	volApp
	volMic
)

// Volume control is Windows-only for now.
func setVolume(kind int, app string, level float32) {}
func getVolume(kind int, app string) (float32, bool) { return 0, false }
func listApps() []string                              { return nil }
