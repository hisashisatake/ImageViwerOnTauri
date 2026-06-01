# syntax=docker/dockerfile:1
FROM pytorch/pytorch:latest AS builder

RUN pip install onnx -q --no-cache-dir

WORKDIR /work

RUN python3 - <<'EOF'
import urllib.request, zipfile, io
print("Downloading weight files...")
url = "https://github.com/bilibili/ailab/releases/download/Real-CUGAN/updated_weights.zip"
data = urllib.request.urlopen(url).read()
zipfile.ZipFile(io.BytesIO(data)).extractall(".")
print("Done.")
EOF

COPY upcunet_v3.py convert.py ./
RUN python convert.py

FROM scratch
COPY --from=builder /work/realcugan_2x_conservative.onnx /
COPY --from=builder /work/realcugan_4x_conservative.onnx /
