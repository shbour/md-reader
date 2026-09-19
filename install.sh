#!/bin/bash
# Build Markdown Reader and install it for the current user (no sudo).
#   ./install.sh              build, install, make it the default .md handler
#   ./install.sh --no-default build and install, leave the default app alone
#   ./install.sh --uninstall  remove everything this script installed
set -euo pipefail
cd "$(dirname "$0")"

APP_ID=io.github.mdreader.MdReader
BIN=~/.local/bin/mdreader
DESKTOP=~/.local/share/applications/$APP_ID.desktop
ICON=~/.local/share/icons/hicolor/scalable/apps/$APP_ID.svg

refresh() {
    update-desktop-database -q ~/.local/share/applications 2>/dev/null || true
    gtk-update-icon-cache -q -t ~/.local/share/icons/hicolor 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    refresh
    echo "Removed. If .md files still open with nothing, pick an app in Files > Open With."
    exit 0
fi

[[ -f ~/.cargo/env ]] && . ~/.cargo/env
cargo build --release

install -Dm755 target/release/mdreader "$BIN"
install -Dm644 data/$APP_ID.svg "$ICON"
install -Dm644 data/$APP_ID.desktop "$DESKTOP"
refresh

if [[ "${1:-}" != "--no-default" ]]; then
    xdg-mime default "$APP_ID.desktop" text/markdown text/x-markdown
fi

echo "Installed $BIN"
echo "Default for text/markdown: $(xdg-mime query default text/markdown)"
