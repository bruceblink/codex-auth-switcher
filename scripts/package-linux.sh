#!/usr/bin/env bash
# Build and validate Linux x86_64 .deb and AppImage release packages.
# Packaging flow adapted from ramag-platform at 59399356bc88f215c8eacca632dcb3e13953632a.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
LINUX_DIR="$SCRIPT_DIR/linux"
APP_ID="com.bruceblink.codex-auth-switcher"

# Pin linuxdeploy and the type2 runtime. The upstream continuous runtime URL is mutable.
LINUXDEPLOY_URL="https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_SHA256="c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d"
APPIMAGE_RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64"
APPIMAGE_RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

# shellcheck source=scripts/package-metadata.sh
source "$SCRIPT_DIR/package-metadata.sh"
# shellcheck source=scripts/linux/release-lib.sh
source "$LINUX_DIR/release-lib.sh"

fail() {
    echo "$1" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "Required command is missing: $1"
}

ensure_target_directory() {
    local directory="$1"
    local expected_path="$TARGET_DIR/${1##*/}"
    [[ ! -L "$directory" ]] || fail "Refusing to use a symlink as a package directory: $directory"
    mkdir -p "$directory"
    local actual_path
    actual_path="$(cd "$directory" && pwd -P)"
    [[ "$actual_path" == "$expected_path" ]] ||
        fail "Package directory resolved outside target: $directory"
}

cleanup_build_directory() {
    if [[ -n "${BUILD_DIR:-}" && -d "$BUILD_DIR" && ! -L "$BUILD_DIR" ]]; then
        case "$BUILD_DIR" in
            "$TARGET_DIR"/linux-package.*) rm -rf -- "$BUILD_DIR" ;;
            *) echo "Refusing to remove unexpected build directory: $BUILD_DIR" >&2 ;;
        esac
    fi
}

download_verified_tool() {
    local url="$1"
    local expected_sha256="$2"
    local destination="$3"
    local partial

    [[ ! -L "$destination" ]] || fail "Refusing to use a symlink as a packaging tool: $destination"
    if [[ -f "$destination" ]] \
        && printf '%s  %s\n' "$expected_sha256" "$destination" | sha256sum --check --status; then
        chmod 0755 "$destination"
        return
    fi

    partial="$(mktemp "$TOOLS_DIR/$(basename "$destination").XXXXXX")"
    if ! curl --fail --location --retry 3 --output "$partial" "$url"; then
        rm -f -- "$partial"
        fail "Failed to download pinned packaging tool: $url"
    fi
    if ! printf '%s  %s\n' "$expected_sha256" "$partial" | sha256sum --check --status; then
        rm -f -- "$partial"
        fail "Downloaded tool checksum mismatch: $url"
    fi
    chmod 0755 "$partial"
    mv -f -- "$partial" "$destination"
}

verify_binary_dependencies() {
    local binary="$1"
    local unresolved
    unresolved="$(ldd "$binary" | grep -F 'not found' || true)"
    [[ -z "$unresolved" ]] || {
        printf 'The release executable has unresolved shared libraries:\n%s\n' "$unresolved" >&2
        return 1
    }
}

verify_deb_package() {
    local package="$1"
    local extraction_dir="$2"
    local contents="$3"

    dpkg-deb --info "$package" >/dev/null
    dpkg-deb --contents "$package" >"$contents"
    grep -Fq './usr/bin/codex-auth-switcher' "$contents"
    dpkg-deb --extract "$package" "$extraction_dir"
    [[ -x "$extraction_dir/usr/bin/codex-auth-switcher" ]]
    [[ -f "$extraction_dir/usr/share/applications/$APP_ID.desktop" ]]
    [[ -f "$extraction_dir/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg" ]]
    [[ -s "$extraction_dir/usr/share/doc/codex-auth-switcher/LICENSE" ]]
    [[ -s "$extraction_dir/usr/share/doc/codex-auth-switcher/THIRD-PARTY-NOTICES.html" ]]
    [[ -s "$extraction_dir/usr/share/doc/codex-auth-switcher/SOURCES.txt" ]]
    desktop-file-validate "$extraction_dir/usr/share/applications/$APP_ID.desktop"
    [[ "$(dpkg-deb -f "$package" Architecture)" == "amd64" ]]
    [[ "$(dpkg-deb -f "$package" Version)" == "$VERSION" ]]
    [[ "$(dpkg-deb -f "$package" Depends)" == "ca-certificates, $RUNTIME_DEPENDENCIES" ]]
}

verify_appimage() {
    local appimage="$1"
    local extraction_dir="$2"
    local extracted="$extraction_dir/squashfs-root"

    file "$appimage" | grep -Fq 'ELF 64-bit'
    mkdir -p "$extraction_dir"
    (
        cd "$extraction_dir"
        "$appimage" --appimage-extract >/dev/null
    )
    [[ -x "$extracted/AppRun" ]]
    [[ -x "$extracted/usr/bin/codex-auth-switcher" ]]
    [[ -f "$extracted/$APP_ID.desktop" ]]
    [[ -f "$extracted/$APP_ID.svg" ]]
    [[ -s "$extracted/usr/share/doc/codex-auth-switcher/LICENSE" ]]
    [[ -s "$extracted/usr/share/doc/codex-auth-switcher/THIRD-PARTY-NOTICES.html" ]]
    [[ -s "$extracted/usr/share/doc/codex-auth-switcher/SOURCES.txt" ]]
    desktop-file-validate "$extracted/$APP_ID.desktop"
}

main() {
    [[ "$(uname -s)" == "Linux" ]] || fail "Linux packages must be built on Linux."
    [[ "$(uname -m)" == "x86_64" ]] || fail "Only Linux x86_64 packaging is supported."

    for command_name in cargo curl desktop-file-validate dpkg-deb dpkg-shlibdeps file \
        install jq ldd mksquashfs readelf rustup sha256sum xz; do
        require_command "$command_name"
    done
    package_assert_stable_toolchain "$REPO_DIR"

    TARGET_DIR="$REPO_DIR/target"
    mkdir -p "$TARGET_DIR"
    TARGET_DIR="$(cd "$TARGET_DIR" && pwd -P)"
    [[ "$TARGET_DIR" == "$REPO_DIR/target" ]] || fail "The repository target directory resolved unexpectedly."

    DIST_DIR="$TARGET_DIR/linux-dist"
    TOOLS_DIR="$TARGET_DIR/linux-tools"
    ensure_target_directory "$DIST_DIR"
    ensure_target_directory "$TOOLS_DIR"

    BUILD_DIR="$(mktemp -d "$TARGET_DIR/linux-package.XXXXXX")"
    trap cleanup_build_directory EXIT

    VERSION="$(package_get_app_version "$REPO_DIR")"
    package_assert_tag_matches_version "$VERSION"
    DEB_NAME="$(linux_get_deb_asset_name "$VERSION")"
    APPIMAGE_NAME="$(linux_get_appimage_asset_name "$VERSION")"
    DESKTOP_FILE="$LINUX_DIR/$APP_ID.desktop"
    SVG_ICON="$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.svg"
    PNG_ICON="$REPO_DIR/crates/switcher-desktop/assets/codex-auth-switcher.png"
    [[ -s "$REPO_DIR/LICENSE" ]] || fail "The GPLv3 license file is missing or empty."
    [[ -s "$REPO_DIR/THIRD-PARTY-NOTICES.html" ]] || fail "Third-party notices are missing or empty."
    [[ -s "$DESKTOP_FILE" && -s "$SVG_ICON" && -s "$PNG_ICON" ]] ||
        fail "A required desktop entry or application icon is missing."
    desktop-file-validate "$DESKTOP_FILE"
    grep -Fxq "Exec=codex-auth-switcher" "$DESKTOP_FILE" ||
        fail "Desktop entry does not launch the packaged executable."
    grep -Fxq "Icon=$APP_ID" "$DESKTOP_FILE" ||
        fail "Desktop entry does not reference the packaged icon."

    cd "$REPO_DIR"
    rustup run stable cargo build --locked --release -p switcher-desktop
    BINARY="$TARGET_DIR/release/codex-auth-switcher"
    [[ -x "$BINARY" ]] || fail "Release executable was not created: $BINARY"
    linux_assert_x86_64_binary "$BINARY"
    verify_binary_dependencies "$BINARY"
    DEB_ROOT="$BUILD_DIR/debian/codex-auth-switcher"
    APP_DOCS="$DEB_ROOT/usr/share/doc/codex-auth-switcher"
    install -Dm755 "$BINARY" "$DEB_ROOT/usr/bin/codex-auth-switcher"
    install -Dm644 "$DESKTOP_FILE" "$DEB_ROOT/usr/share/applications/$APP_ID.desktop"
    install -Dm644 "$SVG_ICON" "$DEB_ROOT/usr/share/icons/hicolor/scalable/apps/$APP_ID.svg"
    install -Dm644 "$REPO_DIR/LICENSE" "$APP_DOCS/LICENSE"
    install -Dm644 "$REPO_DIR/THIRD-PARTY-NOTICES.html" "$APP_DOCS/THIRD-PARTY-NOTICES.html"
    install -Dm644 /dev/null "$APP_DOCS/SOURCES.txt"
    package_write_sources "$REPO_DIR" "$APP_DOCS/SOURCES.txt" "$VERSION"

    mkdir -p "$DEB_ROOT/DEBIAN"
    mkdir -p "$BUILD_DIR/debian"
    cat >"$BUILD_DIR/debian/control" <<'EOF'
Source: codex-auth-switcher
Section: utils
Priority: optional
Maintainer: Codex Auth Switcher Maintainers <opensource@bruceblink.com>
Standards-Version: 4.7.0

Package: codex-auth-switcher
Architecture: any
Depends: ${shlibs:Depends}
Description: Manage Codex authentication profiles and model catalog
 Desktop application to switch Codex authentication profiles and manage
 the model catalog.
EOF
    SHLIBS_OUTPUT="$(cd "$BUILD_DIR" && \
        dpkg-shlibdeps -O debian/codex-auth-switcher/usr/bin/codex-auth-switcher)"
    case "$SHLIBS_OUTPUT" in
        shlibs:Depends=*) RUNTIME_DEPENDENCIES="${SHLIBS_OUTPUT#shlibs:Depends=}" ;;
        *) fail "dpkg-shlibdeps returned an unexpected dependency record: $SHLIBS_OUTPUT" ;;
    esac
    [[ -n "$RUNTIME_DEPENDENCIES" ]] || fail "No Debian runtime dependencies were resolved."

    installed_size="$(du -sk "$DEB_ROOT/usr" | awk '{print $1}')"
    sed \
        -e "s/@VERSION@/$VERSION/g" \
        -e "s/@INSTALLED_SIZE@/$installed_size/g" \
        -e "s/@RUNTIME_DEPENDENCIES@/$RUNTIME_DEPENDENCIES/g" \
        "$LINUX_DIR/debian-control.in" >"$DEB_ROOT/DEBIAN/control"
    DEB_TEMP="$BUILD_DIR/$DEB_NAME"
    dpkg-deb --root-owner-group -Zxz -z9 --build "$DEB_ROOT" "$DEB_TEMP" >/dev/null

    verify_deb_package "$DEB_TEMP" "$BUILD_DIR/deb-extracted" "$BUILD_DIR/deb-contents.txt"

    LINUXDEPLOY="$TOOLS_DIR/linuxdeploy-x86_64.AppImage"
    APPIMAGE_RUNTIME="$TOOLS_DIR/runtime-x86_64"
    download_verified_tool "$LINUXDEPLOY_URL" "$LINUXDEPLOY_SHA256" "$LINUXDEPLOY"
    download_verified_tool "$APPIMAGE_RUNTIME_URL" "$APPIMAGE_RUNTIME_SHA256" "$APPIMAGE_RUNTIME"

    APP_DIR="$BUILD_DIR/Codex-Auth-Switcher.AppDir"
    APPIMAGE_ICON="$BUILD_DIR/$APP_ID.svg"
    install -Dm644 "$SVG_ICON" "$APPIMAGE_ICON"
    APPIMAGE_EXTRACT_AND_RUN=1 "$LINUXDEPLOY" \
        --appdir "$APP_DIR" \
        --executable "$BINARY" \
        --desktop-file "$DESKTOP_FILE" \
        --icon-file "$APPIMAGE_ICON"
    install -Dm644 "$REPO_DIR/LICENSE" "$APP_DIR/usr/share/doc/codex-auth-switcher/LICENSE"
    install -Dm644 "$REPO_DIR/THIRD-PARTY-NOTICES.html" \
        "$APP_DIR/usr/share/doc/codex-auth-switcher/THIRD-PARTY-NOTICES.html"
    install -Dm644 "$APP_DOCS/SOURCES.txt" "$APP_DIR/usr/share/doc/codex-auth-switcher/SOURCES.txt"

    APPIMAGE_SQUASHFS="$BUILD_DIR/Codex-Auth-Switcher.squashfs"
    mksquashfs "$APP_DIR" "$APPIMAGE_SQUASHFS" -noappend -all-root -comp zstd -b 131072 >/dev/null
    APPIMAGE_TEMP="$BUILD_DIR/$APPIMAGE_NAME"
    cp "$APPIMAGE_RUNTIME" "$APPIMAGE_TEMP"
    cat "$APPIMAGE_SQUASHFS" >>"$APPIMAGE_TEMP"
    chmod 0755 "$APPIMAGE_TEMP"
    verify_appimage "$APPIMAGE_TEMP" "$BUILD_DIR/appimage-extracted"

    for output in "$DIST_DIR/$DEB_NAME" "$DIST_DIR/$APPIMAGE_NAME"; do
        [[ ! -L "$output" ]] || fail "Refusing to overwrite a package symlink: $output"
    done
    [[ ! -L "$DIST_DIR/SHA256SUMS.txt" ]] ||
        fail "Refusing to overwrite a checksum symlink: $DIST_DIR/SHA256SUMS.txt"
    install -m0644 "$DEB_TEMP" "$DIST_DIR/$DEB_NAME"
    install -m0755 "$APPIMAGE_TEMP" "$DIST_DIR/$APPIMAGE_NAME"
    (
        cd "$DIST_DIR"
        sha256sum "$DEB_NAME" "$APPIMAGE_NAME" >"$BUILD_DIR/SHA256SUMS.txt"
        sha256sum --check "$BUILD_DIR/SHA256SUMS.txt"
        install -m0644 "$BUILD_DIR/SHA256SUMS.txt" SHA256SUMS.txt
    )

    printf 'Linux packages created and verified:\n%s\n%s\n%s\n' \
        "$DIST_DIR/$DEB_NAME" "$DIST_DIR/$APPIMAGE_NAME" "$DIST_DIR/SHA256SUMS.txt"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
