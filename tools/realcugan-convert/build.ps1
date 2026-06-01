# ONNX モデルを生成して src-tauri/ に配置する
# 重みファイルは Docker ビルド中に自動ダウンロードされる

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$outputDir = Resolve-Path (Join-Path $scriptDir "..\..\src-tauri")

Write-Host "Building ONNX models -> $outputDir"

docker build --output "type=local,dest=$outputDir" "$scriptDir"
