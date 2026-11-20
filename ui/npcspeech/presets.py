"""Versioned preset storage shared with the Rust engine."""

from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path


EFFECT_LIMITS = {
    "pitch_semitones": (-12.0, 12.0),
    "formant_semitones": (-8.0, 8.0),
    "high_pass_hz": (20.0, 800.0),
    "low_pass_hz": (800.0, 20_000.0),
    "body_hz": (80.0, 1_200.0),
    "body_db": (-12.0, 12.0),
    "presence_hz": (700.0, 8_000.0),
    "presence_db": (-12.0, 12.0),
    "drive": (0.0, 1.0),
    "compression": (0.0, 1.0),
    "breath": (0.0, 1.0),
    "modulation_hz": (1.0, 120.0),
    "modulation_mix": (0.0, 1.0),
    "echo_mix": (0.0, 0.6),
    "echo_delay_ms": (20.0, 350.0),
    "gain_db": (-18.0, 18.0),
}
OPTIONAL_EFFECT_DEFAULTS = {
    "body_hz": 220.0,
    "body_db": 0.0,
    "presence_hz": 2_500.0,
    "presence_db": 0.0,
}


def config_dir() -> Path:
    root = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return root / "npcspeech"


def validate(preset: dict) -> dict:
    if not isinstance(preset, dict) or preset.get("schema_version") != 1:
        raise ValueError("Unsupported preset schema")
    preset_id = preset.get("id")
    if not isinstance(preset_id, str) or not re.fullmatch(r"[a-z0-9-]{1,64}", preset_id):
        raise ValueError("Preset ID must use lowercase letters, digits, and hyphens")
    name = preset.get("name")
    if not isinstance(name, str) or not name.strip() or len(name) > 100:
        raise ValueError("Preset name must contain 1–100 characters")
    description = preset.get("description")
    if not isinstance(description, str) or len(description) > 2000:
        raise ValueError("Description is too long")
    tags = preset.get("tags")
    if not isinstance(tags, list) or len(tags) > 16 or any(
        not isinstance(tag, str) or not tag.strip() or len(tag) > 40 for tag in tags
    ):
        raise ValueError("Presets may have up to 16 nonempty tags")
    effects = preset.get("effects")
    if not isinstance(effects, dict):
        raise ValueError("Preset effects do not match schema version 1")
    effects = {**OPTIONAL_EFFECT_DEFAULTS, **effects}
    if set(effects) != set(EFFECT_LIMITS):
        raise ValueError("Preset effects do not match schema version 1")
    for key, (minimum, maximum) in EFFECT_LIMITS.items():
        value = effects[key]
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not minimum <= value <= maximum:
            raise ValueError(f"Invalid {key}")
    if effects["high_pass_hz"] >= effects["low_pass_hz"]:
        raise ValueError("High-pass cutoff must be below low-pass cutoff")
    return {**preset, "effects": effects}


def slugify(name: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:64]
    return slug or "voice"


def load_all(directory: Path | None = None) -> list[dict]:
    directory = directory or config_dir() / "presets"
    if not directory.exists():
        return []
    presets = []
    for path in sorted(directory.glob("*.json")):
        try:
            presets.append(validate(json.loads(path.read_text(encoding="utf-8"))))
        except (ValueError, OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"Invalid preset {path.name}: {exc}") from exc
    return sorted(presets, key=lambda preset: preset["name"].casefold())


def save(preset: dict, directory: Path | None = None) -> Path:
    preset = validate(preset)
    directory = directory or config_dir() / "presets"
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{preset['id']}.json"
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=directory, prefix=".preset-", suffix=".tmp", delete=False
    ) as handle:
        json.dump(preset, handle, indent=2, ensure_ascii=False, allow_nan=False)
        handle.write("\n")
        temporary = Path(handle.name)
    temporary.replace(path)
    return path


def import_file(source: Path, directory: Path | None = None) -> Path:
    preset = validate(json.loads(source.read_text(encoding="utf-8")))
    directory = directory or config_dir() / "presets"
    if (directory / f"{preset['id']}.json").exists():
        base = preset["id"][:58]
        counter = 2
        while (directory / f"{base}-{counter}.json").exists():
            counter += 1
        preset = {**preset, "id": f"{base}-{counter}"}
    return save(preset, directory)


def export_file(preset: dict, destination: Path) -> None:
    preset = validate(preset)
    destination.write_text(json.dumps(preset, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")


def install_examples(examples_dir: Path, directory: Path | None = None) -> None:
    directory = directory or config_dir() / "presets"
    directory.mkdir(parents=True, exist_ok=True)
    for source in examples_dir.glob("*.json"):
        target = directory / source.name
        if not target.exists():
            save(json.loads(source.read_text(encoding="utf-8")), directory)

