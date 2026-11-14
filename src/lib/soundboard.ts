// Orvo Soundboard: shared state for the Soundboard tab, the tray's Soundboard cubes and pad keys.
import type { Action } from "./Action";
import type { ActionInstance } from "./ActionInstance";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type Writable, writable } from "svelte/store";

export const SOUND_ACTION = "com.ninedeck.device.sound";

export type Sound = { id: string; name: string; file: string; volume: number; mode: "overlap" | "restart" | "loop"; duration_ms: number };
export type SoundboardConfig = {
	enabled: boolean;
	mic_device: string | null;
	mic_on: boolean;
	mic_gain: number;
	sound_gain: number;
	monitor_device: string | null;
	monitor_on: boolean;
	monitor_gain: number;
	mic_preset: MicPreset;
	mic_fx: MicFx;
};

export type MicPreset = "off" | "natural" | "isolation" | "studio" | "custom";
export type MicFx = {
	rumble: boolean;
	denoise: number;
	breath: number;
	gate: boolean;
	gate_db: number;
	deess: number;
	warmth: number;
	clarity: number;
	comp: number;
	limiter: boolean;
};
/** Voice cleanup presets (Custom keeps whatever you set). */
export const MIC_PRESETS: { [k in Exclude<MicPreset, "custom">]: { label: string; hint: string; fx: MicFx } } = {
	off: {
		label: "Off",
		hint: "Your raw mic, untouched.",
		fx: { rumble: false, denoise: 0, breath: 0, gate: false, gate_db: -55, deess: 0, warmth: 0, clarity: 0, comp: 0, limiter: false },
	},
	natural: {
		label: "Natural",
		hint: "Light cleanup. Sounds like you, just cleaner.",
		fx: { rumble: true, denoise: 0.6, breath: 0.2, gate: false, gate_db: -60, deess: 0.2, warmth: 0, clarity: 0.1, comp: 0.2, limiter: true },
	},
	isolation: {
		label: "Voice isolation",
		hint: "Only your voice gets through. Can sound a bit processed.",
		fx: { rumble: true, denoise: 1, breath: 0.85, gate: true, gate_db: -50, deess: 0.3, warmth: 0, clarity: 0.2, comp: 0.35, limiter: true },
	},
	studio: {
		label: "Studio",
		hint: "Broadcast sound: clean, warm, even and clear.",
		fx: { rumble: true, denoise: 0.9, breath: 0.6, gate: true, gate_db: -55, deess: 0.5, warmth: 0.4, clarity: 0.5, comp: 0.6, limiter: true },
	},
};
export type SoundboardState = {
	sounds: Sound[];
	config: SoundboardConfig;
	inputs: string[];
	outputs: string[];
	status: { running: boolean; cable: boolean; cable_name: string | null; mic_name: string | null; monitor_name: string | null; error: string | null };
	playing: string[];
	cable_installed: boolean;
	cable_url: string;
};

/** Which page the main window shows: the pad's keys or the soundboard. */
export const appView: Writable<"pad" | "soundboard"> = writable("pad");
export const soundboard: Writable<SoundboardState | null> = writable(null);

let pending: Promise<void> | null = null;
export function refreshSoundboard(): Promise<void> {
	pending ??= invoke<SoundboardState>("sb_state")
		.then((s) => soundboard.set(s))
		.catch(() => {})
		.finally(() => (pending = null));
	return pending;
}
refreshSoundboard();
listen("soundboard_changed", () => refreshSoundboard());
// A pad slider moved a soundboard level: update just those numbers.
listen("soundboard_levels", ({ payload }: { payload: { mic_gain: number; sound_gain: number } }) =>
	soundboard.update((s) => (s ? { ...s, config: { ...s.config, ...payload } } : s)),
);

export const SB_SLIDERS = ["com.ninedeck.device.sbmic", "com.ninedeck.device.sbsound"];

/** A tray cube for one sound: the hidden "Play sound" action carrying which sound it plays. */
export type SoundAction = Action & { orvoSound?: { id: string; name: string } };

/** After a sound cube lands on a key: remember the sound and show its name on the key. */
export async function finishSoundKey(instance: ActionInstance | null, action: SoundAction): Promise<ActionInstance | null> {
	if (!instance || !action.orvoSound) return instance;
	const { id, name } = action.orvoSound;
	try {
		const updated = await invoke<ActionInstance>("orvo_set_instance_settings", { context: instance.context, settings: { sound: id } });
		const state = { ...updated.states[0], text: name, show: true };
		await invoke("set_state", { context: instance.context, index: 0, state });
		updated.states[0] = state;
		return updated;
	} catch {
		return instance;
	}
}

/** The plain action (without Orvo's extra field) for the backend. */
export function plainAction(action: SoundAction): Action {
	const { orvoSound: _, ...rest } = action;
	return rest;
}
