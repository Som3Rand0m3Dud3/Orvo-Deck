//! Decodes a sound file (mp3, wav, ogg, flac, m4a/aac) into 48 kHz stereo f32.

use std::path::Path;

use anyhow::{Context, anyhow};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use super::engine::CLIP_RATE;

/// Longest sound accepted (10 minutes), so a dropped album doesn't eat all memory.
const MAX_SECONDS: usize = 600;

pub fn decode(path: &Path) -> anyhow::Result<Vec<f32>> {
	let file = std::fs::File::open(path).with_context(|| format!("can't open {}", path.display()))?;
	let mss = MediaSourceStream::new(Box::new(file), Default::default());
	let mut hint = Hint::new();
	if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
		hint.with_extension(ext);
	}
	let probed = symphonia::default::get_probe()
		.format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
		.map_err(|_| anyhow!("not a supported sound file"))?;
	let mut format = probed.format;
	let track = format
		.tracks()
		.iter()
		.find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
		.ok_or_else(|| anyhow!("no audio in this file"))?;
	let track_id = track.id;
	let mut decoder = symphonia::default::get_codecs()
		.make(&track.codec_params, &DecoderOptions::default())
		.map_err(|_| anyhow!("this sound format isn't supported"))?;

	let mut rate = track.codec_params.sample_rate.unwrap_or(CLIP_RATE);
	let mut stereo: Vec<f32> = Vec::new();
	let mut buf: Option<SampleBuffer<f32>> = None;

	loop {
		let packet = match format.next_packet() {
			Ok(p) => p,
			Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
			Err(SymError::ResetRequired) => break,
			Err(e) => {
				if stereo.is_empty() {
					return Err(anyhow!("couldn't read the sound: {e}"));
				}
				break;
			}
		};
		if packet.track_id() != track_id {
			continue;
		}
		let decoded = match decoder.decode(&packet) {
			Ok(d) => d,
			Err(SymError::DecodeError(_)) => continue, // skip a damaged frame
			Err(e) => return Err(anyhow!("couldn't decode the sound: {e}")),
		};
		let spec = *decoded.spec();
		rate = spec.rate;
		let channels = spec.channels.count().max(1);
		let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
		if b.capacity() < decoded.capacity() * channels {
			*b = SampleBuffer::new(decoded.capacity() as u64, spec);
		}
		b.copy_interleaved_ref(decoded);
		for frame in b.samples().chunks(channels) {
			let l = frame[0];
			let r = if channels > 1 { frame[1] } else { l };
			stereo.push(l);
			stereo.push(r);
		}
		if stereo.len() / 2 > MAX_SECONDS * rate as usize {
			return Err(anyhow!("sound is longer than 10 minutes"));
		}
	}
	if stereo.is_empty() {
		return Err(anyhow!("the sound is empty"));
	}
	Ok(resample(&stereo, rate, CLIP_RATE))
}

/// Linear resampling of interleaved stereo.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
	if from == to || from == 0 {
		return input.to_vec();
	}
	let frames_in = input.len() / 2;
	let frames_out = (frames_in as u64 * to as u64 / from as u64) as usize;
	let step = from as f64 / to as f64;
	let mut out = Vec::with_capacity(frames_out * 2);
	for n in 0..frames_out {
		let pos = n as f64 * step;
		let i = pos as usize;
		let j = (i + 1).min(frames_in - 1);
		let t = (pos - i as f64) as f32;
		out.push(input[i * 2] * (1.0 - t) + input[j * 2] * t);
		out.push(input[i * 2 + 1] * (1.0 - t) + input[j * 2 + 1] * t);
	}
	out
}
