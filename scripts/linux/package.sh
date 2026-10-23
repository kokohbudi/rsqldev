#!/bin/sh
# Package a built Linux binary: scripts/linux/package.sh <arch> <target dir>
#   -> dist/rsqldev-linux-<arch>.tar.gz (binary, install.sh, .desktop entry, icon)
set -e
cd "$(dirname "$0")/../.."
ARCH=$1; TARGET=${2:-target}
OUT=dist/rsqldev-linux-$ARCH
rm -rf "$OUT" && mkdir -p "$OUT"
cp "$TARGET/release/rsqldev" "$OUT/"
strip "$OUT/rsqldev"
cp app/assets/icon-256.png "$OUT/rsqldev.png"
cp scripts/linux/rsqldev.desktop scripts/linux/install.sh "$OUT/"
tar -C dist -czf "dist/rsqldev-linux-$ARCH.tar.gz" "rsqldev-linux-$ARCH"
rm -rf "$OUT"
echo "Built dist/rsqldev-linux-$ARCH.tar.gz"
