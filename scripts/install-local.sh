#!/usr/bin/env bash
# 本机开发安装（不走 deb/rpm）：装二进制 + 图标 + desktop + 指向 ~/.cargo/bin 的 user unit。
set -euo pipefail
cd "$(dirname "$0")/.."

cargo install --locked --path .

[ -d assets/icons/hicolor ] || ./scripts/build-icons.sh
ICONS="$HOME/.local/share/icons/hicolor"
mkdir -p "$ICONS"
cp -r assets/icons/hicolor/. "$ICONS/"
if command -v gtk-update-icon-cache >/dev/null; then
  gtk-update-icon-cache -f -t "$ICONS" >/dev/null 2>&1 || true
fi

install -Dm644 assets/devtray.desktop "$HOME/.local/share/applications/devtray.desktop"
install -Dm644 devtray.service "$HOME/.config/systemd/user/devtray.service"
mkdir -p "$HOME/.local/state/devtray"
systemctl --user daemon-reload

echo
echo "已安装：$(command -v devtray)"
echo "图标：$ICONS/{scalable,22x22,...}/apps/devtray*.svg|png"
echo "启用登录自启：systemctl --user enable --now devtray.service"
