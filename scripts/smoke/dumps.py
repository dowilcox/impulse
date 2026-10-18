"""Reading the state the app dumps (`dump=<name>` in a smoke run)."""

from __future__ import annotations

import re

ANSI = re.compile(r"\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][A-Z0-9]|\x1b[=>]")


def plain(text: str) -> str:
    """Terminal output without escape sequences."""
    return ANSI.sub("", text or "")


def surfaces(dump: dict, kind: str | None = None) -> list[dict]:
    found = [s for tab in dump["tabs"] for s in tab["surfaces"]]
    return [s for s in found if kind is None or s["kind"] == kind]


def editor(dump: dict, suffix: str) -> dict | None:
    return next((s for s in surfaces(dump, "editor") if (s.get("path") or "").endswith(suffix)), None)


def selected_tab(dump: dict) -> dict | None:
    return next((tab for tab in dump["tabs"] if tab["selected"]), None)


def toast_messages(dump: dict) -> list[str]:
    return [t["message"] for t in dump["toasts"]]
