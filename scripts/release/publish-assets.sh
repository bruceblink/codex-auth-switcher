#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# -ne 4 ]]; then
  echo "Usage: $0 <tag> <owner/repository> <release-assets-dir> <notes-file>" >&2
  exit 2
fi

tag=$1
repository=$2
assets_directory=$3
notes_file=$4

if [[ ! "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Release tag must be a v-prefixed semantic version: $tag" >&2
  exit 1
fi
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "Repository must use the owner/name format." >&2
  exit 1
fi
if [[ -z "${GH_TOKEN:-}" ]]; then
  echo "GH_TOKEN is required to publish release assets." >&2
  exit 1
fi
if [[ ! -s "$notes_file" ]]; then
  echo "Release notes are missing or empty: $notes_file" >&2
  exit 1
fi

version=${tag#v}
manifest_version=$(awk '
  /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
  /^\[/ { in_workspace_package = 0 }
  in_workspace_package && /^[[:space:]]*version[[:space:]]*=/ {
    line = $0
    sub(/^[^"]*"/, "", line)
    sub(/".*$/, "", line)
    print line
    exit
  }
' Cargo.toml)
if [[ -z "$manifest_version" || "$manifest_version" != "$version" ]]; then
  echo "Release tag $tag does not match Cargo workspace version $manifest_version." >&2
  exit 1
fi

tag_object_type=$(git cat-file -t "refs/tags/$tag")
if [[ "$tag_object_type" != "tag" ]]; then
  echo "Release tag $tag must be annotated." >&2
  exit 1
fi
tag_commit=$(git rev-parse --verify "$tag^{}")
head_commit=$(git rev-parse --verify HEAD)
if [[ "$tag_commit" != "$head_commit" ]]; then
  echo "Checked out commit $head_commit does not match release tag commit $tag_commit." >&2
  exit 1
fi

expected_packages=(
  "Codex-Auth-Switcher-$version-windows-x64-setup.exe"
  "Codex-Auth-Switcher-$version-linux-amd64.deb"
  "Codex-Auth-Switcher-$version-linux-x86_64.AppImage"
  "Codex-Auth-Switcher-$version-macos-arm64.dmg"
)
expected_release_assets=(
  "${expected_packages[@]}"
  SHA256SUMS.txt
  update-manifest.json
)
actual_release_assets=()
while IFS= read -r -d '' path; do
  actual_release_assets+=("${path##*/}")
done < <(find "$assets_directory" -mindepth 1 -maxdepth 1 -type f -print0 | sort -z)
if [[ $(printf '%s\n' "${expected_release_assets[@]}" | sort) != \
      $(printf '%s\n' "${actual_release_assets[@]}" | sort) ]]; then
  echo "Release asset directory does not contain exactly the four packages, SHA256SUMS.txt, and update-manifest.json." >&2
  exit 1
fi

(
  cd "$assets_directory"
  sha256sum --check --strict SHA256SUMS.txt
)

jq -e \
  --arg version "$version" \
  --arg tag_name "$tag" \
  --arg notes "$(cat "$notes_file")" \
  --argjson expected_packages "$(printf '%s\n' "${expected_packages[@]}" | jq -R . | jq -s .)" \
  '
    (keys | sort) == ["assets", "notes", "published_at", "schema_version", "tag_name", "version"]
    and .schema_version == 1
    and .version == $version
    and .tag_name == $tag_name
    and .notes == $notes
    and (.published_at | fromdateiso8601? != null)
    and ([.assets[].name] | sort) == ($expected_packages | sort)
    and (.assets | length == 4)
    and all(.assets[]; (keys | sort) == ["name", "sha256", "size"])
    and all(.assets[]; (.size | type == "number" and . > 0 and . <= 536870912))
    and all(.assets[]; (.sha256 | test("^[0-9a-f]{64}$")))
  ' "$assets_directory/update-manifest.json" >/dev/null

for name in "${expected_packages[@]}"; do
  manifest_hash=$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .sha256' \
    "$assets_directory/update-manifest.json")
  manifest_size=$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .size' \
    "$assets_directory/update-manifest.json")
  actual_hash=$(sha256sum -- "$assets_directory/$name" | awk '{ print $1 }')
  actual_size=$(stat --format='%s' "$assets_directory/$name")
  if [[ "$manifest_hash" != "$actual_hash" || "$manifest_size" != "$actual_size" ]]; then
    echo "Manifest hash or size does not match package: $name" >&2
    exit 1
  fi
done

release_title="Codex Auth Switcher $version"
expected_names=$(printf '%s\n' "${expected_release_assets[@]}" | sort)
temporary_root=$(mktemp -d)
trap 'rm -rf -- "$temporary_root"' EXIT
if gh release view "$tag" --repo "$repository" >/dev/null 2>&1; then
  existing_tag=$(gh release view "$tag" --repo "$repository" --json tagName --jq .tagName)
  if [[ "$existing_tag" != "$tag" ]]; then
    echo "Existing GitHub Release refers to an unexpected tag: $existing_tag" >&2
    exit 1
  fi
  existing_names=$(gh api "repos/$repository/releases/tags/$tag" --jq '.assets | map(.name) | sort | join("\n")')
  if [[ "$existing_names" != "$expected_names" ]]; then
    echo "Existing release $tag has a different asset set; refusing to change the published release." >&2
    exit 1
  fi
  existing_tag_commit=$(gh api "repos/$repository/commits/$tag" --jq .sha)
  if [[ "$existing_tag_commit" != "$head_commit" ]]; then
    echo "Existing release tag $tag resolves to a different commit; refusing to replace its assets." >&2
    exit 1
  fi

  existing_manifest_directory="$temporary_root/existing-manifest"
  mkdir -p "$existing_manifest_directory"
  gh release download "$tag" --repo "$repository" \
    --pattern update-manifest.json \
    --dir "$existing_manifest_directory"
  jq -e \
    --arg version "$version" \
    --arg tag_name "$tag" \
    '
      .schema_version == 1
      and .version == $version
      and .tag_name == $tag_name
      and (.assets | length == 4)
    ' "$existing_manifest_directory/update-manifest.json" >/dev/null

  gh release upload "$tag" --repo "$repository" --clobber \
    "${expected_release_assets[@]/#/$assets_directory/}"
  gh release edit "$tag" --repo "$repository" \
    --title "$release_title" \
    --notes-file "$notes_file"
  is_draft=$(gh release view "$tag" --repo "$repository" --json isDraft --jq .isDraft)
  if [[ "$is_draft" == "true" ]]; then
    gh release edit "$tag" --repo "$repository" --draft=false
  fi
else
  gh release create "$tag" --repo "$repository" \
    --verify-tag \
    --title "$release_title" \
    --notes-file "$notes_file" \
    "${expected_release_assets[@]/#/$assets_directory/}"
fi

published_names=$(gh api "repos/$repository/releases/tags/$tag" --jq '.assets | map(.name) | sort | join("\n")')
if [[ "$published_names" != "$expected_names" ]]; then
  echo "Published GitHub Release does not contain exactly the expected assets." >&2
  exit 1
fi

verification_directory="$temporary_root/published-assets"
mkdir -p "$verification_directory"
gh release download "$tag" --repo "$repository" --dir "$verification_directory"
downloaded_names=()
while IFS= read -r -d '' path; do
  downloaded_names+=("${path##*/}")
done < <(find "$verification_directory" -mindepth 1 -maxdepth 1 -type f -print0 | sort -z)
if [[ $(printf '%s\n' "${expected_release_assets[@]}" | sort) != \
      $(printf '%s\n' "${downloaded_names[@]}" | sort) ]]; then
  echo "Downloaded GitHub Release assets do not match the expected asset set." >&2
  exit 1
fi

(
  cd "$verification_directory"
  sha256sum --check --strict SHA256SUMS.txt
)
for name in "${expected_release_assets[@]}"; do
  if ! cmp --silent -- "$assets_directory/$name" "$verification_directory/$name"; then
    echo "Published asset differs from the validated local asset: $name" >&2
    exit 1
  fi
done

is_draft=$(gh release view "$tag" --repo "$repository" --json isDraft --jq .isDraft)
if [[ "$is_draft" != "false" ]]; then
  echo "GitHub Release remains a draft after publication." >&2
  exit 1
fi

echo "Published and verified $tag with four packages, SHA256SUMS.txt, and update-manifest.json."
