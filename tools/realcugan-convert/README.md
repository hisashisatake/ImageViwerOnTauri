# realcugan-convert

Real-CUGAN の学習済みモデルを ONNX 形式に変換し、`src-tauri/` に配置するツールです。

## 前提条件

- Docker Desktop がインストール・起動済みであること

## 使い方

### Windows (PowerShell)

```powershell
.\build.ps1
```

### Linux / macOS

```bash
chmod +x build.sh
./build.sh
```

実行すると以下が自動で行われます：

1. PyTorch の Docker イメージを取得
2. 重みファイル（`.pth`）を GitHub から自動ダウンロード
3. `convert.py` で ONNX 形式に変換
4. 生成した `.onnx` ファイルを `src-tauri/` に配置

## 生成されるファイル

| ファイル | 説明 |
|---|---|
| `src-tauri/realcugan_2x_conservative.onnx` | 2倍超解像モデル（保守版） |
| `src-tauri/realcugan_4x_conservative.onnx` | 4倍超解像モデル（保守版） |

## ファイル構成

```
realcugan-convert/
├── Dockerfile       # ビルド定義
├── build.ps1        # 実行スクリプト (Windows)
├── build.sh         # 実行スクリプト (Linux/macOS)
├── convert.py       # ONNX 変換スクリプト
└── upcunet_v3.py    # Real-CUGAN モデル定義 (bilibili/ailab 製、パッチ適用済み)
```

## 備考

- 重みファイル（`updated_weights/*.pth`）はビルド時に自動取得されます。手動配置は不要です。
- Docker のレイヤーキャッシュにより、2回目以降は `pip install` やダウンロードがスキップされ高速になります。
- モデルは [bilibili/ailab](https://github.com/bilibili/ailab/tree/main/Real-CUGAN) の Apache 2.0 ライセンスで配布されています。
