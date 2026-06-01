#!/usr/bin/env bash
# ONNX モデルを生成して src-tauri/ に配置する
# 重みファイルは Docker ビルド中に自動ダウンロードされる

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="$(cd "$SCRIPT_DIR/../../src-tauri" && pwd)"

echo "Building ONNX models -> $OUTPUT_DIR"

docker build --output "type=local,dest=$OUTPUT_DIR" "$SCRIPT_DIR"
