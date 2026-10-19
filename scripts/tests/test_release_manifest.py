import importlib.util
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "release-manifest.py"
SPEC = importlib.util.spec_from_file_location("release_manifest", SCRIPT)
release_manifest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_manifest)


class ReleaseManifestTests(unittest.TestCase):
    def run_manifest(self, directory, packages=None, during_metadata=None):
        metadata = {"packages": packages or []}

        def cargo(args, **kwargs):
            if args[-1] == "-vV":
                return "cargo 1.98.1\nhost: aarch64-apple-darwin\n"
            result = json.dumps(metadata)
            if during_metadata:
                during_metadata()
            return result

        with patch.object(sys, "argv", [str(SCRIPT), str(directory)]), patch.object(
            release_manifest.subprocess, "check_output", side_effect=cargo
        ):
            release_manifest.main()

    def test_manifest_records_curated_files_and_explicitly_does_not_sign(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "bundle"
            directory.mkdir()
            (directory / "wk-agent").write_bytes(b"synthetic binary")
            (directory / "NOTICE").write_text("synthetic notice\n")

            self.run_manifest(directory, [{"name": "example", "version": "1.0", "license": "MIT"}])

            sums = (directory / "SHA256SUMS").read_text()
            self.assertIn("  NOTICE\n", sums)
            self.assertIn("  wk-agent\n", sums)
            for line in sums.splitlines():
                digest, name = line.split("  ", 1)
                self.assertEqual(hashlib.sha256((directory / name).read_bytes()).hexdigest(), digest)
            inventory = json.loads((directory / "dependencies.spdx.json").read_text())
            self.assertEqual(inventory["packages"][0]["name"], "example")
            self.assertFalse((directory / "SHA256SUMS.sig").exists())

    def test_symlink_is_rejected_before_manifest_outputs_are_written(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "bundle"
            directory.mkdir()
            (directory / "outside").write_text("synthetic\n")
            (directory / "linked").symlink_to(directory / "outside")

            with self.assertRaisesRegex(ValueError, "must not contain symlinks"):
                self.run_manifest(directory)

            self.assertFalse((directory / "SHA256SUMS").exists())
            self.assertFalse((directory / "dependencies.spdx.json").exists())

    def test_symlink_as_release_directory_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            directory.mkdir()
            link = root / "bundle-link"
            link.symlink_to(directory, target_is_directory=True)

            with self.assertRaisesRegex(ValueError, "release directory must not be a symlink"):
                self.run_manifest(link)

            self.assertFalse((directory / "SHA256SUMS").exists())
            self.assertFalse((directory / "dependencies.spdx.json").exists())

    def test_symlink_created_during_metadata_is_rejected_without_following_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            directory.mkdir()
            outside = root / "outside.json"
            outside.write_text("synthetic original\n")

            def add_output_symlink():
                (directory / "dependencies.spdx.json").symlink_to(outside)

            with self.assertRaisesRegex(ValueError, "must not contain symlinks"):
                self.run_manifest(directory, during_metadata=add_output_symlink)

            self.assertEqual(outside.read_text(), "synthetic original\n")
            self.assertFalse((directory / "SHA256SUMS").exists())

    def test_manifest_rejects_input_symlink_swapped_after_scan(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            directory.mkdir()
            directory = directory.resolve()
            artifact = directory / "wk-agent"
            artifact.write_bytes(b"synthetic binary")
            outside = root / "outside.txt"
            outside.write_text("synthetic original\n")
            path_type = type(artifact)
            real_is_symlink = path_type.is_symlink
            checks = 0

            def swap_after_hash_scan(path):
                nonlocal checks
                result = real_is_symlink(path)
                if path == artifact:
                    checks += 1
                if path == artifact and checks == 3:
                    artifact.unlink()
                    artifact.symlink_to(outside)
                return result

            with patch.object(path_type, "is_symlink", swap_after_hash_scan):
                try:
                    self.run_manifest(directory)
                except ValueError as error:
                    message = str(error)
                else:
                    message = ""

            self.assertEqual(checks, 3)
            self.assertIn("must be regular non-symlink files", message)
            self.assertEqual(outside.read_text(), "synthetic original\n")
            self.assertFalse((directory / "SHA256SUMS").exists())
            self.assertFalse((directory / "dependencies.spdx.json").exists())

    def test_atomic_manifest_write_replaces_a_racing_symlink_without_following_it(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "SHA256SUMS"
            outside = root / "outside.txt"
            outside.write_text("synthetic original\n")
            directory_fd = release_manifest.open_release_directory(root)
            real_rename = release_manifest.os.rename

            def race_symlink(source, target, **kwargs):
                (root / target).symlink_to(outside)
                real_rename(source, target, **kwargs)

            try:
                with patch.object(release_manifest.os, "rename", side_effect=race_symlink):
                    release_manifest.write_atomically(directory_fd, "SHA256SUMS", "synthetic checksum\n")
            finally:
                release_manifest.os.close(directory_fd)

            self.assertEqual(outside.read_text(), "synthetic original\n")
            self.assertFalse(destination.is_symlink())
            self.assertEqual(destination.read_text(), "synthetic checksum\n")

    def test_manifest_outputs_stay_in_open_directory_if_its_path_is_swapped(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            directory.mkdir()
            (directory / "wk-agent").write_bytes(b"synthetic binary")
            moved = root / "bundle-moved"
            outside = root / "outside"
            outside.mkdir()
            swapped = False
            real_write = release_manifest.write_atomically

            def swap_before_first_write(directory_fd, name, contents):
                nonlocal swapped
                if not swapped:
                    directory.rename(moved)
                    directory.symlink_to(outside, target_is_directory=True)
                    swapped = True
                return real_write(directory_fd, name, contents)

            with patch.object(release_manifest, "write_atomically", side_effect=swap_before_first_write):
                self.run_manifest(directory)

            self.assertTrue(swapped)
            self.assertTrue((moved / "SHA256SUMS").is_file())
            self.assertTrue((moved / "dependencies.spdx.json").is_file())
            self.assertEqual(list(outside.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
