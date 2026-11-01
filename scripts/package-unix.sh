#!/usr/bin/env bash
# Compatibility dispatcher for Unix packaging. Prefer the platform-specific entry point.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
case "$(uname -s)" in
    Linux)
        exec bash "$SCRIPT_DIR/package-linux.sh" "$@"
        ;;
    Darwin)
        exec bash "$SCRIPT_DIR/package-macos.sh" "$@"
        ;;
    *)
        echo "Unix packaging requires Linux or macOS." >&2
        exit 1
        ;;
esac
