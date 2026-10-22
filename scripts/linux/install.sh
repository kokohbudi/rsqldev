#!/bin/sh
# Install rsqldev for the current user: binary in ~/.local/bin, menu entry and icon.
set -e
cd "$(dirname "$0")"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/256x256/apps"
install -m 755 rsqldev "$HOME/.local/bin/rsqldev"
install -m 644 rsqldev.png "$HOME/.local/share/icons/hicolor/256x256/apps/rsqldev.png"
sed "s|^Exec=.*|Exec=$HOME/.local/bin/rsqldev|" rsqldev.desktop > "$HOME/.local/share/applications/rsqldev.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$HOME/.local/share/applications" || true
echo "Installed. Start rsqldev from the application menu, or run: $HOME/.local/bin/rsqldev"
