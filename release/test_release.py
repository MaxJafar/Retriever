import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import export
from validate import validate

ROOT = Path(__file__).resolve().parents[1]

class ReleaseTests(unittest.TestCase):
    def test_export_contains_updater_not_private_workspace(self):
        paths = {name for name, _, _ in export.entries(ROOT)}
        self.assertIn("retriever-rs/crates/updater/src/lib.rs", paths)
        self.assertIn("native/retriever-macos/Updates.swift", paths)
        self.assertIn("native/retriever-windows/Updates.cs", paths)
        self.assertFalse(any(p.startswith(("artifacts/", ".cleanup/", "searchx/")) for p in paths))
        self.assertFalse(any(Path(p).name in {"coordinator.json", "api-token", "update-token", "installation.json"} for p in paths))

    def test_archive_reproducible_and_manifest_verified(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(export, "entries", return_value=[("README.md", b"example\n", 0o644)]):
            a = Path(tmp) / "a.tar.gz"; b = Path(tmp) / "b.tar.gz"
            export.export(ROOT, a); export.export(ROOT, b)
            self.assertEqual(a.read_bytes(), b.read_bytes())
            export.verify(a)

    def test_versions_and_fork_repository_guard(self):
        data = validate(ROOT)
        validate(ROOT, "v" + data["version"], data["repository"])
        with self.assertRaises(ValueError): validate(ROOT, "v0.0.0", data["repository"])
        with self.assertRaises(ValueError): validate(ROOT, repository="example/fork")

if __name__ == "__main__":
    unittest.main()
