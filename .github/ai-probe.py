import sys, os, time, json, subprocess
import numpy as np, onnxruntime as ort, soundfile as sf, torch
os.makedirs("out", exist_ok=True)
def info(p):
    s = ort.InferenceSession(p, providers=["CPUExecutionProvider"])
    print("==", p)
    for i in s.get_inputs(): print("  in ", i.name, i.shape, i.type)
    for o in s.get_outputs(): print("  out", o.name, o.shape, o.type)
    print("  meta", dict(s.get_modelmeta().custom_metadata_map))
    return s
cv = info("m/contentvec.onnx"); rm = info("m/rmvpe.onnx")
if os.path.exists("m/pitch_onnx.py"):
    src = open("m/pitch_onnx.py").read(); print("---- pitch_onnx.py ----"); print(src[:6000])
# export voice pth -> onnx with RVC's own exporter
sys.path.insert(0, "rvc")
cpt = torch.load("m/voice.pth", map_location="cpu", weights_only=False)
print("pth keys", list(cpt.keys()), "config", cpt.get("config"), "sr", cpt.get("sr"), "f0", cpt.get("f0"), "version", cpt.get("version"))
import glob; print("exporters", glob.glob("rvc/**/onnx/export.py", recursive=True))
try:
    from infer.modules.onnx.export import export_onnx
except Exception as e:
    print("infer path failed", e)
    from rvc.modules.onnx.export import export_onnx
t = time.time(); export_onnx("m/voice.pth", "m/voice.onnx"); print("export s", time.time() - t, os.path.getsize("m/voice.onnx"))
vg = info("m/voice.onnx")
# test speech
subprocess.run(["espeak-ng", "-v", "en-us", "-s", "150", "-w", "out/in.wav", "Hello, this is a test of the Orvo voice changer. One two three four five."], check=True)
x, sr = sf.read("out/in.wav", dtype="float32")
from scipy.signal import resample_poly
from math import gcd
g = gcd(sr, 16000); x16 = resample_poly(x, 16000 // g, sr // g).astype(np.float32)
print("in sr", sr, "len16", len(x16))
def run(sess, feeds):
    t = time.time(); r = sess.run(None, feeds); return r, time.time() - t
(feat,), tc = run(cv, {"input_values": x16[None, :], "attention_mask": np.ones((1, len(x16)), np.int64)})
print("contentvec out", feat.shape, "s", tc, "frames/len", feat.shape[1], len(x16) / 320)
(sal,), tr = run(rm, {"audio": x16[None, :]}); sal = sal[0].astype(np.float64)
print("rmvpe", sal.shape, "s", tr)
CENTS = 1997.3794084376191 + 20.0 * np.arange(360)
idx = np.clip(sal.argmax(1)[:, None] + np.arange(-4, 5)[None, :], 0, 359)
w = np.maximum(np.take_along_axis(sal, idx, 1), 0)
cents = (w * CENTS[idx]).sum(1) / np.maximum(w.sum(1), 1e-12)
f0 = np.where(sal.max(1) > 0.03, 10 * 2 ** (cents / 1200), 0.0)
print("f0 voiced median", np.median(f0[f0 > 0]) if (f0 > 0).any() else 0, "voiced frac", (f0 > 0).mean())
for KEY in [0, 12]:
    T = feat.shape[1] * 2
    ph = np.repeat(feat, 2, axis=1)[:, :T].astype(np.float32)
    ff = np.zeros(T); n = min(T, len(f0)); ff[:n] = f0[:n]
    ff = ff * 2 ** (KEY / 12)
    mel = 1127 * np.log(1 + ff / 700); mmin = 1127 * np.log(1 + 50 / 700); mmax = 1127 * np.log(1 + 1100 / 700)
    coarse = np.where(mel > 0, (mel - mmin) * 254 / (mmax - mmin) + 1, mel)
    coarse = np.clip(np.rint(coarse), 1, 255).astype(np.int64)
    feeds = {"phone": ph, "phone_lengths": np.array([T], np.int64), "pitch": coarse[None], "pitchf": ff[None].astype(np.float32),
             "ds": np.array([0], np.int64), "rnd": np.random.randn(1, 192, T).astype(np.float32)}
    (aud,), tv = run(vg, feeds)
    aud = aud.reshape(-1); print("key", KEY, "voice out", aud.shape, "samples/frame", len(aud) / T, "s", tv, "rms", float(np.sqrt((aud ** 2).mean())), "peak", float(abs(aud).max()))
    sf.write(f"out/voice_key{KEY}.wav", aud, 40000)
    np.save(f"out/io_key{KEY}.npz.npy", np.array([0]))
np.save("out/feat.npy", feat); np.save("out/f0.npy", f0)
# speed test: 1 s windows on CPU
w1 = x16[:16000]
t = time.time()
for _ in range(3):
    cv.run(None, {"input_values": w1[None], "attention_mask": np.ones((1, 16000), np.int64)}); rm.run(None, {"audio": w1[None]})
print("1s window contentvec+rmvpe x3 s", time.time() - t)
