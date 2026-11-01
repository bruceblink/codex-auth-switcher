#!/usr/bin/env bash
# macOS app bundle names and validation shared by the macOS packaging scripts.

macos_get_bundle_version() {
    local version="$1"
    local bundle_version

    package_is_supported_semver "$version" || {
        echo "Unsupported Cargo version for macOS packaging: $version" >&2
        return 1
    }
    bundle_version="${version%%[-+]*}"
    [[ "$bundle_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
        echo "Invalid macOS bundle version: $bundle_version" >&2
        return 1
    }
    printf '%s\n' "$bundle_version"
}

macos_get_release_asset_name() {
    local version="$1"
    local architecture="$2"

    package_is_supported_semver "$version" || return 1
    case "$architecture" in
        arm64) ;;
        *)
            echo "Unsupported macOS release architecture: $architecture" >&2
            return 1
            ;;
    esac
    printf 'Codex-Auth-Switcher-%s-macos-%s.dmg\n' "$version" "$architecture"
}

macos_get_rust_target() {
    case "$1" in
        arm64) printf 'aarch64-apple-darwin\n' ;;
        *)
            echo "Unsupported macOS architecture: $1" >&2
            return 1
            ;;
    esac
}

macos_assert_architecture() {
    local binary="$1"
    local expected_architecture="$2"
    local actual_architectures

    actual_architectures="$(lipo -archs "$binary")"
    [[ "$actual_architectures" == "$expected_architecture" ]] || {
        echo "Unexpected macOS architecture: expected $expected_architecture, got $actual_architectures." >&2
        return 1
    }
}

macos_assert_minimum_version() {
    local binary="$1"
    local minimum_versions

    minimum_versions="$(
        vtool -show-build "$binary" |
            awk '$1 == "minos" { print $2 }' |
            sort -u
    )"
    [[ "$minimum_versions" == "12.0" ]] || {
        echo "Unexpected macOS deployment target: ${minimum_versions:-missing}" >&2
        return 1
    }
}
