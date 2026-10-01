#!/bin/sh
# Install UOC Journal for the current user (no root needed).
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
mkdir -p "$BIN" "$DATA/applications" "$DATA/icons/hicolor/64x64/apps"
install -m 755 "$HERE/uoc-journal" "$BIN/uoc-journal"
install -m 644 "$HERE/uoc-journal.png" "$DATA/icons/hicolor/64x64/apps/uoc-journal.png"
sed "s|^Exec=.*|Exec=$BIN/uoc-journal|" "$HERE/uoc-journal.desktop" > "$DATA/applications/uoc-journal.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" || true
echo "Installed to $BIN/uoc-journal (menu entry: UOC Journal)."
case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: $BIN is not on your PATH." ;; esac
