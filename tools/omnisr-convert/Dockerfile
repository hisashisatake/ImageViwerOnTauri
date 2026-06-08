# syntax=docker/dockerfile:1
FROM pytorch/pytorch:latest AS builder

RUN pip install onnx onnxruntime spandrel -q --no-cache-dir

WORKDIR /work

RUN python3 - <<'EOF'
import urllib.request
print("Downloading weight files...")
files = {
    "DWTP_descreenton_VL4.pth": "https://drive.google.com/uc?export=download&id=1U1BUbqRE6rxHWVUDNnvfrlG83vHo5bvp",
    "DWTP_descreenton_VH4.pth": "https://drive.google.com/uc?export=download&id=16BPbb1cjp2v9Z_XKZdDUTPFayOcfDIs6",
}
for name, url in files.items():
    print(f"  {name}")
    urllib.request.urlretrieve(url, name)
print("Done.")
EOF

COPY convert.py verify.py ./
RUN python convert.py
RUN python verify.py

FROM scratch
COPY --from=builder /work/descreenton_vl4.onnx /
COPY --from=builder /work/descreenton_vh4.onnx /
