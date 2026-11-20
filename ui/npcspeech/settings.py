"""Small user settings file; preset content stays in individual JSON files."""

from __future__ import annotations

import json
from .presets import config_dir


def load() -> dict:
    path = config_dir() / "settings.json"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
        return value if isinstance(value, dict) else {}
    except (OSError, ValueError):
        return {}


def save(value: dict) -> None:
    directory = config_dir()
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / "settings.json"
    temporary = directory / "settings.json.tmp"
    temporary.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)

