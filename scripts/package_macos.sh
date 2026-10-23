#!/usr/bin/env bash
set -e

echo "=== Building Eclipse Release Binary ==="
cargo build --release

APP_NAME="Eclipse"
BUNDLE_DIR="$APP_NAME.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "=== Creating macOS App Bundle ($BUNDLE_DIR) ==="
rm -rf "$BUNDLE_DIR"
mkdir -p "$MACOS_DIR"
mkdir -p "$RESOURCES_DIR"

# Copy binary
cp target/release/eclipse "$MACOS_DIR/eclipse"
chmod +x "$MACOS_DIR/eclipse"

# Generate Iconset and .icns
echo "=== Generating macOS AppIcon.icns ==="
python3 scripts/generate_icon.py

ICONSET="scripts/AppIcon.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET"

BASE_ICON="scripts/icon_512.png"
sips -z 16 16     "$BASE_ICON" --out "$ICONSET/icon_16x16.png" >/dev/null
sips -z 32 32     "$BASE_ICON" --out "$ICONSET/icon_16x16@2x.png" >/dev/null
sips -z 32 32     "$BASE_ICON" --out "$ICONSET/icon_32x32.png" >/dev/null
sips -z 64 64     "$BASE_ICON" --out "$ICONSET/icon_32x32@2x.png" >/dev/null
sips -z 128 128   "$BASE_ICON" --out "$ICONSET/icon_128x128.png" >/dev/null
sips -z 256 256   "$BASE_ICON" --out "$ICONSET/icon_128x128@2x.png" >/dev/null
sips -z 256 256   "$BASE_ICON" --out "$ICONSET/icon_256x256.png" >/dev/null
sips -z 512 512   "$BASE_ICON" --out "$ICONSET/icon_256x256@2x.png" >/dev/null
sips -z 512 512   "$BASE_ICON" --out "$ICONSET/icon_512x512.png" >/dev/null

iconutil -c icns "$ICONSET" -o "$RESOURCES_DIR/AppIcon.icns"
rm -rf "$ICONSET"

# Write Info.plist
cat << 'EOF' > "$CONTENTS_DIR/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>eclipse</string>
    <key>CFBundleIdentifier</key>
    <string>com.eclipse.browser</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Eclipse</string>
    <key>CFBundleDisplayName</key>
    <string>Eclipse Browser</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2026 Eclipse Team. All rights reserved.</string>
    <key>NSAppTransportSecurity</key>
    <dict>
        <key>NSAllowsArbitraryLoads</key>
        <true/>
    </dict>
</dict>
</plist>
EOF

# Ad-hoc sign the app so macOS runs it natively without Gatekeeper prompt
echo "=== Ad-hoc Signing Eclipse.app ==="
codesign --force --deep -s - "$BUNDLE_DIR"

echo "=== Packaging DMG Installer (Eclipse-Installer.dmg) ==="
DMG_STAGE="dmg_stage"
rm -rf "$DMG_STAGE" Eclipse-Installer.dmg
mkdir -p "$DMG_STAGE"

# Copy App into staging
cp -R "$BUNDLE_DIR" "$DMG_STAGE/"

# Create symlink to /Applications for standard drag-and-drop installation
ln -s /Applications "$DMG_STAGE/Applications"

# Create DMG
hdiutil create -volname "Eclipse Browser" \
               -srcfolder "$DMG_STAGE" \
               -ov \
               -format UDZO \
               "Eclipse-Installer.dmg"

rm -rf "$DMG_STAGE"

echo ""
echo "===================================================="
echo "✅ Eclipse.app and Eclipse-Installer.dmg created successfully!"
echo "📍 Application Bundle: $(pwd)/$BUNDLE_DIR"
echo "📍 DMG Installer:     $(pwd)/Eclipse-Installer.dmg"
echo "===================================================="
