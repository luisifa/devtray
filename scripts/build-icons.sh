#!/usr/bin/env bash
# 从 assets/*.svg 生成各尺寸 PNG 到 assets/icons/hicolor/<size>/apps/。
# 换图标后跑这个，再跑 scripts/install-local.sh（或重新打包）就生效。
set -euo pipefail
cd "$(dirname "$0")/.."

sizes=(16 22 24 32 48 64 128 256)
names=(devtray devtray-starting devtray-error devtray-idle)

command -v rsvg-convert >/dev/null || {
  echo "需要 rsvg-convert（Arch: librsvg / Debian: librsvg2-bin）" >&2
  exit 1
}

for name in "${names[@]}"; do
  svg="assets/$name.svg"
  [ -f "$svg" ] || { echo "跳过：$svg 不存在" >&2; continue; }
  install -Dm644 "$svg" "assets/icons/hicolor/scalable/apps/$name.svg"
  for size in "${sizes[@]}"; do
    install -d "assets/icons/hicolor/${size}x${size}/apps"
    rsvg-convert -w "$size" -h "$size" "$svg" -o "assets/icons/hicolor/${size}x${size}/apps/$name.png"
  done
done

python3 scripts/export-icon-pixmaps.py

echo "图标已生成：assets/icons/hicolor/"
