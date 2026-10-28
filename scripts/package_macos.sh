#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Make a normal Finder-launched terminal find rustup/cargo without hardcoding
# one developer's home directory.
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

APP_NAME="Eclipse"
BUNDLE_DIR="$APP_NAME.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "=== Building Eclipse Release Binary ==="
if command -v cargo >/dev/null 2>&1; then
    cargo build --release
else
    echo "cargo not found; falling back to the standalone Objective-C WebKit build"
    echo "note: the standalone build does not include the native menu/token bridge from src/"
    python3 - <<'PY'
import json
for name in ["sidebar", "newtab", "settings"]:
    content = open(f"assets/{name}.html", "r", encoding="utf-8").read()
    with open(f"assets/generated_{name}_cstr.h", "w", encoding="utf-8") as f:
        f.write(json.dumps(content))
PY
    mkdir -p target/release
    clang -fobjc-arc -O3 -flto \
          -framework Cocoa -framework WebKit \
          scripts/standalone_macos.m \
          -o target/release/eclipse
    strip -x target/release/eclipse
    rm -f assets/generated_*_cstr.h
fi

echo "=== Creating macOS App Bundle ($BUNDLE_DIR) ==="
rm -rf "$BUNDLE_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp target/release/eclipse "$MACOS_DIR/eclipse"
chmod +x "$MACOS_DIR/eclipse"

echo "=== Generating macOS AppIcon.icns ==="
python3 scripts/generate_icon.py

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
    <string>1.2.0</string>
    <key>CFBundleVersion</key>
    <string>3</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.productivity</string>
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
        <key>NSAllowsLocalNetworking</key>
        <true/>
    </dict>
    <key>NSCameraUsageDescription</key>
    <string>Eclipse gives camera access to a website only after you allow it.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Eclipse gives microphone access to a website only after you allow it.</string>
    <key>NSLocalNetworkUsageDescription</key>
    <string>Eclipse may access your local network when a website asks for it (for example, video calls or local development servers).</string>
</dict>
</plist>
EOF

plutil -lint "$CONTENTS_DIR/Info.plist"

echo "=== Ad-hoc Signing Eclipse.app with Runtime Entitlements ==="
xattr -cr "$BUNDLE_DIR"
codesign --force --deep --options runtime --entitlements scripts/entitlements.plist -s - "$BUNDLE_DIR"
xattr -cr "$BUNDLE_DIR"
codesign --verify --deep --strict --verbose=1 "$BUNDLE_DIR" 2>&1 | tail -2

if command -v hdiutil >/dev/null 2>&1 || command -v diskutil >/dev/null 2>&1; then
    echo "=== Creating Eclipse-Installer.dmg (Drag-to-Applications Installer) ==="
    rm -f "Eclipse-Installer.dmg"
    DMG_STAGING="target/dmg_staging"
    rm -rf "$DMG_STAGING"
    mkdir -p "$DMG_STAGING"
    cp -R "$BUNDLE_DIR" "$DMG_STAGING/"
    xattr -cr "$DMG_STAGING"
    ln -s /Applications "$DMG_STAGING/Applications"

    dmg_ok=0
    if command -v hdiutil >/dev/null 2>&1; then
        if hdiutil create -volname "Eclipse" -srcfolder "$DMG_STAGING" -ov -format UDZO -fs HFS+ \
            "Eclipse-Installer.dmg" >/dev/null 2>&1 \
            && [ -f "Eclipse-Installer.dmg" ]; then
            dmg_ok=1
        fi
    fi
    if [ "$dmg_ok" -eq 0 ] && command -v diskutil >/dev/null 2>&1; then
        if diskutil image create from "$DMG_STAGING" --volumeName "Eclipse" \
            --format UDZO "Eclipse-Installer.dmg" >/dev/null 2>&1 \
            && [ -f "Eclipse-Installer.dmg" ]; then
            dmg_ok=1
        fi
    fi
    if [ "$dmg_ok" -eq 0 ] && [ -f "scripts/make_dmg.py" ]; then
        if python3 scripts/make_dmg.py "$DMG_STAGING" "Eclipse" "Eclipse-Installer.dmg" \
            && [ -f "Eclipse-Installer.dmg" ]; then
            dmg_ok=1
        fi
    fi
    rm -rf "$DMG_STAGING"
    if [ "$dmg_ok" -eq 1 ]; then
        xattr -cr "Eclipse-Installer.dmg"
        cp -f "Eclipse-Installer.dmg" "new.dmg"
        xattr -cr "new.dmg"
    else
        echo "disk image creation is unavailable in this environment; using a zip instead"
    fi
fi

echo "=== Installing directly to /Applications/Eclipse.app ==="
if [ -d "/Applications" ]; then
    rm -rf /Applications/Eclipse.app
    cp -R "$BUNDLE_DIR" /Applications/
    xattr -cr /Applications/Eclipse.app
    if [ -x "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister" ]; then
        /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f /Applications/Eclipse.app
    fi
fi

# A zip is always produced so the app can be moved to /Applications even when
# the surrounding sandbox does not allow disk-image creation.
rm -f "Eclipse.zip"
ditto -c -k --norsrc --noextattr --keepParent "$BUNDLE_DIR" "Eclipse.zip"

echo ""
echo "===================================================="
echo "✅ Eclipse.app built and ad-hoc signed"
echo "📍 $(pwd)/$BUNDLE_DIR"
[ -f "Eclipse-Installer.dmg" ] && echo "📦 $(pwd)/Eclipse-Installer.dmg"
[ -f "Eclipse.zip" ] && echo "📦 $(pwd)/Eclipse.zip"
echo "===================================================="
