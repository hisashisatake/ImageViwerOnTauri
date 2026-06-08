import torch
from spandrel import ModelLoader


def convert(weight_path: str, output_path: str) -> None:
    """OmniSR系モデル(.pth)をONNXへ変換する。
    spandrelがOmniSRアーキテクチャの実装と重みのロードを担うため、
    RealCUGANの変換と異なりアーキテクチャを手動で再実装する必要はない。

    注意: forward()内部のwindow_size単位パディング/クロップは Python の
    if 分岐や reshape で実装されており、PyTorch 2.2 のトレースベース
    ONNXエクスポートでは「ダミー入力サイズに応じた固定形状」として
    グラフに焼き込まれてしまう(=他サイズの入力に汎化しない)。
    そのため、ここでは入力を「FIXED_SIZE固定」で変換し、
    呼び出し側(Rust)が常にこのサイズへパディングしてから推論する設計とする。

    入力: float32 (1, 3, FIXED_SIZE, FIXED_SIZE) [0, 1]
    出力: float32 (1, 3, FIXED_SIZE*scale, FIXED_SIZE*scale) [0, 1]
    """
    print(f"Loading weights: {weight_path}")
    descriptor = ModelLoader().load_from_file(weight_path)
    model = descriptor.model
    model.eval()

    dummy = torch.zeros(1, 3, FIXED_SIZE, FIXED_SIZE)

    print(f"Exporting to: {output_path}")
    with torch.no_grad():
        torch.onnx.export(
            model,
            dummy,
            output_path,
            opset_version=17,
            input_names=["input"],
            output_names=["output"],
        )
    print("Done.")


# src-tauri/src/upscale.rs の TILE_SIZE(256) + TILE_OVERLAP(32)*2 = 320 と
# 一致させる(タイル分割が生成しうる最大の拡張タイルサイズを固定入力として
# カバーするため)。タイル分割側の定数を変更する場合はここも合わせて
# 再変換が必要になる。
FIXED_SIZE = 128


if __name__ == "__main__":
    convert("DWTP_descreenton_VL4.pth", "descreenton_vl4.onnx")
    convert("DWTP_descreenton_VH4.pth", "descreenton_vh4.onnx")
