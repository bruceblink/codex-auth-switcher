#!/usr/bin/env bash
# Build, validate, and checksum one architecture-specific macOS DMG.
# Packaging flow adapted from ramag-platform at 59399356bc88f215c8eacca632dcb3e13953632a.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
TARGET_DIR="$REPO_DIR/target"
BUILD_SCRIPT="$SCRIPT_DIR/build-dmg.sh"
APP_NAME="Codex Auth Switcher"
APP_ID="com.bruceblink.codex-auth-switcher"
CURRENT_MOUNT_DIR=""
CURRENT_MOUNTED=false

# shellcheck source=scripts/package-metadata.sh
source "$SCRIPT_DIR/package-metadata.sh"
# shellcheck source=scripts/macos/release-lib.sh
source "$SCRIPT_DIR/macos/release-lib.sh"

usage() {
    printf 'Usage: %s --architecture arm64\n' "$0"
}

fail() {
    echo "$1" >&2
    exit 1
}

ARCHITECTURE=""
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
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "Unknown argument: $1"
            ;;
    esac
done

if [[ -z "$ARCHITECTURE" ]]; then
    case "$(uname -m)" in
        arm64|aarch64) ARCHITECTURE="arm64" ;;
        *) fail "macOS packages require an Apple Silicon host." ;;
    esac
fi
case "$ARCHITECTURE" in
    arm64) ;;
    *) fail "Unsupported macOS architecture: $ARCHITECTURE. Only arm64 is supported." ;;
esac
[[ "$(uname -s)" == "Darwin" ]] || fail "macOS packages must be built on macOS."

for command_name in cargo codesign hdiutil jq lipo plutil rustup shasum vtool; do
    command -v "$command_name" >/dev/null 2>&1 || fail "Required command is missing: $command_name"
done
[[ -f "$BUILD_SCRIPT" ]] || fail "macOS build script is missing: $BUILD_SCRIPT"
package_assert_stable_toolchain "$REPO_DIR"

mkdir -p "$TARGET_DIR"
TARGET_DIR="$(cd "$TARGET_DIR" && pwd -P)"
[[ "$TARGET_DIR" == "$REPO_DIR/target" ]] || fail "The repository target directory resolved unexpectedly."

DIST_DIR="$TARGET_DIR/macos-dist"
[[ ! -L "$DIST_DIR" ]] || fail "Refusing to use a symlink as the macOS output directory."
mkdir -p "$DIST_DIR"
[[ "$(cd "$DIST_DIR" && pwd -P)" == "$TARGET_DIR/macos-dist" ]] ||
    fail "macOS output directory resolved outside target."

VERSION="$(package_get_app_version "$REPO_DIR")"
BUNDLE_VERSION="$(macos_get_bundle_version "$VERSION")"
package_assert_tag_matches_version "$VERSION"
ASSET_NAME="$(macos_get_release_asset_name "$VERSION" "$ARCHITECTURE")"
WORK_DIR="$(mktemp -d "$TARGET_DIR/macos-package.XXXXXX")"

cleanup_current_mount() {
    if [[ "$CURRENT_MOUNTED" == true && -n "$CURRENT_MOUNT_DIR" ]]; then
        if hdiutil detach "$CURRENT_MOUNT_DIR" -quiet; then
            CURRENT_MOUNTED=false
        else
            echo "Failed to detach temporary DMG mount: $CURRENT_MOUNT_DIR" >&2
            return 1
        fi
    fi
}

cleanup_work_directory() {
    local exit_status="$?"
    if ! cleanup_current_mount; then
        [[ "$exit_status" -ne 0 ]] || exit_status=1
        return "$exit_status"
    fi
    if [[ -d "$WORK_DIR" && ! -L "$WORK_DIR" ]]; then
        case "$WORK_DIR" in
            "$TARGET_DIR"/macos-package.*) rm -rf -- "$WORK_DIR" ;;
            *) echo "Refusing to remove unexpected build directory: $WORK_DIR" >&2 ;;
        esac
    fi
    return "$exit_status"
}
trap cleanup_work_directory EXIT

APP_OUTPUT_DIR="$WORK_DIR"
bash "$BUILD_SCRIPT" --architecture "$ARCHITECTURE" --output-dir "$APP_OUTPUT_DIR"

APP="$APP_OUTPUT_DIR/$APP_NAME.app"
DMG="$APP_OUTPUT_DIR/$ASSET_NAME"
PLIST="$APP/Contents/Info.plist"
EXECUTABLE="$APP/Contents/MacOS/codex-auth-switcher"
[[ -s "$PLIST" && -x "$EXECUTABLE" ]] || fail "The generated .app bundle is incomplete."
[[ -s "$APP/Contents/Resources/codex-auth-switcher.icns" ]] || fail "The app icon is missing."
[[ -s "$APP/Contents/Resources/LICENSE" ]] || fail "The app GPLv3 license is missing."
[[ -s "$APP/Contents/Resources/THIRD-PARTY-NOTICES.html" ]] ||
    fail "The app third-party notices are missing."
[[ -s "$APP/Contents/Resources/SOURCES.txt" ]] || fail "The app source metadata is missing."
plutil -lint "$PLIST" >/dev/null
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$PLIST")" == "$APP_ID" ]] ||
    fail "Unexpected macOS bundle identifier."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$PLIST")" == "$APP_NAME" ]] ||
    fail "Unexpected macOS app name."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$PLIST")" == "$BUNDLE_VERSION" ]] ||
    fail "macOS bundle version does not match Cargo version $VERSION."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$PLIST")" == "12.0" ]] ||
    fail "Unexpected macOS minimum system version in app metadata."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CodexAuthSwitcherCargoVersion' "$PLIST")" == "$VERSION" ]] ||
    fail "macOS Cargo version metadata does not match $VERSION."
macos_assert_architecture "$EXECUTABLE" "$ARCHITECTURE"
macos_assert_minimum_version "$EXECUTABLE"
codesign --verify --deep --strict --verbose=2 "$APP"
[[ -s "$DMG" ]] || fail "The generated macOS DMG is missing."
hdiutil verify "$DMG" >/dev/null

CURRENT_MOUNT_DIR="$WORK_DIR/mount"
mkdir -p "$CURRENT_MOUNT_DIR"
hdiutil attach "$DMG" -readonly -nobrowse -mountpoint "$CURRENT_MOUNT_DIR" >/dev/null
CURRENT_MOUNTED=true
MOUNTED_APP="$CURRENT_MOUNT_DIR/$APP_NAME.app"
[[ -d "$MOUNTED_APP" ]] || fail "The DMG does not contain the application bundle."
[[ -L "$CURRENT_MOUNT_DIR/Applications" ]] || fail "The DMG Applications shortcut is missing."
[[ "$(readlink "$CURRENT_MOUNT_DIR/Applications")" == "/Applications" ]] ||
    fail "The DMG Applications shortcut has an unexpected target."
MOUNTED_PLIST="$MOUNTED_APP/Contents/Info.plist"
MOUNTED_EXECUTABLE="$MOUNTED_APP/Contents/MacOS/codex-auth-switcher"
[[ -s "$MOUNTED_PLIST" && -x "$MOUNTED_EXECUTABLE" ]] ||
    fail "The mounted DMG application bundle is incomplete."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$MOUNTED_PLIST")" == "$APP_ID" ]] ||
    fail "The mounted DMG app bundle identifier is incorrect."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$MOUNTED_PLIST")" == "$BUNDLE_VERSION" ]] ||
    fail "The mounted DMG app version is incorrect."
[[ "$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$MOUNTED_PLIST")" == "12.0" ]] ||
    fail "The mounted DMG app minimum system version is incorrect."
[[ -s "$MOUNTED_APP/Contents/Resources/codex-auth-switcher.icns" ]] ||
    fail "The mounted DMG app icon is missing."
[[ -s "$MOUNTED_APP/Contents/Resources/LICENSE" ]] || fail "The mounted DMG app license is missing."
[[ -s "$MOUNTED_APP/Contents/Resources/THIRD-PARTY-NOTICES.html" ]] ||
    fail "The mounted DMG third-party notices are missing."
[[ -s "$MOUNTED_APP/Contents/Resources/SOURCES.txt" ]] ||
    fail "The mounted DMG source metadata is missing."
macos_assert_architecture "$MOUNTED_EXECUTABLE" "$ARCHITECTURE"
macos_assert_minimum_version "$MOUNTED_EXECUTABLE"
codesign --verify --deep --strict --verbose=2 "$MOUNTED_APP"
hdiutil detach "$CURRENT_MOUNT_DIR" -quiet
CURRENT_MOUNTED=false
rmdir "$CURRENT_MOUNT_DIR"
CURRENT_MOUNT_DIR=""

OUTPUT_PATH="$DIST_DIR/$ASSET_NAME"
[[ ! -L "$OUTPUT_PATH" ]] || fail "Refusing to overwrite a DMG symlink: $OUTPUT_PATH"
[[ ! -L "$DIST_DIR/SHA256SUMS.txt" ]] ||
    fail "Refusing to overwrite a checksum symlink: $DIST_DIR/SHA256SUMS.txt"
install -m 0644 "$DMG" "$OUTPUT_PATH"
(
    cd "$DIST_DIR"
    shasum -a 256 "Codex-Auth-Switcher-$VERSION-macos-"*.dmg >"$WORK_DIR/SHA256SUMS.txt"
    shasum -a 256 -c "$WORK_DIR/SHA256SUMS.txt"
    install -m 0644 "$WORK_DIR/SHA256SUMS.txt" SHA256SUMS.txt
)

printf 'macOS package created and verified:\n%s\n%s\nArchitecture: %s\nSignature: ad hoc; not notarized\n' \
    "$OUTPUT_PATH" "$DIST_DIR/SHA256SUMS.txt" "$ARCHITECTURE"
