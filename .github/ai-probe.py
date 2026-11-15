import sys, os, time, subprocess
import numpy as np, onnxruntime as ort, soundfile as sf, torch
from scipy.signal import resample_poly
os.makedirs("out", exist_ok=True)
sys.path.insert(0, os.path.abspath("rvc"))
from infer.lib.infer_pack.models_onnx import SynthesizerTrnMsNSFsidM
W = 100  # window, 10 ms frames
def export(pth, out, T):
    cpt = torch.load(pth, map_location="cpu", weights_only=False)
    cpt["config"][-3] = cpt["weight"]["emb_g.weight"].shape[0]
    net = SynthesizerTrnMsNSFsidM(*cpt["config"], is_half=False, version=cpt.get("version", "v1"))
    net.load_state_dict(cpt["weight"], strict=False); net.eval()
    args = (torch.rand(1, T, 768), torch.tensor([T]).long(), torch.randint(5, 255, (1, T)), torch.rand(1, T) * 200, torch.LongTensor([0]), torch.rand(1, 192, T))
    torch.onnx.export(net, args, out, do_constant_folding=False, opset_version=17, input_names=["phone", "phone_lengths", "pitch", "pitchf", "ds", "rnd"], output_names=["audio"])
    return int(cpt["config"][-1])
t = time.time(); SR = export("m/voice.pth", "m/voice.onnx", W); print("export", time.time() - t, "sr", SR, os.path.getsize("m/voice.onnx"))
so = ort.SessionOptions(); so.intra_op_num_threads = 2
cv = ort.InferenceSession("m/contentvec.onnx", so); rm = ort.InferenceSession("m/rmvpe.onnx", so); vg = ort.InferenceSession("m/voice.onnx", so)
for i in vg.get_inputs(): print(" in", i.name, i.shape)
subprocess.run(["espeak-ng", "-v", "en-us", "-s", "150", "-w", "out/in.wav", "Hello, this is a test of the Orvo voice changer. One two three four five."], check=True)
x, sr = sf.read("out/in.wav", dtype="float32"); x16 = resample_poly(x, 320, 441).astype(np.float32)  # 22050 -> 16000
CENTS = 1997.3794084376191 + 20.0 * np.arange(360)
def f0_of(a):
    sal = rm.run(None, {"audio": a[None]})[0][0].astype(np.float64)
    idx = np.clip(sal.argmax(1)[:, None] + np.arange(-4, 5)[None, :], 0, 359)
    w = np.maximum(np.take_along_axis(sal, idx, 1), 0)
    c = (w * CENTS[idx]).sum(1) / np.maximum(w.sum(1), 1e-12)
    return np.where(sal.max(1) > 0.03, 10 * 2 ** (c / 1200), 0.0)
def convert_window(a16, key):
    feat = cv.run(None, {"input_values": a16[None], "attention_mask": np.ones((1, len(a16)), np.int64)})[0]
    ph = np.repeat(feat, 2, axis=1)
    if ph.shape[1] < W: ph = np.concatenate([ph, np.repeat(ph[:, -1:], W - ph.shape[1], 1)], 1)
    ph = ph[:, :W].astype(np.float32)
    f0 = f0_of(a16)[:W] * 2 ** (key / 12)
    mel = 1127 * np.log(1 + f0 / 700); mn = 1127 * np.log(1 + 50 / 700); mx = 1127 * np.log(1 + 1100 / 700)
    coarse = np.clip(np.rint(np.where(mel > 0, (mel - mn) * 254 / (mx - mn) + 1, 1)), 1, 255).astype(np.int64)
    out = vg.run(None, {"phone": ph, "phone_lengths": np.array([W], np.int64), "pitch": coarse[None], "pitchf": f0[None].astype(np.float32),
                        "ds": np.array([0], np.int64), "rnd": np.random.randn(1, 192, W).astype(np.float32)})[0].reshape(-1)
    return out
# realtime simulation: block B frames, window W frames, crossfade XF samples at model sr
for key, B in [(0, 25), (12, 15)]:
    hop16 = 160; win = W * hop16; blk = B * hop16; spf = SR // 100; XF = spf * 2
    buf = np.zeros(win, np.float32); out = []; tail = None; times = []
    for s in range(0, len(x16) - blk, blk):
        buf = np.concatenate([buf[blk:], x16[s:s + blk]])
        t = time.time(); y = convert_window(buf, key); times.append(time.time() - t)
        n = len(y); seg = y[n - B * spf - XF: n - XF].copy(); nxt = y[n - XF:]
        if tail is not None:
            r = np.linspace(0, 1, XF, dtype=np.float32); seg[:XF] = seg[:XF] * r + tail * (1 - r)
        tail = nxt; out.append(seg)
    y = np.concatenate(out); sf.write(f"out/rt_key{key}_b{B}.wav", y, SR)
    yi16 = resample_poly(y, 2, 5).astype(np.float32)
    fi = f0_of(x16); fo = f0_of(yi16)
    print(f"key {key} block {B}: per-call ms mean {1000*np.mean(times):.0f} max {1000*np.max(times):.0f} (block {B*10} ms)",
          f"| in f0 med {np.median(fi[fi>0]):.0f} out f0 med {np.median(fo[fo>0]) if (fo>0).any() else 0:.0f} expect {np.median(fi[fi>0])*2**(key/12):.0f}",
          f"| out rms {np.sqrt((y**2).mean()):.3f} in rms {np.sqrt((x16**2).mean()):.3f} out len s {len(y)/SR:.2f} in len s {len(x16)/16000:.2f}")
# per-model timing split for the window
a = x16[:win]; ts = []
for f in [lambda: cv.run(None, {"input_values": a[None], "attention_mask": np.ones((1, win), np.int64)}), lambda: f0_of(a)]:
    t = time.time(); f(); f(); ts.append((time.time() - t) / 2 * 1000)
print("window ms contentvec %.0f rmvpe %.0f" % tuple(ts))
