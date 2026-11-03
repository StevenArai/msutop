#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
  echo "This script builds the macOS-only app; use the root Cargo project on Linux/Windows." >&2
  exit 1
fi
mkdir -p build
./scripts/build-renderer.sh
./scripts/build-sensors.sh
python3 scripts/pack-volume-font.py
xcrun clang -O2 -Wall -Wextra -mmacosx-version-min=13.0 -c Sources/Metrics.c -o build/Metrics.o
xcrun swiftc -O -swift-version 5 -target "$(uname -m)-apple-macosx13.0" \
  -import-objc-header Sources/Metrics.h Sources/PixelFont.swift Sources/VolumeDotFont.swift Sources/Animation.swift Sources/Sensors.swift Sources/main.swift build/Metrics.o build/libmsutop_mac_renderer.a \
  -framework AppKit -framework IOKit -framework CoreFoundation -framework SystemConfiguration -framework CoreVideo -o build/msutop-mac
# File Provider/iCloud can attach FinderInfo to .app directories after signing.
# Package in the local cache, then expose a convenient link from this workspace.
PACKAGE_ROOT="${MSUTOP_PACKAGE_DIR:-$HOME/Library/Caches/msutop-mac}"
APP="$PACKAGE_ROOT/msutop Mac.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/licenses" "$APP/Contents/Resources/fonts"
cp assets/fonts/k6x8/k6x8-gothic.bin "$APP/Contents/Resources/fonts/"
cp build/msutop-mac "$APP/Contents/MacOS/msutop-mac"
cp build/msutop-sensors "$APP/Contents/MacOS/msutop-sensors"
cp -R licenses/. "$APP/Contents/Resources/licenses/"
cp NOTICE.txt "$APP/Contents/Resources/NOTICE.txt"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>msutop Mac</string>
<key>CFBundleDisplayName</key><string>msutop Mac</string>
<key>CFBundleIdentifier</key><string>local.msutop.mac</string>
<key>CFBundleExecutable</key><string>msutop-mac</string>
<key>CFBundleVersion</key><string>9</string>
<key>CFBundleShortVersionString</key><string>0.8.0</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
build/msutop-mac --icon build/icon.png
mkdir -p build/AppIcon.iconset
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" build/icon.png --out "build/AppIcon.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" build/icon.png --out "build/AppIcon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns build/AppIcon.iconset -o "$APP/Contents/Resources/AppIcon.icns"
xattr -cr "$APP"
codesign --force --sign - "$APP/Contents/MacOS/msutop-sensors"
codesign --force --sign - "$APP"
codesign --verify --deep --strict "$APP"
if [[ -d "build/msutop Mac.app" && ! -L "build/msutop Mac.app" ]]; then
  rm -rf -- "build/msutop Mac.app"
fi
ln -sfn "$APP" "build/msutop Mac.app"
echo "Built: $APP"
