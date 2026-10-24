"""Authentication validation and current-Windows-user DPAPI encryption."""

from __future__ import annotations

import ctypes
import json
import os
from ctypes import wintypes
from pathlib import Path

from .config import SwitcherError


def read_auth(data: bytes | None) -> dict:
    if data is None:
        return {}
    try:
        value = json.loads(data.decode("utf-8-sig"))
        if not isinstance(value, dict):
            raise TypeError
        return value
    except (ValueError, TypeError, UnicodeError):
        raise SwitcherError(
            "Invalid authentication JSON; credential values are hidden."
        ) from None


def auth_kind(data: bytes | None) -> str:
    value = read_auth(data)
    tokens = value.get("tokens")
    chatgpt = isinstance(tokens, dict) and all(
        isinstance(tokens.get(key), str) and tokens[key]
        for key in ("access_token", "refresh_token", "id_token", "account_id")
    )
    api = isinstance(value.get("OPENAI_API_KEY"), str) and bool(
        value["OPENAI_API_KEY"].strip()
    )
    if chatgpt and api:
        raise SwitcherError(
            "Authentication contains both ChatGPT tokens and an API key. Select a valid source first."
        )
    if chatgpt and value.get("auth_mode", "chatgpt") == "chatgpt":
        return "subscription"
    if api and value.get("auth_mode", "api") in {"api", "apikey"}:
        return "api"
    return "missing"


def api_auth(key: str) -> bytes:
    if not key.strip() or any(ord(char) < 33 or ord(char) == 127 for char in key):
        raise SwitcherError(
            "API key cannot be empty or contain whitespace/control characters."
        )
    return (json.dumps({"OPENAI_API_KEY": key}, indent=2) + "\n").encode("utf-8")


def environment_key(name: str, home: Path) -> str | None:
    """Read an optional key without logging it; saved auth is handled separately."""
    value = os.environ.get(name, "").strip()
    if value:
        return value
    if os.name == "nt":
        import winreg

        for hive, subkey in (
            (winreg.HKEY_CURRENT_USER, "Environment"),
            (
                winreg.HKEY_LOCAL_MACHINE,
                r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
            ),
        ):
            try:
                with winreg.OpenKey(hive, subkey) as opened:
                    value, _ = winreg.QueryValueEx(opened, name)
                    if isinstance(value, str) and value.strip():
                        return value.strip()
            except OSError:
                continue
    path = home / ".env"
    if path.is_file():
        # Keys are opaque strings: do not evaluate expansions or execute dotenv.
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            assignment, separator, value = line.removeprefix("export ").partition("=")
            if separator and assignment.strip() == name:
                value = value.strip()
                if len(value) > 1 and value[0] == value[-1] and value[0] in {"'", '"'}:
                    value = value[1:-1]
                return value or None
    return None


class _Blob(ctypes.Structure):
    _fields_ = [("size", wintypes.DWORD), ("data", ctypes.POINTER(ctypes.c_ubyte))]


class WindowsProtector:
    """DPAPI binds ciphertext to the Windows user; no plaintext fallback exists."""

    def _crypt(self, data: bytes, *, decrypt: bool) -> bytes:
        if os.name != "nt":
            raise SwitcherError("Credential storage requires Windows DPAPI.")
        source_buffer = ctypes.create_string_buffer(data)
        source = _Blob(
            len(data), ctypes.cast(source_buffer, ctypes.POINTER(ctypes.c_ubyte))
        )
        destination = _Blob()
        library = ctypes.WinDLL("crypt32", use_last_error=True)
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.LocalFree.argtypes = [ctypes.c_void_p]
        kernel.LocalFree.restype = ctypes.c_void_p
        operation = library.CryptUnprotectData if decrypt else library.CryptProtectData
        operation.argtypes = [
            ctypes.POINTER(_Blob),
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_void_p,
            wintypes.DWORD,
            ctypes.POINTER(_Blob),
        ]
        operation.restype = wintypes.BOOL
        try:
            if not operation(
                ctypes.byref(source),
                None,
                None,
                None,
                None,
                1,
                ctypes.byref(destination),
            ):
                raise SwitcherError(
                    "Windows credential encryption/decryption failed for this user."
                )
            return ctypes.string_at(destination.data, destination.size)
        finally:
            if destination.data:
                kernel.LocalFree(destination.data)
            ctypes.memset(source_buffer, 0, len(source_buffer))

    def protect(self, data: bytes) -> bytes:
        return self._crypt(data, decrypt=False)

    def unprotect(self, data: bytes) -> bytes:
        return self._crypt(data, decrypt=True)
