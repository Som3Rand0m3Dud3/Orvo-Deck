//! Effects baked into one sound (speed, pitch, bass, reverb, echo, distortion, reverse,
//! normalize, fade-in). Rendered once when the sound is first played after a change, so they
//! cost nothing while playing.

use serde::{Deserialize, Serialize};

use super::effects::{Biquad, Effect, EffectSpec};
use super::engine::CLIP_RATE;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SoundFx {
	/// 0.5 – 2 (also changes the pitch, like a record player)
	pub speed: f32,
	/// semitones, keeps the speed
	pub pitch: f32,
	pub bass_db: f32,
	pub reverb: f32,
	pub echo: f32,
	pub distortion: f32,
	pub reverse: bool,
	pub normalize: bool,
	pub fade_in: bool,
}

impl Default for SoundFx {
	fn default() -> Self {
		SoundFx {
			speed: 1.0,
			pitch: 0.0,
			bass_db: 0.0,
			reverb: 0.0,
			echo: 0.0,
			distortion: 0.0,
			reverse: false,
			normalize: false,
			fade_in: false,
		}
	}
}

impl SoundFx {
	pub fn is_neutral(&self) -> bool {
		*self == SoundFx::default()
	}
	pub fn speed(&self) -> f32 {
		if self.speed.is_finite() { self.speed.clamp(0.25, 4.0) } else { 1.0 }
	}
}

const SR: f32 = CLIP_RATE as f32;

/// `raw` is interleaved stereo at CLIP_RATE; `start`/`end` are frames (already trimmed bounds).
pub fn render(raw: &[f32], start: usize, end: usize, fx: &SoundFx) -> Vec<f32> {
	let end = end.min(raw.len() / 2);
	let start = start.min(end);
	let mut a: Vec<f32> = raw[start * 2..end * 2].to_vec();
	if a.is_empty() {
		return a;
	}
	if fx.reverse {
		let frames = a.len() / 2;
		let mut r = Vec::with_capacity(a.len());
		for f in (0..frames).rev() {
			r.push(a[f * 2]);
			r.push(a[f * 2 + 1]);
		}
		a = r;
	}
	let speed = fx.speed();
	if (speed - 1.0).abs() > 0.001 {
		let frames = a.len() / 2;
		let out_frames = ((frames as f32) / speed) as usize;
		let mut r = Vec::with_capacity(out_frames * 2);
		for n in 0..out_frames {
			let pos = n as f32 * speed;
			let i = (pos as usize).min(frames - 1);
			let j = (i + 1).min(frames - 1);
			let t = pos - i as f32;
			r.push(a[i * 2] * (1.0 - t) + a[j * 2] * t);
			r.push(a[i * 2 + 1] * (1.0 - t) + a[j * 2 + 1] * t);
		}
		a = r;
	}
	let pitch = if fx.pitch.is_finite() { fx.pitch.clamp(-24.0, 24.0) } else { 0.0 };
	if pitch.abs() > 0.01 && a.len() >= 4 {
		let mut st = signalsmith_stretch::Stretch::preset_default(2, CLIP_RATE);
		st.set_transpose_factor_semitones(pitch, None);
		let mut out = vec![0.0f32; a.len()];
		if st.exact(&a[..], &mut out[..]) {
			a = out;
		}
	}
	if fx.bass_db > 0.05 {
		let mut f = [Biquad::default(); 2];
		for b in f.iter_mut() {
			b.shelf(120.0, fx.bass_db.min(18.0), false, SR);
		}
		for fr in a.chunks_mut(2) {
			fr[0] = f[0].run(fr[0]);
			fr[1] = f[1].run(fr[1]);
		}
	}
	let run = |kind: &str, params: &[(&str, f32)], a: &mut Vec<f32>| {
		if let Some(mut e) = Effect::new(EffectSpec::new(kind, params), SR, 2) {
			e.process(a, 2);
		}
	};
	if fx.distortion > 0.01 {
		run("distortion", &[("drive", fx.distortion.clamp(0.0, 1.0)), ("tone", 0.6), ("mix", 1.0)], &mut a);
	}
	let tail = fx.echo > 0.01 || fx.reverb > 0.01;
	if tail {
		a.extend(std::iter::repeat_n(0.0, (SR * 2.5) as usize * 2));
		if fx.echo > 0.01 {
			run("echo", &[("time_ms", 280.0), ("feedback", 0.4), ("mix", fx.echo.clamp(0.0, 1.0))], &mut a);
		}
		if fx.reverb > 0.01 {
			run("reverb", &[("size", 0.75), ("damping", 0.5), ("mix", fx.reverb.clamp(0.0, 1.0))], &mut a);
		}
		// drop the silent part of the tail
		let last = a.iter().rposition(|x| x.abs() > 0.0005).map(|i| i / 2 + 1).unwrap_or(0);
		a.truncate(last * 2);
	}
	if fx.normalize {
		let peak = a.iter().fold(0f32, |m, x| m.max(x.abs()));
		if peak > 1e-4 {
			let g = 0.89 / peak;
			a.iter_mut().for_each(|x| *x *= g);
		}
	}
	if fx.fade_in {
		let n = ((SR * 0.15) as usize).min(a.len() / 2);
		for f in 0..n {
			let g = f as f32 / n as f32;
			a[f * 2] *= g;
			a[f * 2 + 1] *= g;
		}
	}
	a
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn speed_reverse_and_tails() {
		let raw: Vec<f32> = (0..48_000)
			.flat_map(|i| {
				let v = (i as f32 * 0.05).sin() * 0.5;
				[v, v]
			})
			.collect();
		let fast = render(&raw, 0, 48_000, &SoundFx { speed: 2.0, ..Default::default() });
		assert_eq!(fast.len() / 2, 24_000);
		let echo = render(&raw, 0, 24_000, &SoundFx { echo: 0.5, ..Default::default() });
		assert!(echo.len() / 2 > 24_000, "echo should add a tail");
		let rev = render(&raw, 0, 100, &SoundFx { reverse: true, ..Default::default() });
		assert_eq!(rev[0], raw[99 * 2]);
		let norm = render(
			&raw,
			0,
			48_000,
			&SoundFx {
				normalize: true,
				..Default::default()
			},
		);
		let peak = norm.iter().fold(0f32, |m, x| m.max(x.abs()));
		assert!((peak - 0.89).abs() < 0.01);
		let pitched = render(&raw, 0, 48_000, &SoundFx { pitch: 5.0, ..Default::default() });
		assert_eq!(pitched.len(), raw.len());
		assert!(pitched.iter().all(|x| x.is_finite()));
	}
}
