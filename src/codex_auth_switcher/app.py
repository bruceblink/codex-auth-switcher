"""Tkinter desktop application and CLI entry point."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import replace
from pathlib import Path

try:
    import tkinter as tk
    from tkinter import messagebox, ttk
except ImportError:  # pragma: no cover - only reached on Python builds without Tk.
    tk = None
    messagebox = None
    ttk = None

from .config import (
    CodexPaths,
    SwitcherSettings,
    inspect_state,
    load_settings,
    save_settings,
    switch_provider,
)


def project_root() -> Path:
    """Return the project root used for the optional settings file."""

    return Path(__file__).resolve().parents[2]


def default_settings_path() -> Path:
    """Resolve the checked-in settings file without touching Codex state."""

    return project_root() / "settings.json"


class MaskedSecretEntry(ttk.Entry):
    """Entry that keeps the key private while showing only its ends.

    The displayed string has the same length as the secret, so normal cursor
    movement and selection still work.  Clipboard paste is handled explicitly;
    copying the field copies the masked display rather than the secret.
    """

    def __init__(self, master: tk.Misc, **kwargs: object) -> None:
        self._secret = ""
        self._variable = tk.StringVar()
        super().__init__(master, textvariable=self._variable, **kwargs)
        self.bind("<KeyPress>", self._on_key_press, add="+")
        self.bind("<Control-KeyPress-v>", self._on_paste, add="+")
        self.bind("<Control-KeyPress-V>", self._on_paste, add="+")

    @staticmethod
    def _mask(secret: str) -> str:
        if len(secret) <= 1:
            return "·" if secret else ""
        if len(secret) == 2:
            return secret
        return f"{secret[0]}{'·' * (len(secret) - 2)}{secret[-1]}"

    def _selection(self) -> tuple[int, int]:
        try:
            return self.index("sel.first"), self.index("sel.last")
        except tk.TclError:
            cursor = self.index(tk.INSERT)
            return cursor, cursor

    def _render(self, cursor: int | None = None) -> None:
        if cursor is None:
            cursor = self.index(tk.INSERT)
        self._variable.set(self._mask(self._secret))
        self.icursor(max(0, min(cursor, len(self._secret))))

    def _replace_selection(self, value: str) -> None:
        start, end = self._selection()
        self._secret = f"{self._secret[:start]}{value}{self._secret[end:]}"
        self._render(start + len(value))

    def _on_paste(self, _event: tk.Event[tk.Misc]) -> str:
        try:
            value = self.clipboard_get().strip()
        except tk.TclError:
            value = ""
        if value:
            self._replace_selection(value)
        return "break"

    def _on_key_press(self, event: tk.Event[tk.Misc]) -> str | None:
        key = str(event.keysym).lower()
        if event.state & 0x0004:  # Control modifier on Windows.
            if key == "v":
                return self._on_paste(event)
            if key == "a":
                self.selection_range(0, tk.END)
                return "break"
            if key in {"c", "left", "right", "home", "end"}:
                return None
            return "break"

        start, end = self._selection()
        cursor = self.index(tk.INSERT)
        if key == "backspace":
            if start == end and cursor > 0:
                start, end = cursor - 1, cursor
            self._secret = f"{self._secret[:start]}{self._secret[end:]}"
            self._render(start)
            return "break"
        if key == "delete":
            if start == end:
                start, end = cursor, min(cursor + 1, len(self._secret))
            self._secret = f"{self._secret[:start]}{self._secret[end:]}"
            self._render(start)
            return "break"
        if key in {"left", "right", "home", "end", "tab", "return", "escape"}:
            return None
        if event.char and event.char.isprintable():
            self._replace_selection(event.char)
            return "break"
        return "break"

    def get_secret(self) -> str:
        """Return the key to the caller without placing it in a log or label."""

        return self._secret

    def clear(self) -> None:
        """Clear the in-memory key and its masked display."""

        self._secret = ""
        self._variable.set("")


class SwitcherApp:
    """Small desktop window for switching the active Codex provider."""

    def __init__(
        self,
        root: tk.Tk,
        paths: CodexPaths,
        settings: SwitcherSettings,
        settings_path: Path,
    ) -> None:
        self.root = root
        self.paths = paths
        self.settings = settings
        self.settings_path = settings_path
        self.root.title("Codex 认证切换工具")
        self.root.geometry("760x650")
        self.root.minsize(680, 580)

        self.provider_value = tk.StringVar(value="读取中")
        self.key_value = tk.StringVar(value="读取中")
        self.config_value = tk.StringVar(value=str(paths.config))
        self.profile_value = tk.StringVar(value=str(paths.subscription_profile))
        self.api_base_value = tk.StringVar(value=settings.api_base_url)
        self.persist_key_value = tk.BooleanVar(value=True)
        self.remember_base_value = tk.BooleanVar(value=False)
        self.api_key_entry: MaskedSecretEntry | None = None
        self.log_text: tk.Text | None = None
        self._build_ui()
        self.refresh()

    def _build_ui(self) -> None:
        """Build the stable controls used by both provider actions."""

        style = ttk.Style(self.root)
        try:
            style.theme_use("vista")
        except tk.TclError:
            pass
        style.configure("Title.TLabel", font=("Segoe UI", 16, "bold"))
        style.configure("Action.TButton", padding=(16, 10))

        outer = ttk.Frame(self.root, padding=20)
        outer.pack(fill="both", expand=True)

        ttk.Label(outer, text="Codex 认证切换工具", style="Title.TLabel").pack(anchor="w")
        ttk.Label(
            outer,
            text="切换只修改主 config.toml。autocode.config.toml 和 auth.json 不会被修改；API key 只按下方选项保存到 .env。",
            wraplength=700,
        ).pack(anchor="w", pady=(6, 16))

        state_frame = ttk.LabelFrame(outer, text="当前状态", padding=12)
        state_frame.pack(fill="x")
        self._state_row(state_frame, 0, "当前 provider", self.provider_value)
        self._state_row(state_frame, 1, "API key 状态", self.key_value)
        self._state_row(state_frame, 2, "主配置", self.config_value)
        self._state_row(state_frame, 3, "订阅 profile（保留）", self.profile_value)

        action_frame = ttk.LabelFrame(outer, text="切换", padding=12)
        action_frame.pack(fill="x", pady=(16, 0))
        ttk.Button(
            action_frame,
            text=f"切换到订阅（{self.settings.subscription_provider}）",
            style="Action.TButton",
            command=lambda: self.switch("subscription"),
        ).grid(row=0, column=0, sticky="ew", padx=(0, 8))
        ttk.Button(
            action_frame,
            text=f"切换到 {self.settings.api_display_name}",
            style="Action.TButton",
            command=lambda: self.switch("api"),
        ).grid(row=0, column=1, sticky="ew", padx=(8, 0))
        action_frame.columnconfigure(0, weight=1)
        action_frame.columnconfigure(1, weight=1)

        api_frame = ttk.LabelFrame(outer, text="API key 参数", padding=12)
        api_frame.pack(fill="x", pady=(12, 0))
        ttk.Label(api_frame, text="base_url", width=18).grid(row=0, column=0, sticky="w", pady=3)
        ttk.Entry(api_frame, textvariable=self.api_base_value).grid(
            row=0, column=1, columnspan=2, sticky="ew", pady=3
        )
        ttk.Label(api_frame, text="API_KEY（首尾可见，中间使用 ·）", width=28).grid(
            row=1, column=0, sticky="w", pady=3
        )
        self.api_key_entry = MaskedSecretEntry(api_frame, width=54)
        self.api_key_entry.grid(row=1, column=1, columnspan=2, sticky="ew", pady=3)
        ttk.Checkbutton(
            api_frame,
            text=f"保存到 {self.paths.dotenv}",
            variable=self.persist_key_value,
        ).grid(row=2, column=1, sticky="w", pady=(5, 0))
        ttk.Checkbutton(
            api_frame,
            text="记住 base_url 为默认值",
            variable=self.remember_base_value,
        ).grid(row=2, column=2, sticky="w", pady=(5, 0))
        api_frame.columnconfigure(1, weight=1)
        api_frame.columnconfigure(2, weight=1)

        utility_frame = ttk.Frame(outer)
        utility_frame.pack(fill="x", pady=(12, 0))
        ttk.Button(utility_frame, text="刷新状态", command=self.refresh).pack(side="left")
        ttk.Button(utility_frame, text="打开 .codex 目录", command=self.open_codex_home).pack(
            side="left", padx=(8, 0)
        )
        ttk.Label(
            utility_frame,
            text="修改后请完全退出并重新打开桌面 Codex。",
            foreground="#6b4f00",
        ).pack(side="right")

        log_frame = ttk.LabelFrame(outer, text="操作记录", padding=8)
        log_frame.pack(fill="both", expand=True, pady=(14, 0))
        self.log_text = tk.Text(log_frame, height=7, state="disabled", wrap="word")
        self.log_text.pack(fill="both", expand=True)

    @staticmethod
    def _state_row(parent: ttk.Frame, row: int, label: str, value: tk.StringVar) -> None:
        """Add one redacted state row to the status panel."""

        ttk.Label(parent, text=label, width=18).grid(row=row, column=0, sticky="w", pady=2)
        ttk.Label(parent, textvariable=value).grid(row=row, column=1, sticky="w", pady=2)
        parent.columnconfigure(1, weight=1)

    def _log(self, lines: list[str]) -> None:
        """Append status text without allowing edits from the user."""

        if self.log_text is None:
            return
        self.log_text.configure(state="normal")
        self.log_text.insert("end", "\n".join(lines) + "\n")
        self.log_text.see("end")
        self.log_text.configure(state="disabled")

    def refresh(self) -> None:
        """Refresh redacted provider and key-source information."""

        try:
            state = inspect_state(self.paths, self.settings)
        except Exception as exc:  # noqa: BLE001 - the UI must show a useful path error.
            self.provider_value.set("读取失败")
            self.key_value.set(str(exc))
            self._log([f"读取状态失败：{exc}"])
            return

        self.provider_value.set(str(state["current_provider"]))
        if state["api_key_detected"]:
            self.key_value.set(f"已检测（{state['api_key_source']}）")
        else:
            self.key_value.set("未检测到")
        self.profile_value.set(
            f"{state['subscription_profile']}（{'存在' if state['subscription_profile_exists'] else '未找到'}）"
        )

    def switch(self, target: str) -> None:
        """Apply one switch and explain backup/restart state in the UI."""

        api_base_url: str | None = None
        api_key: str | None = None
        persist_api_key = False
        if target == "api":
            api_base_url = self.api_base_value.get().strip()
            api_key = self.api_key_entry.get_secret() if self.api_key_entry else None
            persist_api_key = self.persist_key_value.get()

        try:
            result = switch_provider(
                self.paths,
                self.settings,
                target,  # type: ignore[arg-type]
                api_base_url=api_base_url,
                api_key=api_key or None,
                persist_api_key=persist_api_key,
            )
        except Exception as exc:  # noqa: BLE001 - surface actionable file errors.
            self._log([f"切换失败：{exc}"])
            messagebox.showerror("切换失败", str(exc))
            return

        if target == "api" and self.remember_base_value.get() and api_base_url:
            try:
                self.settings = replace(self.settings, api_base_url=api_base_url)
                save_settings(self.settings_path, self.settings)
            except Exception as exc:  # noqa: BLE001 - the provider switch already succeeded.
                self._log([f"默认 base_url 保存失败：{exc}"])

        lines = [
            f"已选择 {result.provider}。配置是否改变：{'是' if result.changed else '否'}。",
        ]
        if result.backup_path:
            lines.append(f"备份：{result.backup_path}")
        lines.extend(result.warnings)
        lines.append("请完全退出并重新打开桌面 Codex 后开始新任务。")
        self._log(lines)
        if self.api_key_entry:
            self.api_key_entry.clear()
        self.refresh()
        messagebox.showinfo("切换完成", "provider 已更新。请重启桌面 Codex 使设置生效。")

    def open_codex_home(self) -> None:
        """Open the Codex state directory in Windows Explorer."""

        self.paths.home.mkdir(parents=True, exist_ok=True)
        if hasattr(os, "startfile"):
            os.startfile(str(self.paths.home))  # type: ignore[attr-defined]
        else:  # pragma: no cover - useful only when manually running on Unix.
            self._log([f"Codex 目录：{self.paths.home}"])


def run_gui(paths: CodexPaths, settings: SwitcherSettings, settings_path: Path) -> int:
    """Start the Tkinter desktop application."""

    if tk is None:
        raise RuntimeError("当前 Python 没有 Tkinter，请安装带 Tcl/Tk 的 Python 发行版。")
    root = tk.Tk()
    SwitcherApp(root, paths, settings, settings_path)
    root.mainloop()
    return 0


def build_parser() -> argparse.ArgumentParser:
    """Create the command-line interface used by scripts and diagnostics."""

    parser = argparse.ArgumentParser(description="Switch the active Codex provider safely.")
    parser.add_argument(
        "--target",
        choices=("gui", "subscription", "api", "status"),
        default="gui",
        help="open the GUI or perform one redacted CLI action",
    )
    parser.add_argument("--codex-home", help="override CODEX_HOME for testing or another profile")
    parser.add_argument("--settings", type=Path, help="path to a settings.json file")
    parser.add_argument("--api-base-url", help="API provider base URL for --target api")
    parser.add_argument(
        "--api-key-stdin",
        action="store_true",
        help="read the API key from stdin for --target api; never pass keys as command-line arguments",
    )
    parser.add_argument(
        "--no-save-api-key",
        action="store_true",
        help="do not write a stdin API key to the Codex .env file",
    )
    parser.add_argument(
        "--remember-base-url",
        action="store_true",
        help="save --api-base-url as the project default",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Run the GUI or one command-line operation."""

    parser = build_parser()
    args = parser.parse_args(argv)
    paths = CodexPaths.resolve(args.codex_home)
    settings = load_settings(args.settings or default_settings_path())

    if args.target == "gui":
        return run_gui(paths, settings, args.settings or default_settings_path())
    if args.target == "status":
        print(json.dumps(inspect_state(paths, settings), ensure_ascii=False, indent=2))
        return 0

    if args.target != "api" and (
        args.api_base_url or args.api_key_stdin or args.no_save_api_key or args.remember_base_url
    ):
        parser.error("API input options require --target api")
    api_key = sys.stdin.read().strip() if args.api_key_stdin else None
    result = switch_provider(
        paths,
        settings,
        args.target,  # type: ignore[arg-type]
        api_base_url=args.api_base_url,
        api_key=api_key or None,
        persist_api_key=bool(api_key) and not args.no_save_api_key,
    )
    if args.target == "api" and args.remember_base_url and args.api_base_url:
        save_settings(args.settings or default_settings_path(), replace(settings, api_base_url=args.api_base_url))
    print(f"provider={result.provider}")
    print(f"changed={str(result.changed).lower()}")
    print(f"api_key_detected={str(result.api_key.detected).lower()}")
    print(f"api_key_source={result.api_key.source or 'none'}")
    if result.backup_path:
        print(f"backup={result.backup_path}")
    for warning in result.warnings:
        print(f"warning={warning}")
    print("restart_required=true")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
