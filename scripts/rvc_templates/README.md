# RVC templates

How `src-tauri/src/soundboard/rvc/*.tpl` were made. Orvo Studio fills these empty RVC v2
graphs with a voice's weights to turn a `.pth` into an `.onnx` (see `soundboard/pth.rs`).

1. Clone https://github.com/RVC-Project/Retrieval-based-Voice-Conversion-WebUI to `/home/claude/rvc`
   (or change the paths at the top of `export.py`), and `pip install torch onnx`.
2. `python export.py 32k`, `python export.py 40k`, `python export.py 48k`
3. `python mktpl.py` strips the weights and writes `rvc_v2_*.tpl`.
   Write the matching `rvc_v2_*.txt` (one initializer per line: name and dims, in graph order).

`export.py` patches RVC's relative attention so the graph accepts any length.
