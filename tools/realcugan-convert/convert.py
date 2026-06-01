import sys
import torch
import torch.nn as nn
import torch.nn.functional as F

sys.path.insert(0, ".")
from upcunet_v3 import UpCunet2x, UpCunet4x


class UpCunet2xONNX(nn.Module):
    """tile_mode=0 (全体処理) に固定した ONNX エクスポート用ラッパー。
    入力: float32 (1, 3, H, W) [0, 1]、H と W は偶数であること。
    出力: float32 (1, 3, H*2, W*2) [0, 1]
    """

    def __init__(self, model: UpCunet2x):
        super().__init__()
        self.unet1 = model.unet1
        self.unet2 = model.unet2

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        x = F.pad(x, (18, 18, 18, 18), "reflect")
        feat = self.unet1(x)
        x0 = self.unet2(feat, 1.0)
        feat = F.pad(feat, (-20, -20, -20, -20))
        out = torch.add(x0, feat)
        return out.clamp(0.0, 1.0)


class UpCunet4xONNX(nn.Module):
    """tile_mode=0 (全体処理) に固定した ONNX エクスポート用ラッパー。
    入力: float32 (1, 3, H, W) [0, 1]、H と W は偶数であること。
    出力: float32 (1, 3, H*4, W*4) [0, 1]
    """

    def __init__(self, model: UpCunet4x):
        super().__init__()
        self.unet1 = model.unet1
        self.unet2 = model.unet2
        self.conv_final = model.conv_final
        self.ps = model.ps

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        x00 = x
        x = F.pad(x, (19, 19, 19, 19), "reflect")
        feat = self.unet1(x)
        x0 = self.unet2(feat, 1.0)
        x1 = F.pad(feat, (-20, -20, -20, -20))
        x = torch.add(x0, x1)
        x = self.conv_final(x)
        x = F.pad(x, (-1, -1, -1, -1))
        x = self.ps(x)
        x = x + F.interpolate(x00, scale_factor=4, mode="nearest")
        return x.clamp(0.0, 1.0)


def convert2x(weight_path: str, output_path: str) -> None:
    print(f"Loading weights: {weight_path}")
    base = UpCunet2x()
    state = torch.load(weight_path, map_location="cpu")
    base.load_state_dict(state, strict=True)
    base.eval()

    model = UpCunet2xONNX(base)
    model.eval()

    dummy = torch.zeros(1, 3, 256, 256)

    print(f"Exporting to: {output_path}")
    with torch.no_grad():
        torch.onnx.export(
            model,
            dummy,
            output_path,
            opset_version=17,
            input_names=["input"],
            output_names=["output"],
            dynamic_axes={
                "input":  {2: "height", 3: "width"},
                "output": {2: "height", 3: "width"},
            },
        )
    print("Done.")


def convert4x(weight_path: str, output_path: str) -> None:
    print(f"Loading weights: {weight_path}")
    base = UpCunet4x()
    state = torch.load(weight_path, map_location="cpu")
    base.load_state_dict(state, strict=True)
    base.eval()

    model = UpCunet4xONNX(base)
    model.eval()

    dummy = torch.zeros(1, 3, 256, 256)

    print(f"Exporting to: {output_path}")
    with torch.no_grad():
        torch.onnx.export(
            model,
            dummy,
            output_path,
            opset_version=17,
            input_names=["input"],
            output_names=["output"],
            dynamic_axes={
                "input":  {2: "height", 3: "width"},
                "output": {2: "height", 3: "width"},
            },
        )
    print("Done.")


if __name__ == "__main__":
    convert2x(
        "updated_weights/up2x-latest-conservative.pth",
        "realcugan_2x_conservative.onnx",
    )
    convert4x(
        "updated_weights/up4x-latest-conservative.pth",
        "realcugan_4x_conservative.onnx",
    )
