//! AI voices (RVC) — the conversion itself, with no app glue so it can be tested alone.
//!
//! Per step: the last second of mic audio at 16 kHz goes through ContentVec (what is said) and
//! RMVPE (the pitch), then the voice model (net_g) speaks it in its own voice. Only the newest
//! block of the result is kept, cross-faded into the previous one.

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;

/// 10 ms frames at 16 kHz.
pub const HOP: usize = 160;
/// Default window when a voice model accepts any length: 1 s.
pub const DEFAULT_WINDOW: usize = 100;

/// Loads ONNX Runtime (and DirectML next to it) from the engine folder. Call once.
pub fn load_runtime(dir: &Path) -> anyhow::Result<()> {
	#[cfg(windows)]
	{
		let dml = dir.join("DirectML.dll");
		if dml.exists() {
			// keep it loaded so onnxruntime.dll finds this copy, not an older one in System32
			let lib = unsafe { libloading::Library::new(&dml) }?;
			std::mem::forget(lib);
		}
	}
	let name = if cfg!(windows) {
		"onnxruntime.dll"
	} else if cfg!(target_os = "macos") {
		"libonnxruntime.dylib"
	} else {
		"libonnxruntime.so"
	};
	let lib = std::env::var("ORVO_ORT_LIB").map(std::path::PathBuf::from).unwrap_or_else(|_| dir.join(name));
	ort::init_from(&lib)?.with_name("orvo").commit();
	Ok(())
}

fn err(e: impl std::fmt::Display) -> anyhow::Error {
	anyhow::anyhow!("{e}")
}

/// Opens a model, on the GPU (DirectML) when asked and possible, else on the CPU.
/// Returns the session and whether it runs on the GPU.
pub fn open(path: &Path, gpu: bool) -> anyhow::Result<(Session, bool)> {
	let cpu = || -> anyhow::Result<Session> {
		let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
		Ok(Session::builder()?.with_intra_threads(threads).map_err(err)?.commit_from_file(path)?)
	};
	if gpu {
		let dml = || -> anyhow::Result<Session> {
			Ok(Session::builder()?
				.with_memory_pattern(false)
				.map_err(err)?
				.with_parallel_execution(false)
				.map_err(err)?
				.with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
				.map_err(err)?
				.commit_from_file(path)?)
		};
		match dml() {
			Ok(s) => return Ok((s, true)),
			Err(e) => log::warn!("AI voices: GPU not available for {}, using the CPU ({e})", path.display()),
		}
	}
	Ok((cpu()?, false))
}

fn dims(s: &Session, input: &str) -> Option<Vec<i64>> {
	s.inputs().iter().find(|i| i.name() == input).and_then(|i| i.dtype().tensor_shape()).map(|sh| sh.iter().copied().collect())
}

/// What a voice model needs: its sample rate and window length (10 ms frames).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VoiceInfo {
	pub sr: u32,
	pub window: usize,
}

/// Checks a voice model is an RVC v2 model with pitch, and measures its sample rate.
pub fn inspect_voice(s: &mut Session) -> anyhow::Result<VoiceInfo> {
	for need in ["phone", "phone_lengths", "pitch", "pitchf", "ds", "rnd"] {
		if !s.inputs().iter().any(|i| i.name() == need) {
			anyhow::bail!("this isn't an RVC voice with pitch (it has no \"{need}\" input)");
		}
	}
	let phone = dims(s, "phone").unwrap_or_default();
	match phone.get(2) {
		Some(768) => {}
		Some(256) => anyhow::bail!("this is an old RVC v1 voice; only v2 voices work"),
		_ => {}
	}
	let window = match phone.get(1) {
		Some(&n) if n > 0 => n as usize,
		_ => DEFAULT_WINDOW,
	};
	let out = run_voice(s, window, &vec![0.0; window * 768], &vec![0.0; window], 0)?;
	let raw = out.len() as f64 / window as f64 * 100.0;
	let sr = [32_000u32, 40_000, 44_100, 48_000].into_iter().min_by_key(|r| (*r as f64 - raw).abs() as u64).unwrap();
	if (sr as f64 - raw).abs() > 2_000.0 {
		anyhow::bail!("couldn't work out the voice's sample rate ({raw:.0})");
	}
	Ok(VoiceInfo { sr, window })
}

fn coarse(f0: f32) -> i64 {
	if f0 <= 0.0 {
		return 1;
	}
	let mel = 1127.0 * (1.0 + f0 / 700.0).ln();
	let (mn, mx) = (1127.0 * (1.0 + 50.0f32 / 700.0).ln(), 1127.0 * (1.0 + 1100.0f32 / 700.0).ln());
	((mel - mn) * 254.0 / (mx - mn) + 1.0).round().clamp(1.0, 255.0) as i64
}

fn run_voice(s: &mut Session, t: usize, phone: &[f32], f0: &[f32], seed: u64) -> anyhow::Result<Vec<f32>> {
	let pitch: Vec<i64> = f0.iter().map(|&f| coarse(f)).collect();
	let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
	let rnd: Vec<f32> = (0..192 * t).map(|_| gauss(&mut rng)).collect();
	let outputs = s.run(ort::inputs![
		"phone" => Tensor::from_array(([1usize, t, 768], phone.to_vec()))?,
		"phone_lengths" => Tensor::from_array(([1usize], vec![t as i64]))?,
		"pitch" => Tensor::from_array(([1usize, t], pitch))?,
		"pitchf" => Tensor::from_array(([1usize, t], f0.to_vec()))?,
		"ds" => Tensor::from_array(([1usize], vec![0i64]))?,
		"rnd" => Tensor::from_array(([1usize, 192, t], rnd))?,
	])?;
	let (_, audio) = outputs[0].try_extract_tensor::<f32>()?;
	Ok(audio.to_vec())
}

fn gauss(s: &mut u64) -> f32 {
	let mut u = || {
		*s ^= *s << 13;
		*s ^= *s >> 7;
		*s ^= *s << 17;
		((*s >> 11) as f64 + 0.5) / (1u64 << 53) as f64
	};
	let (a, b) = (u(), u());
	((-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()) as f32
}

/// ContentVec: 16 kHz audio → 768 features every 20 ms, doubled to 10 ms frames.
fn features(cv: &mut Session, audio: &[f32], frames: usize) -> anyhow::Result<Vec<f32>> {
	let n = audio.len();
	let outputs = cv.run(ort::inputs![
		"input_values" => Tensor::from_array(([1usize, n], audio.to_vec()))?,
		"attention_mask" => Tensor::from_array(([1usize, n], vec![1i64; n]))?,
	])?;
	let (shape, hidden) = outputs[0].try_extract_tensor::<f32>()?;
	let have = shape[1] as usize;
	let mut out = Vec::with_capacity(frames * 768);
	for f in 0..frames {
		let src = (f / 2).min(have.saturating_sub(1));
		out.extend_from_slice(&hidden[src * 768..src * 768 + 768]);
	}
	Ok(out)
}

/// RMVPE: 16 kHz audio → pitch in Hz every 10 ms (0 = no pitch).
fn pitch(rm: &mut Session, audio: &[f32], frames: usize) -> anyhow::Result<Vec<f32>> {
	let outputs = rm.run(ort::inputs!["audio" => Tensor::from_array(([1usize, audio.len()], audio.to_vec()))?])?;
	let (shape, sal) = outputs[0].try_extract_tensor::<f32>()?;
	let have = shape[1] as usize;
	Ok((0..frames).map(|f| decode_pitch(&sal[f.min(have - 1) * 360..][..360])).collect())
}

/// RMVPE's decoder: weighted average of the 9 bins around the peak; 20-cent bins from 32.7 Hz.
pub fn decode_pitch(sal: &[f32]) -> f32 {
	let (peak, &best) = sal.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
	if best <= 0.03 {
		return 0.0;
	}
	let (mut wsum, mut csum) = (0.0f64, 0.0f64);
	for i in peak.saturating_sub(4)..=(peak + 4).min(359) {
		let w = sal[i].max(0.0) as f64;
		wsum += w;
		csum += w * (1997.379_408_437_619_1 + 20.0 * i as f64);
	}
	if wsum <= 1e-12 {
		return 0.0;
	}
	(10.0 * 2f64.powf(csum / wsum / 1200.0)) as f32
}

/// The running conversion for one voice.
pub struct Converter {
	pub info: VoiceInfo,
	/// The last `window` frames of mic audio at 16 kHz.
	buf: Vec<f32>,
	tail: Vec<f32>,
	seed: u64,
}

impl Converter {
	pub fn new(info: VoiceInfo) -> Self {
		Converter { info, buf: vec![0.0; info.window * HOP], tail: Vec::new(), seed: 1 }
	}

	/// Samples per 10 ms frame at the voice's rate.
	pub fn spf(&self) -> usize {
		self.info.sr as usize / 100
	}

	/// Converts one block of new 16 kHz audio (a whole number of 10 ms frames, shorter than the
	/// window) and returns that block at the voice's rate.
	pub fn step(&mut self, cv: &mut Session, rm: &mut Session, voice: &mut Session, new16: &[f32], key: f32) -> anyhow::Result<Vec<f32>> {
		let w = self.info.window;
		let blk = new16.len() / HOP;
		anyhow::ensure!(blk > 0 && blk < w, "block must be shorter than the window");
		self.buf.drain(..new16.len());
		self.buf.extend_from_slice(new16);
		let phone = features(cv, &self.buf, w)?;
		let ratio = 2f32.powf(key / 12.0);
		let f0: Vec<f32> = pitch(rm, &self.buf, w)?.into_iter().map(|f| f * ratio).collect();
		self.seed += 1;
		let y = run_voice(voice, w, &phone, &f0, self.seed)?;
		let spf = self.spf();
		let xf = spf * 2;
		let n = y.len();
		let want = blk * spf;
		anyhow::ensure!(n >= want + xf, "voice gave too little audio");
		let mut seg = y[n - want - xf..n - xf].to_vec();
		if self.tail.len() == xf {
			for i in 0..xf {
				let r = i as f32 / xf as f32;
				seg[i] = seg[i] * r + self.tail[i] * (1.0 - r);
			}
		}
		self.tail = y[n - xf..].to_vec();
		Ok(seg)
	}
}

/// 48 kHz → 16 kHz: low-pass at ~7 kHz, keep every third sample. Streams.
pub struct Down3 {
	hist: Vec<f32>,
	taps: Vec<f32>,
	phase: usize,
}

impl Down3 {
	pub fn new() -> Self {
		let n = 63;
		let fc = 7_200.0 / 48_000.0;
		let taps: Vec<f32> = (0..n)
			.map(|i| {
				let m = i as f32 - (n - 1) as f32 / 2.0;
				let sinc = if m == 0.0 { 2.0 * fc } else { (std::f32::consts::TAU * fc * m).sin() / (std::f32::consts::PI * m) };
				let win = 0.54 - 0.46 * (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos();
				sinc * win
			})
			.collect();
		let sum: f32 = taps.iter().sum();
		Down3 { hist: vec![0.0; n], taps: taps.into_iter().map(|t| t / sum).collect(), phase: 0 }
	}

	pub fn push(&mut self, x: f32, out: &mut Vec<f32>) {
		self.hist.rotate_left(1);
		*self.hist.last_mut().unwrap() = x;
		self.phase += 1;
		if self.phase == 3 {
			self.phase = 0;
			out.push(self.hist.iter().zip(&self.taps).map(|(a, b)| a * b).sum());
		}
	}
}

impl Default for Down3 {
	fn default() -> Self {
		Self::new()
	}
}

/// Any rate → 48 kHz, cubic (Catmull-Rom) interpolation. Streams.
pub struct Up {
	step: f64,
	pos: f64,
	h: [f32; 4],
}

impl Up {
	pub fn new(from: u32) -> Self {
		Up { step: from as f64 / 48_000.0, pos: 0.0, h: [0.0; 4] }
	}

	pub fn push(&mut self, x: f32, out: &mut Vec<f32>) {
		self.h = [self.h[1], self.h[2], self.h[3], x];
		while self.pos < 1.0 {
			let t = self.pos as f32;
			let [a, b, c, d] = self.h;
			let v = b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)));
			out.push(v);
			self.pos += self.step;
		}
		self.pos -= 1.0;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn pitch_decoder_finds_the_peak() {
		let mut sal = vec![0.0f32; 360];
		// bin 100 → 10 Hz · 2^((1997.38 + 2000) / 1200) ≈ 100.64 Hz
		sal[100] = 0.9;
		let f = decode_pitch(&sal);
		assert!((f - 100.64).abs() < 0.05, "{f}");
		assert_eq!(decode_pitch(&vec![0.01; 360]), 0.0);
	}

	#[test]
	fn coarse_pitch_matches_rvc() {
		assert_eq!(coarse(0.0), 1);
		assert_eq!(coarse(50.0), 1);
		assert_eq!(coarse(1100.0), 255);
		assert_eq!(coarse(200.0), 54); // RVC: round(53.897)
	}

	#[test]
	fn resamplers_keep_rate_and_tone() {
		let mut d = Down3::new();
		let mut o = Vec::new();
		for i in 0..48_000 {
			d.push((std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin(), &mut o);
		}
		assert_eq!(o.len(), 16_000);
		let rms = (o[1000..].iter().map(|v| v * v).sum::<f32>() / 15_000.0).sqrt();
		assert!((rms - 0.707).abs() < 0.02, "{rms}");
		let mut u = Up::new(40_000);
		let mut o2 = Vec::new();
		for i in 0..40_000 {
			u.push((std::f32::consts::TAU * 440.0 * i as f32 / 40_000.0).sin(), &mut o2);
		}
		assert!((o2.len() as i64 - 48_000).abs() <= 2, "{}", o2.len());
	}
}
