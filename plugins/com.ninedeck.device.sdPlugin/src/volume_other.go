//go:build !windows

package main

const (
	volMaster = iota
	volApp
	volMic
)

func setVolume(kind int, app string, level float32) {} // volume control is Windows-only for now
