#!/usr/bin/env bash
# Linux package names and checks shared by the Linux packaging entry points.

linux_get_deb_asset_name() {
    local version="$1"
    package_is_supported_semver "$version" || return 1
    printf 'Codex-Auth-Switcher-%s-linux-amd64.deb\n' "$version"
}

linux_get_appimage_asset_name() {
    local version="$1"
    package_is_supported_semver "$version" || return 1
    printf 'Codex-Auth-Switcher-%s-linux-x86_64.AppImage\n' "$version"
}

linux_assert_x86_64_binary() {
    local binary="$1"
    readelf -h "$binary" | grep -Fq 'Advanced Micro Devices X86-64' || {
        echo "Release binary is not Linux x86_64: $binary" >&2
        return 1
    }
}
