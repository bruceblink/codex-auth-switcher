#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# -ne 6 ]]; then
  echo "Usage: $0 <tag> <notes-file> <output-dir> <windows-dir> <linux-dir> <macos-arm64-dir>" >&2
  exit 2
fi

tag=$1
notes_file=$2
output_dir=$3
windows_dir=$4
linux_dir=$5
macos_arm64_dir=$6

if [[ ! "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Release tag must be a v-prefixed semantic version: $tag" >&2
  exit 1
fi
if [[ ! -s "$notes_file" ]]; then
  echo "Release notes are missing or empty: $notes_file" >&2
  exit 1
fi
if [[ -e "$output_dir" ]]; then
  echo "Output directory already exists: $output_dir" >&2
  exit 1
fi

version=${tag#v}
windows_asset="Codex-Auth-Switcher-$version-windows-x64-setup.exe"
linux_deb="Codex-Auth-Switcher-$version-linux-amd64.deb"
linux_appimage="Codex-Auth-Switcher-$version-linux-x86_64.AppImage"
macos_arm64="Codex-Auth-Switcher-$version-macos-arm64.dmg"
release_assets=(
  "$windows_asset"
  "$linux_deb"
  "$linux_appimage"
  "$macos_arm64"
)

validate_package_directory() {
  local directory=$1
  shift
  local expected=("$@")
  local actual=()
  local checksummed=()
  local path name size

  if [[ ! -d "$directory" || ! -f "$directory/SHA256SUMS.txt" ]]; then
    echo "Package directory or SHA256SUMS.txt is missing: $directory" >&2
    return 1
  fi
  if find "$directory" -mindepth 1 -maxdepth 1 ! -type f -print -quit | grep -q .; then
    echo "Unexpected non-file entries found in package directory: $directory" >&2
    return 1
  fi

  while IFS= read -r -d '' path; do
    actual+=("${path##*/}")
  done < <(find "$directory" -mindepth 1 -maxdepth 1 -type f -print0 | sort -z)

  for name in "${expected[@]}"; do
    if [[ ! -s "$directory/$name" ]]; then
      echo "Expected package is missing or empty: $directory/$name" >&2
      return 1
    fi
    size=$(stat --format='%s' "$directory/$name")
    if [[ ! "$size" =~ ^[0-9]+$ ]] || (( size == 0 || size > 536870912 )); then
      echo "Package size is outside the allowed range (1..536870912 bytes): $directory/$name" >&2
      return 1
    fi
  done

  if [[ ${#actual[@]} -ne $((${#expected[@]} + 1)) ]]; then
    echo "Unexpected files found in package directory: $directory" >&2
    return 1
  fi

  while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ ! "$line" =~ ^[0-9a-f]{64}[[:space:]][[:space:]][^/]+$ ]]; then
      echo "Invalid checksum line in $directory/SHA256SUMS.txt" >&2
      return 1
    fi
    checksummed+=("${line#*  }")
  done <"$directory/SHA256SUMS.txt"

  if [[ ${#checksummed[@]} -ne ${#expected[@]} ]]; then
    echo "Checksum list does not cover exactly the expected packages: $directory" >&2
    return 1
  fi

  for name in "${expected[@]}"; do
    if [[ ! " ${checksummed[*]} " == *" $name "* ]]; then
      echo "Checksum list does not contain the expected package: $name" >&2
      return 1
    fi
  done

  (
    cd "$directory"
    sha256sum --check --strict SHA256SUMS.txt
  )
}

validate_package_directory "$windows_dir" "$windows_asset"
validate_package_directory "$linux_dir" "$linux_deb" "$linux_appimage"
validate_package_directory "$macos_arm64_dir" "$macos_arm64"

mkdir -p "$output_dir"
cp -- "$windows_dir/$windows_asset" "$output_dir/"
cp -- "$linux_dir/$linux_deb" "$output_dir/"
cp -- "$linux_dir/$linux_appimage" "$output_dir/"
cp -- "$macos_arm64_dir/$macos_arm64" "$output_dir/"

(
  cd "$output_dir"
  sha256sum -- "${release_assets[@]}" >SHA256SUMS.txt
  sha256sum --check --strict SHA256SUMS.txt
)

notes=$(<"$notes_file")
published_at=$(date -u +'%Y-%m-%dT%H:%M:%SZ')
assets_json='[]'
for name in "${release_assets[@]}"; do
  digest=$(sha256sum -- "$output_dir/$name" | awk '{ print $1 }')
  size=$(stat --format='%s' "$output_dir/$name")
  assets_json=$(jq -cn \
    --argjson assets "$assets_json" \
    --arg name "$name" \
    --argjson size "$size" \
    --arg sha256 "$digest" \
    '$assets + [{name: $name, size: $size, sha256: $sha256}]')
done

jq -n \
  --arg version "$version" \
  --arg tag_name "$tag" \
  --arg notes "$notes" \
  --arg published_at "$published_at" \
  --argjson assets "$assets_json" \
  '{
    schema_version: 1,
    version: $version,
    tag_name: $tag_name,
    notes: $notes,
    published_at: $published_at,
    assets: $assets
  }' >"$output_dir/update-manifest.json"

jq -e \
  --arg version "$version" \
  --arg tag_name "$tag" \
  '
    (keys | sort) == ["assets", "notes", "published_at", "schema_version", "tag_name", "version"]
    and .schema_version == 1
    and .version == $version
    and .tag_name == $tag_name
    and (.notes | type == "string" and length > 0)
    and (.published_at | fromdateiso8601? != null)
    and (.assets | length == 4)
    and all(.assets[]; (keys | sort) == ["name", "sha256", "size"])
    and all(.assets[]; (.size | type == "number" and . > 0 and . <= 536870912))
    and all(.assets[]; (.sha256 | test("^[0-9a-f]{64}$")))
  ' "$output_dir/update-manifest.json" >/dev/null

expected_output_files=("${release_assets[@]}" SHA256SUMS.txt update-manifest.json)
actual_output_files=()
while IFS= read -r -d '' path; do
  actual_output_files+=("${path##*/}")
done < <(find "$output_dir" -mindepth 1 -maxdepth 1 -type f -print0 | sort -z)
if [[ $(printf '%s\n' "${expected_output_files[@]}" | sort) != \
      $(printf '%s\n' "${actual_output_files[@]}" | sort) ]]; then
  echo "Aggregated release directory contains an unexpected file set." >&2
  exit 1
fi
