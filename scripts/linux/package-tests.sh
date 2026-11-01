#!/usr/bin/env bash
# Check Linux package naming and package metadata without building Rust or downloading tools.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# shellcheck source=scripts/package-metadata.sh
source "$SCRIPT_DIR/../package-metadata.sh"
# shellcheck source=scripts/linux/release-lib.sh
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

assert_equal "Codex-Auth-Switcher-1.2.3-linux-amd64.deb" \
    "$(linux_get_deb_asset_name "1.2.3")" "Debian package name"
assert_equal "Codex-Auth-Switcher-1.2.3-linux-x86_64.AppImage" \
    "$(linux_get_appimage_asset_name "1.2.3")" "AppImage package name"
assert_equal "Codex-Auth-Switcher-1.2.3-rc.1+build.5-linux-amd64.deb" \
    "$(linux_get_deb_asset_name "1.2.3-rc.1+build.5")" "Prerelease package name"
assert_fails "Invalid Debian package version" linux_get_deb_asset_name "1.2"
assert_fails "Invalid AppImage package version" linux_get_appimage_asset_name "next"
package_is_supported_semver "1.2.3"
assert_fails "Invalid semantic version" package_is_supported_semver "1.2"

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

DESKTOP_FILE="$SCRIPT_DIR/com.bruceblink.codex-auth-switcher.desktop"
grep -Fxq "Name=Codex Auth Switcher" "$DESKTOP_FILE"
grep -Fxq "Exec=codex-auth-switcher" "$DESKTOP_FILE"
grep -Fxq "Icon=com.bruceblink.codex-auth-switcher" "$DESKTOP_FILE"
grep -Fxq "Package: codex-auth-switcher" "$SCRIPT_DIR/debian-control.in"
grep -Fxq "Depends: ca-certificates, @RUNTIME_DEPENDENCIES@" "$SCRIPT_DIR/debian-control.in"
grep -Fxq 'license = "GPL-3.0-only"' "$REPO_DIR/Cargo.toml"
[[ -s "$REPO_DIR/LICENSE" && -s "$REPO_DIR/THIRD-PARTY-NOTICES.html" ]]
[[ -s "$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.png" ]]
[[ -s "$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.svg" ]]
grep -Fq "c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d" \
    "$SCRIPT_DIR/../package-linux.sh"
grep -Fq "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d" \
    "$SCRIPT_DIR/../package-linux.sh"
grep -Fq 'rustup run stable cargo build' "$SCRIPT_DIR/../package-linux.sh"

if command -v dpkg-shlibdeps >/dev/null 2>&1 && [[ -x /bin/sleep ]]; then
    SMOKE_DIR="$(mktemp -d)"
    cleanup_smoke_dir() {
        if [[ -n "$SMOKE_DIR" && -d "$SMOKE_DIR" && ! -L "$SMOKE_DIR" ]]; then
            rm -rf -- "$SMOKE_DIR"
        fi
    }
    trap cleanup_smoke_dir EXIT
    mkdir -p "$SMOKE_DIR/debian/package-dependency-smoke/DEBIAN"
    mkdir -p "$SMOKE_DIR/debian/package-dependency-smoke/usr/bin"
    cat >"$SMOKE_DIR/debian/control" <<'EOF'
Source: package-dependency-smoke
Section: utils
Priority: optional
Maintainer: Package Test <package-test@example.invalid>
Standards-Version: 4.7.0

Package: package-dependency-smoke
Architecture: any
Depends: ${shlibs:Depends}
Description: Verify Debian shared library dependency resolution
 Test package control data for dpkg-shlibdeps.
EOF
    install -m 0755 /bin/sleep \
        "$SMOKE_DIR/debian/package-dependency-smoke/usr/bin/sleep"
    SHLIBS_SMOKE_OUTPUT="$(cd "$SMOKE_DIR" && \
        dpkg-shlibdeps -O debian/package-dependency-smoke/usr/bin/sleep)"
    case "$SHLIBS_SMOKE_OUTPUT" in
        shlibs:Depends=*) ;;
        *) echo "dpkg-shlibdeps returned an unexpected smoke-test result." >&2; exit 1 ;;
    esac
    [[ "$SHLIBS_SMOKE_OUTPUT" == *libc6* ]] ||
        { echo "dpkg-shlibdeps did not resolve libc6 for /bin/sleep." >&2; exit 1; }
fi

echo "Linux package logic tests passed."
