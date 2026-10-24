"""Isolated mode restoration and failure tests; all credentials are synthetic."""

from __future__ import annotations

import contextlib
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from codex_auth_switcher.app import main
from codex_auth_switcher.config import (
    CodexPaths,
    SwitcherError,
    SwitcherSettings,
    apply_mode,
    capture_mode,
    encode_text,
    parse_config,
    validate_base_url,
    write_atomic,
)
from codex_auth_switcher.credentials import WindowsProtector, api_auth, auth_kind
from codex_auth_switcher.modes import ModeStore, exclusive

SUBSCRIPTION = """# subscription defaults
model_provider = 'openai' # keep case
model = "test-subscription-model"
model_reasoning_effort = "max"
model_context_window = 375000
model_auto_compact_token_limit = 375000
service_tier = "default"
personality = "pragmatic"
[model_providers.OpenAI]
name = "OpenAI"
wire_api = "responses"
requires_openai_auth = true
[mcp_servers.legacy]
command = "old-command"
[shell_environment_policy.set]
UNRELATED_AUTH_TOKEN = "synthetic-legacy-value"
"""
API = """# current shared configuration
model_provider = "OpenAI"
model = "test-api-model"
model_reasoning_effort = "xhigh"
model_context_window = 372000
model_auto_compact_token_limit = 360000
disable_response_storage = true
[model_providers."OpenAI"] # quoted header
name = "OpenAI"
base_url = "https://gateway.example.invalid/v1" # keep endpoint
wire_api = "responses"
requires_openai_auth = true
supports_websockets = true
supports_standalone_web_search = true
[mcp_servers.current]
command = "keep-current-command"
[features]
multi_agent = true
[agents]
max_threads = 4
[projects."F:/project/example"]
trust_level = "trusted"
"""
SYNTHETIC_AUTH = encode_text(
    json.dumps(
        {
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": None,
            "last_refresh": "test-time",
            "tokens": {
                key: "synthetic-" + key
                for key in ("access_token", "refresh_token", "id_token", "account_id")
            },
        }
    )
)
SYNTHETIC_KEY = "synthetic-api-value-for-tests"


@unittest.skipUnless(os.name == "nt", "Windows DPAPI is required")
class ModeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="codex-switcher-test-")
        self.addCleanup(self.temporary.cleanup)
        self.paths = CodexPaths(Path(self.temporary.name))
        write_atomic(self.paths.config, encode_text(API))
        write_atomic(self.paths.auth, api_auth(SYNTHETIC_KEY))
        self.subscription_config = self.paths.home / "config.toml.bak-20261024-170412"
        write_atomic(self.subscription_config, encode_text(SUBSCRIPTION))
        self.subscription_auth = self.paths.home / "auth.json.bak"
        write_atomic(self.subscription_auth, SYNTHETIC_AUTH)
        self.store = ModeStore(self.paths, SwitcherSettings())
        self.environment = patch(
            "codex_auth_switcher.modes.environment_key", return_value=None
        )
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def active(self):
        return self.paths.config.read_bytes(), self.paths.auth.read_bytes()

    def test_initialize_uses_current_api_and_matching_subscription_backup(self):
        before = self.active()
        result = self.store.initialize()
        self.assertEqual(before, self.active())
        self.assertTrue(all(p["ready"] for p in result["profiles"].values()))
        self.assertEqual(
            result["profiles"]["subscription"]["config_source"],
            self.subscription_config.name,
        )
        self.assertEqual(result["profiles"]["api"]["provider"], "OpenAI")
        self.assertNotIn(SYNTHETIC_KEY.encode(), self.store.vault.read_bytes())
        self.assertNotIn(b"synthetic-refresh_token", self.store.vault.read_bytes())
        decrypted = self.store.protector.unprotect(self.store.vault.read_bytes())
        self.assertNotIn(b"synthetic-legacy-value", decrypted)

    def test_round_trip_restores_model_auth_and_preserves_shared_settings(self):
        self.store.initialize()
        result = self.store.switch("subscription")
        self.assertTrue(result["changed"])
        subscription = parse_config(self.paths.config.read_bytes())
        self.assertEqual(subscription["model"], "test-subscription-model")
        self.assertEqual(subscription["model_reasoning_effort"], "max")
        self.assertEqual(subscription["model_context_window"], 375000)
        self.assertNotIn("disable_response_storage", subscription)
        self.assertEqual(self.paths.auth.read_bytes(), SYNTHETIC_AUTH)
        for key in ("mcp_servers", "features", "agents", "projects"):
            self.assertEqual(
                subscription[key].unwrap(), parse_config(API)[key].unwrap()
            )
        self.assertNotIn("shell_environment_policy", subscription)
        result = self.store.switch("api")
        restored = parse_config(self.paths.config.read_bytes())
        self.assertEqual(restored["model"], "test-api-model")
        self.assertEqual(restored["model_reasoning_effort"], "xhigh")
        self.assertEqual(restored["model_context_window"], 372000)
        self.assertNotIn("service_tier", restored)
        self.assertNotIn("personality", restored)
        self.assertEqual(
            restored["model_providers"]["OpenAI"].unwrap(),
            parse_config(API)["model_providers"]["OpenAI"].unwrap(),
        )
        self.assertEqual(auth_kind(self.paths.auth.read_bytes()), "api")
        self.assertEqual(self.subscription_auth.read_bytes(), SYNTHETIC_AUTH)
        self.assertNotIn(b"\r", self.paths.config.read_bytes())

    def test_same_mode_is_noop_and_does_not_backup(self):
        self.store.initialize()
        before = self.active()
        result = self.store.switch("api", provider="OpenAI")
        self.assertFalse(result["changed"])
        self.assertIsNone(result["backup"])
        self.assertEqual(before, self.active())

    def test_leaving_mode_captures_user_edits_and_refreshed_tokens(self):
        self.store.initialize()
        self.store.switch("subscription")
        document = parse_config(self.paths.config.read_bytes())
        document["model_reasoning_effort"] = "high"
        import tomlkit

        write_atomic(self.paths.config, encode_text(tomlkit.dumps(document)))
        refreshed = json.loads(SYNTHETIC_AUTH)
        refreshed["tokens"]["refresh_token"] = "synthetic-refreshed-token"
        write_atomic(self.paths.auth, encode_text(json.dumps(refreshed)))
        self.store.switch("api")
        self.store.switch("subscription")
        self.assertEqual(
            parse_config(self.paths.config.read_bytes())["model_reasoning_effort"],
            "high",
        )
        self.assertEqual(json.loads(self.paths.auth.read_bytes()), refreshed)

    def test_missing_subscription_auth_does_not_modify_active_files(self):
        self.subscription_auth.unlink()
        self.store.initialize()
        before = self.active()
        with self.assertRaisesRegex(SwitcherError, "authentication is missing"):
            self.store.switch("subscription")
        self.assertEqual(before, self.active())

    def test_wrong_auth_source_rejected_without_active_changes(self):
        before = self.active()
        with self.assertRaisesRegex(SwitcherError, "different mode"):
            self.store.initialize({"subscription_auth": self.paths.auth})
        self.assertEqual(before, self.active())

    def test_input_overrides_environment_and_is_not_written_to_config(self):
        self.store.initialize()
        self.store.switch("subscription")
        with patch(
            "codex_auth_switcher.modes.environment_key",
            return_value="synthetic-env-key",
        ):
            self.store.switch(
                "api",
                api_key="synthetic-user-key",
                api_base_url="https://input.example.invalid/v1",
                provider="custom.api",
            )
        document = parse_config(self.paths.config.read_bytes())
        self.assertEqual(document["model_provider"], "custom.api")
        self.assertEqual(
            document["model_providers"]["custom.api"]["base_url"],
            "https://input.example.invalid/v1",
        )
        self.assertEqual(
            json.loads(self.paths.auth.read_bytes())["OPENAI_API_KEY"],
            "synthetic-user-key",
        )
        self.assertNotIn(b"synthetic-user-key", self.paths.config.read_bytes())
        self.assertFalse((self.paths.home / ".env").exists())

    def test_environment_overrides_saved_api_auth(self):
        self.store.initialize()
        with patch(
            "codex_auth_switcher.modes.environment_key",
            return_value="synthetic-env-key",
        ):
            self.store.switch("api")
        self.assertEqual(
            json.loads(self.paths.auth.read_bytes())["OPENAI_API_KEY"],
            "synthetic-env-key",
        )

    def test_no_profile_file_required_when_starting_in_subscription(self):
        document = parse_config(SUBSCRIPTION)
        del document["shell_environment_policy"]
        import tomlkit

        write_atomic(self.paths.config, encode_text(tomlkit.dumps(document)))
        write_atomic(self.paths.auth, SYNTHETIC_AUTH)
        state = self.store.initialize()
        self.assertEqual(state["profiles"]["api"]["config_source"], "settings defaults")
        self.store.switch("api", api_key="synthetic-input-key")
        self.assertEqual(
            self.store.inspect()["base_url"], "https://vip.auto-code.net/v1"
        )

    def test_invalid_toml_does_not_expose_source_or_create_vault(self):
        write_atomic(self.paths.config, b'api_key = "synthetic-confidential-value\n')
        with self.assertRaises(SwitcherError) as result:
            self.store.initialize()
        self.assertNotIn("synthetic-confidential", str(result.exception))
        self.assertFalse(self.store.vault.exists())

    def test_config_write_failure_restores_auth_and_config(self):
        self.store.initialize()
        before = self.active()
        original_write = write_atomic

        def fail_config(path, data):
            if path == self.paths.config:
                raise OSError("synthetic-write-failure")
            original_write(path, data)

        with (
            patch("codex_auth_switcher.modes.write_atomic", side_effect=fail_config),
            self.assertRaisesRegex(SwitcherError, "were restored"),
        ):
            self.store.switch("subscription")
        self.assertEqual(before, self.active())
        self.assertFalse(self.store.journal.exists())

    def test_crash_record_is_encrypted_and_recovered_on_next_initialize(self):
        self.store.initialize()
        before = self.active()
        original_write = write_atomic

        def interrupt_config(path, data):
            if path == self.paths.config:
                raise KeyboardInterrupt
            original_write(path, data)

        with (
            patch(
                "codex_auth_switcher.modes.write_atomic", side_effect=interrupt_config
            ),
            self.assertRaises(KeyboardInterrupt),
        ):
            self.store.switch("subscription")
        self.assertNotEqual(before, self.active())
        self.assertNotIn(b"synthetic-refresh_token", self.store.journal.read_bytes())
        self.store.initialize()
        self.assertEqual(before, self.active())
        self.assertFalse(self.store.journal.exists())

    def test_external_write_is_not_overwritten_during_preparation(self):
        self.store.initialize()
        original_check = self.store._check_model

        def external_write(document):
            original_check(document)
            write_atomic(self.paths.config, encode_text(API + "\n# external update\n"))

        with (
            patch.object(self.store, "_check_model", side_effect=external_write),
            self.assertRaisesRegex(SwitcherError, "changed during preparation"),
        ):
            self.store.switch("subscription")
        self.assertIn(b"external update", self.paths.config.read_bytes())
        self.assertEqual(auth_kind(self.paths.auth.read_bytes()), "api")

    def test_secret_in_live_config_blocks_plaintext_backup(self):
        write_atomic(
            self.paths.config,
            encode_text(API + '\n[extra]\napi_key = "synthetic-other-secret"\n'),
        )
        self.store.initialize()
        before = self.active()
        with self.assertRaisesRegex(SwitcherError, "contains credentials"):
            self.store.switch("subscription")
        self.assertEqual(before, self.active())

    def test_second_process_lock_is_rejected(self):
        with (
            exclusive(self.paths),
            self.assertRaisesRegex(SwitcherError, "Another switcher"),
        ):
            self.store.initialize()

    def test_model_effort_is_checked_before_writing(self):
        self.store.initialize()
        catalog = {
            "models": [
                {
                    "slug": "test-subscription-model",
                    "supported_reasoning_levels": [{"effort": "high"}],
                }
            ]
        }
        write_atomic(
            self.paths.home / "models_cache.json", encode_text(json.dumps(catalog))
        )
        before = self.active()
        with self.assertRaisesRegex(SwitcherError, "not supported"):
            self.store.switch("subscription")
        self.assertEqual(before, self.active())

    def test_cli_status_and_api_switch_are_redacted(self):
        self.store.initialize()
        stdout, stderr = io.StringIO(), io.StringIO()
        with (
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
            patch("sys.stdin", io.StringIO("synthetic-stdin-key")),
        ):
            code = main(
                [
                    "--codex-home",
                    str(self.paths.home),
                    "--target",
                    "api",
                    "--api-key-stdin",
                ]
            )
        self.assertEqual(code, 0, stderr.getvalue())
        self.assertNotIn("synthetic-stdin-key", stdout.getvalue() + stderr.getvalue())
        self.assertEqual(
            json.loads(self.paths.auth.read_bytes())["OPENAI_API_KEY"],
            "synthetic-stdin-key",
        )


class ParsingTests(unittest.TestCase):
    def test_subscription_without_override_removes_stale_openai_table(self):
        current = (
            API
            + '\n[model_providers.openai]\nbase_url = "https://stale.example.invalid/v1"\n'
        )
        restored = parse_config(
            apply_mode(encode_text(current), capture_mode(parse_config(SUBSCRIPTION)))
        )
        self.assertNotIn("openai", restored["model_providers"])
        self.assertIn("OpenAI", restored["model_providers"])

    def test_quoted_and_dotted_provider_table_is_preserved(self):
        source = API.replace(
            'model_provider = "OpenAI"', 'model_provider = "custom.api"'
        ).replace('[model_providers."OpenAI"]', '[model_providers."custom.api"]')
        fragment = capture_mode(parse_config(source))
        restored = apply_mode(encode_text(API), fragment)
        self.assertEqual(
            parse_config(restored)["model_providers"]["custom.api"]["base_url"],
            "https://gateway.example.invalid/v1",
        )
        self.assertIn(b"# keep endpoint", restored)

    def test_unsafe_urls_are_rejected(self):
        for value in (
            "https://user:pass@example.invalid/v1",
            "https://example.invalid?key=synthetic",
            "https:///v1",
            "ftp://example.invalid",
            "https://example.invalid/\n",
        ):
            with self.subTest(value=value), self.assertRaises(SwitcherError):
                validate_base_url(value)

    def test_invalid_auth_and_key_errors_are_redacted(self):
        for action in (
            lambda: auth_kind(b'{"OPENAI_API_KEY":"synthetic-secret'),
            lambda: api_auth("synthetic\nkey"),
        ):
            with self.assertRaises(SwitcherError) as result:
                action()
            self.assertNotIn("synthetic", str(result.exception))

    @unittest.skipUnless(os.name == "nt", "Windows DPAPI is required")
    def test_dpapi_roundtrip_and_tamper_rejection(self):
        protector = WindowsProtector()
        original = b"synthetic-private-test-data"
        ciphertext = protector.protect(original)
        self.assertNotIn(original, ciphertext)
        self.assertEqual(protector.unprotect(ciphertext), original)
        with self.assertRaises(SwitcherError):
            protector.unprotect(ciphertext[:-10] + b"bad")


if __name__ == "__main__":
    unittest.main()
