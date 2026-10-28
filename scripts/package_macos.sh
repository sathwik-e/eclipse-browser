#!/usr/bin/env bash
# Build Eclipse.app plus a versioned .dmg and .zip into dist/.
#
#   scripts/package_macos.sh [--universal] [--install]
#
#   --universal  build aarch64 + x86_64 and lipo them into one binary (needs both
#                rustup targets; otherwise falls back to a native build)
#   --install    also copy the app to /Applications/Eclipse.app. An existing
#                Eclipse.app is replaced only if it is this browser (same
#                CFBundleIdentifier); anything else (e.g. the Eclipse IDE) is
#                left alone and the script fails.
#
# ECLIPSE_SIGN_IDENTITY selects the codesign identity (default "-", ad-hoc).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
    sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

INSTALL=0
UNIVERSAL=0
for arg in "$@"; do
    case "$arg" in
        --install) INSTALL=1 ;;
        --universal) UNIVERSAL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $arg" >&2; usage >&2; exit 2 ;;
    esac
done

# Make a normal Finder-launched terminal find rustup/cargo without hardcoding
# one developer's home directory.
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo is required to build Eclipse. Install Rust from https://rustup.rs and re-run." >&2
    exit 1
fi

APP_NAME="Eclipse"
BUNDLE_ID="io.github.sathwik-e.eclipse"
MIN_MACOS="11.0"
DIST_DIR="dist"
BUNDLE_DIR="$DIST_DIR/$APP_NAME.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
SIGN_IDENTITY="${ECLIPSE_SIGN_IDENTITY:--}"

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
if [ -z "$VERSION" ]; then
    echo "error: could not read the package version from Cargo.toml" >&2
    exit 1
fi
BUILD_NUMBER="$(git rev-list --count HEAD 2>/dev/null || echo 1)"

# Keep the binary's minimum OS in sync with LSMinimumSystemVersion.
export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS"

target_installed() {
    [ -d "$(rustc --print sysroot)/lib/rustlib/$1/lib" ]
}

echo "=== Building Eclipse $VERSION ($BUILD_NUMBER) ==="
BINARY="target/release/eclipse"
if [ "$UNIVERSAL" -eq 1 ]; then
    if target_installed aarch64-apple-darwin && target_installed x86_64-apple-darwin; then
        cargo build --release --target aarch64-apple-darwin
        cargo build --release --target x86_64-apple-darwin
        mkdir -p target/universal-apple-darwin/release
        BINARY="target/universal-apple-darwin/release/eclipse"
        lipo -create \
            target/aarch64-apple-darwin/release/eclipse \
            target/x86_64-apple-darwin/release/eclipse \
            -output "$BINARY"
    else
        echo "note: --universal needs both rustup targets (rustup target add aarch64-apple-darwin x86_64-apple-darwin);"
        echo "      building for this Mac's architecture only"
        UNIVERSAL=0
    fi
fi
if [ "$UNIVERSAL" -eq 0 ]; then
    cargo build --release
fi

if [ "$UNIVERSAL" -eq 1 ]; then
    ARCH_LABEL="universal"
else
    ARCH_LABEL="$(lipo -archs "$BINARY")"
fi
ARTIFACT_BASE="$DIST_DIR/$APP_NAME-$VERSION-macos-$ARCH_LABEL"
DMG_PATH="$ARTIFACT_BASE.dmg"
ZIP_PATH="$ARTIFACT_BASE.zip"

echo "=== Creating macOS App Bundle ($BUNDLE_DIR) ==="
rm -rf "$BUNDLE_DIR" "$DMG_PATH" "$ZIP_PATH"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BINARY" "$MACOS_DIR/eclipse"
chmod +x "$MACOS_DIR/eclipse"

echo "=== Generating macOS AppIcon.icns ==="
python3 scripts/generate_icon.py "$RESOURCES_DIR/AppIcon.icns"

cat << EOF > "$CONTENTS_DIR/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>eclipse</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>Eclipse Browser</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$VERSION</string>
    <key>CFBundleVersion</key>
    <string>$BUILD_NUMBER</string>
    <key>LSMinimumSystemVersion</key>
    <string>$MIN_MACOS</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>NSHumanReadableCopyright</key>
    <string>Copyright © 2026 Sathwik Elaprolu</string>
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

echo "=== Signing $BUNDLE_DIR (identity: $SIGN_IDENTITY) ==="
xattr -cr "$BUNDLE_DIR"
sign_args=(--force --options runtime --entitlements scripts/entitlements.plist --sign "$SIGN_IDENTITY")
if [ "$SIGN_IDENTITY" != "-" ]; then
    sign_args+=(--timestamp)
fi
# The bundle holds a single executable and no nested code, so --deep is not needed.
codesign "${sign_args[@]}" "$BUNDLE_DIR"
codesign --verify --strict --verbose=1 "$BUNDLE_DIR"

echo "=== Creating $DMG_PATH ==="
DMG_STAGING="target/dmg_staging"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -R "$BUNDLE_DIR" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

dmg_ok=0
if command -v hdiutil >/dev/null 2>&1; then
    if hdiutil create -volname "$APP_NAME" -srcfolder "$DMG_STAGING" -ov -format UDZO -fs HFS+ \
        "$DMG_PATH" >/dev/null 2>&1 && [ -f "$DMG_PATH" ]; then
        dmg_ok=1
    fi
fi
if [ "$dmg_ok" -eq 0 ] && command -v diskutil >/dev/null 2>&1; then
    if diskutil image create from "$DMG_STAGING" --volumeName "$APP_NAME" \
        --format UDZO "$DMG_PATH" >/dev/null 2>&1 && [ -f "$DMG_PATH" ]; then
        dmg_ok=1
    fi
fi
# hdiutil/DiskArbitration can be unavailable in sandboxed environments.
if [ "$dmg_ok" -eq 0 ]; then
    rm -f "$DMG_PATH"
    if python3 scripts/make_dmg.py "$DMG_STAGING" "$APP_NAME" "$DMG_PATH" && [ -f "$DMG_PATH" ]; then
        dmg_ok=1
    fi
fi
rm -rf "$DMG_STAGING"
if [ "$dmg_ok" -eq 0 ]; then
    rm -f "$DMG_PATH"
    echo "disk image creation is unavailable in this environment; only the zip will be produced"
fi

# A zip is always produced so the app can be moved to /Applications even when
# the surrounding environment does not allow disk-image creation.
echo "=== Creating $ZIP_PATH ==="
ditto -c -k --norsrc --noextattr --keepParent "$BUNDLE_DIR" "$ZIP_PATH"

if [ "$INSTALL" -eq 1 ]; then
    INSTALL_PATH="/Applications/$APP_NAME.app"
    echo "=== Installing to $INSTALL_PATH ==="
    if [ -e "$INSTALL_PATH" ]; then
        existing_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INSTALL_PATH/Contents/Info.plist" 2>/dev/null || true)"
        if [ "$existing_id" != "$BUNDLE_ID" ]; then
            echo "error: $INSTALL_PATH already exists and is not this browser (CFBundleIdentifier: ${existing_id:-unknown})." >&2
            echo "       Refusing to replace it. Move it away yourself if you are sure, then re-run with --install." >&2
            exit 1
        fi
    fi
    # Copy next to the destination first so a failed copy never leaves the
    # user without a working app.
    STAGED_INSTALL="/Applications/.$APP_NAME.app.installing"
    rm -rf "$STAGED_INSTALL"
    ditto "$BUNDLE_DIR" "$STAGED_INSTALL"
    rm -rf "$INSTALL_PATH"
    mv "$STAGED_INSTALL" "$INSTALL_PATH"
    LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
    if [ -x "$LSREGISTER" ]; then
        "$LSREGISTER" -f "$INSTALL_PATH"
    fi
fi

echo ""
echo "===================================================="
echo "Eclipse $VERSION ($BUILD_NUMBER, $ARCH_LABEL) built and signed"
echo "  $ROOT_DIR/$BUNDLE_DIR"
[ -f "$DMG_PATH" ] && echo "  $ROOT_DIR/$DMG_PATH"
echo "  $ROOT_DIR/$ZIP_PATH"
[ "$INSTALL" -eq 1 ] && echo "  installed to /Applications/$APP_NAME.app"
echo "===================================================="
