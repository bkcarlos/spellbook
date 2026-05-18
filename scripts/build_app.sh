#!/usr/bin/env bash
# Build Spellbook.app — a macOS bundle that can be dragged to /Applications,
# launched from Spotlight / Launchpad, and pinned to the Dock.
#
# Usage:
#   ./scripts/build_app.sh          # builds for the host architecture
#   ./scripts/build_app.sh universal # arm64 + x86_64 lipo'd into one binary
#
# Output: target/Spellbook.app
#
# This is unsigned. macOS Gatekeeper will warn on first launch; users right-
# click → Open the first time to bypass. Codesigning + notarization requires
# an Apple Developer ID ($99/yr) — out of scope for now.
set -eu

cd "$(dirname "$0")/.."

MODE="${1:-host}"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
APP_DIR="target/Spellbook.app"
ICONSET=/tmp/Spellbook.iconset

echo "→ building binary (${MODE})"
case "$MODE" in
  universal)
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    mkdir -p target/universal
    lipo -create -output target/universal/spellbook \
      target/aarch64-apple-darwin/release/spellbook \
      target/x86_64-apple-darwin/release/spellbook
    BIN=target/universal/spellbook
    ;;
  host|*)
    cargo build --release
    BIN=target/release/spellbook
    ;;
esac

echo "→ regenerating AppIcon.icns from assets/icon-source.png"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"
for spec in 16:16x16 32:16x16@2x 32:32x32 64:32x32@2x 128:128x128 256:128x128@2x 256:256x256 512:256x256@2x 512:512x512 1024:512x512@2x; do
  size=${spec%%:*}
  name=${spec#*:}
  sips -Z "$size" assets/icon-source.png --out "$ICONSET/icon_${name}.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o assets/AppIcon.icns
rm -rf "$ICONSET"

echo "→ assembling ${APP_DIR}"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cp "$BIN" "$APP_DIR/Contents/MacOS/spellbook"
chmod +x "$APP_DIR/Contents/MacOS/spellbook"
cp assets/AppIcon.icns "$APP_DIR/Contents/Resources/AppIcon.icns"

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>                <string>Spellbook</string>
  <key>CFBundleDisplayName</key>         <string>Spellbook</string>
  <key>CFBundleIdentifier</key>          <string>io.github.bkcarlos.spellbook</string>
  <key>CFBundleVersion</key>             <string>${VERSION}</string>
  <key>CFBundleShortVersionString</key>  <string>${VERSION}</string>
  <key>CFBundleExecutable</key>          <string>spellbook</string>
  <key>CFBundleIconFile</key>            <string>AppIcon</string>
  <key>CFBundlePackageType</key>         <string>APPL</string>
  <key>CFBundleSignature</key>           <string>????</string>
  <key>LSMinimumSystemVersion</key>      <string>10.15</string>
  <key>NSHighResolutionCapable</key>     <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
  <key>LSApplicationCategoryType</key>   <string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

# Tell macOS its metadata changed (so the new icon shows up in Finder)
touch "$APP_DIR"

echo
echo "✓ built  ${APP_DIR}"
du -sh "$APP_DIR" | awk '{print "  size:  " $1}'
file "$APP_DIR/Contents/MacOS/spellbook" | sed 's/^/  arch:  /'
echo
echo "Try it:"
echo "  open $APP_DIR"
echo
echo "Install:"
echo "  mv $APP_DIR /Applications/"
echo "  open /Applications/Spellbook.app"
