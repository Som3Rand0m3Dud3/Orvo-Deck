//! Orvo's effects library, shared by the voice, the soundboard, the output and single sounds.
//!
//! An effect is `{ id, kind, on, params }`; params are plain numbers in real units (dB, ms,
//! semitones…) with defaults, so the app can show any of them as a slider.

use std::collections::HashMap;
use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EffectSpec {
	pub id: String,
	pub kind: String,
	#[serde(default = "yes")]
	pub on: bool,
	#[serde(default)]
	pub params: HashMap<String, f32>,
}
fn yes() -> bool {
	true
}

impl EffectSpec {
	pub fn new(kind: &str, params: &[(&str, f32)]) -> Self {
		EffectSpec {
			id: kind.to_owned(),
			kind: kind.to_owned(),
			on: true,
			params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
		}
	}
	fn p(&self, key: &str, default: f32) -> f32 {
		self.params.get(key).copied().filter(|v| v.is_finite()).unwrap_or(default)
	}
}

// ---------------------------------------------------------------- building blocks

#[derive(Clone, Copy, Default)]
pub struct Biquad {
	b0: f32,
	b1: f32,
	b2: f32,
	a1: f32,
	a2: f32,
	z1: f32,
	z2: f32,
}

impl Biquad {
	fn set(&mut self, b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) {
		self.b0 = b0 / a0;
		self.b1 = b1 / a0;
		self.b2 = b2 / a0;
		self.a1 = a1 / a0;
		self.a2 = a2 / a0;
	}
	fn w(f: f32, sr: f32) -> (f32, f32) {
		let w = 2.0 * PI * (f / sr).clamp(1e-5, 0.49);
		w.sin_cos()
	}
	pub fn highpass(&mut self, f: f32, q: f32, sr: f32) {
		let (s, c) = Self::w(f, sr);
		let al = s / (2.0 * q);
		self.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al);
	}
	pub fn lowpass(&mut self, f: f32, q: f32, sr: f32) {
		let (s, c) = Self::w(f, sr);
		let al = s / (2.0 * q);
		self.set((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al);
	}
	pub fn peak(&mut self, f: f32, q: f32, db: f32, sr: f32) {
		let a = 10f32.powf(db / 40.0);
		let (s, c) = Self::w(f, sr);
		let al = s / (2.0 * q);
		self.set(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a);
	}
	pub fn shelf(&mut self, f: f32, db: f32, high: bool, sr: f32) {
		let a = 10f32.powf(db / 40.0);
		let (s, c) = Self::w(f, sr);
		let al = s / 2.0 * std::f32::consts::SQRT_2;
		let sa = 2.0 * a.sqrt() * al;
		if high {
			self.set(
				a * ((a + 1.0) + (a - 1.0) * c + sa),
				-2.0 * a * ((a - 1.0) + (a + 1.0) * c),
				a * ((a + 1.0) + (a - 1.0) * c - sa),
				(a + 1.0) - (a - 1.0) * c + sa,
				2.0 * ((a - 1.0) - (a + 1.0) * c),
				(a + 1.0) - (a - 1.0) * c - sa,
			);
		} else {
			self.set(
				a * ((a + 1.0) - (a - 1.0) * c + sa),
				2.0 * a * ((a - 1.0) - (a + 1.0) * c),
				a * ((a + 1.0) - (a - 1.0) * c - sa),
				(a + 1.0) + (a - 1.0) * c + sa,
				-2.0 * ((a - 1.0) + (a + 1.0) * c),
				(a + 1.0) + (a - 1.0) * c - sa,
			);
		}
	}
	#[inline]
	pub fn run(&mut self, x: f32) -> f32 {
		let y = self.b0 * x + self.z1;
		self.z1 = self.b1 * x - self.a1 * y + self.z2;
		self.z2 = self.b2 * x - self.a2 * y;
		y
	}
}

fn undb(d: f32) -> f32 {
	10f32.powf(d / 20.0)
}
fn db(x: f32) -> f32 {
	20.0 * x.max(1e-9).log10()
}
fn coef(ms: f32, sr: f32) -> f32 {
	(-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

/// A delay line with fractional reads.
#[derive(Clone)]
struct Delay {
	buf: Vec<f32>,
	pos: usize,
}
impl Delay {
	fn new(len: usize) -> Self {
		Delay { buf: vec![0.0; len.max(4)], pos: 0 }
	}
	fn push(&mut self, x: f32) {
		self.buf[self.pos] = x;
		self.pos = (self.pos + 1) % self.buf.len();
	}
	/// `d` samples ago (1 = the last pushed).
	fn read(&self, d: f32) -> f32 {
		let n = self.buf.len();
		let d = d.clamp(1.0, (n - 2) as f32);
		let i = d.floor() as usize;
		let t = d - i as f32;
		let a = self.buf[(self.pos + n - i) % n];
		let b = self.buf[(self.pos + n - i - 1) % n];
		a + (b - a) * t
	}
}

// ---------------------------------------------------------------- reverb (Freeverb style)

#[derive(Clone)]
struct Comb {
	buf: Vec<f32>,
	i: usize,
	store: f32,
}
impl Comb {
	fn run(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
		let y = self.buf[self.i];
		self.store = y * (1.0 - damp) + self.store * damp;
		self.buf[self.i] = x + self.store * feedback;
		self.i = (self.i + 1) % self.buf.len();
		y
	}
}
#[derive(Clone)]
struct Allpass {
	buf: Vec<f32>,
	i: usize,
}
impl Allpass {
	fn run(&mut self, x: f32) -> f32 {
		let b = self.buf[self.i];
		let y = b - x;
		self.buf[self.i] = x + b * 0.5;
		self.i = (self.i + 1) % self.buf.len();
		y
	}
}
struct Reverb {
	combs: [Vec<Comb>; 2],
	alls: [Vec<Allpass>; 2],
}
impl Reverb {
	fn new(sr: f32) -> Self {
		let s = sr / 44_100.0;
		let comb = |n: usize| Comb {
			buf: vec![0.0; ((n as f32 * s) as usize).max(8)],
			i: 0,
			store: 0.0,
		};
		let all = |n: usize| Allpass {
			buf: vec![0.0; ((n as f32 * s) as usize).max(8)],
			i: 0,
		};
		const C: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
		const A: [usize; 4] = [556, 441, 341, 225];
		Reverb {
			combs: [C.iter().map(|&n| comb(n)).collect(), C.iter().map(|&n| comb(n + 23)).collect()],
			alls: [A.iter().map(|&n| all(n)).collect(), A.iter().map(|&n| all(n + 23)).collect()],
		}
	}
	fn run(&mut self, x: f32, ch: usize, size: f32, damp: f32) -> f32 {
		let fb = 0.7 + 0.28 * size;
		let input = x * 0.015;
		let mut out = 0.0;
		for c in self.combs[ch].iter_mut() {
			out += c.run(input, fb, damp * 0.4);
		}
		for a in self.alls[ch].iter_mut() {
			out = a.run(out);
		}
		out
	}
}

// ---------------------------------------------------------------- the processors

/// Pitch / formant shifter (Signalsmith Stretch). The wrapper holds a raw pointer;
/// each instance is only ever used from one audio thread at a time.
struct Shifter(signalsmith_stretch::Stretch);
unsafe impl Send for Shifter {}

enum State {
	Pitch {
		st: Box<Shifter>,
		ch: usize,
		inb: Vec<f32>,
		outb: Vec<f32>,
		last: (f32, f32),
	},
	Echo {
		lines: Vec<Delay>,
	},
	Reverb(Box<Reverb>),
	Eq {
		f: Vec<[Biquad; 3]>,
		last: (f32, f32, f32),
	},
	Filter {
		f: Vec<[Biquad; 2]>,
		last: (f32, f32),
	},
	Distortion {
		tone: Vec<Biquad>,
		last: f32,
	},
	Robot {
		phase: f32,
	},
	Chorus {
		lines: Vec<Delay>,
		phase: f32,
	},
	Compressor {
		env: f32,
	},
	Limiter {
		g: f32,
	},
	Leveler {
		ms: f32,
		g: f32,
	},
}

pub struct Effect {
	pub spec: EffectSpec,
	sr: f32,
	state: State,
}

#[allow(dead_code)]
pub const KINDS: [&str; 11] = ["pitch", "echo", "reverb", "eq", "filter", "distortion", "robot", "chorus", "compressor", "limiter", "leveler"];

impl Effect {
	pub fn new(spec: EffectSpec, sr: f32, channels: usize) -> Option<Self> {
		let ch = channels.max(1);
		let state = match spec.kind.as_str() {
			"pitch" => {
				// 1536 / 384 at 48 kHz: about 32 ms of delay, good quality for voices
				let scale = sr / 48_000.0;
				let st = signalsmith_stretch::Stretch::new(ch as u32, (1536.0 * scale) as usize, (384.0 * scale) as usize);
				State::Pitch {
					st: Box::new(Shifter(st)),
					ch,
					inb: Vec::new(),
					outb: Vec::new(),
					last: (f32::NAN, f32::NAN),
				}
			}
			"echo" => State::Echo {
				lines: (0..ch).map(|_| Delay::new((sr * 1.05) as usize)).collect(),
			},
			"reverb" => State::Reverb(Box::new(Reverb::new(sr))),
			"eq" => State::Eq {
				f: vec![[Biquad::default(); 3]; ch],
				last: (f32::NAN, 0.0, 0.0),
			},
			"filter" => State::Filter {
				f: vec![[Biquad::default(); 2]; ch],
				last: (f32::NAN, 0.0),
			},
			"distortion" => State::Distortion {
				tone: vec![Biquad::default(); ch],
				last: f32::NAN,
			},
			"robot" => State::Robot { phase: 0.0 },
			"chorus" => State::Chorus {
				lines: (0..ch).map(|_| Delay::new((sr * 0.05) as usize)).collect(),
				phase: 0.0,
			},
			"compressor" => State::Compressor { env: 0.0 },
			"limiter" => State::Limiter { g: 1.0 },
			"leveler" => State::Leveler { ms: 0.0, g: 1.0 },
			_ => return None,
		};
		Some(Effect { spec, sr, state })
	}

	/// Processes interleaved audio in place.
	pub fn process(&mut self, buf: &mut [f32], channels: usize) {
		if !self.spec.on {
			return;
		}
		let ch = channels.max(1);
		let sr = self.sr;
		let s = &self.spec;
		match &mut self.state {
			State::Pitch { st, ch: c, inb, outb, last } => {
				let semis = s.p("semitones", 0.0).clamp(-24.0, 24.0);
				let formant = s.p("formant", 0.0).clamp(-24.0, 24.0);
				if (semis, formant) != *last {
					*last = (semis, formant);
					st.0.set_transpose_factor_semitones(semis, None);
					// formant 0 keeps the voice's natural size while the pitch moves
					st.0.set_formant_factor_semitones(formant, true);
				}
				if *c != ch {
					return;
				}
				inb.clear();
				inb.extend_from_slice(buf);
				outb.resize(buf.len(), 0.0);
				st.0.process(&inb[..], &mut outb[..]);
				let mix = s.p("mix", 1.0).clamp(0.0, 1.0);
				for (x, y) in buf.iter_mut().zip(outb.iter()) {
					*x = if mix >= 1.0 { *y } else { *y * mix + *x * (1.0 - mix) };
				}
			}
			State::Echo { lines } => {
				let d = (s.p("time_ms", 250.0).clamp(10.0, 1000.0) * 0.001 * sr).max(1.0);
				let fb = s.p("feedback", 0.35).clamp(0.0, 0.92);
				let mix = s.p("mix", 0.3).clamp(0.0, 1.0);
				let last_line = lines.len() - 1;
				for frame in buf.chunks_mut(ch) {
					for (c, x) in frame.iter_mut().enumerate() {
						let line = &mut lines[c.min(last_line)];
						let wet = line.read(d);
						line.push(*x + wet * fb);
						*x += wet * mix;
					}
				}
			}
			State::Reverb(r) => {
				let size = s.p("size", 0.5).clamp(0.0, 1.0);
				let damp = s.p("damping", 0.5).clamp(0.0, 1.0);
				let mix = s.p("mix", 0.25).clamp(0.0, 1.0);
				for frame in buf.chunks_mut(ch) {
					let mono = frame.iter().sum::<f32>() / frame.len() as f32;
					for (c, x) in frame.iter_mut().enumerate() {
						let wet = r.run(mono, c.min(1), size, damp);
						*x = *x * (1.0 - mix * 0.5) + wet * mix * 3.0;
					}
				}
			}
			State::Eq { f, last } => {
				let want = (s.p("bass_db", 0.0).clamp(-15.0, 15.0), s.p("mid_db", 0.0).clamp(-15.0, 15.0), s.p("treble_db", 0.0).clamp(-15.0, 15.0));
				if want != *last {
					*last = want;
					for b in f.iter_mut() {
						b[0].shelf(150.0, want.0, false, sr);
						b[1].peak(1000.0, 0.8, want.1, sr);
						b[2].shelf(5000.0, want.2, true, sr);
					}
				}
				let lastf = f.len() - 1;
				for frame in buf.chunks_mut(ch) {
					for (c, x) in frame.iter_mut().enumerate() {
						let b = &mut f[c.min(lastf)];
						let y = b[0].run(*x);
						let y = b[1].run(y);
						*x = b[2].run(y);
					}
				}
			}
			State::Filter { f, last } => {
				let want = (s.p("lowcut_hz", 20.0).clamp(20.0, 3000.0), s.p("highcut_hz", 20_000.0).clamp(500.0, 20_000.0));
				if want != *last {
					*last = want;
					for b in f.iter_mut() {
						b[0].highpass(want.0, 0.707, sr);
						b[1].lowpass(want.1.min(sr * 0.45), 0.707, sr);
					}
				}
				let lastf = f.len() - 1;
				for frame in buf.chunks_mut(ch) {
					for (c, x) in frame.iter_mut().enumerate() {
						let b = &mut f[c.min(lastf)];
						let y = b[0].run(*x);
						*x = b[1].run(y);
					}
				}
			}
			State::Distortion { tone, last } => {
				let drive = s.p("drive", 0.4).clamp(0.0, 1.0);
				let t = s.p("tone", 0.5).clamp(0.0, 1.0);
				let mix = s.p("mix", 1.0).clamp(0.0, 1.0);
				if t != *last {
					*last = t;
					for b in tone.iter_mut() {
						b.lowpass(1500.0 * 8f32.powf(t), 0.707, sr);
					}
				}
				let k = 1.0 + drive * 30.0;
				let norm = 1.0 / k.tanh();
				let lastt = tone.len() - 1;
				for frame in buf.chunks_mut(ch) {
					for (c, x) in frame.iter_mut().enumerate() {
						let y = tone[c.min(lastt)].run((*x * k).tanh() * norm * 0.6);
						*x = y * mix + *x * (1.0 - mix);
					}
				}
			}
			State::Robot { phase } => {
				let f = s.p("freq_hz", 60.0).clamp(10.0, 1000.0);
				let mix = s.p("mix", 0.8).clamp(0.0, 1.0);
				let step = 2.0 * PI * f / sr;
				for frame in buf.chunks_mut(ch) {
					let m = phase.sin();
					*phase = (*phase + step) % (2.0 * PI);
					for x in frame.iter_mut() {
						*x = *x * m * mix + *x * (1.0 - mix);
					}
				}
			}
			State::Chorus { lines, phase } => {
				let rate = s.p("rate_hz", 0.8).clamp(0.05, 8.0);
				let depth = s.p("depth", 0.5).clamp(0.0, 1.0);
				let mix = s.p("mix", 0.5).clamp(0.0, 1.0);
				let flange = s.p("flanger", 0.0) >= 0.5;
				let (base, swing, fb) = if flange { (0.002, 0.002, 0.6) } else { (0.015, 0.007, 0.0) };
				let step = 2.0 * PI * rate / sr;
				let last_line = lines.len() - 1;
				for frame in buf.chunks_mut(ch) {
					for (c, x) in frame.iter_mut().enumerate() {
						let ph = *phase + c as f32 * 0.5 * PI; // stereo spread
						let d = (base + swing * depth * ph.sin()) * sr;
						let line = &mut lines[c.min(last_line)];
						let wet = line.read(d.max(1.0));
						line.push(*x + wet * fb);
						*x = *x * (1.0 - mix * 0.5) + wet * mix;
					}
					*phase = (*phase + step) % (2.0 * PI);
				}
			}
			State::Compressor { env } => {
				let amount = s.p("amount", 0.5).clamp(0.0, 1.0);
				let thr = -14.0 - 16.0 * amount;
				let ratio = 1.0 + 7.0 * amount;
				let makeup = undb((-thr) * (1.0 - 1.0 / ratio) * 0.45);
				let (a, r) = (coef(5.0, sr), coef(150.0, sr));
				for frame in buf.chunks_mut(ch) {
					let peak = frame.iter().fold(0f32, |m, x| m.max(x.abs()));
					*env = peak + (*env - peak) * if peak > *env { a } else { r };
					let over = db(*env) - thr;
					let g = if over > 0.0 { undb(-over * (1.0 - 1.0 / ratio)) } else { 1.0 } * makeup;
					for x in frame.iter_mut() {
						*x *= g;
					}
				}
			}
			State::Limiter { g } => {
				let ceil = undb(s.p("ceiling_db", -1.0).clamp(-24.0, 0.0));
				let r = coef(60.0, sr);
				for frame in buf.chunks_mut(ch) {
					let peak = frame.iter().fold(0f32, |m, x| m.max(x.abs()));
					let need = if peak > ceil { ceil / peak } else { 1.0 };
					*g = if need < *g { need } else { need + (*g - need) * r };
					for x in frame.iter_mut() {
						*x *= *g;
					}
				}
			}
			State::Leveler { ms, g } => {
				// slow automatic volume: brings quiet and loud material toward one loudness
				let target = undb(-30.0 + 16.0 * s.p("target", 0.5).clamp(0.0, 1.0));
				let speed = s.p("speed", 0.3).clamp(0.0, 1.0);
				let avg = coef(400.0, sr);
				let move_c = coef(2000.0 - 1700.0 * speed, sr);
				for frame in buf.chunks_mut(ch) {
					let p = frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32;
					*ms = p + (*ms - p) * avg;
					let rms = ms.sqrt();
					if rms > 0.003 {
						let want = (target / rms).clamp(undb(-12.0), undb(12.0));
						*g = want + (*g - want) * move_c;
					}
					for x in frame.iter_mut() {
						*x *= *g;
					}
				}
			}
		}
	}
}

/// An ordered list of effects that follows its spec list, keeping each effect's state
/// (echo tails, reverb rooms…) when only its sliders move.
pub struct Chain {
	sr: f32,
	ch: usize,
	pub effects: Vec<Effect>,
}

impl Chain {
	pub fn new(sr: f32, ch: usize) -> Self {
		Chain { sr, ch, effects: Vec::new() }
	}
	pub fn update(&mut self, specs: &[EffectSpec]) {
		let mut old: Vec<Effect> = std::mem::take(&mut self.effects);
		for spec in specs {
			if let Some(i) = old.iter().position(|e| e.spec.id == spec.id && e.spec.kind == spec.kind) {
				let mut e = old.remove(i);
				e.spec = spec.clone();
				self.effects.push(e);
			} else if let Some(e) = Effect::new(spec.clone(), self.sr, self.ch) {
				self.effects.push(e);
			}
		}
	}
	pub fn process(&mut self, buf: &mut [f32]) {
		for e in self.effects.iter_mut() {
			e.process(buf, self.ch);
		}
	}
	pub fn is_empty(&self) -> bool {
		self.effects.iter().all(|e| !e.spec.on)
	}
}

// ---------------------------------------------------------------- voice changer presets

pub struct VoicePreset {
	pub id: &'static str,
	pub name: &'static str,
	pub hint: &'static str,
	pub chain: fn() -> Vec<EffectSpec>,
}

fn e(kind: &str, params: &[(&str, f32)]) -> EffectSpec {
	EffectSpec::new(kind, params)
}

pub const VOICE_PRESETS: &[VoicePreset] = &[
	VoicePreset {
		id: "off",
		name: "Off",
		hint: "Your voice, just cleaned up.",
		chain: Vec::new,
	},
	VoicePreset {
		id: "deep",
		name: "Deep",
		hint: "Lower and bigger.",
		chain: || vec![e("pitch", &[("semitones", -5.0), ("formant", -3.0)]), e("eq", &[("bass_db", 4.0), ("treble_db", -2.0)])],
	},
	VoicePreset {
		id: "demon",
		name: "Demon",
		hint: "Very low, gritty and huge.",
		chain: || {
			vec![
				e("pitch", &[("semitones", -9.0), ("formant", -6.0)]),
				e("distortion", &[("drive", 0.35), ("tone", 0.3), ("mix", 0.6)]),
				e("reverb", &[("size", 0.7), ("damping", 0.7), ("mix", 0.25)]),
			]
		},
	},
	VoicePreset {
		id: "chipmunk",
		name: "Chipmunk",
		hint: "High and tiny.",
		chain: || vec![e("pitch", &[("semitones", 7.0), ("formant", 5.0)])],
	},
	VoicePreset {
		id: "robot",
		name: "Robot",
		hint: "Metallic machine voice.",
		chain: || {
			vec![
				e("robot", &[("freq_hz", 60.0), ("mix", 0.8)]),
				e("filter", &[("lowcut_hz", 300.0), ("highcut_hz", 4500.0)]),
				e("echo", &[("time_ms", 120.0), ("feedback", 0.25), ("mix", 0.2)]),
			]
		},
	},
	VoicePreset {
		id: "radio",
		name: "Radio",
		hint: "Walkie-talkie.",
		chain: || {
			vec![
				e("filter", &[("lowcut_hz", 500.0), ("highcut_hz", 3200.0)]),
				e("distortion", &[("drive", 0.5), ("tone", 0.4), ("mix", 0.7)]),
				e("compressor", &[("amount", 0.7)]),
			]
		},
	},
	VoicePreset {
		id: "telephone",
		name: "Telephone",
		hint: "Old phone line.",
		chain: || {
			vec![
				e("filter", &[("lowcut_hz", 400.0), ("highcut_hz", 3400.0)]),
				e("distortion", &[("drive", 0.15), ("tone", 0.5), ("mix", 0.5)]),
			]
		},
	},
	VoicePreset {
		id: "megaphone",
		name: "Megaphone",
		hint: "Loud and honky.",
		chain: || {
			vec![
				e("filter", &[("lowcut_hz", 700.0), ("highcut_hz", 5000.0)]),
				e("eq", &[("mid_db", 8.0)]),
				e("distortion", &[("drive", 0.6), ("tone", 0.5), ("mix", 0.8)]),
			]
		},
	},
	VoicePreset {
		id: "cave",
		name: "Cave",
		hint: "Big stone room.",
		chain: || {
			vec![
				e("reverb", &[("size", 0.9), ("damping", 0.6), ("mix", 0.45)]),
				e("echo", &[("time_ms", 180.0), ("feedback", 0.3), ("mix", 0.15)]),
			]
		},
	},
	VoicePreset {
		id: "echo",
		name: "Echo",
		hint: "Repeats like a canyon.",
		chain: || vec![e("echo", &[("time_ms", 380.0), ("feedback", 0.45), ("mix", 0.4)])],
	},
	VoicePreset {
		id: "alien",
		name: "Alien",
		hint: "Wobbly and strange.",
		chain: || {
			vec![
				e("pitch", &[("semitones", 3.0), ("formant", 6.0)]),
				e("chorus", &[("rate_hz", 3.0), ("depth", 0.8), ("mix", 0.6), ("flanger", 1.0)]),
				e("robot", &[("freq_hz", 30.0), ("mix", 0.3)]),
			]
		},
	},
	VoicePreset {
		id: "underwater",
		name: "Underwater",
		hint: "Muffled and swimming.",
		chain: || {
			vec![
				e("filter", &[("lowcut_hz", 20.0), ("highcut_hz", 700.0)]),
				e("chorus", &[("rate_hz", 0.6), ("depth", 0.9), ("mix", 0.7)]),
				e("reverb", &[("size", 0.5), ("damping", 0.8), ("mix", 0.3)]),
			]
		},
	},
	VoicePreset {
		id: "ghost",
		name: "Ghost",
		hint: "Airy and haunting.",
		chain: || {
			vec![
				e("pitch", &[("semitones", 2.0), ("formant", 3.0), ("mix", 0.5)]),
				e("chorus", &[("rate_hz", 0.3), ("depth", 0.7), ("mix", 0.5)]),
				e("reverb", &[("size", 0.95), ("damping", 0.2), ("mix", 0.55)]),
			]
		},
	},
];

pub fn preset_chain(id: &str) -> Option<Vec<EffectSpec>> {
	VOICE_PRESETS.iter().find(|p| p.id == id).map(|p| (p.chain)())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tone(n: usize, amp: f32) -> Vec<f32> {
		(0..n).map(|i| (2.0 * PI * 220.0 * i as f32 / 48_000.0).sin() * amp).collect()
	}

	#[test]
	fn every_effect_and_preset_runs_and_stays_finite() {
		for kind in KINDS {
			for ch in [1usize, 2] {
				let mut e = Effect::new(EffectSpec::new(kind, &[]), 48_000.0, ch).unwrap();
				let mut buf: Vec<f32> = tone(4_800 * ch, 0.5);
				for _ in 0..5 {
					e.process(&mut buf, ch);
				}
				assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 4.0), "{kind} x{ch} blew up");
			}
		}
		for p in VOICE_PRESETS {
			let mut c = Chain::new(48_000.0, 1);
			c.update(&(p.chain)());
			let mut out_energy = 0.0;
			for _ in 0..40 {
				let mut f = tone(480, 0.3);
				c.process(&mut f);
				out_energy += f.iter().map(|x| x * x).sum::<f32>();
			}
			assert!(out_energy.is_finite() && out_energy > 0.0, "{} made silence or junk", p.id);
		}
	}

	#[test]
	fn chain_keeps_state_when_sliders_move() {
		let mut c = Chain::new(48_000.0, 1);
		let mut spec = EffectSpec::new("echo", &[("time_ms", 100.0), ("mix", 0.5)]);
		c.update(std::slice::from_ref(&spec));
		let mut f = tone(4_800, 0.5);
		c.process(&mut f);
		spec.params.insert("mix".into(), 0.6);
		c.update(std::slice::from_ref(&spec));
		// echo tail continues: silence in, sound out
		let mut silent = vec![0.0f32; 4_800];
		c.process(&mut silent);
		assert!(silent.iter().any(|x| x.abs() > 0.01));
	}
}
