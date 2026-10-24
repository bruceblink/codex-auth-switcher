"""Preserving TOML edits and non-secret switcher settings."""

from __future__ import annotations

import copy
import json
import os
import re
import tempfile
import tomllib
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from urllib.parse import urlsplit

import tomlkit

# Only these root settings follow a mode. Shared settings remain in the live file.
MODE_FIELDS = (
    "model_provider",
    "model",
    "model_reasoning_effort",
    "plan_mode_reasoning_effort",
    "model_reasoning_summary",
    "model_supports_reasoning_summaries",
    "model_context_window",
    "model_auto_compact_token_limit",
    "model_catalog_json",
    "disable_response_storage",
    "service_tier",
    "personality",
    "forced_login_method",
    "forced_chatgpt_workspace_id",
)


class SwitcherError(Exception):
    """An actionable error whose message never includes credential values."""


@dataclass(frozen=True)
class SwitcherSettings:
    subscription_provider: str = "openai"
    api_provider: str = "OpenAI"
    api_base_url: str = "https://vip.auto-code.net/v1"
    api_display_name: str = "AutoCode API key"
    api_env_key: str = "AUTOCODE_API_KEY"
    api_base_url_env_key: str = "AUTOCODE_BASE_URL"

    def validate(self) -> None:
        for value in (self.subscription_provider, self.api_provider):
            validate_provider(value)
        for value in (self.api_env_key, self.api_base_url_env_key):
            if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", value):
                raise SwitcherError("Invalid environment variable identifier.")
        validate_base_url(self.api_base_url)


@dataclass(frozen=True)
class CodexPaths:
    home: Path

    @classmethod
    def resolve(cls, codex_home: str | Path | None = None) -> CodexPaths:
        return cls(
            Path(codex_home or os.environ.get("CODEX_HOME") or Path.home() / ".codex")
            .expanduser()
            .resolve()
        )

    @property
    def config(self) -> Path:
        return self.home / "config.toml"

    @property
    def auth(self) -> Path:
        return self.home / "auth.json"

    @property
    def state_dir(self) -> Path:
        return self.home / "auth-switcher"


def validate_provider(value: str) -> None:
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", value):
        raise SwitcherError("Invalid provider identifier.")


def validate_base_url(value: str) -> None:
    try:
        url = urlsplit(value)
        valid = url.scheme in {"http", "https"} and url.hostname and url.port != 0
        valid = valid and not (
            url.username or url.password or url.query or url.fragment
        )
        valid = valid and not any(char.isspace() or ord(char) < 32 for char in value)
    except ValueError:
        valid = False
    if not valid:
        raise SwitcherError(
            "Base URL must be HTTP(S), with a host and without credentials, query or fragment."
        )


def load_settings(path: Path) -> SwitcherSettings:
    if not path.is_file():
        return SwitcherSettings()
    try:
        raw = json.loads(path.read_text(encoding="utf-8-sig"))
        if not isinstance(raw, dict):
            raise TypeError
        defaults = SwitcherSettings()
        values = {
            key: raw.get(key, getattr(defaults, key))
            for key in defaults.__dataclass_fields__
        }
        if not all(isinstance(value, str) for value in values.values()):
            raise ValueError
        settings = SwitcherSettings(**values)
        settings.validate()
        return settings
    except (ValueError, TypeError, UnicodeError):
        raise SwitcherError(
            "Invalid switcher settings; check provider identifiers and base URL."
        ) from None


def save_settings(path: Path, settings: SwitcherSettings) -> None:
    settings.validate()
    write_atomic(
        path, encode_text(json.dumps(settings.__dict__, indent=2, ensure_ascii=False))
    )


def write_atomic(path: Path, data: bytes) -> None:
    """Replace a file in its directory and remove temporary data on failure."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(
            dir=path.parent, prefix=f".{path.name}.tmp-", delete=False
        ) as stream:
            temporary = Path(stream.name)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def encode_text(text: str) -> bytes:
    return (text.replace("\r\n", "\n").replace("\r", "\n").rstrip("\n") + "\n").encode(
        "utf-8"
    )


def parse_config(data: bytes | str):
    """Hide parser context because invalid TOML may contain credentials."""
    try:
        text = data.decode("utf-8-sig") if isinstance(data, bytes) else data
        tomllib.loads(text)
        return tomlkit.parse(text)
    except (ValueError, TypeError, UnicodeError, tomlkit.exceptions.TOMLKitError):
        raise SwitcherError(
            "Invalid UTF-8/TOML configuration; no active files were changed."
        ) from None


def provider_name(document) -> str:
    value = document.get("model_provider", "openai")
    if not isinstance(value, str) or not value:
        raise SwitcherError("model_provider must be a non-empty string.")
    return str(value)


def provider_table(document):
    providers = document.get("model_providers", {})
    if not hasattr(providers, "get"):
        raise SwitcherError("model_providers must contain provider tables.")
    table = providers.get(provider_name(document), {})
    if not hasattr(table, "get"):
        raise SwitcherError("The selected provider must be a TOML table.")
    return table


def capture_mode(document) -> str:
    """Extract mode settings and its selected provider, excluding shared tables."""
    fragment = tomlkit.document()
    for key in MODE_FIELDS:
        if key in document:
            fragment[key] = copy.deepcopy(document[key])
    if "model_provider" not in fragment:
        fragment["model_provider"] = "openai"
    name = provider_name(document)
    if name in document.get("model_providers", {}):
        fragment["model_providers"] = tomlkit.table()
        fragment["model_providers"][name] = copy.deepcopy(
            document["model_providers"][name]
        )
    return tomlkit.dumps(fragment)


def apply_mode(current: bytes, fragment: str) -> bytes:
    document = parse_config(current)
    snapshot = parse_config(fragment)
    for key in MODE_FIELDS:
        if key in snapshot:
            document[key] = copy.deepcopy(snapshot[key])
        elif key in document:
            del document[key]
    name = provider_name(snapshot)
    if name in snapshot.get("model_providers", {}):
        if "model_providers" not in document:
            document["model_providers"] = tomlkit.table()
        document["model_providers"][name] = copy.deepcopy(
            snapshot["model_providers"][name]
        )
    elif name in document.get("model_providers", {}):
        del document["model_providers"][name]
    result = encode_text(tomlkit.dumps(document))
    parse_config(result)
    return result


def contains_credentials(value) -> bool:
    """Reject cleartext credentials before creating a configuration backup."""
    if hasattr(value, "items"):
        for key, child in value.items():
            name = str(key).lower()
            sensitive = re.search(
                r"password|passwd|secret|token|api[_-]?key|authorization|cookie|credential",
                name,
            )
            exempt = any(
                word in name
                for word in (
                    "limit",
                    "window",
                    "env_key",
                    "instructions",
                    "credentials_store",
                )
            )
            if sensitive and not exempt and isinstance(child, str) and child:
                return True
            if name == "base_url" and isinstance(child, str):
                validate_base_url(child)
            if contains_credentials(child):
                return True
    elif isinstance(value, list):
        return any(contains_credentials(child) for child in value)
    return False


def backup_config(path: Path, original: bytes) -> Path:
    """Reserve a unique backup without overwriting a same-second operation."""
    if contains_credentials(parse_config(original)):
        raise SwitcherError(
            "Configuration contains credentials. Move them to a credential store before switching."
        )
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    suffix = 0
    while True:
        ending = f"-{suffix:02d}" if suffix else ""
        backup = path.with_name(f"{path.name}.bak-{stamp}{ending}")
        try:
            with backup.open("xb") as stream:
                stream.write(original)
            return backup
        except FileExistsError:
            suffix += 1
