# makes the 48 kHz test input and checks the Rust output's pitch and level
import sys, numpy as np, soundfile as sf, onnxruntime as ort
from scipy.signal import resample_poly
mode = sys.argv[1]
if mode == "make":
    x, sr = sf.read("out/in.wav", dtype="float32")
    y = resample_poly(x, 320, 147).astype(np.float32)  # 22050 -> 48000
    y.tofile("out/in48.f32"); print("in48", len(y))
else:
    rm = ort.InferenceSession("m/rmvpe.onnx")
    C = 1997.3794084376191 + 20.0 * np.arange(360)
    def f0(a):
        s = rm.run(None, {"audio": a[None]})[0][0].astype(np.float64)
        i = np.clip(s.argmax(1)[:, None] + np.arange(-4, 5)[None], 0, 359); w = np.maximum(np.take_along_axis(s, i, 1), 0)
        c = (w * C[i]).sum(1) / np.maximum(w.sum(1), 1e-12); return np.where(s.max(1) > 0.03, 10 * 2 ** (c / 1200), 0)
    a = np.fromfile("out/in48.f32", np.float32); b = np.fromfile(sys.argv[2], np.float32)
    fa = f0(resample_poly(a, 1, 3).astype(np.float32)); fb = f0(resample_poly(b, 1, 3).astype(np.float32))
    ma, mb = np.median(fa[fa > 0]), np.median(fb[fb > 0]) if (fb > 0).any() else 0
    ra, rb = np.sqrt((a ** 2).mean()), np.sqrt((b ** 2).mean())
    print(f"in f0 {ma:.0f} out f0 {mb:.0f} | in rms {ra:.3f} out rms {rb:.3f} | voiced in {(fa>0).mean():.2f} out {(fb>0).mean():.2f}")
    sf.write(sys.argv[2] + ".wav", b, 48000)
    assert abs(mb / ma - 1) < 0.08, "pitch off"
    assert rb > ra * 0.3, "too quiet"
    print("OK")
