"""Mode snapshots, encrypted recovery records, and coordinated file updates."""

from __future__ import annotations

import base64
import copy
import json
import os
from contextlib import contextmanager
from pathlib import Path

import tomlkit

from .config import (
    CodexPaths,
    SwitcherError,
    SwitcherSettings,
    apply_mode,
    backup_config,
    capture_mode,
    contains_credentials,
    encode_text,
    parse_config,
    provider_name,
    provider_table,
    validate_base_url,
    validate_provider,
    write_atomic,
)
from .credentials import WindowsProtector, api_auth, auth_kind, environment_key


def optional_bytes(path: Path) -> bytes | None:
    return path.read_bytes() if path.is_file() else None


def packed(data: bytes | None) -> str | None:
    return base64.b64encode(data).decode("ascii") if data is not None else None


def unpacked(value: str | None) -> bytes | None:
    return base64.b64decode(value, validate=True) if value is not None else None


@contextmanager
def exclusive(paths: CodexPaths):
    """Serialize switcher processes; external Codex writes are checked separately."""
    paths.state_dir.mkdir(parents=True, exist_ok=True)
    with (paths.state_dir / "operation.lock").open("a+b") as stream:
        if stream.tell() == 0:
            stream.write(b"0")
        stream.flush()
        stream.seek(0)
        try:
            if os.name == "nt":
                import msvcrt

                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            raise SwitcherError(
                "Another switcher operation is running. Try again after it finishes."
            ) from None
        try:
            yield
        finally:
            if os.name == "nt":
                stream.seek(0)
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(stream, fcntl.LOCK_UN)


class ModeStore:
    """Keep one encrypted vault and an encrypted, crash-recoverable write record.

    The vault contains only mode fields, the selected provider, and authentication.
    Active config/auth writes are individually atomic, not jointly atomic. Before
    either write, a DPAPI record stores both previous versions for recovery. A
    later operation refuses recovery if an external writer changed either file.
    """

    def __init__(self, paths: CodexPaths, settings: SwitcherSettings, protector=None):
        self.paths = paths
        self.settings = settings
        self.protector = protector or WindowsProtector()
        self.vault = paths.state_dir / "modes.dpapi"
        self.journal = paths.state_dir / "pending.dpapi"

    def _load(self, path: Path) -> dict:
        if not path.is_file():
            return {}
        try:
            data = json.loads(self.protector.unprotect(path.read_bytes()))
            if not isinstance(data, dict) or data.get("version") != 1:
                raise ValueError
            return data
        except (SwitcherError, OSError, ValueError, TypeError):
            raise SwitcherError(
                "Cannot read encrypted mode state for this Windows user. Active files were not changed."
            ) from None

    def _save(self, path: Path, value: dict) -> None:
        payload = json.dumps({**value, "version": 1}, ensure_ascii=True).encode("utf-8")
        write_atomic(path, self.protector.protect(payload))

    def _active(self) -> tuple[bytes, bytes | None]:
        if not self.paths.config.is_file():
            raise SwitcherError(
                "Codex config.toml was not found. Choose the correct Codex home."
            )
        return self.paths.config.read_bytes(), optional_bytes(self.paths.auth)

    @staticmethod
    def _mode(config: bytes) -> str:
        return (
            "subscription" if provider_name(parse_config(config)) == "openai" else "api"
        )

    def _active_mode(self, config: bytes, auth: bytes | None) -> str:
        document = parse_config(config)
        if provider_table(document).get("env_key"):
            return "api"
        return auth_kind(auth) if auth_kind(auth) != "missing" else self._mode(config)

    def _check_profile(self, mode: str, profile: dict) -> tuple[bytes, bytes]:
        try:
            fragment = profile["config"]
            document = parse_config(fragment)
            auth = unpacked(profile.get("auth"))
        except (KeyError, TypeError, ValueError):
            raise SwitcherError(
                "Invalid saved mode state. Reinitialize from valid sources."
            ) from None
        validate_provider(provider_name(document))
        if contains_credentials(document):
            raise SwitcherError(
                "A provider contains cleartext credentials. Use auth.json for the API key."
            )
        if auth_kind(auth) != mode:
            raise SwitcherError(
                "Target authentication is missing. Initialize from a matching auth file or sign in with Codex first."
            )
        storage = document.get("cli_auth_credentials_store", "file")
        if storage != "file":
            raise SwitcherError("Only file-based Codex authentication is supported.")
        forced = document.get("forced_login_method")
        if forced and forced != ("chatgpt" if mode == "subscription" else "api"):
            raise SwitcherError("forced_login_method conflicts with the target mode.")
        table = provider_table(document)
        if mode == "subscription":
            if table.get("env_key") or table.get("requires_openai_auth") is False:
                raise SwitcherError(
                    "The subscription provider has conflicting API authentication overrides."
                )
            if table.get("base_url"):
                validate_base_url(str(table["base_url"]))
        else:
            if provider_name(document) in {"openai", "ollama", "lmstudio"}:
                raise SwitcherError(
                    "The API gateway requires a custom provider ID. Use OpenAI or another custom identifier."
                )
            if (
                not table
                or table.get("requires_openai_auth") is not True
                or table.get("env_key")
            ):
                raise SwitcherError(
                    "This API profile uses environment authentication. Import an auth.json-based API configuration first."
                )
            validate_base_url(str(table.get("base_url", "")))
        return encode_text(fragment), auth

    def _check_model(self, document) -> None:
        """Validate restored model/effort when a local catalog is available."""
        model = document.get("model")
        effort = document.get("model_reasoning_effort")
        if not model:
            return
        explicit = document.get("model_catalog_json")
        catalogs = (
            [Path(str(explicit)).expanduser()]
            if explicit
            else [
                self.paths.home / "models_cache.json",
                self.paths.home / "models-codex.json",
                self.paths.home / "models.json",
            ]
        )
        for path in catalogs:
            if not path.is_absolute():
                path = self.paths.home / path
            if not path.is_file():
                if explicit:
                    raise SwitcherError("The target mode's model catalog is missing.")
                continue
            try:
                raw = json.loads(path.read_text(encoding="utf-8-sig"))
                entries = raw.get("models", []) if isinstance(raw, dict) else raw
                match = next(
                    (
                        item
                        for item in entries
                        if item.get("slug", item.get("id")) == model
                    ),
                    None,
                )
            except (ValueError, TypeError, AttributeError):
                raise SwitcherError("The local model catalog is invalid.") from None
            if match is None:
                if explicit:
                    raise SwitcherError(
                        "The target model is absent from its model catalog."
                    )
                continue
            levels = match.get("supported_reasoning_levels", [])
            supported = {
                item.get("effort") if isinstance(item, dict) else item
                for item in levels
            }
            if effort and supported and effort not in supported:
                raise SwitcherError(
                    "The saved reasoning level is not supported by the target model."
                )
            return

    def _recover(self) -> None:
        if not self.journal.exists():
            return
        record = self._load(self.journal)
        before_config, before_auth = (
            unpacked(record["config_before"]),
            unpacked(record["auth_before"]),
        )
        after_config, after_auth = (
            unpacked(record["config_after"]),
            unpacked(record["auth_after"]),
        )
        config, auth = self._active()
        if config not in (before_config, after_config) or auth not in (
            before_auth,
            after_auth,
        ):
            raise SwitcherError(
                "An external change blocks recovery. Keep pending.dpapi and resolve the active files before retrying."
            )
        if config != before_config:
            write_atomic(self.paths.config, before_config)
        if auth != before_auth:
            if before_auth is None:
                self.paths.auth.unlink(missing_ok=True)
            else:
                write_atomic(self.paths.auth, before_auth)
        self.journal.unlink()

    def initialize(self, sources: dict[str, Path] | None = None) -> dict:
        """Import existing modes without changing the active config or auth files."""
        with exclusive(self.paths):
            self._recover()
            state = self._load(self.vault)
            profiles = state.setdefault("profiles", {})
            config, auth = self._active()
            active_document = parse_config(config)
            if active_document.get("cli_auth_credentials_store", "file") != "file":
                raise SwitcherError(
                    "Only file-based Codex authentication is supported."
                )
            source_map = sources or {}
            active_mode = self._active_mode(config, auth)
            config_files = [self.paths.config] + sorted(
                self.paths.home.glob("config.toml.bak-*"),
                key=lambda p: p.stat().st_mtime,
                reverse=True,
            )
            auth_files = [self.paths.auth] + sorted(
                self.paths.home.glob("auth.json.bak*"),
                key=lambda p: p.stat().st_mtime,
                reverse=True,
            )
            for mode in ("subscription", "api"):
                profile = copy.deepcopy(profiles.get(mode, {}))
                requested_config = source_map.get(f"{mode}_config")
                requested_auth = source_map.get(f"{mode}_auth")
                if requested_config or not profile.get("config"):
                    for path in (
                        [requested_config] if requested_config else config_files
                    ):
                        try:
                            candidate = path.read_bytes()
                            candidate_mode = (
                                active_mode
                                if path == self.paths.config
                                else self._mode(candidate)
                            )
                            if not requested_config and candidate_mode != mode:
                                continue
                            fragment = capture_mode(parse_config(candidate))
                            if contains_credentials(parse_config(fragment)):
                                raise SwitcherError(
                                    "The selected provider contains credentials in TOML."
                                )
                        except (OSError, SwitcherError):
                            if requested_config or path == self.paths.config:
                                raise
                            continue
                        profile.update(config=fragment, config_source=path.name)
                        break
                if requested_auth or not profile.get("auth"):
                    for path in [requested_auth] if requested_auth else auth_files:
                        try:
                            candidate = optional_bytes(path)
                            if auth_kind(candidate) != mode:
                                if requested_auth:
                                    raise SwitcherError(
                                        "The specified auth source belongs to a different mode."
                                    )
                                continue
                        except (OSError, SwitcherError):
                            if requested_auth:
                                raise
                            continue
                        profile.update(auth=packed(candidate), auth_source=path.name)
                        break
                if mode == "api" and profile.get("config") and not profile.get("auth"):
                    template = parse_config(profile["config"])
                    key_name = str(
                        provider_table(template).get("env_key")
                        or self.settings.api_env_key
                    )
                    key = environment_key(key_name, self.paths.home)
                    if key:
                        profile.update(
                            auth=packed(api_auth(key)),
                            auth_source="environment / Codex .env",
                        )
                        if provider_table(template).get("env_key"):
                            del provider_table(template)["env_key"]
                            provider_table(template)["requires_openai_auth"] = True
                            profile["config"] = tomlkit.dumps(template)
                if mode == "api" and not profile.get("config"):
                    template = parse_config(capture_mode(active_document))
                    template["model_provider"] = self.settings.api_provider
                    if "model_providers" in template:
                        del template["model_providers"]
                    template["model_providers"] = tomlkit.table()
                    template["model_providers"][self.settings.api_provider] = {
                        "name": self.settings.api_display_name,
                        "base_url": os.environ.get(self.settings.api_base_url_env_key)
                        or self.settings.api_base_url,
                        "wire_api": "responses",
                        "requires_openai_auth": True,
                    }
                    if "forced_login_method" in template:
                        template["forced_login_method"] = "api"
                    profile.update(
                        config=tomlkit.dumps(template),
                        config_source="settings defaults",
                    )
                if profile:
                    profiles[mode] = profile
            self._save(self.vault, state)
        return self.inspect()

    def inspect(self) -> dict:
        """Return only non-secret summaries. No status action writes any files."""
        config, auth = self._active()
        document = parse_config(config)
        state = self._load(self.vault)
        mode = self._active_mode(config, auth)
        kind = auth_kind(auth)
        profiles = {}
        for name, profile in state.get("profiles", {}).items():
            snapshot = parse_config(profile.get("config", ""))
            try:
                self._check_profile(name, profile)
                ready, issue = True, ""
            except SwitcherError as exc:
                ready, issue = False, str(exc)
            profiles[name] = {
                "provider": provider_name(snapshot),
                "model": str(snapshot.get("model", "(default)")),
                "reasoning_effort": str(
                    snapshot.get("model_reasoning_effort", "(default)")
                ),
                "ready": ready,
                "issue": issue,
                "config_source": profile.get("config_source", "saved mode"),
                "auth_source": profile.get("auth_source", "saved mode"),
            }
        table = provider_table(document)
        base_url = str(table.get("base_url", ""))
        if base_url:
            validate_base_url(base_url)
        return {
            "config_path": str(self.paths.config),
            "mode": mode,
            "auth_mode": kind,
            "consistent": mode == kind,
            "current_provider": provider_name(document),
            "model": str(document.get("model", "(default)")),
            "reasoning_effort": str(
                document.get("model_reasoning_effort", "(default)")
            ),
            "base_url": base_url,
            "initialized": bool(state),
            "recovery_pending": self.journal.exists(),
            "profiles": profiles,
            "existing_threads": "Existing chats may retain their original provider. Restart Codex and use a new chat after switching.",
        }

    def switch(
        self,
        target: str,
        *,
        provider: str | None = None,
        api_key: str | None = None,
        api_base_url: str | None = None,
    ) -> dict:
        """Capture the leaving mode, then restore config/auth with failure recovery."""
        if target not in {"subscription", "api"}:
            raise SwitcherError("Unsupported target mode.")
        if target != "api" and (api_key is not None or api_base_url is not None):
            raise SwitcherError("API input options require API mode.")
        with exclusive(self.paths):
            self._recover()
            state = self._load(self.vault)
            if not state:
                raise SwitcherError("Initialize both modes before switching.")
            before_config, before_auth = self._active()
            current = parse_config(before_config)
            if current.get("cli_auth_credentials_store", "file") != "file":
                raise SwitcherError(
                    "Only file-based Codex authentication is supported."
                )
            leaving = self._active_mode(before_config, before_auth)
            if auth_kind(before_auth) != leaving:
                raise SwitcherError(
                    "Active provider and authentication do not match. Initialize valid sources before switching."
                )
            profiles = state["profiles"]
            profiles[leaving] = {
                "config": capture_mode(current),
                "auth": packed(before_auth),
                "config_source": "latest active config",
                "auth_source": "latest active auth",
            }
            if target not in profiles or not profiles[target].get("config"):
                raise SwitcherError(
                    "Target configuration is missing. Initialize from a matching configuration backup."
                )
            destination = profiles[target]
            fragment = parse_config(destination["config"])
            if provider:
                validate_provider(provider)
                if target == "api" and provider in {"openai", "ollama", "lmstudio"}:
                    raise SwitcherError(
                        "Use a custom provider identifier for the API gateway."
                    )
                old = provider_name(fragment)
                if provider != old:
                    table = copy.deepcopy(provider_table(fragment))
                    fragment["model_provider"] = provider
                    if "model_providers" in fragment:
                        del fragment["model_providers"]
                    if table:
                        fragment["model_providers"] = tomlkit.table()
                        fragment["model_providers"][provider] = table
            if target == "api":
                key_name = str(
                    provider_table(fragment).get("env_key") or self.settings.api_env_key
                )
                if api_key is None:
                    api_key = environment_key(key_name, self.paths.home)
                if provider_table(fragment).get("env_key"):
                    api_key = api_key or environment_key(key_name, self.paths.home)
                    if not api_key:
                        raise SwitcherError(
                            "The API environment key is missing. Enter a key before switching."
                        )
                    del provider_table(fragment)["env_key"]
                    provider_table(fragment)["requires_openai_auth"] = True
            destination["config"] = tomlkit.dumps(fragment)
            if api_key is not None:
                destination["auth"] = packed(api_auth(api_key))
                destination["auth_source"] = "user input"
            if api_base_url is not None:
                validate_base_url(api_base_url)
                fragment = parse_config(destination["config"])
                provider_table(fragment)["base_url"] = api_base_url
                destination["config"] = tomlkit.dumps(fragment)
            fragment, after_auth = self._check_profile(target, destination)
            same_config = (
                target == leaving
                and parse_config(destination["config"]).unwrap()
                == parse_config(capture_mode(current)).unwrap()
            )
            after_config = (
                before_config
                if same_config
                else apply_mode(before_config, fragment.decode("utf-8"))
            )
            self._check_model(parse_config(after_config))
            changed = (before_config, before_auth) != (after_config, after_auth)
            if self._active() != (before_config, before_auth):
                raise SwitcherError(
                    "Codex files changed during preparation. Refresh and retry."
                )
            backup = (
                backup_config(self.paths.config, before_config)
                if before_config != after_config
                else None
            )
            self._save(self.vault, state)
            if changed:
                self._save(
                    self.journal,
                    {
                        "config_before": packed(before_config),
                        "auth_before": packed(before_auth),
                        "config_after": packed(after_config),
                        "auth_after": packed(after_auth),
                    },
                )
                try:
                    if self._active() != (before_config, before_auth):
                        raise SwitcherError("Codex files changed before writing.")
                    if before_auth != after_auth:
                        write_atomic(self.paths.auth, after_auth)
                    if (
                        optional_bytes(self.paths.auth) != after_auth
                        or self.paths.config.read_bytes() != before_config
                    ):
                        raise SwitcherError(
                            "An external change interrupted the switch."
                        )
                    if before_config != after_config:
                        write_atomic(self.paths.config, after_config)
                    if self._active() != (after_config, after_auth):
                        raise SwitcherError(
                            "An external change interrupted verification."
                        )
                    self.journal.unlink()
                except (SwitcherError, OSError, ValueError, TypeError):
                    try:
                        self._recover()
                    except (SwitcherError, OSError, ValueError, TypeError, KeyError):
                        raise SwitcherError(
                            "Switch incomplete; encrypted recovery is pending. Close Codex and retry initialization."
                        ) from None
                    raise SwitcherError(
                        "Switch failed; previous configuration and authentication were restored."
                    ) from None
        return {
            "target": target,
            "changed": changed,
            "backup": str(backup) if backup else None,
            "restart_required": changed,
        }
