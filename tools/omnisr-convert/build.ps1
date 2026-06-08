# ONNX モデルを生成して src-tauri/ に配置する
# 重みファイルは Docker ビルド中に自動ダウンロードされる
#
# オプション:
#   -NoCache    Docker キャッシュを使わず全ステップを再実行する

param(
    [switch]$NoCache
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$outputDir = Resolve-Path (Join-Path $scriptDir "..\..\src-tauri")

Write-Host "Building ONNX models -> $outputDir"

if ($NoCache) {
    Write-Host "Running with --no-cache"
    docker build --no-cache --output "type=local,dest=$outputDir" "$scriptDir"
} else {
    docker build --output "type=local,dest=$outputDir" "$scriptDir"
}
