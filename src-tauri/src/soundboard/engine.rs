//! Real-time part of the soundboard: one thread owns the audio streams.
//!
//! - Virtual mic output (VB-Cable's "CABLE Input"): your mic + the soundboard.
//! - Monitor output (your headphones): the soundboard only, so you hear what you play.
//!
//! Sounds are decoded once into 48 kHz stereo f32 and shared between both outputs.
//! Each output resamples on the fly (linear), so devices may run at any rate.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};

use super::fx::{FRAME, MicFx, Processor};

pub const CLIP_RATE: u32 = 48_000;

/// Interleaved stereo samples at CLIP_RATE.
pub type Clip = Arc<Vec<f32>>;

#[derive(Clone)]
pub enum VoiceCmd {
	Play { key: String, clip: Clip, gain: f32, looped: bool },
	Stop(String),
	StopAll,
}

/// Levels that change without restarting the streams.
#[derive(Default)]
pub struct Levels {
	pub mic_on: AtomicBool,
	/// Hear your own (processed) voice in your headphones, for tuning.
	pub listen_self: AtomicBool,
	pub meter_level: AtomicU32,
	pub meter_out: AtomicU32,
	pub meter_gate: AtomicBool,
	pub meter_voice: AtomicU32,
	pub mic_gain: AtomicU32,
	pub sound_gain: AtomicU32,
	pub monitor_on: AtomicBool,
	pub monitor_gain: AtomicU32,
}

fn load(a: &AtomicU32) -> f32 {
	f32::from_bits(a.load(Ordering::Relaxed))
}
pub fn store(a: &AtomicU32, v: f32) {
	a.store(v.to_bits(), Ordering::Relaxed);
}

/// Which devices to open. `None` means the Windows default.
#[derive(Clone, Default, PartialEq)]
pub struct Routing {
	pub mic: Option<String>,
	pub monitor: Option<String>,
}

#[derive(Clone, Default, serde::Serialize)]
pub struct EngineStatus {
	pub running: bool,
	pub cable: bool,
	pub cable_name: Option<String>,
	pub mic_name: Option<String>,
	pub monitor_name: Option<String>,
	pub error: Option<String>,
}

enum Ctl {
	Start(Routing),
	Stop,
}

pub struct Engine {
	ctl: Sender<Ctl>,
	/// Voice cleanup settings, read by the mic thread every 10 ms.
	pub fx: Arc<Mutex<MicFx>>,
	voices: Arc<Mutex<Vec<Sender<VoiceCmd>>>>,
	pub levels: Arc<Levels>,
	pub status: Arc<Mutex<EngineStatus>>,
	/// key -> number of outputs still playing it (for the UI)
	pub playing: Arc<Mutex<HashMap<String, usize>>>,
}

pub fn is_cable_input(name: &str) -> bool {
	let n = name.to_lowercase();
	n.contains("cable input") || n.starts_with("cable in ")
}
pub fn is_cable(name: &str) -> bool {
	let n = name.to_lowercase();
	n.contains("cable input") || n.contains("cable output") || n.contains("vb-audio")
}

pub fn device_names() -> (Vec<String>, Vec<String>) {
	let host = cpal::default_host();
	#[allow(deprecated)]
	let inputs = host.input_devices().map(|d| d.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
	#[allow(deprecated)]
	let outputs = host.output_devices().map(|d| d.filter_map(|d| d.name().ok()).collect()).unwrap_or_default();
	(inputs, outputs)
}

impl Engine {
	pub fn new() -> Self {
		let (ctl, rx) = channel::<Ctl>();
		let levels = Arc::new(Levels::default());
		let status = Arc::new(Mutex::new(EngineStatus::default()));
		let voices = Arc::new(Mutex::new(Vec::new()));
		let playing = Arc::new(Mutex::new(HashMap::new()));
		let fx = Arc::new(Mutex::new(MicFx::default()));
		{
			let (levels, status, voices, playing, fx) = (levels.clone(), status.clone(), voices.clone(), playing.clone(), fx.clone());
			std::thread::Builder::new()
				.name("orvo-soundboard".into())
				.spawn(move || audio_thread(rx, levels, status, voices, playing, fx))
				.expect("soundboard thread");
		}
		Engine {
			ctl,
			fx,
			voices,
			levels,
			status,
			playing,
		}
	}

	pub fn start(&self, routing: Routing) {
		let _ = self.ctl.send(Ctl::Start(routing));
	}
	pub fn stop(&self) {
		let _ = self.ctl.send(Ctl::Stop);
	}
	pub fn send(&self, cmd: VoiceCmd) {
		if let VoiceCmd::Play { key, .. } = &cmd {
			let outputs = self.voices.lock().unwrap().len();
			if outputs > 0 {
				*self.playing.lock().unwrap().entry(key.clone()).or_default() += outputs;
			}
		}
		for tx in self.voices.lock().unwrap().iter() {
			let _ = tx.send(cmd.clone());
		}
	}
}

fn audio_thread(
	rx: Receiver<Ctl>,
	levels: Arc<Levels>,
	status: Arc<Mutex<EngineStatus>>,
	voices: Arc<Mutex<Vec<Sender<VoiceCmd>>>>,
	playing: Arc<Mutex<HashMap<String, usize>>>,
	fx: Arc<Mutex<MicFx>>,
) {
	let mut streams: Vec<cpal::Stream> = Vec::new();
	while let Ok(ctl) = rx.recv() {
		// Always tear down first: device changes rebuild everything.
		streams.clear();
		voices.lock().unwrap().clear();
		playing.lock().unwrap().clear();
		match ctl {
			Ctl::Stop => {
				*status.lock().unwrap() = EngineStatus::default();
			}
			Ctl::Start(routing) => {
				let mut st = EngineStatus::default();
				match open_all(&routing, &levels, &voices, &playing, &fx, &mut st) {
					Ok(s) => {
						streams = s;
						st.running = !streams.is_empty();
					}
					Err(e) => st.error = Some(e.to_string()),
				}
				*status.lock().unwrap() = st;
			}
		}
	}
}

#[allow(deprecated)]
fn find(devices: impl Iterator<Item = cpal::Device>, pred: impl Fn(&str) -> bool) -> Option<cpal::Device> {
	devices.into_iter().find(|d| d.name().map(|n| pred(&n)).unwrap_or(false))
}

#[allow(deprecated)]
fn open_all(
	routing: &Routing,
	levels: &Arc<Levels>,
	voices: &Arc<Mutex<Vec<Sender<VoiceCmd>>>>,
	playing: &Arc<Mutex<HashMap<String, usize>>>,
	fx: &Arc<Mutex<MicFx>>,
	st: &mut EngineStatus,
) -> anyhow::Result<Vec<cpal::Stream>> {
	let host = cpal::default_host();
	let mut streams = Vec::new();

	// Virtual mic (VB-Cable): mic + sounds.
	let cable = find(host.output_devices()?, is_cable_input);
	st.cable = cable.is_some();
	st.cable_name = cable.as_ref().and_then(|d| d.name().ok());

	let mut monitor_mic = None;
	if let Some(cable) = cable {
		// Your real mic, never the cable itself (that would echo forever).
		let mic = match &routing.mic {
			Some(name) => find(host.input_devices()?, |n| n == name),
			None => host.default_input_device().filter(|d| !d.name().map(|n| is_cable(&n)).unwrap_or(true)),
		};
		let mic_ring = match mic {
			Some(mic) => {
				st.mic_name = mic.name().ok();
				match open_mic(&mic, fx.clone(), levels.clone()) {
					Ok((stream, cons, listen)) => {
						stream.play()?;
						streams.push(stream);
						monitor_mic = Some((listen, MIC_RATE));
						Some((cons, MIC_RATE))
					}
					Err(e) => {
						log::warn!("soundboard: mic failed: {e:#}");
						st.error = Some(format!("Couldn't open your mic: {e}"));
						None
					}
				}
			}
			None => None,
		};
		let (tx, rx) = channel();
		let stream = open_output(&cable, rx, mic_ring, levels.clone(), true, playing.clone())?;
		stream.play()?;
		streams.push(stream);
		voices.lock().unwrap().push(tx);
	}

	// Your headphones: sounds only.
	let monitor = match &routing.monitor {
		Some(name) => find(host.output_devices()?, |n| n == name),
		None => host.default_output_device(),
	}
	.filter(|d| !d.name().map(|n| is_cable_input(&n)).unwrap_or(false));
	if let Some(monitor) = monitor {
		st.monitor_name = monitor.name().ok();
		let (tx, rx) = channel();
		match open_output(&monitor, rx, monitor_mic, levels.clone(), false, playing.clone()) {
			Ok(stream) => {
				stream.play()?;
				streams.push(stream);
				voices.lock().unwrap().push(tx);
			}
			Err(e) => log::warn!("soundboard: headphones failed: {e:#}"),
		}
	}
	Ok(streams)
}

type MicCons = ringbuf::HeapCons<f32>;
type MicProd = ringbuf::HeapProd<f32>;

/// The mic is cleaned up at 48 kHz (what RNNoise needs) whatever rate the mic runs at.
pub const MIC_RATE: u32 = 48_000;

/// Opens the mic. Returns its stream and two queues of cleaned-up 48 kHz voice:
/// one for the virtual mic, one for "hear myself".
fn open_mic(device: &cpal::Device, fx: Arc<Mutex<MicFx>>, levels: Arc<Levels>) -> anyhow::Result<(cpal::Stream, MicCons, MicCons)> {
	let config = device.default_input_config()?;
	let rate = config.sample_rate().0;
	let channels = config.channels() as usize;
	// Half a second of headroom; the readers keep the delay short.
	let (prod, cons) = HeapRb::<f32>::new(MIC_RATE as usize / 2).split();
	let (listen_prod, listen_cons) = HeapRb::<f32>::new(MIC_RATE as usize / 2).split();
	let chain = VoiceChain::new(rate, fx, levels, prod, listen_prod);
	let stream = match config.sample_format() {
		cpal::SampleFormat::F32 => build_input::<f32>(device, &config.into(), channels, chain)?,
		cpal::SampleFormat::I16 => build_input::<i16>(device, &config.into(), channels, chain)?,
		cpal::SampleFormat::I32 => build_input::<i32>(device, &config.into(), channels, chain)?,
		cpal::SampleFormat::U16 => build_input::<u16>(device, &config.into(), channels, chain)?,
		other => anyhow::bail!("unsupported mic format {other:?}"),
	};
	Ok((stream, cons, listen_cons))
}

/// Mic thread side: mono → 48 kHz → 10 ms frames → voice cleanup → queues.
pub struct VoiceChain {
	step: f64,
	pos: f64,
	prev: f32,
	frame: [f32; FRAME],
	filled: usize,
	proc: Processor,
	fx: Arc<Mutex<MicFx>>,
	levels: Arc<Levels>,
	prod: MicProd,
	listen: MicProd,
}

impl VoiceChain {
	fn new(rate: u32, fx: Arc<Mutex<MicFx>>, levels: Arc<Levels>, prod: MicProd, listen: MicProd) -> Self {
		let start = *fx.lock().unwrap();
		VoiceChain {
			step: rate as f64 / MIC_RATE as f64,
			pos: 0.0,
			prev: 0.0,
			frame: [0.0; FRAME],
			filled: 0,
			proc: Processor::new(start),
			fx,
			levels,
			prod,
			listen,
		}
	}

	fn push(&mut self, s: f32) {
		// linear resampling to 48 kHz
		while self.pos <= 1.0 {
			let v = self.prev + (s - self.prev) * self.pos as f32;
			self.frame[self.filled] = v;
			self.filled += 1;
			if self.filled == FRAME {
				self.flush();
			}
			self.pos += self.step;
		}
		self.pos -= 1.0;
		self.prev = s;
	}

	fn flush(&mut self) {
		self.filled = 0;
		if let Ok(fx) = self.fx.try_lock() {
			let fx = *fx;
			self.proc.set(fx);
		}
		self.proc.process(&mut self.frame);
		let m = self.proc.meter;
		store(&self.levels.meter_level, m.level_db);
		store(&self.levels.meter_out, m.out_db);
		store(&self.levels.meter_voice, m.voice);
		self.levels.meter_gate.store(m.gate_open, Ordering::Relaxed);
		let listening = self.levels.listen_self.load(Ordering::Relaxed);
		for &v in self.frame.iter() {
			let _ = self.prod.try_push(v);
			if listening {
				let _ = self.listen.try_push(v);
			}
		}
	}
}

fn build_input<T>(device: &cpal::Device, config: &cpal::StreamConfig, channels: usize, mut chain: VoiceChain) -> anyhow::Result<cpal::Stream>
where
	T: SizedSample,
	f32: FromSample<T>,
{
	Ok(device.build_input_stream(
		config,
		move |data: &[T], _| {
			for frame in data.chunks(channels) {
				let mono = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
				chain.push(mono);
			}
		},
		|e| log::warn!("soundboard mic stream: {e}"),
		None,
	)?)
}

fn open_output(
	device: &cpal::Device,
	rx: Receiver<VoiceCmd>,
	mic: Option<(MicCons, u32)>,
	levels: Arc<Levels>,
	is_virtual: bool,
	playing: Arc<Mutex<HashMap<String, usize>>>,
) -> anyhow::Result<cpal::Stream> {
	let config = device.default_output_config()?;
	let mixer = Mixer::new(config.sample_rate().0, config.channels() as usize, rx, mic, levels, is_virtual, playing);
	Ok(match config.sample_format() {
		cpal::SampleFormat::F32 => build_output::<f32>(device, &config.into(), mixer)?,
		cpal::SampleFormat::I16 => build_output::<i16>(device, &config.into(), mixer)?,
		cpal::SampleFormat::I32 => build_output::<i32>(device, &config.into(), mixer)?,
		cpal::SampleFormat::U16 => build_output::<u16>(device, &config.into(), mixer)?,
		other => anyhow::bail!("unsupported output format {other:?}"),
	})
}

fn build_output<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut mixer: Mixer) -> anyhow::Result<cpal::Stream>
where
	T: SizedSample + FromSample<f32>,
{
	Ok(device.build_output_stream(config, move |data: &mut [T], _| mixer.fill(data), |e| log::warn!("soundboard output stream: {e}"), None)?)
}

struct Voice {
	key: String,
	clip: Clip,
	pos: f64,
	gain: f32,
	looped: bool,
}

pub struct Mixer {
	channels: usize,
	step: f64,
	rx: Receiver<VoiceCmd>,
	voices: Vec<Voice>,
	mic: Option<MicReader>,
	levels: Arc<Levels>,
	is_virtual: bool,
	playing: Arc<Mutex<HashMap<String, usize>>>,
	finished: Vec<String>,
}

impl Mixer {
	fn new(rate: u32, channels: usize, rx: Receiver<VoiceCmd>, mic: Option<(MicCons, u32)>, levels: Arc<Levels>, is_virtual: bool, playing: Arc<Mutex<HashMap<String, usize>>>) -> Self {
		Mixer {
			channels: channels.max(1),
			step: CLIP_RATE as f64 / rate as f64,
			rx,
			voices: Vec::new(),
			mic: mic.map(|(cons, mic_rate)| MicReader::new(cons, mic_rate, rate)),
			levels,
			is_virtual,
			playing,
			finished: Vec::new(),
		}
	}

	fn fill<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
		while let Ok(cmd) = self.rx.try_recv() {
			match cmd {
				VoiceCmd::Play { key, clip, gain, looped } => self.voices.push(Voice { key, clip, pos: 0.0, gain, looped }),
				VoiceCmd::Stop(key) => self.voices.retain(|v| {
					let keep = v.key != key;
					if !keep {
						self.finished.push(v.key.clone());
					}
					keep
				}),
				VoiceCmd::StopAll => {
					self.finished.extend(self.voices.drain(..).map(|v| v.key));
				}
			}
		}

		let (sound_gain, mic_gain, mic_on) = if self.is_virtual {
			(load(&self.levels.sound_gain), load(&self.levels.mic_gain), self.levels.mic_on.load(Ordering::Relaxed))
		} else {
			let on = self.levels.monitor_on.load(Ordering::Relaxed);
			(
				if on { load(&self.levels.monitor_gain) } else { 0.0 },
				load(&self.levels.mic_gain),
				self.levels.listen_self.load(Ordering::Relaxed),
			)
		};

		for frame in data.chunks_mut(self.channels) {
			let (mut l, mut r) = (0.0f32, 0.0f32);
			for v in self.voices.iter_mut() {
				let frames = v.clip.len() / 2;
				if frames == 0 {
					continue;
				}
				let mut i = v.pos as usize;
				if i >= frames {
					if v.looped {
						v.pos -= frames as f64;
						i = v.pos as usize;
					} else {
						continue;
					}
				}
				let t = (v.pos - i as f64) as f32;
				let j = if i + 1 < frames {
					i + 1
				} else if v.looped {
					0
				} else {
					i
				};
				l += (v.clip[i * 2] * (1.0 - t) + v.clip[j * 2] * t) * v.gain;
				r += (v.clip[i * 2 + 1] * (1.0 - t) + v.clip[j * 2 + 1] * t) * v.gain;
				v.pos += self.step;
			}
			l *= sound_gain;
			r *= sound_gain;
			if let Some(mic) = &mut self.mic {
				let m = mic.next();
				if mic_on {
					l += m * mic_gain;
					r += m * mic_gain;
				}
			}
			// soft limit so stacked sounds never clip harshly
			let (l, r) = (soft(l), soft(r));
			match frame.len() {
				1 => frame[0] = T::from_sample((l + r) * 0.5),
				_ => {
					frame[0] = T::from_sample(l);
					frame[1] = T::from_sample(r);
					for s in frame.iter_mut().skip(2) {
						*s = T::from_sample(0.0f32);
					}
				}
			}
		}

		let before = self.voices.len();
		let finished = &mut self.finished;
		self.voices.retain(|v| {
			let alive = v.looped || (v.pos as usize) < v.clip.len() / 2;
			if !alive {
				finished.push(v.key.clone());
			}
			alive
		});
		if (before != self.voices.len() || !self.finished.is_empty())
			&& let Ok(mut p) = self.playing.try_lock()
		{
			for key in self.finished.drain(..) {
				if let Some(n) = p.get_mut(&key) {
					*n = n.saturating_sub(1);
					if *n == 0 {
						p.remove(&key);
					}
				}
			}
		}
	}
}

fn soft(x: f32) -> f32 {
	if x.abs() <= 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((x.abs() - 0.9) / 0.1).tanh()) }
}

/// Reads the mic ring at the output's rate, keeping the delay short (about 10–60 ms).
struct MicReader {
	cons: MicCons,
	step: f64,
	frac: f64,
	a: f32,
	b: f32,
	priming: bool,
	low: usize,
	high: usize,
}

impl MicReader {
	fn new(cons: MicCons, mic_rate: u32, out_rate: u32) -> Self {
		MicReader {
			cons,
			step: mic_rate as f64 / out_rate as f64,
			frac: 0.0,
			a: 0.0,
			b: 0.0,
			priming: true,
			low: mic_rate as usize / 100,
			high: mic_rate as usize * 6 / 100,
		}
	}
	fn next(&mut self) -> f32 {
		let queued = self.cons.occupied_len();
		if self.priming {
			if queued < self.low * 2 {
				return 0.0;
			}
			self.priming = false;
		}
		// Too far behind (the clocks drift): skip ahead.
		if queued > self.high {
			let skip = queued - self.low * 2;
			self.cons.skip(skip);
		}
		self.frac += self.step;
		while self.frac >= 1.0 {
			match self.cons.try_pop() {
				Some(s) => {
					self.a = self.b;
					self.b = s;
				}
				None => {
					self.priming = true;
					self.frac = 0.0;
					return 0.0;
				}
			}
			self.frac -= 1.0;
		}
		self.a + (self.b - self.a) * self.frac as f32
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mixes_sound_and_mic_then_finishes() {
		let levels = Arc::new(Levels::default());
		levels.mic_on.store(true, Ordering::Relaxed);
		store(&levels.mic_gain, 1.0);
		store(&levels.sound_gain, 1.0);
		let (mut prod, cons) = HeapRb::<f32>::new(48_000).split();
		for _ in 0..4_000 {
			let _ = prod.try_push(0.25);
		}
		let (tx, rx) = channel();
		let playing = Arc::new(Mutex::new(HashMap::new()));
		playing.lock().unwrap().insert("a".to_string(), 1);
		// output at 44.1 kHz, mic at 48 kHz: both get resampled
		let mut m = Mixer::new(44_100, 2, rx, Some((cons, 48_000)), levels, true, playing.clone());
		let clip: Clip = Arc::new(vec![0.5; 48_000 * 2 / 10]); // 0.1 s
		tx.send(VoiceCmd::Play {
			key: "a".into(),
			clip,
			gain: 1.0,
			looped: false,
		})
		.unwrap();
		let mut buf = vec![0.0f32; 2 * 441]; // 10 ms
		m.fill(&mut buf);
		let last = buf[buf.len() - 2];
		assert!(last > 0.6 && last < 0.8, "sound + mic expected, got {last}");
		for _ in 0..20 {
			m.fill(&mut buf);
		}
		assert!(m.voices.is_empty(), "sound should have finished");
		assert!(playing.lock().unwrap().is_empty(), "playing list should be cleared");
	}
}
