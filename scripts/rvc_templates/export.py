import sys, json, torch
sys.path.insert(0, "/home/claude/rvc")
from infer.module.models import SynthesizerTrnMs768NSFsid


import torch.nn.functional as F
from infer.module import attentions as _att

def _rel_emb(self, rel, length):
    w = self.window_size
    r = torch.arange(2 * length - 1, device=rel.device) - (length - 1)
    keep = (r.abs() <= w).to(rel.dtype)
    idx = (r.clamp(-w, w) + w)
    return rel.index_select(1, idx) * keep.view(1, -1, 1)

def _rel2abs(self, x):
    batch, heads, length, _ = x.size()
    x = F.pad(x, [0, 1, 0, 0, 0, 0, 0, 0])
    x_flat = x.view([batch, heads, length * 2 * length])
    x_flat = F.pad(x_flat, [0, length - 1, 0, 0, 0, 0])
    return x_flat.view([batch, heads, length + 1, 2 * length - 1])[:, :, :length, length - 1:]

def _abs2rel(self, x):
    batch, heads, length, _ = x.size()
    x = F.pad(x, [0, length - 1, 0, 0, 0, 0, 0, 0])
    x_flat = x.view([batch, heads, length * (2 * length - 1)])
    x_flat = F.pad(x_flat, [length, 0, 0, 0, 0, 0])
    return x_flat.view([batch, heads, length, 2 * length])[:, :, :, 1:]

_att.MultiHeadAttention._get_relative_embeddings = _rel_emb
_att.MultiHeadAttention._relative_position_to_absolute_position = _rel2abs
_att.MultiHeadAttention._absolute_position_to_relative_position = _abs2rel

CFG = {"32k": "v2/32k.json", "40k": "v1/40k.json", "48k": "v2/48k.json"}

def config_list(sr):
    c = json.load(open(f"/home/claude/rvc/configs/{CFG[sr]}"))
    d, m, t = c["data"], c["model"], c["train"]
    return [d["filter_length"] // 2 + 1, t["segment_size"] // d["hop_length"], m["inter_channels"], m["hidden_channels"],
            m["filter_channels"], m["n_heads"], m["n_layers"], m["kernel_size"], m["p_dropout"], m["resblock"],
            m["resblock_kernel_sizes"], m["resblock_dilation_sizes"], m["upsample_rates"], m["upsample_initial_channel"],
            m["upsample_kernel_sizes"], m["spk_embed_dim"], m["gin_channels"], d["sampling_rate"]]

class Wrap(torch.nn.Module):
    def __init__(self, net):
        super().__init__(); self.net = net
    def forward(self, phone, phone_lengths, pitch, pitchf, ds, rnd):
        n = self.net
        g = n.emb_g(ds).unsqueeze(-1)
        m_p, logs_p, x_mask = n.enc_p(phone, pitch, phone_lengths)
        z_p = (m_p + torch.exp(logs_p) * rnd * 0.66666) * x_mask
        z = n.flow(z_p, x_mask, g=g, reverse=True)
        return n.dec(z * x_mask, pitchf, g=g)

def build(cfg):
    net = SynthesizerTrnMs768NSFsid(*cfg, is_half=False)
    del net.enc_q
    net.eval()
    return net

if __name__ == "__main__":
    sr = sys.argv[1]
    cfg = config_list(sr)
    net = build(cfg)
    net.remove_weight_norm()
    # give every weight distinct values so the exporter can't merge equal ones into one
    with torch.no_grad():
        for p in net.parameters():
            p.add_(torch.rand_like(p) * 1e-3)
    w = Wrap(net).eval()
    T = 64
    args = (torch.randn(1, T, 768), torch.tensor([T]), torch.randint(1, 255, (1, T)), torch.rand(1, T) * 200 + 100,
            torch.tensor([0]), torch.randn(1, 192, T))
    names = ["phone", "phone_lengths", "pitch", "pitchf", "ds", "rnd"]
    torch.onnx.export(w, args, f"full_{sr}.onnx", input_names=names, output_names=["audio"], opset_version=17,
                      do_constant_folding=False, dynamo=False,
                      dynamic_axes={"phone": [1], "pitch": [1], "pitchf": [1], "rnd": [2], "audio": [2]})
    json.dump(cfg, open(f"cfg_{sr}.json", "w"))
    print("ok", sr)
