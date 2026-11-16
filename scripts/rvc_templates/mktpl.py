import onnx, json, sys
for sr in ["32k", "40k", "48k"]:
    m = onnx.load(f"full_{sr}.onnx")
    del m.graph.initializer[:]
    cfg = json.load(open(f"cfg_{sr}.json"))
    m.metadata_props.add(key="metadata", value=json.dumps({"samplingRate": cfg[-1]}))
    onnx.save(m, f"rvc_v2_{sr}.tpl")
    print(sr, len(m.SerializeToString()), "bytes")
