"""Desktop and CLI controls for restoring complete Codex authentication modes."""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import replace
from pathlib import Path

try:
    import tkinter as tk
    from tkinter import filedialog, messagebox, ttk
except ImportError:
    tk = ttk = filedialog = messagebox = None

from .config import CodexPaths, SwitcherError, load_settings, save_settings
from .modes import ModeStore


def default_settings_path() -> Path:
    return Path(__file__).resolve().parents[2] / "settings.json"


class MaskedSecretEntry(ttk.Entry if ttk else object):
    """Keep the actual key out of Tk text values and clipboard copy operations."""

    def __init__(self, master, **kwargs):
        self._secret = ""
        self._variable = tk.StringVar(master=master)
        super().__init__(master, textvariable=self._variable, **kwargs)
        self.bind("<KeyPress>", self._on_key_press)
        self.bind("<<Paste>>", self._on_paste)
        self.bind("<Shift-Insert>", self._on_paste)
        self.bind("<<Cut>>", lambda event: "break")

    @staticmethod
    def _mask(secret):
        if len(secret) <= 1:
            return "·" if secret else ""
        return secret[0] + "·" * (len(secret) - 2) + secret[-1]

    def _selection(self):
        try:
            return self.index("sel.first"), self.index("sel.last")
        except tk.TclError:
            return self.index(tk.INSERT), self.index(tk.INSERT)

    def _render(self, cursor):
        self._variable.set(self._mask(self._secret))
        self.icursor(max(0, min(cursor, len(self._secret))))
        self.selection_clear()

    def _replace_selection(self, value):
        start, end = self._selection()
        self._secret = self._secret[:start] + value + self._secret[end:]
        self._render(start + len(value))

    def _on_paste(self, _event):
        try:
            value = self.clipboard_get().strip()
        except tk.TclError:
            return "break"
        if value and not any(ord(char) < 33 for char in value):
            self._replace_selection(value)
        return "break"

    def _on_key_press(self, event):
        key = str(event.keysym).lower()
        if event.state & 0x0004:
            if key == "v":
                return self._on_paste(event)
            if key == "a":
                self.selection_range(0, tk.END)
                return "break"
            return None if key in {"c", "left", "right", "home", "end"} else "break"
        start, end = self._selection()
        cursor = self.index(tk.INSERT)
        if key in {"backspace", "delete"}:
            if start == end:
                start, end = (
                    (max(0, cursor - 1), cursor)
                    if key == "backspace"
                    else (cursor, cursor + 1)
                )
            self._secret = self._secret[:start] + self._secret[end:]
            self._render(start)
            return "break"
        if key in {"left", "right", "home", "end", "tab", "return", "escape"}:
            return None
        if event.char and event.char.isprintable() and not event.char.isspace():
            self._replace_selection(event.char)
        return "break"

    def get_secret(self):
        return self._secret

    def clear(self):
        self._secret = ""
        self._variable.set("")


def error_message(exc):
    return (
        str(exc)
        if isinstance(exc, SwitcherError)
        else "文件操作失败。请检查目录权限、文件占用和磁盘空间。"
    )


class SwitcherApp:
    def __init__(self, root, paths, settings, settings_path):
        self.root, self.paths, self.settings, self.settings_path = (
            root,
            paths,
            settings,
            settings_path,
        )
        self.store = ModeStore(paths, settings)
        root.title("Codex 认证切换工具")
        root.geometry("820x720")
        root.minsize(740, 680)
        self.summary = tk.StringVar(master=root)
        self.subscription_provider = tk.StringVar(
            master=root, value=settings.subscription_provider
        )
        self.api_provider = tk.StringVar(master=root, value=settings.api_provider)
        self.base_url = tk.StringVar(master=root)
        self.remember_base = tk.BooleanVar(master=root, value=False)
        outer = ttk.Frame(root, padding=20)
        outer.pack(fill="both", expand=True)
        ttk.Label(outer, text="Codex 认证切换工具", font=("Segoe UI", 16, "bold")).pack(
            anchor="w"
        )
        ttk.Label(outer, textvariable=self.summary, wraplength=730).pack(
            anchor="w", pady=(12, 8)
        )
        ttk.Label(outer, text=str(paths.config), wraplength=730).pack(
            anchor="w", pady=(0, 12)
        )
        self.tree = ttk.Treeview(
            outer,
            columns=("provider", "model", "effort", "ready"),
            height=2,
            selectmode="none",
        )
        self.tree.heading("#0", text="模式")
        self.tree.column("#0", width=75, stretch=False)
        for key, title, width in (
            ("provider", "Provider", 110),
            ("model", "模型", 160),
            ("effort", "推理级别", 100),
            ("ready", "认证状态", 100),
        ):
            self.tree.heading(key, text=title)
            self.tree.column(key, width=width)
        self.tree.pack(fill="x")
        toolbar = ttk.Frame(outer)
        toolbar.pack(fill="x", pady=10)
        ttk.Button(toolbar, text="初始化 / 补齐模式", command=self.initialize).pack(
            side="left"
        )
        imports = ttk.Menubutton(toolbar, text="导入来源")
        menu = tk.Menu(imports, tearoff=False)
        for key, title in (
            ("subscription_config", "订阅配置"),
            ("subscription_auth", "订阅认证"),
            ("api_config", "API 配置"),
            ("api_auth", "API 认证"),
        ):
            menu.add_command(
                label=title, command=lambda k=key, t=title: self.import_source(k, t)
            )
        imports.configure(menu=menu)
        imports.pack(side="left", padx=8)
        ttk.Button(toolbar, text="刷新状态", command=self.refresh).pack(side="right")
        parameters = ttk.Frame(outer)
        parameters.pack(fill="x", pady=(8, 12))
        fields = (
            ("订阅 model_provider", self.subscription_provider),
            ("API provider ID", self.api_provider),
            ("API base_url", self.base_url),
        )
        for row, (label, variable) in enumerate(fields):
            ttk.Label(parameters, text=label, width=24).grid(
                row=row, column=0, sticky="w", pady=5
            )
            ttk.Entry(parameters, textvariable=variable).grid(
                row=row, column=1, sticky="ew", pady=5
            )
        ttk.Label(parameters, text="API key", width=24).grid(
            row=3, column=0, sticky="w", pady=5
        )
        self.key_entry = MaskedSecretEntry(parameters)
        self.key_entry.grid(row=3, column=1, sticky="ew", pady=5)
        ttk.Checkbutton(
            parameters, text="记住 base_url 为默认值", variable=self.remember_base
        ).grid(row=4, column=1, sticky="w", pady=4)
        parameters.columnconfigure(1, weight=1)
        actions = ttk.Frame(outer)
        actions.pack(fill="x", pady=(0, 12))
        ttk.Button(
            actions, text="切换到订阅", command=lambda: self.switch("subscription")
        ).pack(side="left", fill="x", expand=True, padx=(0, 6))
        ttk.Button(
            actions, text="切换到 API key", command=lambda: self.switch("api")
        ).pack(side="left", fill="x", expand=True, padx=(6, 0))
        ttk.Label(outer, text="操作记录").pack(anchor="w")
        self.log = tk.Text(outer, height=9, state="disabled", wrap="word")
        self.log.pack(fill="both", expand=True, pady=(6, 0))
        self.refresh(fill_inputs=True)
        self.append("旧会话可能保留原 provider。切换后请重启 Codex 并使用新任务。")

    def append(self, text):
        self.log.configure(state="normal")
        self.log.insert("end", text + "\n")
        self.log.see("end")
        self.log.configure(state="disabled")

    def refresh(self, fill_inputs=False):
        try:
            state = self.store.inspect()
            self.summary.set(
                f"当前模式：{state['mode']}    Provider：{state['current_provider']}    模型：{state['model']} / {state['reasoning_effort']}    认证：{state['auth_mode']}"
            )
            self.tree.delete(*self.tree.get_children())
            for mode, label in (("subscription", "订阅"), ("api", "API key")):
                profile = state["profiles"].get(mode, {})
                self.tree.insert(
                    "",
                    "end",
                    iid=mode,
                    text=label,
                    values=(
                        profile.get("provider", "未初始化"),
                        profile.get("model", "-"),
                        profile.get("reasoning_effort", "-"),
                        "已就绪" if profile.get("ready") else "待补齐",
                    ),
                )
            if fill_inputs:
                api = state["profiles"].get("api", {})
                self.api_provider.set(
                    os.environ.get("CODEX_API_PROVIDER")
                    or api.get("provider")
                    or (
                        state["current_provider"]
                        if state["mode"] == "api"
                        else self.settings.api_provider
                    )
                )
                self.subscription_provider.set(
                    os.environ.get("CODEX_MODEL_PROVIDER")
                    or state["profiles"].get("subscription", {}).get("provider")
                    or self.settings.subscription_provider
                )
                self.base_url.set(
                    os.environ.get(self.settings.api_base_url_env_key)
                    or (state["base_url"] if state["mode"] == "api" else "")
                )
            if state["recovery_pending"]:
                self.append("检测到未完成写入。请关闭 Codex 后执行初始化以恢复。")
            return state
        except (SwitcherError, OSError, ValueError, TypeError) as exc:
            self.summary.set("读取失败")
            self.append(error_message(exc))
            return None

    def initialize(self, sources=None):
        try:
            self.store.initialize(sources)
            self.refresh(fill_inputs=True)
            state = self.store.inspect()
            for mode, profile in state["profiles"].items():
                self.append(
                    f"{mode}：配置来源 {profile['config_source']}；认证来源 {profile['auth_source']}"
                )
                if profile["issue"]:
                    self.append(profile["issue"])
            self.append("模式状态已加密保存。当前配置和认证保持原样。")
        except (SwitcherError, OSError, ValueError, TypeError) as exc:
            messagebox.showerror("初始化失败", error_message(exc), parent=self.root)

    def import_source(self, key, title):
        path = filedialog.askopenfilename(
            title=f"选择{title}文件", initialdir=self.paths.home, parent=self.root
        )
        if path:
            self.initialize({key: Path(path)})

    def switch(self, target):
        try:
            provider = (
                (self.api_provider if target == "api" else self.subscription_provider)
                .get()
                .strip()
            )
            provider = provider or os.environ.get(
                "CODEX_API_PROVIDER" if target == "api" else "CODEX_MODEL_PROVIDER"
            )
            base = (
                (
                    self.base_url.get().strip()
                    or os.environ.get(self.settings.api_base_url_env_key)
                )
                if target == "api"
                else None
            )
            key = (self.key_entry.get_secret() or None) if target == "api" else None
            result = self.store.switch(
                target, provider=provider, api_base_url=base, api_key=key
            )
        except (SwitcherError, OSError, ValueError, TypeError) as exc:
            self.append(error_message(exc))
            self.refresh()
            messagebox.showerror("切换失败", error_message(exc), parent=self.root)
            return
        self.key_entry.clear()
        self.refresh()
        self.append(
            f"已选择 {target}；文件变化：{'是' if result['changed'] else '否'}。"
        )
        if result["backup"]:
            self.append(f"配置备份：{result['backup']}")
        if target == "api" and base and self.remember_base.get():
            try:
                self.settings = replace(self.settings, api_base_url=base)
                save_settings(self.settings_path, self.settings)
            except (SwitcherError, OSError, ValueError, TypeError) as exc:
                self.append("切换已完成，但默认地址保存失败：" + error_message(exc))
        self.append("请完全退出并重新打开桌面 Codex，再开始新任务。")


def build_parser():
    parser = argparse.ArgumentParser(
        description="Restore Codex subscription/API configuration and authentication together."
    )
    parser.add_argument(
        "--target",
        choices=("gui", "status", "initialize", "subscription", "api"),
        default="gui",
    )
    parser.add_argument("--codex-home")
    parser.add_argument("--settings", type=Path)
    for mode in ("subscription", "api"):
        for kind in ("config", "auth"):
            parser.add_argument(
                f"--{mode}-{kind}", type=Path, help="initialization source file"
            )
    parser.add_argument(
        "--provider", help="explicit provider ID; overrides environment and saved mode"
    )
    parser.add_argument("--api-base-url")
    parser.add_argument(
        "--api-key-stdin",
        action="store_true",
        help="read a key from stdin, never from a command argument",
    )
    parser.add_argument("--remember-base-url", action="store_true")
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    sources = {
        f"{mode}_{kind}": getattr(args, f"{mode}_{kind}")
        for mode in ("subscription", "api")
        for kind in ("config", "auth")
        if getattr(args, f"{mode}_{kind}")
    }
    if sources and args.target != "initialize":
        parser.error("source files require --target initialize")
    if args.target != "api" and (
        args.api_base_url is not None or args.api_key_stdin or args.remember_base_url
    ):
        parser.error("API input options require --target api")
    if args.provider and args.target not in {"subscription", "api"}:
        parser.error("--provider requires a switch target")
    if args.remember_base_url and not args.api_base_url:
        parser.error("--remember-base-url requires --api-base-url")
    try:
        settings_path = args.settings or default_settings_path()
        settings = load_settings(settings_path)
        paths = CodexPaths.resolve(args.codex_home)
        store = ModeStore(paths, settings)
        if args.target == "gui":
            if tk is None:
                raise SwitcherError(
                    "Install Python with Tcl/Tk to open the desktop window."
                )
            root = tk.Tk()
            SwitcherApp(root, paths, settings, settings_path)
            root.mainloop()
            return 0
        if args.target == "status":
            result = store.inspect()
        elif args.target == "initialize":
            result = store.initialize(sources)
        else:
            key = sys.stdin.read().strip() if args.api_key_stdin else None
            if args.api_key_stdin and not key:
                raise SwitcherError("No API key was provided on stdin.")
            provider = args.provider or os.environ.get(
                "CODEX_API_PROVIDER" if args.target == "api" else "CODEX_MODEL_PROVIDER"
            )
            result = store.switch(
                args.target,
                provider=provider,
                api_key=key,
                api_base_url=args.api_base_url
                or (
                    os.environ.get(settings.api_base_url_env_key)
                    if args.target == "api"
                    else None
                ),
            )
            if args.remember_base_url:
                try:
                    save_settings(
                        settings_path, replace(settings, api_base_url=args.api_base_url)
                    )
                except (SwitcherError, OSError, ValueError, TypeError):
                    result["warning"] = (
                        "Switch completed, but saving the default base URL failed."
                    )
        print(json.dumps(result, indent=2, ensure_ascii=False))
        return 0
    except (SwitcherError, OSError, ValueError, TypeError) as exc:
        print(error_message(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
