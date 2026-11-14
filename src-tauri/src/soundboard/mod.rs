//! Orvo Soundboard: sounds you drop in, played into a virtual mic (VB-Cable) together with
//! your real mic, and into your headphones so you hear them too.
//!
//! Files: <config>/soundboard/sounds.json, settings.json and files/<id>.<ext>.

pub mod decode;
pub mod engine;
pub mod fx;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, command};

use engine::{Clip, Engine, EngineStatus, Routing, VoiceCmd, store};

use crate::events::frontend::Error;

/// The pad action that plays a sound ("sound" setting = sound id).
pub const SOUND_ACTION: &str = "com.ninedeck.device.sound";
pub const CABLE_URL: &str = "https://vb-audio.com/Cable/";
const EXTENSIONS: [&str; 7] = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "oga"];

#[derive(Clone, Serialize, Deserialize)]
pub struct Sound {
	pub id: String,
	pub name: String,
	pub file: String,
	#[serde(default = "one")]
	pub volume: f32,
	/// "overlap" (play on top), "restart" (start again), "loop" (press again to stop)
	#[serde(default = "overlap")]
	pub mode: String,
	#[serde(default)]
	pub duration_ms: u64,
}
fn one() -> f32 {
	1.0
}
fn overlap() -> String {
	"overlap".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
	#[serde(default = "yes")]
	pub enabled: bool,
	#[serde(default)]
	pub mic_device: Option<String>,
	#[serde(default = "yes")]
	pub mic_on: bool,
	#[serde(default = "one")]
	pub mic_gain: f32,
	#[serde(default = "one")]
	pub sound_gain: f32,
	#[serde(default)]
	pub monitor_device: Option<String>,
	#[serde(default = "yes")]
	pub monitor_on: bool,
	#[serde(default = "half")]
	pub monitor_gain: f32,
	/// "off", "natural", "isolation", "studio" or "custom" (the app keeps the preset values).
	#[serde(default = "studio")]
	pub mic_preset: String,
	#[serde(default)]
	pub mic_fx: fx::MicFx,
}
fn studio() -> String {
	"studio".into()
}
fn yes() -> bool {
	true
}
fn half() -> f32 {
	0.6
}
impl Default for Config {
	fn default() -> Self {
		serde_json::from_str("{}").unwrap()
	}
}

struct State {
	sounds: Vec<Sound>,
	config: Config,
	clips: HashMap<String, Clip>,
	engine: Option<Engine>,
	loaded: bool,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
	Mutex::new(State {
		sounds: Vec::new(),
		config: Config::default(),
		clips: HashMap::new(),
		engine: None,
		loaded: false,
	})
});

fn dir() -> PathBuf {
	crate::shared::config_dir().join("soundboard")
}
fn files_dir() -> PathBuf {
	dir().join("files")
}

fn load(st: &mut State) {
	if st.loaded {
		return;
	}
	st.loaded = true;
	if let Ok(text) = std::fs::read_to_string(dir().join("sounds.json")) {
		st.sounds = serde_json::from_str(&text).unwrap_or_default();
	}
	if let Ok(text) = std::fs::read_to_string(dir().join("settings.json")) {
		st.config = serde_json::from_str(&text).unwrap_or_default();
	}
}

fn save(st: &State) {
	let _ = std::fs::create_dir_all(dir());
	let write = |name: &str, text: String| {
		let tmp = dir().join(format!("{name}.temp"));
		if std::fs::write(&tmp, text).is_ok() {
			let _ = std::fs::rename(&tmp, dir().join(name));
		}
	};
	write("sounds.json", serde_json::to_string_pretty(&st.sounds).unwrap_or_default());
	write("settings.json", serde_json::to_string_pretty(&st.config).unwrap_or_default());
}

fn apply_levels(st: &State) {
	if let Some(engine) = &st.engine {
		let c = &st.config;
		let l = &engine.levels;
		l.mic_on.store(c.mic_on, std::sync::atomic::Ordering::Relaxed);
		l.monitor_on.store(c.monitor_on, std::sync::atomic::Ordering::Relaxed);
		store(&l.mic_gain, c.mic_gain.clamp(0.0, 2.0));
		store(&l.sound_gain, c.sound_gain.clamp(0.0, 2.0));
		store(&l.monitor_gain, c.monitor_gain.clamp(0.0, 2.0));
		if let Ok(mut fx) = engine.fx.lock() {
			*fx = c.mic_fx;
		}
	}
}

/// Tells the pad plugin the soundboard levels, so its sliders pick up from the right place.
fn send_levels_to_plugin(c: &Config) {
	let msg = serde_json::json!({ "event": "orvoSoundboardLevels", "mic": c.mic_gain.min(1.0), "sound": c.sound_gain.min(1.0) });
	tauri::async_runtime::spawn(async move {
		let _ = crate::events::outbound::orvo_send_to_plugin(crate::events::frontend::pad::NINEDECK_PLUGIN, &msg).await;
	});
}

fn routing(c: &Config) -> Routing {
	Routing {
		mic: c.mic_device.clone(),
		monitor: c.monitor_device.clone(),
	}
}

/// Starts (or restarts) the audio with the saved devices.
fn restart(st: &mut State) {
	if !st.config.enabled {
		if let Some(e) = &st.engine {
			e.stop();
		}
		return;
	}
	let engine = st.engine.get_or_insert_with(Engine::new);
	engine.start(routing(&st.config));
	apply_levels(st);
}

/// Called once at start-up: only opens the audio if the user has set up the soundboard.
pub fn init() {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	if !st.sounds.is_empty() && st.config.enabled {
		restart(&mut st);
	}
	let config = st.config.clone();
	drop(st);
	// the plugin may still be starting: tell it twice
	send_levels_to_plugin(&config);
	std::thread::sleep(std::time::Duration::from_secs(5));
	send_levels_to_plugin(&config);
}

static LEVEL_SAVE: LazyLock<Mutex<Option<std::time::Instant>>> = LazyLock::new(|| Mutex::new(None));

/// A pad slider moved (Voice volume / Sounds volume): 0 – 1.
pub fn set_level(which: &str, value: f32) {
	let value = if value.is_finite() { value.clamp(0.0, 1.0) } else { return };
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	match which {
		"mic" => st.config.mic_gain = value,
		"sound" => st.config.sound_gain = value,
		_ => return,
	}
	apply_levels(&st);
	let (mic, sound) = (st.config.mic_gain, st.config.sound_gain);
	drop(st);
	if let Some(app) = crate::APP_HANDLE.get() {
		let _ = app.emit("soundboard_levels", serde_json::json!({ "mic_gain": mic, "sound_gain": sound }));
	}
	// save once the slider rests
	let first = {
		let mut due = LEVEL_SAVE.lock().unwrap();
		let first = due.is_none();
		*due = Some(std::time::Instant::now());
		first
	};
	if first {
		std::thread::spawn(|| {
			loop {
				std::thread::sleep(std::time::Duration::from_millis(300));
				let mut due = LEVEL_SAVE.lock().unwrap();
				if due.map(|t| t.elapsed().as_millis() >= 400).unwrap_or(true) {
					*due = None;
					break;
				}
			}
			let st = STATE.lock().unwrap();
			save(&st);
		});
	}
}

fn clip_for(st: &mut State, id: &str) -> anyhow::Result<(Clip, Sound)> {
	let sound = st.sounds.iter().find(|s| s.id == id).cloned().ok_or_else(|| anyhow::anyhow!("sound not found"))?;
	if let Some(c) = st.clips.get(id) {
		return Ok((c.clone(), sound));
	}
	let data = decode::decode(&files_dir().join(&sound.file))?;
	let clip: Clip = std::sync::Arc::new(data);
	st.clips.insert(id.to_owned(), clip.clone());
	Ok((clip, sound))
}

pub fn play(id: &str) -> anyhow::Result<()> {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	if st.engine.is_none() || !st.config.enabled {
		st.config.enabled = true;
		restart(&mut st);
	}
	let (clip, sound) = clip_for(&mut st, id)?;
	let engine = st.engine.as_ref().unwrap();
	let is_playing = engine.playing.lock().unwrap().contains_key(id);
	match sound.mode.as_str() {
		"loop" if is_playing => engine.send(VoiceCmd::Stop(id.to_owned())),
		"restart" | "loop" => {
			engine.send(VoiceCmd::Stop(id.to_owned()));
			engine.send(VoiceCmd::Play {
				key: id.to_owned(),
				clip,
				gain: sound.volume.clamp(0.0, 2.0),
				looped: sound.mode == "loop",
			});
		}
		_ => engine.send(VoiceCmd::Play {
			key: id.to_owned(),
			clip,
			gain: sound.volume.clamp(0.0, 2.0),
			looped: false,
		}),
	}
	drop(st);
	notify();
	Ok(())
}

/// Plays the sound chosen on a pad key, if this is a soundboard key.
pub fn on_key_down(action_uuid: &str, settings: &serde_json::Value) {
	if action_uuid != SOUND_ACTION {
		return;
	}
	if let Some(id) = settings.get("sound").and_then(|v| v.as_str()) {
		let id = id.to_owned();
		std::thread::spawn(move || {
			if let Err(e) = play(&id) {
				log::warn!("soundboard: can't play {id}: {e:#}");
			}
		});
	}
}

fn notify() {
	if let Some(app) = crate::APP_HANDLE.get() {
		let _ = app.emit("soundboard_changed", ());
	}
}

#[derive(Serialize)]
pub struct SoundboardState {
	sounds: Vec<Sound>,
	config: Config,
	inputs: Vec<String>,
	outputs: Vec<String>,
	status: EngineStatus,
	playing: Vec<String>,
	cable_installed: bool,
	cable_url: &'static str,
}

#[command]
pub fn sb_state() -> SoundboardState {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	let (inputs, outputs) = engine::device_names();
	let cable_installed = outputs.iter().any(|n| engine::is_cable_input(n));
	let (status, playing) = match &st.engine {
		Some(e) => (e.status.lock().unwrap().clone(), e.playing.lock().unwrap().keys().cloned().collect()),
		None => (EngineStatus::default(), Vec::new()),
	};
	SoundboardState {
		sounds: st.sounds.clone(),
		config: st.config.clone(),
		inputs: inputs.into_iter().filter(|n| !engine::is_cable(n)).collect(),
		outputs: outputs.into_iter().filter(|n| !engine::is_cable(n)).collect(),
		status,
		playing,
		cable_installed,
		cable_url: CABLE_URL,
	}
}

fn new_id() -> String {
	let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
	format!("s{:x}{:04x}", nanos, rand_u16())
}
fn rand_u16() -> u16 {
	use std::hash::{BuildHasher, Hasher};
	let mut h = std::collections::hash_map::RandomState::new().build_hasher();
	h.write_u8(0);
	h.finish() as u16
}

fn nice_name(file_name: &str) -> String {
	let stem = Path::new(file_name).file_stem().and_then(|s| s.to_str()).unwrap_or("Sound");
	let name = stem.replace(['_', '-'], " ");
	let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
	if name.is_empty() { "Sound".into() } else { name.chars().take(60).collect() }
}

/// Copies one sound into Orvo's folder, checks it decodes, and adds it.
fn add_one(file_name: &str, bytes: &[u8]) -> anyhow::Result<Sound> {
	let ext = Path::new(file_name).extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
	if !EXTENSIONS.contains(&ext.as_str()) {
		anyhow::bail!("not a sound file");
	}
	std::fs::create_dir_all(files_dir())?;
	let id = new_id();
	let file = format!("{id}.{ext}");
	let path = files_dir().join(&file);
	std::fs::write(&path, bytes)?;
	let data = match decode::decode(&path) {
		Ok(d) => d,
		Err(e) => {
			let _ = std::fs::remove_file(&path);
			return Err(e);
		}
	};
	let sound = Sound {
		id: id.clone(),
		name: nice_name(file_name),
		file,
		volume: 1.0,
		mode: "overlap".into(),
		duration_ms: (data.len() / 2) as u64 * 1000 / engine::CLIP_RATE as u64,
	};
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	st.clips.insert(id, std::sync::Arc::new(data));
	st.sounds.push(sound.clone());
	save(&st);
	Ok(sound)
}

#[derive(Serialize)]
pub struct AddResult {
	added: Vec<Sound>,
	failed: Vec<String>,
}

#[command]
pub async fn sb_add_files(paths: Vec<String>) -> AddResult {
	tokio::task::spawn_blocking(move || {
		let mut result = AddResult { added: vec![], failed: vec![] };
		for p in paths {
			let path = PathBuf::from(&p);
			let name = path.file_name().and_then(|n| n.to_str()).unwrap_or(&p).to_owned();
			match std::fs::read(&path).map_err(anyhow::Error::from).and_then(|b| add_one(&name, &b)) {
				Ok(s) => result.added.push(s),
				Err(e) => result.failed.push(format!("{name}: {e}")),
			}
		}
		notify();
		result
	})
	.await
	.unwrap_or(AddResult {
		added: vec![],
		failed: vec!["internal error".into()],
	})
}

/// Raw bytes of one dropped file; its name comes in the "x-name" header (URI-encoded).
#[command]
pub async fn sb_add_bytes(request: tauri::ipc::Request<'_>) -> Result<Sound, Error> {
	let name = request
		.headers()
		.get("x-name")
		.and_then(|v| v.to_str().ok())
		.map(|v| urlencoding_decode(v))
		.unwrap_or_else(|| "Sound.mp3".into());
	let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
		return Err(anyhow::anyhow!("expected the file's bytes").into());
	};
	let bytes = bytes.clone();
	let sound = tokio::task::spawn_blocking(move || add_one(&name, &bytes)).await.map_err(anyhow::Error::from)??;
	notify();
	Ok(sound)
}

fn urlencoding_decode(s: &str) -> String {
	let bytes = s.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] == b'%' && i + 2 < bytes.len() {
			if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
				out.push(b);
				i += 3;
				continue;
			}
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

#[command]
pub fn sb_update(sound: Sound) -> Result<(), Error> {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	let Some(s) = st.sounds.iter_mut().find(|s| s.id == sound.id) else {
		return Err(anyhow::anyhow!("sound not found").into());
	};
	s.name = sound.name.trim().chars().take(60).collect();
	if s.name.is_empty() {
		s.name = "Sound".into();
	}
	s.volume = sound.volume.clamp(0.0, 2.0);
	s.mode = match sound.mode.as_str() {
		"restart" | "loop" => sound.mode,
		_ => "overlap".into(),
	};
	save(&st);
	drop(st);
	notify();
	Ok(())
}

#[command]
pub fn sb_reorder(ids: Vec<String>) {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	let pos: HashMap<&String, usize> = ids.iter().enumerate().map(|(i, id)| (id, i)).collect();
	st.sounds.sort_by_key(|s| pos.get(&s.id).copied().unwrap_or(usize::MAX));
	save(&st);
}

#[command]
pub fn sb_remove(id: String) {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	if let Some(e) = &st.engine {
		e.send(VoiceCmd::Stop(id.clone()));
	}
	if let Some(i) = st.sounds.iter().position(|s| s.id == id) {
		let s = st.sounds.remove(i);
		let _ = std::fs::remove_file(files_dir().join(&s.file));
	}
	st.clips.remove(&id);
	save(&st);
	drop(st);
	notify();
}

#[command]
pub async fn sb_play(id: String) -> Result<(), Error> {
	tokio::task::spawn_blocking(move || play(&id)).await.map_err(anyhow::Error::from)??;
	Ok(())
}

/// Ids of the sounds playing right now (cheap: for the tiles' "playing" look).
#[command]
pub fn sb_playing() -> Vec<String> {
	let st = STATE.lock().unwrap();
	st.engine.as_ref().map(|e| e.playing.lock().unwrap().keys().cloned().collect()).unwrap_or_default()
}

#[command]
pub fn sb_stop_all() {
	let st = STATE.lock().unwrap();
	if let Some(e) = &st.engine {
		e.send(VoiceCmd::StopAll);
	}
}

#[command]
pub fn sb_set_config(config: Config) {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	let devices_changed = routing(&st.config) != routing(&config) || st.config.enabled != config.enabled;
	st.config = config;
	save(&st);
	if devices_changed || st.engine.is_none() {
		restart(&mut st);
	} else {
		apply_levels(&st);
	}
	send_levels_to_plugin(&st.config);
}

/// "Hear myself": your cleaned-up voice in your headphones while tuning (not saved).
#[command]
pub fn sb_listen(on: bool) {
	let st = STATE.lock().unwrap();
	if let Some(e) = &st.engine {
		e.levels.listen_self.store(on, std::sync::atomic::Ordering::Relaxed);
	}
}

#[derive(Serialize)]
pub struct MeterReading {
	level_db: f32,
	out_db: f32,
	gate_open: bool,
	voice: f32,
	running: bool,
}

/// Live mic meter for the Your mic card.
#[command]
pub fn sb_meter() -> MeterReading {
	let st = STATE.lock().unwrap();
	match &st.engine {
		Some(e) => {
			let l = &e.levels;
			let get = |a: &std::sync::atomic::AtomicU32| f32::from_bits(a.load(std::sync::atomic::Ordering::Relaxed));
			MeterReading {
				level_db: get(&l.meter_level),
				out_db: get(&l.meter_out),
				gate_open: l.meter_gate.load(std::sync::atomic::Ordering::Relaxed),
				voice: get(&l.meter_voice),
				running: e.status.lock().unwrap().mic_name.is_some(),
			}
		}
		None => MeterReading {
			level_db: -120.0,
			out_db: -120.0,
			gate_open: false,
			voice: 0.0,
			running: false,
		},
	}
}

/// Re-checks devices (e.g. right after installing VB-Cable) and reopens the audio.
#[command]
pub fn sb_refresh() {
	let mut st = STATE.lock().unwrap();
	load(&mut st);
	st.config.enabled = true;
	save(&st);
	restart(&mut st);
}
