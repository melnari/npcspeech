import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from npcspeech.state import route_view


class RouteViewTests(unittest.TestCase):
    def setUp(self):
        self.presets = [{"id": "dragon", "name": "Ancient Dragon"}]

    def test_original_has_no_bypass_action(self):
        view = route_view({"selected_id": None, "bypass": True}, self.presets)
        self.assertEqual(view["headline"], "Output: Original microphone")
        self.assertFalse(view["can_toggle"])

    def test_bypass_explicitly_reports_original_output(self):
        view = route_view({"selected_id": "dragon", "bypass": True}, self.presets)
        self.assertIn("BYPASS ON", view["headline"])
        self.assertIn("Original microphone", view["headline"])
        self.assertEqual(view["button"], "Enable Ancient Dragon effects")

    def test_active_preset_reports_processing(self):
        view = route_view({"selected_id": "dragon", "bypass": False}, self.presets)
        self.assertIn("Ancient Dragon effects ACTIVE", view["headline"])
        self.assertTrue(view["can_toggle"])


if __name__ == "__main__":
    unittest.main()
