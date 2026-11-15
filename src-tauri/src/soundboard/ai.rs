//! AI voices (RVC): the engine download, the voice list and the real-time worker.
//!
//! The mic thread hands every 10 ms of cleaned-up voice to the worker through a queue and takes
//! back converted voice (`exchange`). The worker converts one block at a time on the GPU
//! (DirectML) or, if there is none, the CPU. Nothing is loaded while no AI voice is on.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, command};

use super::ai_core::{self, Converter, Down3, Up, VoiceInfo};
use crate::events::frontend::Error;

// ---------- engine files ----------

struct EngineFile {
	name: &'static str,
	url: &'static str,
	/// File inside a .nupkg (zip), fetched alone with range requests.
	inner: Option<&'static str>,
	/// Bytes downloaded (compressed size for files inside a package).
	size: u64,
	sha256: &'static str,
	windows_only: bool,
}

const ORT_PKG: &str = "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg";
const DML_PKG: &str = "https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg";

const FILES: [EngineFile; 5] = [
	EngineFile {
		name: "onnxruntime.dll",
		url: ORT_PKG,
		inner: Some("runtimes/win-x64/native/onnxruntime.dll"),
		size: 6_039_111,
		sha256: "e7eedec6a6f26dc39dc948276a75ef6d2bee3fff944d874ceed0bbd3b97bff40",
		windows_only: true,
	},
	EngineFile {
		name: "onnxruntime_providers_shared.dll",
		url: ORT_PKG,
		inner: Some("runtimes/win-x64/native/onnxruntime_providers_shared.dll"),
		size: 12_398,
		sha256: "265c8daf29637cb259cac8be9f08f2cd45f3883f0f0e4949cbfddd5b4cbec3b6",
		windows_only: true,
	},
	EngineFile {
		name: "DirectML.dll",
		url: DML_PKG,
		inner: Some("bin/x64-win/DirectML.dll"),
		size: 9_332_741,
		sha256: "9c9e6d822561c6c41b90e6994b3e8857cf1d66dbfb1e0c4c799c7c89b4e92da1",
		windows_only: true,
	},
	EngineFile {
		name: "contentvec.onnx",
		url: "https://huggingface.co/TigreGotico/voiceclonnx-rvc/resolve/main/contentvec_768l12.onnx",
		inner: None,
		size: 377_755_911,
		sha256: "cbddd8fa9352b3128df6359e2e3da6be0f9072e768e51e3efc3004e5030ea97f",
		windows_only: false,
	},
	EngineFile {
		name: "rmvpe.onnx",
		url: "https://huggingface.co/Scicom-intl/rmvpe-onnx/resolve/main/rmvpe.onnx",
		inner: None,
		size: 185_845_903,
		sha256: "648e1a7bf3795f39498841d35c2a100ab4997c045e7d5ed555b02aef0f3dfa32",
		windows_only: false,
	},
];

fn files() -> impl Iterator<Item = &'static EngineFile> {
	FILES.iter().filter(|f| cfg!(windows) || !f.windows_only)
}

fn root() -> PathBuf {
	let app = crate::APP_HANDLE.get().expect("app handle");
	app.path().app_local_data_dir().unwrap_or_else(|_| crate::shared::config_dir()).join("ai")
}
fn engine_dir() -> PathBuf {
	root().join("engine")
}
fn voices_dir() -> PathBuf {
	root().join("voices")
}

pub fn installed() -> bool {
	let dir = engine_dir();
	files().all(|f| dir.join(f.name).is_file()) && dir.join("ready").is_file()
}

static INSTALLING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
struct Progress {
	done: u64,
	total: u64,
	file: &'static str,
}

fn emit_progress(p: Progress) {
	if let Some(app) = crate::APP_HANDLE.get() {
		let _ = app.emit("ai_install_progress", p);
	}
}

fn client() -> anyhow::Result<reqwest::Client> {
	Ok(reqwest::Client::builder().user_agent("Orvo Studio").connect_timeout(Duration::from_secs(20)).build()?)
}

/// Downloads the AI engine (about 580 MB) and checks every file.
#[command]
pub async fn sb_ai_install() -> Result<(), Error> {
	if INSTALLING.swap(true, Ordering::SeqCst) {
		return Err(anyhow::anyhow!("already downloading").into());
	}
	let r = install().await;
	INSTALLING.store(false, Ordering::SeqCst);
	super::notify();
	r.map_err(|e| anyhow::anyhow!("AI engine download failed: {e:#}").into())
}

async fn install() -> anyhow::Result<()> {
	let dir = engine_dir();
	tokio::fs::create_dir_all(&dir).await?;
	let _ = tokio::fs::remove_file(dir.join("ready")).await;
	let client = client()?;
	let total: u64 = files().map(|f| f.size).sum();
	let mut done = 0u64;
	for f in files() {
		let dest = dir.join(f.name);
		if dest.is_file() && sha256_file(&dest).await.ok().as_deref() == Some(f.sha256) {
			done += f.size;
			emit_progress(Progress { done, total, file: f.name });
			continue;
		}
		let bytes = match f.inner {
			Some(inner) => fetch_from_zip(&client, f.url, inner, &mut |n| emit_progress(Progress { done: done + n, total, file: f.name })).await?,
			None => fetch(&client, f.url, &dest, &mut |n| emit_progress(Progress { done: done + n, total, file: f.name })).await?,
		};
		if let Some(bytes) = bytes {
			tokio::fs::write(&dest, bytes).await?;
		}
		let got = sha256_file(&dest).await?;
		if got != f.sha256 {
			let _ = tokio::fs::remove_file(&dest).await;
			anyhow::bail!("{} didn't match its checksum; try again", f.name);
		}
		done += f.size;
		emit_progress(Progress { done, total, file: f.name });
	}
	tokio::fs::write(dir.join("ready"), b"1").await?;
	Ok(())
}

async fn sha256_file(p: &Path) -> anyhow::Result<String> {
	let p = p.to_owned();
	tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
		use sha2::{Digest, Sha256};
		use std::io::Read;
		let mut f = std::fs::File::open(p)?;
		let mut h = Sha256::new();
		let mut buf = vec![0u8; 1 << 20];
		loop {
			let n = f.read(&mut buf)?;
			if n == 0 {
				break;
			}
			h.update(&buf[..n]);
		}
		Ok(format!("{:x}", h.finalize()))
	})
	.await?
}

/// Streams a big file to disk (returns None: it's already written).
async fn fetch(client: &reqwest::Client, url: &str, dest: &Path, progress: &mut (dyn FnMut(u64) + Send)) -> anyhow::Result<Option<Vec<u8>>> {
	use tokio::io::AsyncWriteExt;
	let part = dest.with_extension("part");
	let mut resp = client.get(url).send().await?.error_for_status()?;
	let mut out = tokio::fs::File::create(&part).await?;
	let mut n = 0u64;
	while let Some(chunk) = resp.chunk().await? {
		out.write_all(&chunk).await?;
		n += chunk.len() as u64;
		progress(n);
	}
	out.flush().await?;
	drop(out);
	tokio::fs::rename(&part, dest).await?;
	Ok(None)
}

async fn range(client: &reqwest::Client, url: &str, from: u64, len: u64) -> anyhow::Result<Vec<u8>> {
	let resp = client.get(url).header("Range", format!("bytes={}-{}", from, from + len - 1)).send().await?.error_for_status()?;
	anyhow::ensure!(resp.status() == reqwest::StatusCode::PARTIAL_CONTENT, "the server doesn't support partial downloads");
	Ok(resp.bytes().await?.to_vec())
}

fn u16le(b: &[u8], at: usize) -> u64 {
	u16::from_le_bytes([b[at], b[at + 1]]) as u64
}
fn u32le(b: &[u8], at: usize) -> u64 {
	u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as u64
}

/// Takes one file out of a zip on a web server without downloading the whole zip.
async fn fetch_from_zip(client: &reqwest::Client, url: &str, inner: &str, progress: &mut (dyn FnMut(u64) + Send)) -> anyhow::Result<Option<Vec<u8>>> {
	let head = client.head(url).send().await?.error_for_status()?;
	let size = head.content_length().ok_or_else(|| anyhow::anyhow!("unknown package size"))?;
	let tail_len = size.min(128 * 1024);
	let tail = range(client, url, size - tail_len, tail_len).await?;
	let eocd = (0..tail.len().saturating_sub(21))
		.rev()
		.find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
		.ok_or_else(|| anyhow::anyhow!("not a zip package"))?;
	let (cd_size, cd_off) = (u32le(&tail, eocd + 12), u32le(&tail, eocd + 16));
	let cd = range(client, url, cd_off, cd_size).await?;
	let mut i = 0usize;
	while i + 46 <= cd.len() && cd[i..i + 4] == [0x50, 0x4b, 0x01, 0x02] {
		let (method, csize, usize_) = (u16le(&cd, i + 10), u32le(&cd, i + 20), u32le(&cd, i + 24));
		let (nlen, xlen, clen) = (u16le(&cd, i + 28) as usize, u16le(&cd, i + 30) as usize, u16le(&cd, i + 32) as usize);
		let local = u32le(&cd, i + 42);
		let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + nlen]);
		if name == inner {
			let lh = range(client, url, local, 30).await?;
			anyhow::ensure!(lh[0..4] == [0x50, 0x4b, 0x03, 0x04], "bad zip entry");
			let start = local + 30 + u16le(&lh, 26) + u16le(&lh, 28);
			let mut resp = client.get(url).header("Range", format!("bytes={}-{}", start, start + csize - 1)).send().await?.error_for_status()?;
			let mut data = Vec::with_capacity(csize as usize);
			while let Some(chunk) = resp.chunk().await? {
				data.extend_from_slice(&chunk);
				progress(data.len() as u64);
			}
			anyhow::ensure!(data.len() as u64 == csize, "download cut short");
			let out = match method {
				0 => data,
				8 => {
					use std::io::Read;
					let mut out = Vec::with_capacity(usize_ as usize);
					flate2::read::DeflateDecoder::new(&data[..]).read_to_end(&mut out)?;
					out
				}
				m => anyhow::bail!("unsupported zip method {m}"),
			};
			return Ok(Some(out));
		}
		i += 46 + nlen + xlen + clen;
	}
	anyhow::bail!("{inner} isn't in the package")
}

// ---------- voices ----------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AiVoice {
	pub id: String,
	pub name: String,
	pub file: String,
	pub sr: u32,
	pub window: usize,
	/// Semitones; +12 for a man using a woman's voice, -12 the other way round.
	#[serde(default)]
	pub pitch: f32,
	/// "fast", "balanced" or "best".
	#[serde(default = "balanced")]
	pub quality: String,
	/// Picture as a data: URL, if the user picked one.
	#[serde(default)]
	pub image: Option<String>,
}
fn balanced() -> String {
	"balanced".into()
}

fn block_frames(quality: &str) -> usize {
	match quality {
		"fast" => 12,
		"best" => 30,
		_ => 20,
	}
}

static RUNTIME: OnceLock<Result<(), String>> = OnceLock::new();
fn runtime() -> anyhow::Result<()> {
	RUNTIME
		.get_or_init(|| ai_core::load_runtime(&engine_dir()).map_err(|e| format!("{e:#}")))
		.clone()
		.map_err(|e| anyhow::anyhow!("couldn't start the AI engine: {e}"))
}

/// Adds a voice model (.onnx). `.pth` files need converting first.
#[command]
pub async fn sb_ai_add(path: String) -> Result<AiVoice, Error> {
	let src = PathBuf::from(&path);
	let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
	if ext == "pth" {
		return Err(anyhow::anyhow!("This is a .pth voice. Orvo can't open those yet — convert it to .onnx first (see the link under Add voice).").into());
	}
	if ext != "onnx" {
		return Err(anyhow::anyhow!("pick a voice model (.onnx)").into());
	}
	if !installed() {
		return Err(anyhow::anyhow!("download the AI engine first").into());
	}
	let voice = tokio::task::spawn_blocking(move || -> anyhow::Result<AiVoice> {
		runtime()?;
		let id = super::new_id();
		let dir = voices_dir();
		std::fs::create_dir_all(&dir)?;
		let file = format!("{id}.onnx");
		let dest = dir.join(&file);
		std::fs::copy(&src, &dest)?;
		let checked = (|| -> anyhow::Result<VoiceInfo> {
			let (mut s, _) = ai_core::open(&dest, false)?;
			ai_core::inspect_voice(&mut s)
		})();
		let info = match checked {
			Ok(i) => i,
			Err(e) => {
				let _ = std::fs::remove_file(&dest);
				return Err(e);
			}
		};
		let name: String = src.file_stem().and_then(|s| s.to_str()).unwrap_or("AI voice").replace(['_', '-'], " ").chars().take(40).collect();
		Ok(AiVoice {
			id,
			name,
			file,
			sr: info.sr,
			window: info.window,
			pitch: 0.0,
			quality: balanced(),
			image: None,
		})
	})
	.await
	.map_err(anyhow::Error::from)??;
	{
		let mut st = super::STATE.lock().unwrap();
		super::load(&mut st);
		st.config.ai_voices.push(voice.clone());
		super::save(&st);
	}
	super::notify();
	Ok(voice)
}

#[derive(Deserialize)]
pub struct AiVoicePatch {
	name: Option<String>,
	pitch: Option<f32>,
	quality: Option<String>,
	/// Some("") clears the picture.
	image: Option<String>,
}

#[command]
pub fn sb_ai_update(id: String, patch: AiVoicePatch) -> Result<(), Error> {
	let mut st = super::STATE.lock().unwrap();
	super::load(&mut st);
	let v = st.config.ai_voices.iter_mut().find(|v| v.id == id).ok_or_else(|| anyhow::anyhow!("no such voice"))?;
	if let Some(n) = patch.name {
		let n: String = n.trim().chars().take(40).collect();
		if !n.is_empty() {
			v.name = n;
		}
	}
	if let Some(p) = patch.pitch {
		v.pitch = p.clamp(-24.0, 24.0);
	}
	if let Some(q) = patch.quality {
		v.quality = q;
	}
	if let Some(i) = patch.image {
		v.image = if i.is_empty() { None } else { Some(i) };
	}
	super::apply_levels(&st);
	super::save(&st);
	drop(st);
	super::notify();
	Ok(())
}

#[command]
pub fn sb_ai_remove(id: String) -> Result<(), Error> {
	let mut st = super::STATE.lock().unwrap();
	super::load(&mut st);
	if let Some(v) = st.config.ai_voices.iter().find(|v| v.id == id) {
		let _ = std::fs::remove_file(voices_dir().join(&v.file));
	}
	st.config.ai_voices.retain(|v| v.id != id);
	st.config.voice_tweaks.remove(&format!("ai:{id}"));
	if st.config.voice_preset == format!("ai:{id}") {
		st.config.voice_preset = "off".into();
	}
	super::apply_levels(&st);
	super::save(&st);
	drop(st);
	super::notify();
	Ok(())
}

#[derive(Clone, Serialize, Default)]
pub struct AiStatus {
	/// "off", "loading", "running" or "error"
	pub state: String,
	pub message: Option<String>,
	pub gpu: bool,
	pub compute_ms: f32,
	pub block_ms: f32,
	pub delay_ms: f32,
}

#[derive(Serialize)]
pub struct AiState {
	supported: bool,
	installed: bool,
	installing: bool,
	download_mb: u64,
	status: AiStatus,
}

#[command]
pub fn sb_ai_state() -> AiState {
	AiState {
		supported: cfg!(windows) || std::env::var("ORVO_ORT_LIB").is_ok(),
		installed: installed(),
		installing: INSTALLING.load(Ordering::SeqCst),
		download_mb: files().map(|f| f.size).sum::<u64>() / 1_000_000,
		status: STATUS.lock().unwrap().clone(),
	}
}

// ---------- real-time ----------

static STATUS: Mutex<AiStatus> = Mutex::new(AiStatus {
	state: String::new(),
	message: None,
	gpu: false,
	compute_ms: 0.0,
	block_ms: 0.0,
	delay_ms: 0.0,
});

fn set_status(f: impl FnOnce(&mut AiStatus)) {
	f(&mut STATUS.lock().unwrap());
}

/// True while an AI voice is replacing the mic.
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
static UNDERRUNS: AtomicU64 = AtomicU64::new(0);

struct Bridge {
	tx: ringbuf::HeapProd<f32>,
	rx: ringbuf::HeapCons<f32>,
	started: bool,
}
static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

/// Mic thread: hands 10 ms of voice to the AI and replaces it with converted voice
/// (silence until the first block is ready).
pub fn exchange(frame: &mut [f32]) {
	let Ok(mut g) = BRIDGE.try_lock() else {
		frame.fill(0.0);
		return;
	};
	let Some(b) = g.as_mut() else { return };
	let _ = b.tx.push_slice(frame);
	if !b.started {
		if b.rx.occupied_len() >= frame.len() {
			b.started = true;
		} else {
			frame.fill(0.0);
			return;
		}
	}
	let n = b.rx.pop_slice(frame);
	if n < frame.len() {
		frame[n..].fill(0.0);
		b.started = false;
		UNDERRUNS.fetch_add(1, Ordering::Relaxed);
	}
}

#[derive(Clone, PartialEq)]
struct Want {
	voice: AiVoice,
}

/// Live settings the worker reads every block.
struct Live {
	pitch: AtomicU32,
	block: AtomicU32,
	stop: AtomicBool,
}

struct Running {
	voice_file: String,
	live: Arc<Live>,
	thread: std::thread::JoinHandle<Option<(ort::session::Session, ort::session::Session, bool)>>,
}

static MANAGER: Mutex<Option<std::sync::mpsc::Sender<Option<Want>>>> = Mutex::new(None);

/// Called whenever the config changes: starts, switches or stops the AI voice. Never blocks.
pub fn sync(c: &super::Config, engine_running: bool) {
	let want = c
		.voice_preset
		.strip_prefix("ai:")
		.filter(|_| engine_running && installed())
		.and_then(|id| c.ai_voices.iter().find(|v| v.id == id))
		.map(|v| Want { voice: v.clone() });
	if want.is_none() && !ACTIVE.load(Ordering::Relaxed) && MANAGER.lock().unwrap().is_none() {
		return;
	}
	let mut m = MANAGER.lock().unwrap();
	let tx = m.get_or_insert_with(|| {
		let (tx, rx) = std::sync::mpsc::channel::<Option<Want>>();
		std::thread::Builder::new().name("orvo-ai-manager".into()).spawn(move || manager(rx)).expect("ai manager");
		tx
	});
	let _ = tx.send(want);
}

fn manager(rx: std::sync::mpsc::Receiver<Option<Want>>) {
	let mut running: Option<Running> = None;
	// ContentVec + RMVPE stay loaded while AI voices are in use
	let mut shared: Option<(ort::session::Session, ort::session::Session, bool)> = None;
	while let Ok(mut want) = rx.recv() {
		// only the newest request matters
		while let Ok(w) = rx.try_recv() {
			want = w;
		}
		match (&want, &running) {
			(Some(w), Some(r)) if r.voice_file == w.voice.file => {
				r.live.pitch.store(w.voice.pitch.to_bits(), Ordering::Relaxed);
				r.live.block.store(block_frames(&w.voice.quality) as u32, Ordering::Relaxed);
				continue;
			}
			_ => {}
		}
		if let Some(r) = running.take() {
			r.live.stop.store(true, Ordering::Relaxed);
			if let Ok(Some(s)) = r.thread.join() {
				shared = Some(s);
			}
		}
		ACTIVE.store(false, Ordering::Relaxed);
		*BRIDGE.lock().unwrap() = None;
		let Some(w) = want else {
			shared = None; // frees the GPU memory
			set_status(|s| {
				*s = AiStatus {
					state: "off".into(),
					..Default::default()
				}
			});
			continue;
		};
		set_status(|s| {
			*s = AiStatus {
				state: "loading".into(),
				..Default::default()
			}
		});
		// silence (not your real voice) until the AI voice is ready
		let (tx_in, rx_in) = HeapRb::<f32>::new(48_000 * 3).split();
		let (tx_out, rx_out) = HeapRb::<f32>::new(48_000 * 3).split();
		*BRIDGE.lock().unwrap() = Some(Bridge {
			tx: tx_in,
			rx: rx_out,
			started: false,
		});
		ACTIVE.store(true, Ordering::Relaxed);
		let loaded = (|| -> anyhow::Result<_> {
			runtime()?;
			let base = match shared.take() {
				Some(s) => s,
				None => {
					let dir = engine_dir();
					let (cv, g1) = ai_core::open(&dir.join("contentvec.onnx"), true)?;
					let (rm, g2) = ai_core::open(&dir.join("rmvpe.onnx"), true)?;
					(cv, rm, g1 && g2)
				}
			};
			let (voice, g3) = ai_core::open(&voices_dir().join(&w.voice.file), true)?;
			Ok((base, voice, g3))
		})();
		let ((cv, rm, g12), voice, g3) = match loaded {
			Ok(x) => x,
			Err(e) => {
				log::error!("AI voice: {e:#}");
				ACTIVE.store(false, Ordering::Relaxed);
				*BRIDGE.lock().unwrap() = None;
				set_status(|s| {
					*s = AiStatus {
						state: "error".into(),
						message: Some(format!("{e:#}")),
						..Default::default()
					}
				});
				continue;
			}
		};
		let live = Arc::new(Live {
			pitch: AtomicU32::new(w.voice.pitch.to_bits()),
			block: AtomicU32::new(block_frames(&w.voice.quality) as u32),
			stop: AtomicBool::new(false),
		});
		let info = VoiceInfo {
			sr: w.voice.sr,
			window: w.voice.window,
		};
		let l2 = live.clone();
		let gpu = g12 && g3;
		let thread = std::thread::Builder::new()
			.name("orvo-ai".into())
			.spawn(move || worker(cv, rm, voice, gpu, info, l2, rx_in, tx_out))
			.expect("ai worker");
		running = Some(Running {
			voice_file: w.voice.file.clone(),
			live,
			thread,
		});
	}
}

#[allow(clippy::too_many_arguments)]
fn worker(
	mut cv: ort::session::Session,
	mut rm: ort::session::Session,
	mut voice: ort::session::Session,
	gpu: bool,
	info: VoiceInfo,
	live: Arc<Live>,
	mut rx: ringbuf::HeapCons<f32>,
	mut tx: ringbuf::HeapProd<f32>,
) -> Option<(ort::session::Session, ort::session::Session, bool)> {
	let mut conv = Converter::new(info);
	let mut down = Down3::new();
	let mut up = Up::new(info.sr);
	let mut buf48 = Vec::new();
	let mut b16 = Vec::new();
	let mut out48 = Vec::new();
	let mut avg_ms = 0.0f32;
	let mut pad_needed = true;
	// warm up (the GPU's first runs are slow) so the first timing doesn't set a big delay
	let warm_blk = (live.block.load(Ordering::Relaxed) as usize).clamp(5, info.window.saturating_sub(10).max(5));
	for _ in 0..3 {
		let t = Instant::now();
		if let Err(e) = conv.step(&mut cv, &mut rm, &mut voice, &vec![0.0; warm_blk * ai_core::HOP], 0.0) {
			log::error!("AI voice warm-up: {e:#}");
			// back to your normal voice rather than silence
			ACTIVE.store(false, Ordering::Relaxed);
			*BRIDGE.lock().unwrap() = None;
			set_status(|s| {
				s.state = "error".into();
				s.message = Some(format!("{e:#}"));
			});
			return Some((cv, rm, gpu));
		}
		avg_ms = t.elapsed().as_secs_f32() * 1000.0;
	}
	conv = Converter::new(info);
	// drop what the mic sent while loading
	let stale = rx.occupied_len();
	rx.skip(stale);
	let mut underruns = UNDERRUNS.load(Ordering::Relaxed);
	set_status(|s| {
		*s = AiStatus {
			state: "running".into(),
			gpu,
			..Default::default()
		}
	});
	while !live.stop.load(Ordering::Relaxed) {
		let blk = (live.block.load(Ordering::Relaxed) as usize).clamp(5, info.window.saturating_sub(10).max(5));
		let need = blk * 480;
		if rx.occupied_len() < need {
			std::thread::sleep(Duration::from_millis(3));
			continue;
		}
		buf48.resize(need, 0.0);
		rx.pop_slice(&mut buf48);
		b16.clear();
		for &x in &buf48 {
			down.push(x, &mut b16);
		}
		b16.resize(blk * ai_core::HOP, 0.0);
		let t = Instant::now();
		let key = f32::from_bits(live.pitch.load(Ordering::Relaxed));
		let seg = match conv.step(&mut cv, &mut rm, &mut voice, &b16, key) {
			Ok(s) => s,
			Err(e) => {
				log::error!("AI voice step: {e:#}");
				set_status(|s| {
					s.state = "error".into();
					s.message = Some(format!("{e:#}"));
				});
				std::thread::sleep(Duration::from_millis(200));
				continue;
			}
		};
		let ms = t.elapsed().as_secs_f32() * 1000.0;
		avg_ms = if avg_ms == 0.0 { ms } else { avg_ms * 0.85 + ms * 0.15 };
		out48.clear();
		for &x in &seg {
			up.push(x, &mut out48);
		}
		let u = UNDERRUNS.load(Ordering::Relaxed);
		if u != underruns {
			underruns = u;
			pad_needed = true;
		}
		// a little silence first so the next block always arrives in time
		let pad_ms = (avg_ms * 1.3 + 25.0).min(400.0);
		let pad = (pad_ms * 48.0) as usize;
		if pad_needed {
			let _ = tx.push_iter(std::iter::repeat_n(0.0, pad));
			pad_needed = false;
		}
		// if the delay has grown (a slow moment), drop the extra to catch up
		let queued = tx.occupied_len();
		let skip = queued.saturating_sub(pad + 960).min(out48.len());
		let _ = tx.push_slice(&out48[skip..]);
		let block_ms = blk as f32 * 10.0;
		set_status(|s| {
			s.state = "running".into();
			s.message = None;
			s.gpu = gpu;
			s.compute_ms = avg_ms;
			s.block_ms = block_ms;
			s.delay_ms = block_ms + pad_ms + 20.0;
		});
	}
	Some((cv, rm, gpu))
}
