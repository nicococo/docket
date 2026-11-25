import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("installer", Path(__file__).parents[1] / "scripts/install-herdr.py")
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)

class InstallerTests(unittest.TestCase):
    def test_register_is_idempotent_and_preserves_config(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.toml"
            path.write_text('# keep this\n[ui]\ntheme = "default"\n')
            installer.register(path, "")
            content = path.read_text()
            self.assertIn('# keep this', content)
            self.assertIn('command = "nicococo.docket.toggle"', content)
            installer.register(path, "")
            self.assertEqual(content, path.read_text())

    def test_conflicting_key_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "config.toml"
            path.write_text('[[keys.command]]\nkey = "prefix+d"\ncommand = "other"\n')
            before = path.read_text()
            self.assertIn('already in use', installer.register(path, ""))
            self.assertEqual(before, path.read_text())

if __name__ == '__main__':
    unittest.main()
