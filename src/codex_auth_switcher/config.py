"""Configuration editing and authentication source detection.

The switcher edits only the active user-level ``config.toml``.  It never edits
the user's profile files, auth cache, or API key value.  The text editor keeps
the existing TOML layout so unrelated Codex settings remain unchanged.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import tempfile
from dataclasses import dataclass, replace
from datetime import datetime
from pathlib import Path
from typing import Any, Literal


TargetMode = Literal["subscription", "api"]


@dataclass(frozen=True)
class SwitcherSettings:
    """Provider identifiers used by the two switch targets.

    ``subscription_provider`` defaults to the custom ``OpenAI`` provider used
    by the existing subscription gateway profile.  Users who use the direct
    OpenAI provider can change this value to ``openai`` in ``settings.json``.
    """

    subscription_provider: str = "OpenAI"
    api_provider: str = "autocode"
    api_env_key: str = "AUTOCODE_API_KEY"
    api_base_url: str = "https://vip.auto-code.net"
    api_display_name: str = "AutoCode API key"

    def validate(self) -> None:
        """Reject values that could create an invalid provider configuration."""

        provider_pattern = re.compile(r"^[A-Za-z0-9_.-]+$")
        for name, value in (
            ("subscription_provider", self.subscription_provider),
            ("api_provider", self.api_provider),
        ):
            if not value or not provider_pattern.fullmatch(value):
                raise ValueError(f"{name} must be a simple Codex provider id")
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", self.api_env_key):
            raise ValueError("api_env_key must be a valid environment variable name")
        if not self.api_base_url.startswith(("https://", "http://")):
            raise ValueError("api_base_url must be an HTTP(S) URL")


@dataclass(frozen=True)
class CodexPaths:
    """Resolved Codex state paths for the current Windows user."""

    home: Path
    config: Path
    dotenv: Path
    subscription_profile: Path

    @classmethod
    def resolve(cls, codex_home: str | Path | None = None) -> "CodexPaths":
        """Resolve ``CODEX_HOME`` while keeping the default Windows location."""

        if codex_home:
            home = Path(codex_home).expanduser()
        else:
            home = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex"))
        return cls(
            home=home,
            config=home / "config.toml",
            dotenv=home / ".env",
            subscription_profile=home / "autocode.config.toml",
        )


@dataclass(frozen=True)
class ApiKeyStatus:
    """Whether the configured provider key is visible to this process."""

    detected: bool
    source: str | None


@dataclass(frozen=True)
class SwitchResult:
    """Result of one provider switch operation."""

    target: TargetMode
    provider: str
    changed: bool
    backup_path: Path | None
    api_key: ApiKeyStatus
    warnings: tuple[str, ...]


def load_settings(path: Path) -> SwitcherSettings:
    """Load optional JSON settings without ever accepting a secret value."""

    defaults = SwitcherSettings()
    if not path.exists():
        defaults.validate()
        return defaults

    try:
        raw: Any = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"Invalid settings JSON: {path}") from exc
    if not isinstance(raw, dict):
        raise ValueError("settings.json must contain a JSON object")

    settings = SwitcherSettings(
        subscription_provider=str(raw.get("subscription_provider", defaults.subscription_provider)),
        api_provider=str(raw.get("api_provider", defaults.api_provider)),
        api_env_key=str(raw.get("api_env_key", defaults.api_env_key)),
        api_base_url=str(raw.get("api_base_url", defaults.api_base_url)),
        api_display_name=str(raw.get("api_display_name", defaults.api_display_name)),
    )
    settings.validate()
    return settings


def save_settings(path: Path, settings: SwitcherSettings) -> None:
    """Persist non-secret defaults such as the preferred API base URL."""

    settings.validate()
    content = json.dumps(
        {
            "subscription_provider": settings.subscription_provider,
            "api_provider": settings.api_provider,
            "api_env_key": settings.api_env_key,
            "api_base_url": settings.api_base_url,
            "api_display_name": settings.api_display_name,
        },
        ensure_ascii=False,
        indent=2,
    )
    write_utf8_lf_atomic(path, content)


def normalize_lf(text: str) -> str:
    """Normalize text written by the switcher to LF line endings."""

    return text.replace("\r\n", "\n").replace("\r", "\n")


def read_utf8(path: Path) -> str:
    """Read UTF-8 text and tolerate an existing UTF-8 BOM."""

    return path.read_text(encoding="utf-8-sig")


def write_utf8_lf_atomic(path: Path, text: str) -> None:
    """Write UTF-8/LF content through a same-directory atomic replacement."""

    path.parent.mkdir(parents=True, exist_ok=True)
    normalized = normalize_lf(text)
    if not normalized.endswith("\n"):
        normalized += "\n"

    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            newline="\n",
            dir=path.parent,
            prefix=f".{path.name}.tmp-",
            delete=False,
        ) as temporary:
            temporary.write(normalized)
            temporary_path = Path(temporary.name)
        os.replace(temporary_path, path)
    finally:
        if temporary_path and temporary_path.exists():
            temporary_path.unlink(missing_ok=True)


def backup_file(path: Path) -> Path:
    """Create a timestamped backup beside a non-secret config file."""

    if not path.is_file():
        raise FileNotFoundError(path)

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    backup = path.with_name(f"{path.name}.bak-{stamp}")
    suffix = 1
    while backup.exists():
        backup = path.with_name(f"{path.name}.bak-{stamp}-{suffix:02d}")
        suffix += 1
    shutil.copy2(path, backup)
    return backup


def _top_level_end(lines: list[str]) -> int:
    """Return the first TOML table line; keys before it are top-level keys."""

    for index, line in enumerate(lines):
        if re.match(r"^\s*\[", line):
            return index
    return len(lines)


def get_current_provider(text: str) -> str | None:
    """Read the top-level ``model_provider`` without interpreting nested tables."""

    lines = normalize_lf(text).split("\n")
    for line in lines[: _top_level_end(lines)]:
        match = re.match(r'^\s*model_provider\s*=\s*(["\'])([^"\']+)\1\s*$', line)
        if match:
            return match.group(2)
    return None


def set_top_level_provider(text: str, provider: str) -> str:
    """Set only the top-level provider selector and preserve all other lines."""

    if not provider:
        raise ValueError("provider cannot be empty")
    lines = normalize_lf(text).split("\n")
    end = _top_level_end(lines)
    field_pattern = re.compile(r"^(?P<indent>\s*)model_provider\s*=")
    for index in range(end):
        match = field_pattern.match(lines[index])
        if match:
            lines[index] = f'{match.group("indent")}model_provider = {json.dumps(provider)}'
            return "\n".join(lines)

    lines.insert(end, f'model_provider = {json.dumps(provider)}')
    return "\n".join(lines)


def _provider_block(lines: list[str], provider: str) -> tuple[int, int] | None:
    """Locate one provider table and the next table boundary."""

    header = re.compile(rf"^\s*\[model_providers\.{re.escape(provider)}\]\s*$")
    for start, line in enumerate(lines):
        if header.match(line):
            end = start + 1
            while end < len(lines) and not re.match(r"^\s*\[", lines[end]):
                end += 1
            return start, end
    return None


def provider_defined(text: str, provider: str) -> bool:
    """Return whether a custom provider table exists in the active config."""

    return _provider_block(normalize_lf(text).split("\n"), provider) is not None


def ensure_api_provider(text: str, settings: SwitcherSettings) -> str:
    """Ensure the API provider uses the configured environment variable."""

    settings.validate()
    lines = normalize_lf(text).split("\n")
    block = _provider_block(lines, settings.api_provider)
    values = (
        ("name", json.dumps(settings.api_display_name)),
        ("base_url", json.dumps(settings.api_base_url)),
        ("wire_api", json.dumps("responses")),
        ("env_key", json.dumps(settings.api_env_key)),
        ("requires_openai_auth", "false"),
        ("supports_websockets", "true"),
    )

    if block is None:
        if lines and lines[-1].strip():
            lines.append("")
        lines.append(f"[model_providers.{settings.api_provider}]")
        lines.extend(f"{field} = {value}" for field, value in values)
        return "\n".join(lines)

    start, end = block
    for field, value in values:
        field_pattern = re.compile(rf"^(?P<indent>\s*){re.escape(field)}\s*=")
        found = False
        for index in range(start + 1, end):
            match = field_pattern.match(lines[index])
            if match:
                lines[index] = f'{match.group("indent")}{field} = {value}'
                found = True
                break
        if not found:
            lines.insert(end, f"{field} = {value}")
            end += 1
    return "\n".join(lines)


def _dotenv_value(path: Path, key: str) -> str | None:
    """Read one dotenv value without logging or returning it to the UI."""

    if not path.is_file():
        return None
    assignment = re.compile(rf"^\s*(?:export\s+)?{re.escape(key)}\s*=\s*(.*?)\s*$")
    for line in read_utf8(path).splitlines():
        if not line or line.lstrip().startswith("#"):
            continue
        match = assignment.match(line)
        if not match:
            continue
        value = match.group(1).strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        return value or None
    return None


def write_dotenv_value(path: Path, key: str, value: str) -> None:
    """Update one key in ``.env`` without creating a backup containing secrets."""

    if not value or "\r" in value or "\n" in value:
        raise ValueError("API key cannot be empty or contain line breaks")
    existing = read_utf8(path) if path.is_file() else ""
    lines = normalize_lf(existing).split("\n")
    assignment = re.compile(rf"^\s*(?:export\s+)?{re.escape(key)}\s*=")
    replacement = f"{key}={value}"
    for index, line in enumerate(lines):
        if assignment.match(line):
            lines[index] = replacement
            write_utf8_lf_atomic(path, "\n".join(lines))
            return
    if lines == [""]:
        lines = [replacement]
    else:
        if lines and lines[-1] != "":
            lines.append("")
        lines.append(replacement)
    write_utf8_lf_atomic(path, "\n".join(lines))


def _registry_value(key: str, hive: Any, subkey: str) -> str | None:
    """Read a Windows environment value for detection only."""

    if os.name != "nt":
        return None
    try:
        import winreg

        with winreg.OpenKey(hive, subkey) as opened:
            value, _ = winreg.QueryValueEx(opened, key)
            return str(value) if value else None
    except (FileNotFoundError, OSError, ImportError):
        return None


def detect_api_key(paths: CodexPaths, env_key: str) -> ApiKeyStatus:
    """Detect a configured key source without exposing its value."""

    if os.environ.get(env_key, "").strip():
        return ApiKeyStatus(True, "process environment")
    if os.name == "nt":
        import winreg

        if _registry_value(env_key, winreg.HKEY_CURRENT_USER, "Environment"):
            return ApiKeyStatus(True, "user environment")
        if _registry_value(
            env_key,
            winreg.HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ):
            return ApiKeyStatus(True, "system environment")
    if _dotenv_value(paths.dotenv, env_key):
        return ApiKeyStatus(True, "Codex .env")
    return ApiKeyStatus(False, None)


def switch_provider(
    paths: CodexPaths,
    settings: SwitcherSettings,
    target: TargetMode,
    *,
    api_base_url: str | None = None,
    api_key: str | None = None,
    persist_api_key: bool = False,
) -> SwitchResult:
    """Switch provider, optionally applying user-entered API settings."""

    runtime_settings = replace(settings, api_base_url=api_base_url) if api_base_url is not None else settings
    runtime_settings.validate()
    if target not in ("subscription", "api"):
        raise ValueError(f"Unsupported target: {target}")
    if not paths.config.is_file():
        raise FileNotFoundError(f"Codex config not found: {paths.config}")

    before = read_utf8(paths.config)
    after = before
    if target == "api":
        after = ensure_api_provider(after, runtime_settings)
        provider = runtime_settings.api_provider
    else:
        provider = runtime_settings.subscription_provider
    after = set_top_level_provider(after, provider)

    changed = normalize_lf(after) != normalize_lf(before)
    backup = backup_file(paths.config) if changed else None
    if changed:
        write_utf8_lf_atomic(paths.config, after)

    if target == "api" and api_key and persist_api_key:
        write_dotenv_value(paths.dotenv, runtime_settings.api_env_key, api_key)

    api_key_status = detect_api_key(paths, runtime_settings.api_env_key)
    warnings: list[str] = []
    if target == "api" and api_key and not persist_api_key:
        warnings.append("本次输入的 API key 未保存；桌面 Codex 重启后不会自动读取它。")
    if target == "api" and not api_key_status.detected:
        warnings.append(
            f"未检测到 {runtime_settings.api_env_key}。请在 {paths.dotenv} 中配置后重启桌面 Codex。"
        )
    if target == "subscription" and not provider_defined(after, provider) and provider != "openai":
        warnings.append(f"活动配置中未找到自定义 provider [{provider}]，请确认订阅配置已存在。")

    return SwitchResult(
        target=target,
        provider=provider,
        changed=changed,
        backup_path=backup,
        api_key=api_key_status,
        warnings=tuple(warnings),
    )


def inspect_state(paths: CodexPaths, settings: SwitcherSettings) -> dict[str, object]:
    """Return redacted state suitable for a status panel or CLI output."""

    if not paths.config.is_file():
        raise FileNotFoundError(f"Codex config not found: {paths.config}")
    text = read_utf8(paths.config)
    key = detect_api_key(paths, settings.api_env_key)
    current = get_current_provider(text)
    return {
        "config_path": str(paths.config),
        "current_provider": current or "(未设置)",
        "subscription_provider": settings.subscription_provider,
        "api_provider": settings.api_provider,
        "api_key_detected": key.detected,
        "api_key_source": key.source or "未检测到",
        "subscription_profile": str(paths.subscription_profile),
        "subscription_profile_exists": paths.subscription_profile.is_file(),
    }
