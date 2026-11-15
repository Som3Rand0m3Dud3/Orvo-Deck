// Orvo Soundboard: shared state for the Soundboard tab, the tray's Soundboard cubes and pad keys.
import type { Action } from "./Action";
import type { ActionInstance } from "./ActionInstance";

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { type Writable, get, writable } from "svelte/store";
import { actionLooks, applyLook, lookKey } from "./actionLooks";

export const SOUND_ACTION = "com.ninedeck.device.sound";

export type Sound = {
	id: string;
	name: string;
	file: string;
	volume: number;
	mode: "overlap" | "restart" | "loop";
	duration_ms: number;
	start_ms: number;
	end_ms: number | null;
	fade_out: boolean;
	effects: SoundFx;
};
export type SoundFx = {
	speed: number;
	pitch: number;
	bass_db: number;
	reverb: number;
	echo: number;
	distortion: number;
	reverse: boolean;
	normalize: boolean;
	fade_in: boolean;
};
export const NEUTRAL_SOUND_FX: SoundFx = {
	speed: 1,
	pitch: 0,
	bass_db: 0,
	reverb: 0,
	echo: 0,
	distortion: 0,
	reverse: false,
	normalize: false,
	fade_in: false,
};
export const hasSoundFx = (s: Sound) => !!s.effects && JSON.stringify({ ...NEUTRAL_SOUND_FX, ...s.effects }) != JSON.stringify(NEUTRAL_SOUND_FX);
/** How long a sound plays after trimming. */
export const playedMs = (s: Sound) =>
	Math.max(0, Math.min(s.end_ms ?? s.duration_ms, s.duration_ms) - (s.start_ms ?? 0)) / Math.max(0.25, s.effects?.speed || 1);
export type SoundboardConfig = {
	enabled: boolean;
	mic_device: string | null;
	mic_on: boolean;
	mic_gain: number;
	sound_gain: number;
	monitor_device: string | null;
	monitor_on: boolean;
	monitor_gain: number;
	master_gain: number;
	mic_preset: MicPreset;
	mic_fx: MicFx;
	out_gain: number;
	out_mute: boolean;
	sound_mute: boolean;
	voice_preset: string;
	voice_fx: EffectSpec[];
	sound_fx: EffectSpec[];
	out_fx: EffectSpec[];
	my_voices: { id: string; name: string; chain: EffectSpec[] }[];
	voice_tweaks: { [voice: string]: VoiceTweak };
	voices_v2: boolean;
	ai_voices: AiVoice[];
};
export type AiVoice = { id: string; name: string; file: string; sr: number; window: number; pitch: number; quality: string; image?: string | null };
export type AiStatus = { state: string; message: string | null; gpu: boolean; compute_ms: number; block_ms: number; delay_ms: number };
export type AiState = { supported: boolean; installed: boolean; installing: boolean; download_mb: number; status: AiStatus };
export type VoiceTweak = { pitch: number; amount: number };

// ---------------- Mixer effects
export type EffectSpec = { id: string; kind: string; on: boolean; params: { [key: string]: number } };
type Param = { key: string; label: string; min: number; max: number; step: number; def: number; unit: "%" | "dB" | "ms" | "Hz" | "st" | "x" | "toggle" };
export const EFFECTS: { [kind: string]: { label: string; desc: string; params: Param[] } } = {
	pitch: {
		label: "Pitch",
		desc: "Deeper or higher voice (adds ~30 ms)",
		params: [
			{ key: "semitones", label: "Pitch", min: -12, max: 12, step: 0.5, def: 0, unit: "st" },
			{ key: "formant", label: "Formant · bigger / smaller", min: -12, max: 12, step: 0.5, def: 0, unit: "st" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 1, unit: "%" },
		],
	},
	echo: {
		label: "Echo",
		desc: "Repeats, from slap-back to canyon",
		params: [
			{ key: "time_ms", label: "Time", min: 20, max: 1000, step: 5, def: 250, unit: "ms" },
			{ key: "feedback", label: "Repeats", min: 0, max: 0.9, step: 0.01, def: 0.35, unit: "%" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 0.3, unit: "%" },
		],
	},
	reverb: {
		label: "Reverb",
		desc: "Room, hall, cave, stadium",
		params: [
			{ key: "size", label: "Room size", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "damping", label: "Damping · dark", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 0.25, unit: "%" },
		],
	},
	eq: {
		label: "EQ",
		desc: "Bass, mid, treble",
		params: [
			{ key: "bass_db", label: "Bass", min: -12, max: 12, step: 0.5, def: 0, unit: "dB" },
			{ key: "mid_db", label: "Mid", min: -12, max: 12, step: 0.5, def: 0, unit: "dB" },
			{ key: "treble_db", label: "Treble", min: -12, max: 12, step: 0.5, def: 0, unit: "dB" },
		],
	},
	filter: {
		label: "Filter",
		desc: "Low-cut / high-cut: radio, phone, underwater",
		params: [
			{ key: "lowcut_hz", label: "Low-cut", min: 20, max: 2000, step: 10, def: 20, unit: "Hz" },
			{ key: "highcut_hz", label: "High-cut", min: 500, max: 20000, step: 100, def: 20000, unit: "Hz" },
		],
	},
	distortion: {
		label: "Distortion",
		desc: "Grit, megaphone, demon",
		params: [
			{ key: "drive", label: "Drive", min: 0, max: 1, step: 0.01, def: 0.4, unit: "%" },
			{ key: "tone", label: "Tone · bright", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 1, unit: "%" },
		],
	},
	robot: {
		label: "Robot",
		desc: "Ring modulator: metallic voice",
		params: [
			{ key: "freq_hz", label: "Frequency · lower = buzzier", min: 10, max: 400, step: 1, def: 60, unit: "Hz" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 0.8, unit: "%" },
		],
	},
	chorus: {
		label: "Chorus / Flanger",
		desc: "Wide, wobbly, alien",
		params: [
			{ key: "rate_hz", label: "Speed", min: 0.1, max: 5, step: 0.05, def: 0.8, unit: "Hz" },
			{ key: "depth", label: "Depth", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "flanger", label: "Flanger", min: 0, max: 1, step: 1, def: 0, unit: "toggle" },
		],
	},
	compressor: {
		label: "Compressor",
		desc: "Evens out loud and quiet",
		params: [{ key: "amount", label: "Amount", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" }],
	},
	limiter: {
		label: "Limiter",
		desc: "Never louder than the ceiling",
		params: [{ key: "ceiling_db", label: "Ceiling", min: -12, max: 0, step: 0.5, def: -1, unit: "dB" }],
	},
	whisper: {
		label: "Whisper",
		desc: "Keeps the words, removes the voice's tone",
		params: [{ key: "mix", label: "Mix", min: 0, max: 1, step: 0.01, def: 1, unit: "%" }],
	},
	leveler: {
		label: "Loudness leveler",
		desc: "Quiet and loud material come out even",
		params: [
			{ key: "target", label: "Target loudness", min: 0, max: 1, step: 0.01, def: 0.5, unit: "%" },
			{ key: "speed", label: "Speed", min: 0, max: 1, step: 0.01, def: 0.3, unit: "%" },
		],
	},
};
export const paramValue = (e: EffectSpec, p: Param) => e.params?.[p.key] ?? p.def;
export function formatParam(p: Param, v: number): string {
	switch (p.unit) {
		case "%":
			return `${Math.round(v * 100)}%`;
		case "dB":
			return `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;
		case "st":
			return `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v)} st`;
		case "Hz":
			return v >= 1000 ? `${(v / 1000).toFixed(1)} kHz` : `${v < 10 ? v.toFixed(1) : Math.round(v)} Hz`;
		case "ms":
			return `${Math.round(v)} ms`;
		default:
			return String(v);
	}
}
export function newEffect(kind: string): EffectSpec {
	return {
		id: `${kind}-${Math.random().toString(36).slice(2, 7)}`,
		kind,
		on: true,
		params: Object.fromEntries(EFFECTS[kind].params.map((p) => [p.key, p.def])),
	};
}
export type VoicePreset = { id: string; name: string; hint: string; chain: EffectSpec[] };

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
	voice_presets: VoicePreset[];
};

/** Which page the main window shows: the pad's keys or the soundboard. */
export const appView: Writable<"pad" | "soundboard" | "mixer" | "voices"> = writable("pad");
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
listen("soundboard_levels", ({ payload }: { payload: Partial<SoundboardConfig> }) =>
	soundboard.update((s) => (s ? { ...s, config: { ...s.config, ...payload } } : s)),
);

export const SB_SLIDERS = ["com.ninedeck.device.sbmic", "com.ninedeck.device.sbsound"];

/** A tray cube that carries its key's settings: a sound to play, a voice to switch to… */
export type SoundAction = Action & { orvoSound?: { id: string; name: string }; orvoPreset?: { settings: any; title: string } };

/** The picture for a voice: Orvo's icon, or a big first letter for your own voices. */
export function voiceIcon(id: string, name = "", image?: string | null): string {
	if (image) return image;
	if (!id.startsWith("my:") && !id.startsWith("ai:")) return `opendeck/voices/${id}.png`;
	const letter = (name.trim()[0] ?? "?").toUpperCase().replace(/[<&>"]/g, "");
	// AI voices get a small "AI" corner tag
	const tag = id.startsWith("ai:")
		? `<rect x="88" y="10" width="46" height="28" rx="8" fill="#4F8CFF"/><text x="111" y="31" font-family="Segoe UI,Arial,sans-serif" font-size="20" font-weight="700" text-anchor="middle" fill="#fff">AI</text>`
		: "";
	const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 144 144"><rect width="144" height="144" rx="22" fill="#1C1F26"/><text x="72" y="104" font-family="Segoe UI,Arial,sans-serif" font-size="84" font-weight="700" text-anchor="middle" fill="${id.startsWith("ai:") ? "#F2F4F8" : "#4F8CFF"}">${letter}</text>${tag}</svg>`;
	return "data:image/svg+xml," + encodeURIComponent(svg);
}

/** After a cube lands on a key: its settings (sound / voice), then its picture and name. */
export async function finishSoundKey(instance: ActionInstance | null, action: SoundAction): Promise<ActionInstance | null> {
	if (!instance) return instance;
	const look = get(actionLooks)[lookKey(action)];
	let updated = instance;
	if (action.orvoSound || action.orvoPreset) {
		const settings = action.orvoSound ? { sound: action.orvoSound.id } : action.orvoPreset!.settings;
		try {
			updated = await invoke<ActionInstance>("orvo_set_instance_settings", { context: instance.context, settings });
		} catch {
			return instance;
		}
	}
	// voices show a picture (no words); sounds show their name
	const preset = action.orvoPreset?.settings?.preset as string | undefined;
	const aiImage = preset?.startsWith("ai:") ? get(soundboard)?.config.ai_voices?.find((v) => `ai:${v.id}` == preset)?.image : undefined;
	const base = preset ? { image: voiceIcon(preset, action.orvoPreset!.title, aiImage), name: "" } : {};
	const title = action.orvoSound?.name;
	if (!look && !preset && title == undefined) return updated;
	return applyLook(updated, { ...base, ...look }, title);
}

/** The plain action (without Orvo's extra field) for the backend. */
export function plainAction(action: SoundAction): Action {
	const { orvoSound: _, orvoPreset: __, ...rest } = action;
	return rest;
}

/** Saves soundboard / mixer settings (debounced), keeping the page in step right away. */
let saveTimer: ReturnType<typeof setTimeout> | null = null;
export function updateConfig(patch: Partial<SoundboardConfig>) {
	soundboard.update((s) => {
		if (!s) return s;
		const config = { ...s.config, ...patch };
		if (saveTimer) clearTimeout(saveTimer);
		saveTimer = setTimeout(() => invoke("sb_set_config", { config }), 120);
		return { ...s, config };
	});
}
export const MIXER_ACTIONS = ["com.ninedeck.device.voice", "com.ninedeck.device.voiceoff", "com.ninedeck.device.voicenext", "com.ninedeck.device.voicemute"];
