#!/usr/bin/env bash
# Check macOS package naming and app metadata without building Rust or creating a DMG.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# shellcheck source=scripts/package-metadata.sh
source "$SCRIPT_DIR/../package-metadata.sh"
# shellcheck source=scripts/macos/release-lib.sh
source "$SCRIPT_DIR/release-lib.sh"

assert_equal() {
    local expected="$1"
    local actual="$2"
    local description="$3"
    if [[ "$actual" != "$expected" ]]; then
        echo "$description: expected '$expected', got '$actual'." >&2
        exit 1
    fi
}

assert_fails() {
    local description="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo "$description: expected failure." >&2
        exit 1
    fi
}

assert_equal "1.2.3" "$(macos_get_bundle_version "1.2.3")" "Numeric bundle version"
assert_equal "1.2.3" "$(macos_get_bundle_version "1.2.3-beta.1+build.5")" \
    "Prerelease bundle version"
assert_fails "Invalid semantic bundle version" macos_get_bundle_version "1.2"
assert_equal "Codex-Auth-Switcher-1.2.3-macos-arm64.dmg" \
    "$(macos_get_release_asset_name "1.2.3" "arm64")" "Apple Silicon package name"
assert_equal "Codex-Auth-Switcher-1.2.3-macos-x86_64.dmg" \
    "$(macos_get_release_asset_name "1.2.3" "x86_64")" "Intel package name"
assert_equal "aarch64-apple-darwin" "$(macos_get_rust_target "arm64")" "Apple Silicon target"
assert_equal "x86_64-apple-darwin" "$(macos_get_rust_target "x86_64")" "Intel target"
assert_fails "Unsupported architecture" macos_get_release_asset_name "1.2.3" "universal"

(
    unset GITHUB_REF_TYPE GITHUB_REF_NAME GITHUB_REF
    package_assert_tag_matches_version "1.2.3"
)
(
    export GITHUB_REF_TYPE="tag"
    export GITHUB_REF_NAME="v1.2.3"
    unset GITHUB_REF
    package_assert_tag_matches_version "1.2.3"
)
# shellcheck disable=SC2016
assert_fails "Mismatched release tag" bash -c '
    source "$1"
    export GITHUB_REF_TYPE=tag GITHUB_REF_NAME=v1.2.4
    package_assert_tag_matches_version 1.2.3
' _ "$SCRIPT_DIR/../package-metadata.sh"

BUILD_SCRIPT="$SCRIPT_DIR/../build-dmg.sh"
bash -n "$BUILD_SCRIPT" "$SCRIPT_DIR/../package-macos.sh"
grep -Fxq 'APP_ID="com.bruceblink.codex-auth-switcher"' "$BUILD_SCRIPT"
grep -Fxq "    <string>\${APP_ID}</string>" "$BUILD_SCRIPT"
grep -Fxq '    <string>codex-auth-switcher</string>' "$BUILD_SCRIPT"
grep -Fxq '    <string>12.0</string>' "$BUILD_SCRIPT"
grep -Fq "codesign --force --sign - \"\$APP\"" "$BUILD_SCRIPT"
grep -Fq 'hdiutil_create_with_retry()' "$BUILD_SCRIPT"
grep -Fq 'for attempt in 1 2 3 4 5' "$BUILD_SCRIPT"
grep -Fq 'sync' "$BUILD_SCRIPT"
grep -Fq -- '-ov' "$BUILD_SCRIPT"
grep -Fq 'THIRD-PARTY-NOTICES.html' "$BUILD_SCRIPT"
grep -Fq 'SOURCES.txt' "$BUILD_SCRIPT"
grep -Fq 'rustup run stable cargo build' "$BUILD_SCRIPT"
[[ -s "$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.png" ]]
[[ -s "$REPO_DIR/LICENSE" && -s "$REPO_DIR/THIRD-PARTY-NOTICES.html" ]]
grep -Fxq 'license = "GPL-3.0-only"' "$REPO_DIR/Cargo.toml"

echo "macOS package logic tests passed."
