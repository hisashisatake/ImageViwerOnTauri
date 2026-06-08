import sys
import numpy as np
import onnxruntime as ort

# convert.py の FIXED_SIZE と一致させる。固定形状エクスポートのため、
# 推論はこのサイズの入力でのみ動作することを確認する。
FIXED_SIZE = 128

def verify(model_path: str, scale: int) -> None:
    sess = ort.InferenceSession(model_path, providers=["CPUExecutionProvider"])
    h = w = FIXED_SIZE
    x = np.random.rand(1, 3, h, w).astype(np.float32)
    out = sess.run(["output"], {"input": x})[0]
    expected = (1, 3, h * scale, w * scale)
    status = "OK" if out.shape == expected else "MISMATCH"
    print(f"  {model_path}: input={x.shape} -> output={out.shape} expected={expected} [{status}]")

if __name__ == "__main__":
    verify("descreenton_vl4.onnx", 4)
    verify("descreenton_vh4.onnx", 4)
