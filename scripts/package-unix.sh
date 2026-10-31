#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest_path="$repository_root/Cargo.toml"
output_directory=${1:-"$repository_root/dist"}

if [[ ! -f "$manifest_path" ]]; then
    echo "The workspace manifest was not found." >&2
    exit 1
fi

host=$(rustc -vV | sed -n 's/^host: //p')
case "$host" in
    *-unknown-linux-*)
        platform=linux
        ;;
    *-apple-darwin)
        platform=macos
        ;;
    *)
        echo "Packaging requires a Linux or macOS host; found '$host'." >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    x86_64|amd64)
        architecture=x64
        ;;
    aarch64|arm64)
        architecture=arm64
        ;;
    *)
        echo "Unsupported host architecture: $(uname -m)." >&2
        exit 1
        ;;
esac

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest_path" | head -n 1)
if [[ -z "$version" ]]; then
    echo "The workspace package version could not be read." >&2
    exit 1
fi

repository=$(sed -n 's/^repository = "\(.*\)"/\1/p' "$manifest_path" | head -n 1)
if [[ -z "$repository" ]]; then
    echo "The workspace repository URL could not be read." >&2
    exit 1
fi
if source_commit=$(git -C "$repository_root" rev-parse --verify HEAD 2>/dev/null) && [[ -n "$source_commit" ]]; then
    source_changes=$(git -C "$repository_root" status --porcelain=v1 --untracked-files=all)
    if [[ -n "$source_changes" ]]; then
        source_label="Source baseline only: $repository/tree/$source_commit"
        source_state="modified worktree or index; the HEAD URL is a baseline, not the exact build source"
        lockfile_source="Cargo.lock from the build working tree (may differ from HEAD)"
    else
        source_label="Source code: $repository/tree/$source_commit"
        source_state="clean worktree and index; the build corresponds to this commit"
        lockfile_source="Cargo.lock at the source commit"
    fi
else
    source_commit=""
    source_label="Source code: $repository"
    source_state="unavailable (no Git HEAD)"
    lockfile_source="Cargo.lock from the selected repository revision"
fi
expected_tag="v${version}"

mkdir -p "$output_directory"
output_directory=$(CDPATH= cd -- "$output_directory" && pwd)
binary_path="$repository_root/target/release/codex-auth-switcher"
release_name="codex-auth-switcher-v${version}-${platform}-${architecture}"
release_binary_path="$output_directory/$release_name"
license_path="$output_directory/LICENSE"
notices_path="$output_directory/THIRD-PARTY-NOTICES.html"
sources_path="$output_directory/SOURCES.txt"

for source in "$repository_root/LICENSE" "$repository_root/THIRD-PARTY-NOTICES.html"; do
    if [[ ! -f "$source" ]]; then
        echo "A required source notice is missing: $source" >&2
        exit 1
    fi
done

for path in "$release_binary_path" "$release_binary_path.sha256" "$license_path" "$notices_path" "$sources_path"; do
    if [[ -e "$path" || -L "$path" ]]; then
        echo "The package output already exists: $path" >&2
        exit 1
    fi
done

cargo build --release --locked --manifest-path "$manifest_path"
if [[ ! -x "$binary_path" ]]; then
    echo "The release executable was not produced: $binary_path" >&2
    exit 1
fi

cp "$binary_path" "$release_binary_path"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

printf '%s  %s\n' "$(sha256_of "$release_binary_path")" "$(basename "$release_binary_path")" \
    > "$release_binary_path.sha256"
cp "$repository_root/LICENSE" "$license_path"
cp "$repository_root/THIRD-PARTY-NOTICES.html" "$notices_path"
{
    printf '%s\n' "$source_label"
    if [[ -n "$source_commit" ]]; then
        printf 'Source commit: %s\n' "$source_commit"
    fi
    printf 'Build source state: %s\n' "$source_state"
    printf 'Expected version tag: %s (tag creation is not assumed)\n' "$expected_tag"
    printf 'Build command: cargo build --release --locked\nDependency lockfile: %s\n' "$lockfile_source"
} > "$sources_path"

printf 'Platform: %s\nArchitecture: %s\nVersion: %s\nExecutable: %s\nChecksum: %s\nLicense: %s\nThird-party notices: %s\nSources: %s\n' \
    "$platform" "$architecture" "$version" "$release_binary_path" "$release_binary_path.sha256" \
    "$license_path" "$notices_path" "$sources_path"
