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
from infer.modules.onnx.export import export_onnx
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
ci = cv.get_inputs()[0]
shape_in = x16[None, None, :] if len(ci.shape) == 3 else x16[None, :]
(feat,), tc = run(cv, {ci.name: shape_in}); print("contentvec out", feat.shape, "s", tc)
ri = rm.get_inputs()
rf = {ri[0].name: (x16[None, :] if len(ri[0].shape) == 2 else x16)}
for extra in ri[1:]:
    rf[extra.name] = np.array(0.03, dtype=np.float32) if "float" in extra.type else np.array(0, dtype=np.int64)
ro, tr = run(rm, rf); print("rmvpe outs", [o.shape for o in ro], "s", tr)
for o in ro: print("   sample", np.round(o.reshape(-1)[:20], 2))
np.save("out/feat.npy", feat); [np.save(f"out/rmvpe{i}.npy", o) for i, o in enumerate(ro)]
