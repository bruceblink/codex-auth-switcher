"""Launch an isolated native acceptance window using synthetic test data."""

import tkinter as tk
from pathlib import Path

from test_modes import API, SUBSCRIPTION, SYNTHETIC_AUTH, SYNTHETIC_KEY

from codex_auth_switcher.app import SwitcherApp
from codex_auth_switcher.config import (
    CodexPaths,
    SwitcherSettings,
    encode_text,
    write_atomic,
)
from codex_auth_switcher.credentials import api_auth


def main():
    home = Path(__file__).resolve().parents[1] / "build" / "ui-acceptance"
    paths = CodexPaths(home)
    write_atomic(paths.config, encode_text(API))
    write_atomic(paths.auth, api_auth(SYNTHETIC_KEY))
    write_atomic(home / "config.toml.bak-20261024-170412", encode_text(SUBSCRIPTION))
    write_atomic(home / "auth.json.bak", SYNTHETIC_AUTH)
    root = tk.Tk()
    SwitcherApp(
        root,
        paths,
        SwitcherSettings(api_env_key="CODEX_SWITCHER_ACCEPTANCE_KEY"),
        home / "settings.json",
    )
    root.mainloop()


if __name__ == "__main__":
    main()
