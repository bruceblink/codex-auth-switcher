"""Convenient source checkout entry point."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "src"))

from codex_auth_switcher.app import main  # noqa: E402


if __name__ == "__main__":
    raise SystemExit(main())
