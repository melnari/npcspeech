import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from npcspeech import presets


EXAMPLE = {
    "schema_version": 1,
    "id": "elf",
    "name": "Elf",
    "description": "Airy and bright",
    "tags": ["fantasy"],
    "effects": {
        "pitch_semitones": 2.0,
        "formant_semitones": 1.0,
        "high_pass_hz": 100.0,
        "low_pass_hz": 16000.0,
        "drive": 0.0,
        "compression": 0.0,
        "breath": 0.1,
        "modulation_hz": 30.0,
        "modulation_mix": 0.0,
        "echo_mix": 0.0,
        "echo_delay_ms": 90.0,
        "gain_db": 0.0,
    },
}


class PresetTests(unittest.TestCase):
    def test_bundled_examples_match_schema(self):
        examples = Path(__file__).resolve().parents[1] / "presets"
        files = list(examples.glob("*.json"))
        self.assertGreaterEqual(len(files), 6)
        for path in files:
            with self.subTest(path=path.name):
                presets.validate(json.loads(path.read_text(encoding="utf-8")))

    def test_save_load_and_round_trip_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            presets.save(EXAMPLE, directory)
            expected = presets.validate(EXAMPLE)
            self.assertEqual(presets.load_all(directory), [expected])
            exported = directory / "export.json"
            presets.export_file(EXAMPLE, exported)
            self.assertEqual(json.loads(exported.read_text()), expected)

    def test_old_preset_receives_neutral_resonance_defaults(self):
        normalized = presets.validate(EXAMPLE)
        self.assertEqual(normalized["effects"]["body_db"], 0.0)
        self.assertEqual(normalized["effects"]["presence_db"], 0.0)
        self.assertEqual(set(normalized["effects"]), set(presets.EFFECT_LIMITS))

    def test_duplicate_import_keeps_existing_preset(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "presets"
            presets.save(EXAMPLE, directory)
            source = Path(temporary) / "incoming.json"
            source.write_text(json.dumps(EXAMPLE))
            imported = presets.import_file(source, directory)
            self.assertEqual(imported.name, "elf-2.json")
            self.assertEqual(len(presets.load_all(directory)), 2)

    def test_rejects_nonfinite_effect(self):
        bad = json.loads(json.dumps(EXAMPLE))
        bad["effects"]["pitch_semitones"] = float("nan")
        with self.assertRaises(ValueError):
            presets.validate(bad)


if __name__ == "__main__":
    unittest.main()
