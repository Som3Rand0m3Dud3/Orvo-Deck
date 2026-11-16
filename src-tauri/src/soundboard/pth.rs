//! Orvo: turns an RVC voice saved by PyTorch (.pth) into an .onnx model, with no Python.
//!
//! A .pth is a zip holding a pickle (`data.pkl`) plus one raw file per tensor. We read the
//! pickle with a tiny interpreter, fold weight norm back into plain weights, and add the
//! weights to a ready-made empty RVC v2 graph (one per sample rate, built with RVC's own
//! code). Protobuf merges repeated messages, so "template + graph-with-weights" written
//! back to back is a complete .onnx file.

use std::collections::HashMap;
use std::io::{Read, Seek};

use anyhow::{Context, anyhow, bail};

struct Template {
	sr: u32,
	upsample: &'static [i64],
	graph: &'static [u8],
	names: &'static str,
}

const TEMPLATES: [Template; 3] = [
	Template { sr: 32_000, upsample: &[10, 8, 2, 2], graph: include_bytes!("rvc/rvc_v2_32k.tpl"), names: include_str!("rvc/rvc_v2_32k.txt") },
	Template { sr: 40_000, upsample: &[10, 10, 2, 2], graph: include_bytes!("rvc/rvc_v2_40k.tpl"), names: include_str!("rvc/rvc_v2_40k.txt") },
	Template { sr: 48_000, upsample: &[12, 10, 2, 2], graph: include_bytes!("rvc/rvc_v2_48k.tpl"), names: include_str!("rvc/rvc_v2_48k.txt") },
];

// ---------- pickle ----------

#[derive(Clone, Debug)]
#[allow(dead_code)]
enum V {
	None,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Bytes(Vec<u8>),
	List(Vec<V>),
	Tuple(Vec<V>),
	Dict(Vec<(V, V)>),
	Global(String, String),
	Storage { key: String, dtype: String },
	Tensor(Tensor),
	Other,
	Mark,
}

#[derive(Clone, Debug)]
struct Tensor {
	key: String,
	dtype: String,
	offset: usize,
	shape: Vec<usize>,
	stride: Vec<usize>,
}

impl V {
	fn int(&self) -> Option<i64> {
		match self {
			V::Int(i) => Some(*i),
			V::Bool(b) => Some(*b as i64),
			_ => None,
		}
	}
	fn ints(&self) -> Option<Vec<i64>> {
		match self {
			V::List(v) | V::Tuple(v) => v.iter().map(|x| x.int()).collect(),
			_ => None,
		}
	}
	fn get(&self, key: &str) -> Option<&V> {
		match self {
			V::Dict(items) => items.iter().find(|(k, _)| matches!(k, V::Str(s) if s == key)).map(|(_, v)| v),
			_ => None,
		}
	}
}

fn unpickle(data: &[u8]) -> anyhow::Result<V> {
	let mut stack: Vec<V> = Vec::new();
	let mut memo: HashMap<u64, V> = HashMap::new();
	let mut p = 0usize;
	let take = |p: &mut usize, n: usize| -> anyhow::Result<&[u8]> {
		let s = data.get(*p..*p + n).ok_or_else(|| anyhow!("the voice file is cut short"))?;
		*p += n;
		Ok(s)
	};
	fn pop_mark(stack: &mut Vec<V>) -> anyhow::Result<Vec<V>> {
		let at = stack.iter().rposition(|v| matches!(v, V::Mark)).ok_or_else(|| anyhow!("bad pickle (no mark)"))?;
		let items = stack.split_off(at + 1);
		stack.pop();
		Ok(items)
	}
	fn pop(stack: &mut Vec<V>) -> anyhow::Result<V> {
		stack.pop().ok_or_else(|| anyhow!("bad pickle (empty stack)"))
	}
	fn line(data: &[u8], p: &mut usize) -> anyhow::Result<String> {
		let end = data[*p..].iter().position(|&b| b == b'\n').ok_or_else(|| anyhow!("bad pickle"))?;
		let s = String::from_utf8_lossy(&data[*p..*p + end]).into_owned();
		*p += end + 1;
		Ok(s)
	}
	loop {
		let op = *data.get(p).ok_or_else(|| anyhow!("the voice file is cut short"))?;
		p += 1;
		match op {
			0x80 => p += 1,                                   // PROTO
			0x95 => p += 8,                                   // FRAME
			b'.' => return pop(&mut stack),                   // STOP
			b'(' => stack.push(V::Mark),                      // MARK
			b'N' => stack.push(V::None),
			0x88 => stack.push(V::Bool(true)),
			0x89 => stack.push(V::Bool(false)),
			b'}' => stack.push(V::Dict(vec![])),
			b']' => stack.push(V::List(vec![])),
			b')' => stack.push(V::Tuple(vec![])),
			0x8f => stack.push(V::List(vec![])),              // EMPTY_SET (treated as a list)
			b'J' => stack.push(V::Int(i32::from_le_bytes(take(&mut p, 4)?.try_into()?) as i64)),
			b'K' => stack.push(V::Int(take(&mut p, 1)?[0] as i64)),
			b'M' => stack.push(V::Int(u16::from_le_bytes(take(&mut p, 2)?.try_into()?) as i64)),
			0x8a => {
				// LONG1
				let n = take(&mut p, 1)?[0] as usize;
				let b = take(&mut p, n)?;
				let mut v: i64 = 0;
				for (i, &x) in b.iter().enumerate().take(8) {
					v |= (x as i64) << (8 * i);
				}
				if n > 0 && n < 8 && b[n - 1] & 0x80 != 0 {
					v -= 1i64 << (8 * n);
				}
				stack.push(V::Int(v));
			}
			b'G' => stack.push(V::Float(f64::from_be_bytes(take(&mut p, 8)?.try_into()?))),
			b'X' => {
				let n = u32::from_le_bytes(take(&mut p, 4)?.try_into()?) as usize;
				stack.push(V::Str(String::from_utf8_lossy(take(&mut p, n)?).into_owned()));
			}
			0x8c => {
				let n = take(&mut p, 1)?[0] as usize;
				stack.push(V::Str(String::from_utf8_lossy(take(&mut p, n)?).into_owned()));
			}
			0x8d => {
				let n = u64::from_le_bytes(take(&mut p, 8)?.try_into()?) as usize;
				stack.push(V::Str(String::from_utf8_lossy(take(&mut p, n)?).into_owned()));
			}
			b'U' => {
				let n = take(&mut p, 1)?[0] as usize;
				stack.push(V::Str(String::from_utf8_lossy(take(&mut p, n)?).into_owned()));
			}
			b'T' => {
				let n = u32::from_le_bytes(take(&mut p, 4)?.try_into()?) as usize;
				stack.push(V::Str(String::from_utf8_lossy(take(&mut p, n)?).into_owned()));
			}
			b'C' => {
				let n = take(&mut p, 1)?[0] as usize;
				stack.push(V::Bytes(take(&mut p, n)?.to_vec()));
			}
			b'B' => {
				let n = u32::from_le_bytes(take(&mut p, 4)?.try_into()?) as usize;
				stack.push(V::Bytes(take(&mut p, n)?.to_vec()));
			}
			b't' => {
				let items = pop_mark(&mut stack)?;
				stack.push(V::Tuple(items));
			}
			0x85 => {
				let a = pop(&mut stack)?;
				stack.push(V::Tuple(vec![a]));
			}
			0x86 => {
				let b = pop(&mut stack)?;
				let a = pop(&mut stack)?;
				stack.push(V::Tuple(vec![a, b]));
			}
			0x87 => {
				let c = pop(&mut stack)?;
				let b = pop(&mut stack)?;
				let a = pop(&mut stack)?;
				stack.push(V::Tuple(vec![a, b, c]));
			}
			b'l' => {
				let items = pop_mark(&mut stack)?;
				stack.push(V::List(items));
			}
			b'd' => {
				let items = pop_mark(&mut stack)?;
				let mut d = vec![];
				let mut it = items.into_iter();
				while let (Some(k), Some(v)) = (it.next(), it.next()) {
					d.push((k, v));
				}
				stack.push(V::Dict(d));
			}
			b'a' => {
				let v = pop(&mut stack)?;
				if let Some(V::List(l)) = stack.last_mut() {
					l.push(v);
				}
			}
			b'e' | 0x90 => {
				// APPENDS / ADDITEMS
				let items = pop_mark(&mut stack)?;
				if let Some(V::List(l)) = stack.last_mut() {
					l.extend(items);
				}
			}
			b's' => {
				let v = pop(&mut stack)?;
				let k = pop(&mut stack)?;
				if let Some(V::Dict(d)) = stack.last_mut() {
					d.push((k, v));
				}
			}
			b'u' => {
				let items = pop_mark(&mut stack)?;
				if let Some(V::Dict(d)) = stack.last_mut() {
					let mut it = items.into_iter();
					while let (Some(k), Some(v)) = (it.next(), it.next()) {
						d.push((k, v));
					}
				}
			}
			b'q' => {
				let i = take(&mut p, 1)?[0] as u64;
				memo.insert(i, stack.last().cloned().unwrap_or(V::None));
			}
			b'r' => {
				let i = u32::from_le_bytes(take(&mut p, 4)?.try_into()?) as u64;
				memo.insert(i, stack.last().cloned().unwrap_or(V::None));
			}
			0x94 => {
				let i = memo.len() as u64;
				memo.insert(i, stack.last().cloned().unwrap_or(V::None));
			}
			b'h' => {
				let i = take(&mut p, 1)?[0] as u64;
				stack.push(memo.get(&i).cloned().ok_or_else(|| anyhow!("bad pickle (memo)"))?);
			}
			b'j' => {
				let i = u32::from_le_bytes(take(&mut p, 4)?.try_into()?) as u64;
				stack.push(memo.get(&i).cloned().ok_or_else(|| anyhow!("bad pickle (memo)"))?);
			}
			b'c' => {
				let m = line(data, &mut p)?;
				let n = line(data, &mut p)?;
				stack.push(V::Global(m, n));
			}
			0x93 => {
				let n = pop(&mut stack)?;
				let m = pop(&mut stack)?;
				match (m, n) {
					(V::Str(m), V::Str(n)) => stack.push(V::Global(m, n)),
					_ => bail!("bad pickle (global)"),
				}
			}
			b'Q' => {
				// BINPERSID: ('storage', <type>, key, location, numel)
				let pid = pop(&mut stack)?;
				let V::Tuple(t) = pid else { bail!("bad pickle (storage)") };
				let dtype = match t.get(1) {
					Some(V::Global(_, n)) => n.clone(),
					_ => String::new(),
				};
				let key = match t.get(2) {
					Some(V::Str(s)) => s.clone(),
					_ => bail!("bad pickle (storage key)"),
				};
				stack.push(V::Storage { key, dtype });
			}
			b'R' | 0x81 => {
				// REDUCE / NEWOBJ
				let args = pop(&mut stack)?;
				let f = pop(&mut stack)?;
				let args = match args {
					V::Tuple(a) => a,
					_ => vec![],
				};
				stack.push(call(f, args)?);
			}
			b'b' => {
				// BUILD: state is ignored (OrderedDict metadata, etc.)
				pop(&mut stack)?;
			}
			b'0' => {
				pop(&mut stack)?;
			}
			b'2' => {
				let v = stack.last().cloned().ok_or_else(|| anyhow!("bad pickle"))?;
				stack.push(v);
			}
			_ => bail!("unsupported pickle opcode 0x{op:02x}"),
		}
	}
}

fn call(f: V, args: Vec<V>) -> anyhow::Result<V> {
	let V::Global(m, n) = &f else { return Ok(V::Other) };
	Ok(match (m.as_str(), n.as_str()) {
		("collections", "OrderedDict") | ("builtins", "dict") => V::Dict(vec![]),
		("torch._utils", "_rebuild_tensor_v2") | ("torch._utils", "_rebuild_tensor") => {
			let (key, dtype) = match args.first() {
				Some(V::Storage { key, dtype }) => (key.clone(), dtype.clone()),
				_ => bail!("bad tensor in the voice file"),
			};
			let offset = args.get(1).and_then(|v| v.int()).unwrap_or(0) as usize;
			let shape: Vec<usize> = args.get(2).and_then(|v| v.ints()).unwrap_or_default().into_iter().map(|x| x as usize).collect();
			let stride: Vec<usize> = args.get(3).and_then(|v| v.ints()).unwrap_or_default().into_iter().map(|x| x as usize).collect();
			V::Tensor(Tensor { key, dtype, offset, shape, stride })
		}
		("torch._utils", "_rebuild_parameter") | ("torch._utils", "_rebuild_parameter_with_state") => args.into_iter().next().unwrap_or(V::Other),
		_ => V::Other,
	})
}

// ---------- tensors ----------

fn read_tensor<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, prefix: &str, t: &Tensor) -> anyhow::Result<Vec<f32>> {
	let mut raw = Vec::new();
	zip.by_name(&format!("{prefix}data/{}", t.key))
		.with_context(|| format!("weight {} is missing from the voice file", t.key))?
		.read_to_end(&mut raw)?;
	let (size, conv): (usize, fn(&[u8]) -> f32) = match t.dtype.as_str() {
		"FloatStorage" => (4, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
		"HalfStorage" => (2, |b| f16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
		"BFloat16Storage" => (2, |b| f32::from_bits((u16::from_le_bytes([b[0], b[1]]) as u32) << 16)),
		"DoubleStorage" => (8, |b| f64::from_le_bytes(b[..8].try_into().unwrap()) as f32),
		other => bail!("the voice has weights of a type Orvo can't read ({other})"),
	};
	let count: usize = t.shape.iter().product();
	let mut out = Vec::with_capacity(count);
	let mut idx = vec![0usize; t.shape.len()];
	for _ in 0..count {
		let at = t.offset + idx.iter().zip(&t.stride).map(|(i, s)| i * s).sum::<usize>();
		let b = raw.get(at * size..at * size + size).ok_or_else(|| anyhow!("a weight in the voice file is cut short"))?;
		out.push(conv(b));
		for d in (0..idx.len()).rev() {
			idx[d] += 1;
			if idx[d] < t.shape[d] {
				break;
			}
			idx[d] = 0;
		}
	}
	Ok(out)
}

fn f16_to_f32(h: u16) -> f32 {
	let s = ((h >> 15) as u32) << 31;
	let e = ((h >> 10) & 0x1f) as i32;
	let m = (h & 0x3ff) as u32;
	let bits = if e == 0 {
		if m == 0 {
			s
		} else {
			// subnormal
			let mut e = -14;
			let mut m = m;
			while m & 0x400 == 0 {
				m <<= 1;
				e -= 1;
			}
			s | (((e + 127) as u32) << 23) | ((m & 0x3ff) << 13)
		}
	} else if e == 31 {
		s | 0x7f80_0000 | (m << 13)
	} else {
		s | (((e - 15 + 127) as u32) << 23) | (m << 13)
	};
	f32::from_bits(bits)
}

/// weight = g * v / |v|, the norm taken over every axis but the first.
fn fold_weight_norm(g: &[f32], v: &[f32]) -> Vec<f32> {
	let rows = g.len().max(1);
	let per = v.len() / rows;
	let mut w = Vec::with_capacity(v.len());
	for r in 0..rows {
		let row = &v[r * per..(r + 1) * per];
		let n = row.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt().max(1e-12) as f32;
		w.extend(row.iter().map(|x| x * g[r] / n));
	}
	w
}

// ---------- protobuf ----------

fn varint(out: &mut Vec<u8>, mut v: u64) {
	loop {
		let b = (v & 0x7f) as u8;
		v >>= 7;
		if v == 0 {
			out.push(b);
			return;
		}
		out.push(b | 0x80);
	}
}

fn field_bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
	varint(out, ((field << 3) | 2) as u64);
	varint(out, data.len() as u64);
	out.extend_from_slice(data);
}

fn tensor_proto(name: &str, dims: &[usize], data: &[f32]) -> Vec<u8> {
	let mut t = Vec::with_capacity(data.len() * 4 + 64);
	for d in dims {
		varint(&mut t, 1 << 3); // dims, field 1
		varint(&mut t, *d as u64);
	}
	varint(&mut t, 2 << 3); // data_type, field 2
	varint(&mut t, 1); // FLOAT
	field_bytes(&mut t, 8, name.as_bytes());
	let mut raw = Vec::with_capacity(data.len() * 4);
	for x in data {
		raw.extend_from_slice(&x.to_le_bytes());
	}
	field_bytes(&mut t, 9, &raw);
	t
}

// ---------- conversion ----------

/// Converts an RVC .pth voice into a complete .onnx model (returned as bytes).
pub fn convert<R: Read + Seek>(reader: R) -> anyhow::Result<Vec<u8>> {
	let mut zip = zip::ZipArchive::new(reader).map_err(|_| anyhow!("this .pth file is in an old format Orvo can't read"))?;
	let pkl = (0..zip.len())
		.filter_map(|i| zip.name_for_index(i).map(str::to_string))
		.find(|n| n.ends_with("data.pkl"))
		.ok_or_else(|| anyhow!("this isn't a PyTorch voice file"))?;
	let prefix = pkl.trim_end_matches("data.pkl").to_string();
	let mut data = Vec::new();
	zip.by_name(&pkl)?.read_to_end(&mut data)?;
	let root = unpickle(&data)?;

	let weights = match root.get("weight") {
		Some(w @ V::Dict(_)) => w,
		_ if root.get("model").is_some() || root.get("optimizer").is_some() => {
			bail!("this is a training checkpoint (like G_2333.pth), not a finished voice. Use the smaller .pth from the voice's weights folder.")
		}
		_ => bail!("this .pth isn't an RVC voice"),
	};
	let V::Dict(items) = weights else { unreachable!() };
	let mut tensors: HashMap<String, Tensor> = HashMap::new();
	for (k, v) in items {
		if let (V::Str(k), V::Tensor(t)) = (k, v) {
			if !k.starts_with("enc_q.") {
				tensors.insert(k.clone(), t.clone());
			}
		}
	}
	if let Some(cfg) = root.get("vocoder") {
		if let V::Str(s) = cfg {
			if !s.eq_ignore_ascii_case("hifi-gan") && !s.eq_ignore_ascii_case("hifigan") {
				bail!("this voice uses the {s} vocoder; only standard (HiFi-GAN) RVC voices work");
			}
		}
	}
	match tensors.get("enc_p.emb_phone.weight").map(|t| t.shape.clone()) {
		Some(s) if s.get(1) == Some(&768) => {}
		Some(s) if s.get(1) == Some(&256) => bail!("this is an old RVC v1 voice; only v2 voices work"),
		_ => bail!("this .pth isn't an RVC voice"),
	}
	if !tensors.contains_key("enc_p.emb_pitch.weight") {
		bail!("this voice has no pitch control (a \"no f0\" model); only voices with pitch work");
	}

	// sample rate: the config's last entry, else the "sr" tag ("40k")
	let config = root.get("config").and_then(|c| match c {
		V::List(v) | V::Tuple(v) => Some(v.clone()),
		_ => None,
	});
	let sr = config
		.as_ref()
		.and_then(|c| c.last())
		.and_then(|v| v.int())
		.or_else(|| match root.get("sr") {
			Some(V::Str(s)) => s.trim_end_matches('k').parse::<i64>().ok().map(|k| k * 1000),
			Some(v) => v.int(),
			None => None,
		})
		.ok_or_else(|| anyhow!("couldn't find the voice's sample rate"))? as u32;
	let tpl = TEMPLATES
		.iter()
		.find(|t| t.sr == sr)
		.ok_or_else(|| anyhow!("this voice is {} kHz; Orvo can open 32, 40 and 48 kHz voices", sr / 1000))?;
	if let Some(up) = config.as_ref().and_then(|c| c.get(12)).and_then(|v| v.ints()) {
		if up != tpl.upsample {
			bail!("this voice has an unusual layout (upsampling {up:?}); Orvo can't convert it");
		}
	}

	let mut out = tpl.graph.to_vec();
	let mut graph = Vec::new();
	for line in tpl.names.lines() {
		let mut parts = line.split_whitespace();
		let Some(full) = parts.next() else { continue };
		let want: Vec<usize> = parts.filter_map(|d| d.parse().ok()).collect();
		let key = full.strip_prefix("net.").unwrap_or(full);
		let (dims, data) = load_weight(&mut zip, &prefix, &tensors, key)?;
		// every weight must match the graph, except the speaker table (its row count varies)
		if dims != want && !(key == "emb_g.weight" && dims.get(1) == want.get(1)) {
			bail!("this voice's {key} has shape {dims:?}, expected {want:?}; it isn't a standard RVC v2 voice");
		}
		field_bytes(&mut graph, 5, &tensor_proto(full, &dims, &data)); // GraphProto.initializer
	}
	field_bytes(&mut out, 7, &graph); // ModelProto.graph, merged into the template's graph
	Ok(out)
}

fn load_weight<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, prefix: &str, t: &HashMap<String, Tensor>, key: &str) -> anyhow::Result<(Vec<usize>, Vec<f32>)> {
	if let Some(x) = t.get(key) {
		return Ok((x.shape.clone(), read_tensor(zip, prefix, x)?));
	}
	// weight norm, old style (weight_g / weight_v) or new (parametrizations.weight.original0/1)
	if let Some(base) = key.strip_suffix(".weight") {
		let pairs = [
			(format!("{base}.weight_g"), format!("{base}.weight_v")),
			(format!("{base}.parametrizations.weight.original0"), format!("{base}.parametrizations.weight.original1")),
		];
		for (g, v) in pairs {
			if let (Some(g), Some(v)) = (t.get(&g), t.get(&v)) {
				let gd = read_tensor(zip, prefix, g)?;
				let vd = read_tensor(zip, prefix, v)?;
				return Ok((v.shape.clone(), fold_weight_norm(&gd, &vd)));
			}
		}
	}
	bail!("the voice file has no {key}; it isn't a standard RVC v2 voice")
}

/// Finds the voice inside a downloaded .zip (the biggest .pth that isn't a training checkpoint).
pub fn pth_from_zip<R: Read + Seek>(reader: R) -> anyhow::Result<Vec<u8>> {
	let mut zip = zip::ZipArchive::new(reader).context("couldn't open the .zip")?;
	let mut best: Option<(usize, u64)> = None;
	let mut onnx: Option<usize> = None;
	for i in 0..zip.len() {
		let f = zip.by_index(i)?;
		let name = f.name().to_ascii_lowercase();
		let file = name.rsplit('/').next().unwrap_or("");
		if name.ends_with(".pth") && !file.starts_with("g_") && !file.starts_with("d_") && best.is_none_or(|(_, s)| f.size() > s) {
			best = Some((i, f.size()));
		}
		if name.ends_with(".onnx") {
			onnx = Some(i);
		}
	}
	let i = best.map(|b| b.0).or(onnx).ok_or_else(|| anyhow!("there's no voice (.pth or .onnx) in this .zip"))?;
	let mut data = Vec::new();
	zip.by_index(i)?.read_to_end(&mut data)?;
	Ok(data)
}
