#!/bin/sh
# Build a universal (Apple Silicon + Intel) rsqldev.app for macOS 11+:
#   scripts/bundle-macos.sh [--install]
# Needs: rustup target add aarch64-apple-darwin x86_64-apple-darwin
set -e
cd "$(dirname "$0")/.."
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release -p rsqldev --target "$t"
done
mkdir -p dist
APP=dist/rsqldev.app
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/rsqldev" \
  target/aarch64-apple-darwin/release/rsqldev target/x86_64-apple-darwin/release/rsqldev
cp app/assets/rsqldev.icns "$APP/Contents/Resources/rsqldev.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>rsqldev</string>
  <key>CFBundleDisplayName</key><string>rsqldev</string>
  <key>CFBundleIdentifier</key><string>dev.rsqldev.app</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>rsqldev</string>
  <key>CFBundleIconFile</key><string>rsqldev</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSArchitecturePriority</key><array><string>arm64</string><string>x86_64</string></array>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature so Gatekeeper accepts the locally built bundle.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
# Zip for sharing (ditto keeps the bundle's signature and symlinks intact).
(cd dist && rm -f rsqldev-macos-universal.zip && ditto -c -k --keepParent rsqldev.app rsqldev-macos-universal.zip)
echo "Built $APP and dist/rsqldev-macos-universal.zip"
if [ "$1" = "--install" ]; then
  rm -rf "$HOME/Applications/rsqldev.app"
  mkdir -p "$HOME/Applications"
  cp -R "$APP" "$HOME/Applications/"
  echo "Installed to ~/Applications/rsqldev.app"
fi
