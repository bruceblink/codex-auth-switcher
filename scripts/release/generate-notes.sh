#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "Usage: $0 <tag> <owner/repository> <output-file> [previous-released-tag]" >&2
  exit 2
fi

tag=$1
repository=$2
output_file=$3
explicit_previous_tag=${4:-}

if [[ ! "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Release tag must be a v-prefixed semantic version: $tag" >&2
  exit 1
fi
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "Repository must use the owner/name format." >&2
  exit 1
fi

tag_object_type=$(git cat-file -t "refs/tags/$tag" 2>/dev/null || true)
if [[ "$tag_object_type" != "tag" ]]; then
  echo "Release tag must exist locally and be annotated: $tag" >&2
  exit 1
fi
tag_commit=$(git rev-parse --verify "$tag^{}")
version=${tag#v}

if [[ $# -eq 4 ]]; then
  previous_tag=$explicit_previous_tag
else
  previous_tag=$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$tag^" 2>/dev/null || true)
fi

if [[ -n "$previous_tag" ]]; then
  if [[ ! "$previous_tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    echo "Previous release tag must be a v-prefixed semantic version: $previous_tag" >&2
    exit 1
  fi
  previous_object_type=$(git cat-file -t "refs/tags/$previous_tag" 2>/dev/null || true)
  if [[ "$previous_object_type" != "tag" ]]; then
    echo "Previous release tag must exist locally and be annotated: $previous_tag" >&2
    exit 1
  fi
  previous_commit=$(git rev-parse --verify "$previous_tag^{}")
  if [[ "$previous_commit" == "$tag_commit" ]] ||
    ! git merge-base --is-ancestor "$previous_commit" "$tag_commit"; then
    echo "Previous release tag must point to an earlier ancestor commit: $previous_tag" >&2
    exit 1
  fi
fi

today=$(date -u +'%Y-%m-%d')

declare -A section_contents=()
if [[ -n "$previous_tag" ]]; then
  range="$previous_tag..$tag"
else
  range=$tag
fi

while IFS=$'\t' read -r commit subject; do
  [[ -n "$commit" ]] || continue
  section=other
  case "$subject" in
    feat:*|feature:*|feat\(*\):*|feature\(*\):*|feat\!*:*|feat\(*\)!:*|feature\(*\)!:*)
      section=features
      ;;
    fix:*|fix\(*\):*|fix\!*:*|fix\(*\)!:*)
      section=bugs
      ;;
    security:*|security\(*\):*|sec:*|sec\(*\):*)
      section=security
      ;;
    perf:*|perf\(*\):*|perf\!*:*|perf\(*\)!:*)
      section=performance
      ;;
    docs:*|docs\(*\):*|refactor:*|refactor\(*\):*|test:*|test\(*\):*|ci:*|ci\(*\):*|build:*|build\(*\):*|chore:*|chore\(*\):*|revert:*|style:*|style\(*\):*)
      section=maintenance
      ;;
  esac

  short_commit=${commit:0:8}
  commit_url="https://github.com/$repository/commit/$commit"
  section_contents[$section]+="- [$subject]($commit_url) (\`$short_commit\`)"$'\n'
done < <(git log --no-merges --format='%H%x09%s' "$range")

if [[ -z "${section_contents[features]:-}${section_contents[bugs]:-}${section_contents[security]:-}${section_contents[performance]:-}${section_contents[maintenance]:-}${section_contents[other]:-}" ]]; then
  section_contents[other]="- No categorized commits were found in this release range."$'\n'
fi

{
  printf '## [%s] - %s\n' "$version" "$today"
  if [[ -n "${section_contents[features]:-}" ]]; then
    printf '\n### 🚀 新功能 / Features\n\n%s' "${section_contents[features]}"
  fi
  if [[ -n "${section_contents[bugs]:-}" ]]; then
    printf '\n### 🐛 问题修复 / Bug Fixes\n\n%s' "${section_contents[bugs]}"
  fi
  if [[ -n "${section_contents[security]:-}" ]]; then
    printf '\n### 🔒 安全 / Security\n\n%s' "${section_contents[security]}"
  fi
  if [[ -n "${section_contents[performance]:-}" ]]; then
    printf '\n### ⚡ 性能优化 / Performance\n\n%s' "${section_contents[performance]}"
  fi
  if [[ -n "${section_contents[maintenance]:-}" ]]; then
    printf '\n### 🧰 维护、文档与测试 / Maintenance, Docs & Tests\n\n%s' \
      "${section_contents[maintenance]}"
  fi
  if [[ -n "${section_contents[other]:-}" ]]; then
    printf '\n### 📝 其他变更 / Other Changes\n\n%s' "${section_contents[other]}"
  fi
  if [[ -n "$previous_tag" ]]; then
    printf '\n[Full Changelog](https://github.com/%s/compare/%s...%s)\n' \
      "$repository" "$previous_tag" "$tag"
  fi
} >"$output_file"

if [[ ! -s "$output_file" ]]; then
  echo "Generated release notes are empty." >&2
  exit 1
fi
