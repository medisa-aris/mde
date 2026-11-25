#!/bin/sh
# Builds target/release/mde.app
set -eu
cd "$(dirname "$0")/.."
cargo build --release
app=target/release/mde.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/mde "$app/Contents/MacOS/mde"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>mde</string>
  <key>CFBundleIdentifier</key><string>local.mde</string>
  <key>CFBundleExecutable</key><string>mde</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$app"
echo "Built $app"
