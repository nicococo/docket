import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("toggle", Path(__file__).parents[1] / "scripts/herdr-toggle.py")
toggle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(toggle)


class ToggleTests(unittest.TestCase):
    def test_open_focus_close_and_stale_id(self):
        with tempfile.TemporaryDirectory() as directory:
            calls = []
            panes = []
            def fake(*args):
                calls.append(args)
                if args[:2] == ("pane", "list"):
                    return {"panes": panes}
                if args[2] == "open":
                    return {"plugin_pane": {"plugin_id": toggle.PLUGIN, "entrypoint": toggle.ENTRYPOINT,
                            "pane": {"pane_id": "w1:p2", "terminal_id": "term1", "workspace_id": "w1"}}}
                return {}
            with patch.object(toggle, "run", side_effect=fake):
                toggle.toggle(Path(directory), "w1", "/socket")
                self.assertIn("open", calls[-1])
                panes.append({"pane_id": "w1:p2", "terminal_id": "term1", "workspace_id": "w1", "focused": False})
                toggle.toggle(Path(directory), "w1", "/socket")
                self.assertEqual(calls[-1], ("plugin", "pane", "focus", "w1:p2"))
                panes[0]["focused"] = True
                toggle.toggle(Path(directory), "w1", "/socket")
                self.assertEqual(calls[-1], ("plugin", "pane", "close", "w1:p2"))
                toggle.toggle(Path(directory), "w1", "/socket")
                panes[0]["terminal_id"] = "different"
                toggle.toggle(Path(directory), "w1", "/socket")
                self.assertIn("open", calls[-1])

    def test_missing_context_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                toggle.toggle(Path(directory), "w1", None)

if __name__ == "__main__":
    unittest.main()
