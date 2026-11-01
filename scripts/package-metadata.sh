#!/usr/bin/env bash
# Shared version and source metadata for Linux and macOS release packages.

package_is_supported_semver() {
    local version="$1"
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]
}

package_assert_stable_toolchain() {
    local repo_dir="$1"
    local active_info stable_info active_release stable_release active_host stable_host

    command -v rustc >/dev/null 2>&1 || {
        echo "Rust compiler is not available on PATH." >&2
        return 1
    }
    command -v rustup >/dev/null 2>&1 || {
        echo "rustup is required to select the stable Rust toolchain." >&2
        return 1
    }
    active_info="$(cd "$repo_dir" && rustc -Vv)" || return 1
    stable_info="$(cd "$repo_dir" && rustup run stable rustc -Vv)" || {
        echo "The stable Rust toolchain is not installed or cannot be accessed." >&2
        return 1
    }
    active_release="$(printf '%s\n' "$active_info" | awk -F': ' '$1 == "release" { print $2; exit }')"
    stable_release="$(printf '%s\n' "$stable_info" | awk -F': ' '$1 == "release" { print $2; exit }')"
    active_host="$(printf '%s\n' "$active_info" | awk -F': ' '$1 == "host" { print $2; exit }')"
    stable_host="$(printf '%s\n' "$stable_info" | awk -F': ' '$1 == "host" { print $2; exit }')"
    [[ -n "$active_release" && -n "$stable_release" && -n "$active_host" && -n "$stable_host" ]] || {
        echo "Could not read the active and stable rustc version/host fields." >&2
        return 1
    }
    [[ "$active_release" == "$stable_release" && "$active_host" == "$stable_host" ]] || {
        echo "Active Rust compiler ($active_release, $active_host) does not match stable ($stable_release, $stable_host)." >&2
        return 1
    }
    printf 'Stable Rust: %s\nHost: %s\n' "$stable_release" "$stable_host"
}

package_get_app_version() {
    local repo_dir="$1"
    local metadata package_count version

    metadata="$(cd "$repo_dir" && rustup run stable cargo metadata --locked --no-deps --format-version 1)" || {
        echo "Failed to read Cargo metadata." >&2
        return 1
    }
    package_count="$(printf '%s' "$metadata" | jq -r \
        '[.packages[] | select(.name == "switcher-desktop")] | length')" || return 1
    if [[ "$package_count" != "1" ]]; then
        echo "Cargo metadata must contain exactly one switcher-desktop package." >&2
        return 1
    fi
    version="$(printf '%s' "$metadata" | jq -er \
        '.packages[] | select(.name == "switcher-desktop") | .version')" || return 1
    if ! package_is_supported_semver "$version"; then
        echo "Unsupported Cargo version for release packaging: $version" >&2
        return 1
    fi
    printf '%s\n' "$version"
}

package_assert_tag_matches_version() {
    local version="$1"
    local tag=""
    local expected_tag="v$version"

    if [[ "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
        tag="${GITHUB_REF_NAME:-}"
        [[ -n "$tag" ]] || {
            echo "GITHUB_REF_TYPE is tag, but GITHUB_REF_NAME is empty." >&2
            return 1
        }
    elif [[ "${GITHUB_REF:-}" == refs/tags/* ]]; then
        tag="${GITHUB_REF#refs/tags/}"
        [[ -n "$tag" ]] || {
            echo "GITHUB_REF contains an empty tag name." >&2
            return 1
        }
    fi
    [[ -z "$tag" || "$tag" == "$expected_tag" ]] || {
        echo "Release tag $tag does not match Cargo version $expected_tag." >&2
        return 1
    }
}

package_sha256_file() {
    local file_path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file_path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file_path" | awk '{print $1}'
    else
        echo "Neither sha256sum nor shasum is available." >&2
        return 1
    fi
}

package_write_sources() {
    local repo_dir="$1"
    local destination="$2"
    local version="$3"
    local repository_url source_commit source_state lockfile_hash

    repository_url="$(sed -n 's/^repository = "\([^"]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -n 1)"
    [[ -n "$repository_url" ]] || {
        echo "The workspace repository URL could not be read from Cargo.toml." >&2
        return 1
    }

    if source_commit="$(git -C "$repo_dir" rev-parse --verify HEAD 2>/dev/null)" \
        && [[ -n "$source_commit" ]]; then
        if [[ -z "$(git -C "$repo_dir" status --porcelain=v1 --untracked-files=all)" ]]; then
            source_state="clean worktree and index"
        else
            source_state="modified worktree or index; commit is the source baseline, not an exact build-source identifier"
        fi
    else
        source_commit="unavailable"
        source_state="unavailable (no Git HEAD)"
    fi
    lockfile_hash="$(package_sha256_file "$repo_dir/Cargo.lock")"

    {
        printf 'Source repository: %s\n' "$repository_url"
        printf 'Source commit: %s\n' "$source_commit"
        printf 'Build source state: %s\n' "$source_state"
        printf 'Cargo package: switcher-desktop\n'
        printf 'Cargo package version: %s\n' "$version"
        printf 'Cargo.lock SHA-256: %s\n' "$lockfile_hash"
        printf 'License: GPL-3.0-only (see LICENSE)\n'
    } >"$destination"
}
