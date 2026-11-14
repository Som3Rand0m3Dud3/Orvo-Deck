//! Voice cleanup for the user's mic (never the soundboard sounds):
//! rumble filter → RNNoise noise removal → breath reduction → noise gate → de-esser →
//! warmth / clarity EQ → compressor → limiter.
//!
//! Works on 48 kHz mono in 10 ms frames (480 samples). RNNoise adds one frame of delay;
//! the dry signal is delayed by the same amount so "strength" mixes cleanly.

use nnnoiseless::DenoiseState;
use serde::{Deserialize, Serialize};

pub const RATE: f32 = 48_000.0;
pub const FRAME: usize = DenoiseState::FRAME_SIZE; // 480

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MicFx {
	/// Cuts rumble below ~80 Hz.
	pub rumble: bool,
	/// Noise removal strength, 0 (off) – 1.
	pub denoise: f32,
	/// Turns down breaths and small sounds between words, 0 – 1 (up to −30 dB).
	pub breath: f32,
	pub gate: bool,
	/// Below this level (dBFS) the mic is muted.
	pub gate_db: f32,
	/// Softens sharp "s" sounds, 0 – 1.
	pub deess: f32,
	/// Low-end body, 0 – 1 (up to +6 dB).
	pub warmth: f32,
	/// Presence / crispness, 0 – 1 (up to +6 dB).
	pub clarity: f32,
	/// Evens out volume, 0 – 1.
	pub comp: f32,
	pub limiter: bool,
}

impl MicFx {
	#[allow(dead_code)]
	pub const OFF: MicFx = MicFx {
		rumble: false,
		denoise: 0.0,
		breath: 0.0,
		gate: false,
		gate_db: -55.0,
		deess: 0.0,
		warmth: 0.0,
		clarity: 0.0,
		comp: 0.0,
		limiter: false,
	};
	pub const STUDIO: MicFx = MicFx {
		rumble: true,
		denoise: 0.9,
		breath: 0.6,
		gate: true,
		gate_db: -55.0,
		deess: 0.5,
		warmth: 0.4,
		clarity: 0.5,
		comp: 0.6,
		limiter: true,
	};

	pub fn is_off(&self) -> bool {
		!self.rumble && self.denoise <= 0.0 && self.breath <= 0.0 && !self.gate && self.deess <= 0.0 && self.warmth <= 0.0 && self.clarity <= 0.0 && self.comp <= 0.0 && !self.limiter
	}

	fn clamp(mut self) -> Self {
		for v in [&mut self.denoise, &mut self.breath, &mut self.deess, &mut self.warmth, &mut self.clarity, &mut self.comp] {
			*v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
		}
		self.gate_db = if self.gate_db.is_finite() { self.gate_db.clamp(-90.0, -10.0) } else { -55.0 };
		self
	}
}

impl Default for MicFx {
	fn default() -> Self {
		MicFx::STUDIO
	}
}

// ---------------------------------------------------------------- building blocks

#[derive(Clone, Copy, Default)]
struct Biquad {
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
	fn highpass(f: f32, q: f32) -> Self {
		let mut b = Biquad::default();
		let w = 2.0 * std::f32::consts::PI * f / RATE;
		let (s, c) = w.sin_cos();
		let al = s / (2.0 * q);
		b.set((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + al, -2.0 * c, 1.0 - al);
		b
	}
	fn low_shelf(&mut self, f: f32, db: f32) {
		let a = 10f32.powf(db / 40.0);
		let w = 2.0 * std::f32::consts::PI * f / RATE;
		let (s, c) = w.sin_cos();
		let al = s / 2.0 * std::f32::consts::SQRT_2;
		let sa = 2.0 * a.sqrt() * al;
		self.set(
			a * ((a + 1.0) - (a - 1.0) * c + sa),
			2.0 * a * ((a - 1.0) - (a + 1.0) * c),
			a * ((a + 1.0) - (a - 1.0) * c - sa),
			(a + 1.0) + (a - 1.0) * c + sa,
			-2.0 * ((a - 1.0) + (a + 1.0) * c),
			(a + 1.0) + (a - 1.0) * c - sa,
		);
	}
	fn peak(&mut self, f: f32, q: f32, db: f32) {
		let a = 10f32.powf(db / 40.0);
		let w = 2.0 * std::f32::consts::PI * f / RATE;
		let (s, c) = w.sin_cos();
		let al = s / (2.0 * q);
		self.set(1.0 + al * a, -2.0 * c, 1.0 - al * a, 1.0 + al / a, -2.0 * c, 1.0 - al / a);
	}
	#[inline]
	fn run(&mut self, x: f32) -> f32 {
		let y = self.b0 * x + self.z1;
		self.z1 = self.b1 * x - self.a1 * y + self.z2;
		self.z2 = self.b2 * x - self.a2 * y;
		y
	}
}

/// One-pole smoothing coefficient for a time constant in milliseconds.
fn coef(ms: f32) -> f32 {
	(-1.0 / (ms * 0.001 * RATE)).exp()
}
fn db(x: f32) -> f32 {
	20.0 * x.max(1e-9).log10()
}
fn undb(d: f32) -> f32 {
	10f32.powf(d / 20.0)
}

/// What the meters show (dBFS levels, gate state, voice probability).
#[derive(Clone, Copy, Default, Serialize)]
pub struct Meter {
	pub level_db: f32,
	pub out_db: f32,
	pub gate_open: bool,
	pub voice: f32,
}

pub struct Processor {
	fx: MicFx,
	dn: Box<DenoiseState<'static>>,
	dry_prev: [f32; FRAME],
	hp: Biquad,
	shelf: Biquad,
	presence: Biquad,
	ess_hp: Biquad,
	eq_for: (f32, f32),
	// envelopes / gains
	breath_g: f32,
	gate_env: f32,
	gate_g: f32,
	gate_hold: u32,
	ess_env: f32,
	all_env: f32,
	comp_env: f32,
	lim_g: f32,
	pub meter: Meter,
	peak_in: f32,
	peak_out: f32,
}

impl Processor {
	pub fn new(fx: MicFx) -> Self {
		let mut p = Processor {
			fx: fx.clamp(),
			dn: DenoiseState::new(),
			dry_prev: [0.0; FRAME],
			hp: Biquad::highpass(80.0, 0.707),
			shelf: Biquad::default(),
			presence: Biquad::default(),
			ess_hp: Biquad::highpass(5500.0, 0.707),
			eq_for: (f32::NAN, f32::NAN),
			breath_g: 1.0,
			gate_env: 0.0,
			gate_g: 1.0,
			gate_hold: 0,
			ess_env: 0.0,
			all_env: 0.0,
			comp_env: 0.0,
			lim_g: 1.0,
			meter: Meter::default(),
			peak_in: 0.0,
			peak_out: 0.0,
		};
		p.update_eq();
		p
	}

	pub fn set(&mut self, fx: MicFx) {
		self.fx = fx.clamp();
		self.update_eq();
	}

	fn update_eq(&mut self) {
		let want = (self.fx.warmth, self.fx.clarity);
		if want == self.eq_for {
			return;
		}
		self.eq_for = want;
		self.shelf.low_shelf(180.0, self.fx.warmth * 6.0);
		self.presence.peak(3500.0, 0.8, self.fx.clarity * 6.0);
	}

	/// Processes one 10 ms frame in place (samples in −1…1).
	pub fn process(&mut self, frame: &mut [f32; FRAME]) {
		let fx = self.fx;
		if fx.is_off() {
			let peak = frame.iter().fold(0f32, |a, x| a.max(x.abs()));
			self.meter = Meter {
				level_db: db(peak),
				out_db: db(peak),
				gate_open: true,
				voice: 1.0,
			};
			return;
		}

		// 1. rumble
		if fx.rumble {
			for x in frame.iter_mut() {
				*x = self.hp.run(*x);
			}
		}

		// 2. noise removal (RNNoise works in 16-bit range); also gives the voice probability
		let mut scaled = [0f32; FRAME];
		for (s, x) in scaled.iter_mut().zip(frame.iter()) {
			*s = x * 32768.0;
		}
		let mut wet = [0f32; FRAME];
		let voice = self.dn.process_frame(&mut wet, &scaled);
		let dry = std::mem::replace(&mut self.dry_prev, *frame);
		let s = fx.denoise;
		for i in 0..FRAME {
			frame[i] = dry[i] * (1.0 - s) + wet[i] / 32768.0 * s;
		}

		// per-sample stages
		let breath_floor = undb(-30.0 * fx.breath);
		// voice → full level; no voice → floor (smoothly in between)
		let breath_target = breath_floor + (1.0 - breath_floor) * ((voice - 0.15) / 0.45).clamp(0.0, 1.0);
		let gate_thr = undb(fx.gate_db);
		let comp_thr = -18.0 - 10.0 * fx.comp; // dB
		let ratio = 1.0 + 5.0 * fx.comp;
		let makeup = undb((-comp_thr) * (1.0 - 1.0 / ratio) * 0.45 * (fx.comp > 0.0) as u8 as f32);
		let (a_fast, r_breath, a_breath) = (coef(3.0), coef(90.0), coef(4.0));
		let (env_a, env_r) = (coef(1.0), coef(60.0));
		let (gate_open_c, gate_close_c) = (coef(2.0), coef(70.0));
		let (comp_a, comp_r) = (coef(6.0), coef(160.0));
		let lim_r = coef(60.0);
		let hold = (0.2 * RATE) as u32;

		let mut peak_in = 0f32;
		let mut peak_out = 0f32;
		let mut open_any = false;
		for x in frame.iter_mut() {
			let mut y = *x;

			// 3. breath reduction
			if fx.breath > 0.0 {
				let c = if breath_target > self.breath_g { a_breath } else { r_breath };
				self.breath_g = breath_target + (self.breath_g - breath_target) * c;
				y *= self.breath_g;
			}

			// level the gate (and the meter) sees
			let ax = y.abs();
			let c = if ax > self.gate_env { env_a } else { env_r };
			self.gate_env = ax + (self.gate_env - ax) * c;
			peak_in = peak_in.max(self.gate_env);

			// 4. gate
			if fx.gate {
				let open = if self.gate_env >= gate_thr {
					self.gate_hold = hold;
					true
				} else if self.gate_hold > 0 {
					self.gate_hold -= 1;
					true
				} else {
					false
				};
				open_any |= open;
				let target = if open { 1.0 } else { 0.001 };
				let c = if target > self.gate_g { gate_open_c } else { gate_close_c };
				self.gate_g = target + (self.gate_g - target) * c;
				y *= self.gate_g;
			} else {
				open_any = true;
			}

			// 5. de-esser: turn the 5.5 kHz+ band down while it dominates
			if fx.deess > 0.0 {
				let hi = self.ess_hp.run(y);
				let (ah, aa) = (hi.abs(), y.abs());
				self.ess_env = ah + (self.ess_env - ah) * if ah > self.ess_env { a_fast } else { r_breath };
				self.all_env = aa + (self.all_env - aa) * if aa > self.all_env { a_fast } else { r_breath };
				let ratio = self.ess_env / self.all_env.max(1e-6);
				let reduce = (fx.deess * ((ratio - 0.3) / 0.3)).clamp(0.0, 0.75 * fx.deess);
				y -= hi * reduce;
			}

			// 6. tone
			if fx.warmth > 0.0 {
				y = self.shelf.run(y);
			}
			if fx.clarity > 0.0 {
				y = self.presence.run(y);
			}

			// 7. compressor
			if fx.comp > 0.0 {
				let a = y.abs();
				let c = if a > self.comp_env { comp_a } else { comp_r };
				self.comp_env = a + (self.comp_env - a) * c;
				let over = db(self.comp_env) - comp_thr;
				let gr = if over > 0.0 { -over * (1.0 - 1.0 / ratio) } else { 0.0 };
				y *= undb(gr) * makeup;
			}

			// 8. limiter (−1 dBFS)
			if fx.limiter {
				let a = y.abs();
				let need = if a > 0.89 { 0.89 / a } else { 1.0 };
				self.lim_g = if need < self.lim_g { need } else { need + (self.lim_g - need) * lim_r };
				y *= self.lim_g;
			}

			peak_out = peak_out.max(y.abs());
			*x = y;
		}

		// meters fall smoothly between frames
		self.peak_in = peak_in.max(self.peak_in * 0.8);
		self.peak_out = peak_out.max(self.peak_out * 0.8);
		self.meter = Meter {
			level_db: db(self.peak_in),
			out_db: db(self.peak_out),
			gate_open: open_any,
			voice,
		};
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tone(n: usize, amp: f32) -> Vec<f32> {
		(0..n)
			.map(|i| {
				let t = i as f32 / RATE;
				let ph = 2.0 * std::f32::consts::PI * (110.0 * t + 25.0 * t * t);
				(1..10).map(|h| (ph * h as f32).sin() / h as f32).sum::<f32>() * amp
			})
			.collect()
	}

	#[test]
	fn studio_keeps_voice_and_mutes_quiet_noise() {
		let mut p = Processor::new(MicFx::STUDIO);
		// loud voice-like signal passes
		let voice = tone(FRAME * 60, 0.2);
		let mut out_peak = 0f32;
		for chunk in voice.chunks(FRAME) {
			let mut f = [0f32; FRAME];
			f.copy_from_slice(chunk);
			p.process(&mut f);
			out_peak = out_peak.max(f.iter().fold(0f32, |a, x| a.max(x.abs())));
		}
		assert!(out_peak > 0.05, "voice should pass, peak {out_peak}");
		assert!(out_peak <= 0.9, "limiter should hold, peak {out_peak}");
		// very quiet hiss is gated away
		let mut seed = 1u32;
		let mut last = 1f32;
		for _ in 0..80 {
			let mut f = [0f32; FRAME];
			for x in f.iter_mut() {
				seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
				*x = ((seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5) * 0.0005;
			}
			p.process(&mut f);
			last = f.iter().fold(0f32, |a, x| a.max(x.abs()));
		}
		assert!(last < 0.0002, "quiet noise should be gated, got {last}");
	}

	#[test]
	fn off_is_bit_exact() {
		let mut p = Processor::new(MicFx::OFF);
		let mut f = [0.123f32; FRAME];
		p.process(&mut f);
		assert!(f.iter().all(|x| *x == 0.123));
	}
}
