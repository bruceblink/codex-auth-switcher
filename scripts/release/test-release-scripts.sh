#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
temporary_root=$(mktemp -d)
trap 'rm -rf -- "$temporary_root"' EXIT

notes_repo="$temporary_root/notes-repo"
mkdir -p "$notes_repo"
git -C "$notes_repo" init --quiet
git -C "$notes_repo" config user.name "Codex Auth Switcher CI"
git -C "$notes_repo" config user.email "ci@example.invalid"
printf 'baseline\n' >"$notes_repo/source.txt"
git -C "$notes_repo" add source.txt
git -C "$notes_repo" commit --quiet -m "build: create release baseline"
git -C "$notes_repo" tag -a v0.1.0 -m "v0.1.0"
printf 'feature\n' >>"$notes_repo/source.txt"
git -C "$notes_repo" commit --quiet -am "feat: add a test feature"
git -C "$notes_repo" tag -a v0.2.0 -m "v0.2.0 unpublished"
printf 'fix\n' >>"$notes_repo/source.txt"
git -C "$notes_repo" commit --quiet -am "fix: correct a test issue"
printf 'docs\n' >>"$notes_repo/source.txt"
git -C "$notes_repo" commit --quiet -am 'docs: preserve literal \t in a commit title'
git -C "$notes_repo" tag -a v0.3.0 -m "v0.3.0"

notes_file="$temporary_root/release-notes.md"
(
  cd "$notes_repo"
  bash "$script_dir/generate-notes.sh" v0.3.0 example/project "$notes_file" v0.1.0
)
grep -Eq '^## \[0\.3\.0\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$' "$notes_file"
grep -Eq '^### 🚀 新功能 / Features$' "$notes_file"
grep -Eq '^### 🐛 问题修复 / Bug Fixes$' "$notes_file"
grep -Eq 'Full Changelog.*v0\.1\.0\.\.\.v0\.3\.0' "$notes_file"
grep -Fq 'feat: add a test feature' "$notes_file"
grep -Fq 'fix: correct a test issue' "$notes_file"
grep -Fq 'preserve literal \t in a commit title' "$notes_file"

legacy_notes_file="$temporary_root/legacy-release-notes.md"
(
  cd "$notes_repo"
  bash "$script_dir/generate-notes.sh" v0.3.0 example/project "$legacy_notes_file"
)
grep -Eq 'Full Changelog.*v0\.2\.0\.\.\.v0\.3\.0' "$legacy_notes_file"

full_history_notes_file="$temporary_root/full-history-release-notes.md"
(
  cd "$notes_repo"
  bash "$script_dir/generate-notes.sh" v0.3.0 example/project "$full_history_notes_file" ""
)
grep -Fq 'build: create release baseline' "$full_history_notes_file"
grep -Fq 'feat: add a test feature' "$full_history_notes_file"
if grep -Fq 'Full Changelog' "$full_history_notes_file"; then
  echo "An explicitly empty release baseline unexpectedly generated a comparison link." >&2
  exit 1
fi

git -C "$notes_repo" switch --quiet --detach v0.1.0
printf 'unrelated\n' >>"$notes_repo/source.txt"
git -C "$notes_repo" commit --quiet -am "feat: add an unrelated branch feature"
git -C "$notes_repo" tag -a v9.0.0 -m "v9.0.0 unrelated"
git -C "$notes_repo" switch --quiet --detach v0.3.0

for invalid_tag in v0.3.0-rc.1 v00.3.0; do
  if (
    cd "$notes_repo"
    bash "$script_dir/generate-notes.sh" "$invalid_tag" example/project "$temporary_root/invalid-notes.md" v0.1.0
  ) >/dev/null 2>&1; then
    echo "Release notes accepted a non-canonical stable tag: $invalid_tag" >&2
    exit 1
  fi
done

for invalid_baseline in v0.3.0-rc.1 v0.3.0 v9.0.0 v8.0.0; do
  if (
    cd "$notes_repo"
    bash "$script_dir/generate-notes.sh" v0.3.0 example/project \
      "$temporary_root/invalid-baseline-notes.md" "$invalid_baseline"
  ) >/dev/null 2>&1; then
    echo "Release notes accepted an invalid, non-ancestor, or non-earlier baseline: $invalid_baseline" >&2
    exit 1
  fi
done

version=0.3.0
windows_dir="$temporary_root/windows"
linux_dir="$temporary_root/linux"
macos_arm64_dir="$temporary_root/macos-arm64"
mkdir -p "$windows_dir" "$linux_dir" "$macos_arm64_dir"

windows_asset="Codex-Auth-Switcher-$version-windows-x64-setup.exe"
linux_deb="Codex-Auth-Switcher-$version-linux-amd64.deb"
linux_appimage="Codex-Auth-Switcher-$version-linux-x86_64.AppImage"
macos_arm64="Codex-Auth-Switcher-$version-macos-arm64.dmg"

printf 'synthetic Windows installer bytes\n' >"$windows_dir/$windows_asset"
printf 'synthetic Linux deb bytes\n' >"$linux_dir/$linux_deb"
printf 'synthetic Linux AppImage bytes\n' >"$linux_dir/$linux_appimage"
printf 'synthetic macOS arm64 disk image bytes\n' >"$macos_arm64_dir/$macos_arm64"
for directory in "$windows_dir" "$linux_dir" "$macos_arm64_dir"; do
  (
    cd "$directory"
    shopt -s nullglob
    package_files=(*)
    sha256sum -- "${package_files[@]}" >SHA256SUMS.txt
  )
done

output_dir="$temporary_root/release-assets"
bash "$script_dir/aggregate-assets.sh" \
  v0.3.0 \
  "$notes_file" \
  "$output_dir" \
  "$windows_dir" \
  "$linux_dir" \
  "$macos_arm64_dir"

if bash "$script_dir/aggregate-assets.sh" \
  v0.3.0-rc.1 \
  "$notes_file" \
  "$temporary_root/invalid-release-assets" \
  "$windows_dir" \
  "$linux_dir" \
  "$macos_arm64_dir" >/dev/null 2>&1; then
  echo "Release aggregation accepted a prerelease tag." >&2
  exit 1
fi

expected_count=6
actual_count=$(find "$output_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)
[[ "$actual_count" -eq "$expected_count" ]]
(cd "$output_dir" && sha256sum --check --strict SHA256SUMS.txt)
jq -e \
  --arg version "$version" \
  --arg tag_name "v$version" \
  --arg notes "$(cat "$notes_file")" \
  '
    .schema_version == 1
    and .version == $version
    and .tag_name == $tag_name
    and .notes == $notes
    and (.assets | length == 4)
    and ([.assets[].name] | sort) == [
      "Codex-Auth-Switcher-0.3.0-linux-amd64.deb",
      "Codex-Auth-Switcher-0.3.0-linux-x86_64.AppImage",
      "Codex-Auth-Switcher-0.3.0-macos-arm64.dmg",
      "Codex-Auth-Switcher-0.3.0-windows-x64-setup.exe"
    ]
  ' "$output_dir/update-manifest.json" >/dev/null

tampered_dir="$temporary_root/tampered"
cp -a -- "$linux_dir" "$tampered_dir"
printf 'tampered\n' >>"$tampered_dir/$linux_deb"
if bash "$script_dir/aggregate-assets.sh" \
  v0.3.0 \
  "$notes_file" \
  "$temporary_root/should-not-exist" \
  "$windows_dir" \
  "$tampered_dir" \
  "$macos_arm64_dir" >/dev/null 2>&1; then
  echo "Aggregation accepted a package whose checksum was invalid." >&2
  exit 1
fi

echo "Release note and synthetic aggregation checks passed."
