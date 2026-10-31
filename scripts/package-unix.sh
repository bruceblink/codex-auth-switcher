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

mkdir -p "$output_directory"
output_directory=$(CDPATH= cd -- "$output_directory" && pwd)
binary_path="$repository_root/target/release/codex-auth-switcher"
release_name="codex-auth-switcher-v${version}-${platform}-${architecture}"
release_binary_path="$output_directory/$release_name"

for path in "$release_binary_path" "$release_binary_path.sha256"; do
    if [[ -e "$path" ]]; then
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

printf 'Platform: %s\nArchitecture: %s\nVersion: %s\nExecutable: %s\nChecksum: %s\n' \
    "$platform" "$architecture" "$version" "$release_binary_path" "$release_binary_path.sha256"
