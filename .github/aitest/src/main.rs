#[path = "../../../src-tauri/src/soundboard/ai_core.rs"]
mod ai_core;
use ai_core::*;
use std::path::Path;
fn main() -> anyhow::Result<()> {
	let a: Vec<String> = std::env::args().collect();
	let (engine, voice_p, input, output, gpu) = (Path::new(&a[1]), Path::new(&a[2]), &a[3], &a[4], a.get(5).map(|s| s == "gpu").unwrap_or(false));
	load_runtime(engine)?;
	if gpu {
		for (label, f) in [("gpu", ort::ep::directml::DeviceFilter::Gpu), ("any", ort::ep::directml::DeviceFilter::Any)] {
			let r = ort::session::Session::builder().map_err(|e| anyhow::anyhow!("{e}")).and_then(|b| {
				b.with_memory_pattern(false).map_err(|e| anyhow::anyhow!("{e}"))?
					.with_execution_providers([ort::ep::DirectML::default().with_device_filter(f).build().error_on_failure()]).map_err(|e| anyhow::anyhow!("{e}"))?
					.commit_from_file(engine.join("rmvpe.onnx")).map_err(|e| anyhow::anyhow!("{e}"))
			});
			println!("DirectML {label}: {}", match r { Ok(_) => "OK".to_string(), Err(e) => format!("{e:#}") });
		}
	}
	let (mut cv, g1) = open(&engine.join("contentvec.onnx"), gpu)?;
	let (mut rm, g2) = open(&engine.join("rmvpe.onnx"), gpu)?;
	let (mut vc, g3) = open(voice_p, gpu)?;
	println!("gpu: cv {g1} rm {g2} voice {g3}");
	let (mut chk, _) = open(voice_p, false)?;
	let info = inspect_voice(&mut chk)?;
	println!("voice info {info:?}");
	let raw = std::fs::read(input)?;
	let x: Vec<f32> = raw.chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
	let mut conv = Converter::new(info);
	let (mut down, mut up) = (Down3::new(), Up::new(info.sr));
	let blk = 20;
	let mut out = Vec::new();
	let mut times = Vec::new();
	for chunk in x.chunks(blk * 480) {
		if chunk.len() < blk * 480 { break; }
		let mut b16 = Vec::new();
		for &s in chunk { down.push(s, &mut b16); }
		b16.resize(blk * HOP, 0.0);
		let t = std::time::Instant::now();
		let seg = conv.step(&mut cv, &mut rm, &mut vc, &b16, 0.0)?;
		times.push(t.elapsed().as_secs_f32() * 1000.0);
		for &s in &seg { up.push(s, &mut out); }
	}
	times.sort_by(|a, b| a.total_cmp(b));
	println!("blocks {} ms median {:.0} max {:.0} (block {} ms) in {} out {}", times.len(), times[times.len() / 2], times[times.len() - 1], blk * 10, x.len(), out.len());
	let bytes: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
	std::fs::write(output, bytes)?;
	Ok(())
}
