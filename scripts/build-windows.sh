#!/bin/sh
# Cross-build rsqldev for Windows x64 from macOS/Linux and package dist/rsqldev-windows-x64.zip.
# Needs: rustup target add x86_64-pc-windows-gnu; brew install mingw-w64 nasm
set -e
cd "$(dirname "$0")/.."
T=x86_64-pc-windows-gnu
cargo build --release -p rsqldev --target $T
OUT=dist/rsqldev-windows-x64
rm -rf "$OUT" && mkdir -p "$OUT"
cp target/$T/release/rsqldev.exe "$OUT/"

# Ship any MinGW runtime DLL the exe imports (libstdc++, libgcc, winpthread);
# Windows system DLLs are not found in the toolchain and are left alone.
PREFIX=$(cd "$(dirname "$(readlink -f "$(command -v x86_64-w64-mingw32-gcc)")")/.." && pwd)
copy_deps() {
  for dll in $(x86_64-w64-mingw32-objdump -p "$1" | awk '/DLL Name:/ {print $3}' | sort -u); do
    [ -f "$OUT/$dll" ] && continue
    src=$(find "$PREFIX" -path '*x86_64*' -name "$dll" 2>/dev/null | head -1)
    if [ -n "$src" ]; then
      cp "$src" "$OUT/"
      echo "bundled $dll"
      copy_deps "$OUT/$dll"
    fi
  done
}
copy_deps "$OUT/rsqldev.exe"
# C++ deps (DuckDB, wxWidgets) leave symbols behind: strip to cut the download size.
x86_64-w64-mingw32-strip --strip-unneeded "$OUT"/*.exe "$OUT"/*.dll

(cd dist && rm -f rsqldev-windows-x64.zip && zip -qr rsqldev-windows-x64.zip rsqldev-windows-x64)
rm -rf "$OUT"   # keep dist/ to final files only
echo "Built dist/rsqldev-windows-x64.zip"
