#!/usr/bin/env bash
# Build one architecture-specific Codex Auth Switcher .app and DMG.
# Packaging flow adapted from ramag-platform at 59399356bc88f215c8eacca632dcb3e13953632a.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
TARGET_DIR="$REPO_DIR/target"

# shellcheck source=scripts/package-metadata.sh
source "$SCRIPT_DIR/package-metadata.sh"
# shellcheck source=scripts/macos/release-lib.sh
source "$SCRIPT_DIR/macos/release-lib.sh"

usage() {
    sed -n '1,5p' "$0"
    printf '\nUsage: %s --architecture arm64 --output-dir PATH\n' "$0"
}

fail() {
    echo "$1" >&2
    exit 1
}

ARCHITECTURE=""
OUTPUT_DIR=""
while (($# > 0)); do
    case "$1" in
        --architecture)
            (($# >= 2)) || fail "--architecture requires a value."
            ARCHITECTURE="$2"
            shift 2
            ;;
        --architecture=*)
            ARCHITECTURE="${1#--architecture=}"
            shift
            ;;
        --output-dir)
            (($# >= 2)) || fail "--output-dir requires a path."
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --output-dir=*)
            OUTPUT_DIR="${1#--output-dir=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "Unknown argument: $1"
            ;;
    esac
done

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS packages must be built on macOS."
case "$ARCHITECTURE" in
    arm64) ;;
    *) fail "Pass --architecture arm64. Only Apple Silicon is supported." ;;
esac
[[ -n "$OUTPUT_DIR" ]] || fail "Pass --output-dir within the repository target directory."

for command_name in cargo iconutil lipo sips codesign hdiutil vtool jq rustup; do
    command -v "$command_name" >/dev/null 2>&1 || fail "Required command is missing: $command_name"
done
package_assert_stable_toolchain "$REPO_DIR"

mkdir -p "$TARGET_DIR"
TARGET_DIR="$(cd "$TARGET_DIR" && pwd -P)"
[[ "$TARGET_DIR" == "$REPO_DIR/target" ]] || fail "The repository target directory resolved unexpectedly."
mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd -P)"
case "$OUTPUT_DIR/" in
    "$TARGET_DIR"/*) ;;
    *) fail "Output directory must be inside $TARGET_DIR." ;;
esac

VERSION="$(package_get_app_version "$REPO_DIR")"
BUNDLE_VERSION="$(macos_get_bundle_version "$VERSION")"
package_assert_tag_matches_version "$VERSION"
RUST_TARGET="$(macos_get_rust_target "$ARCHITECTURE")"
APP_NAME="Codex Auth Switcher"
APP_ID="com.bruceblink.codex-auth-switcher"
APP="$OUTPUT_DIR/$APP_NAME.app"
DMG_NAME="$(macos_get_release_asset_name "$VERSION" "$ARCHITECTURE")"
DMG="$OUTPUT_DIR/$DMG_NAME"
ICON_SOURCE="$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.png"
[[ -s "$ICON_SOURCE" ]] || fail "Application PNG icon is missing: $ICON_SOURCE"
[[ -s "$REPO_DIR/LICENSE" ]] || fail "GPLv3 license file is missing."
[[ -s "$REPO_DIR/THIRD-PARTY-NOTICES.html" ]] || fail "Third-party notices are missing."

# GitHub macOS runners can briefly report Resource busy while Spotlight releases the staging folder.
hdiutil_create_with_retry() {
    local attempt

    for attempt in 1 2 3 4 5; do
        if [[ -L "$DMG" ]]; then
            fail "Refusing to replace a DMG symlink: $DMG"
        fi
        if [[ -e "$DMG" ]]; then
            rm -f -- "$DMG"
        fi
        sync
        if hdiutil create \
            -volname "$APP_NAME" \
            -srcfolder "$STAGING_DIR" \
            -fs HFS+ \
            -format ULMO \
            -ov \
            "$DMG" >/dev/null; then
            return 0
        fi
        if [[ "$attempt" == 5 ]]; then
            echo "hdiutil create failed after $attempt attempts." >&2
            return 1
        fi
        echo "hdiutil create failed (attempt $attempt of 5); retrying in $((attempt * 5))s." >&2
        sleep "$((attempt * 5))"
    done
}

if ! rustup target list --toolchain stable --installed | grep -Fxq "$RUST_TARGET"; then
    fail "Rust target $RUST_TARGET is not installed. Install it with rustup target add $RUST_TARGET."
fi
export MACOSX_DEPLOYMENT_TARGET="12.0"

ICON_WORK_DIR=""
STAGING_DIR=""
cleanup_build_temp() {
    if [[ -d "$ICON_WORK_DIR" && ! -L "$ICON_WORK_DIR" ]]; then
        case "$ICON_WORK_DIR" in
            "$OUTPUT_DIR"/iconset-"$ARCHITECTURE".*) rm -rf -- "$ICON_WORK_DIR" ;;
            *) echo "Refusing to remove unexpected icon work directory: $ICON_WORK_DIR" >&2 ;;
        esac
    fi
    if [[ -d "$STAGING_DIR" && ! -L "$STAGING_DIR" ]]; then
        case "$STAGING_DIR" in
            "$OUTPUT_DIR"/dmg-staging-"$ARCHITECTURE".*) rm -rf -- "$STAGING_DIR" ;;
            *) echo "Refusing to remove unexpected DMG staging directory: $STAGING_DIR" >&2 ;;
        esac
    fi
}
trap cleanup_build_temp EXIT

ICON_WORK_DIR="$(mktemp -d "$OUTPUT_DIR/iconset-$ARCHITECTURE.XXXXXX")"
STAGING_DIR="$(mktemp -d "$OUTPUT_DIR/dmg-staging-$ARCHITECTURE.XXXXXX")"
ICONSET="$ICON_WORK_DIR/codex-auth-switcher.iconset"
ICNS="$ICON_WORK_DIR/codex-auth-switcher.icns"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    doubled=$((size * 2))
    sips -s format png -z "$size" "$size" "$ICON_SOURCE" \
        --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -s format png -z "$doubled" "$doubled" "$ICON_SOURCE" \
        --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$ICNS"

cd "$REPO_DIR"
rustup run stable cargo build --locked --release -p switcher-desktop --target "$RUST_TARGET"
BINARY="$TARGET_DIR/$RUST_TARGET/release/codex-auth-switcher"
[[ -x "$BINARY" ]] || fail "Release executable was not created: $BINARY"
macos_assert_architecture "$BINARY" "$ARCHITECTURE"
macos_assert_minimum_version "$BINARY"

[[ ! -e "$APP" && ! -L "$APP" ]] || fail "Refusing to replace an existing application bundle: $APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 0755 "$BINARY" "$APP/Contents/MacOS/codex-auth-switcher"
install -m 0644 "$ICNS" "$APP/Contents/Resources/codex-auth-switcher.icns"
install -m 0644 "$REPO_DIR/LICENSE" "$APP/Contents/Resources/LICENSE"
install -m 0644 "$REPO_DIR/THIRD-PARTY-NOTICES.html" \
    "$APP/Contents/Resources/THIRD-PARTY-NOTICES.html"
install -m 0644 /dev/null "$APP/Contents/Resources/SOURCES.txt"
package_write_sources "$REPO_DIR" "$APP/Contents/Resources/SOURCES.txt" "$VERSION"

cat >"$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleIdentifier</key>
    <string>${APP_ID}</string>
    <key>CFBundleVersion</key>
    <string>${BUNDLE_VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${BUNDLE_VERSION}</string>
    <key>CodexAuthSwitcherCargoVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>codex-auth-switcher</string>
    <key>CFBundleIconFile</key>
    <string>codex-auth-switcher</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
</dict>
</plist>
EOF

plutil -lint "$APP/Contents/Info.plist" >/dev/null
codesign --force --sign - "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

[[ ! -e "$DMG" && ! -L "$DMG" ]] || fail "Refusing to replace an existing DMG: $DMG"
cp -R "$APP" "$STAGING_DIR/$APP_NAME.app"
ln -s /Applications "$STAGING_DIR/Applications"
hdiutil_create_with_retry
hdiutil verify "$DMG" >/dev/null

printf 'macOS app and DMG created:\n%s\n%s\nArchitecture: %s\nVersion: %s\nSignature: ad hoc; not notarized\n' \
    "$APP" "$DMG" "$ARCHITECTURE" "$VERSION"
