import importlib.util
import io
from contextlib import redirect_stderr
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("check_agent_sdk", Path(__file__).with_name("check-agent-sdk.py"))
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class SdkBoundaryTests(unittest.TestCase):
    def metadata(self):
        return {"packages": [
            {"name": "kivio-agent-sdk", "source": None, "manifest_path": str(checker.SDK), "license": "GPL-3.0-or-later"},
            {"name": "kivio-agent-sdk-consumer", "source": None, "manifest_path": str(checker.CONSUMER)},
            {"name": "reqwest", "source": "registry+https://github.com/rust-lang/crates.io-index"},
        ]}

    def test_independent_consumer_is_allowed(self):
        checker.check_metadata(self.metadata())

    def test_forbidden_transitive_packages_are_rejected(self):
        for name in ("kivio", "tauri", "tauri-plugin-store", "wry", "rusqlite", "ort"):
            with self.subTest(name=name):
                metadata = self.metadata()
                metadata["packages"].append({"name": name, "source": "registry+example"})
                with self.assertRaisesRegex(ValueError, "desktop boundary"):
                    checker.check_metadata(metadata)

    def test_mcp_is_allowed_only_when_explicitly_enabled(self):
        metadata = self.metadata()
        metadata["packages"][0]["id"] = "sdk"
        metadata["packages"].append({"id": "rmcp", "name": "rmcp", "source": "registry+example"})
        metadata["resolve"] = {"nodes": [{"id": "sdk", "features": ["mcp"]}, {"id": "rmcp", "features": []}]}
        with self.assertRaisesRegex(ValueError, "must not enable MCP"):
            checker.check_metadata(metadata)
        checker.check_metadata(metadata, allow_mcp=True)
        metadata["resolve"]["nodes"][0]["features"] = []
        with self.assertRaisesRegex(ValueError, "requires the SDK mcp feature"):
            checker.check_metadata(metadata, allow_mcp=True)
        metadata["resolve"]["nodes"].pop()
        checker.check_metadata(metadata)

    def test_mcp_does_not_allow_desktop_dependencies(self):
        metadata = self.metadata()
        metadata["packages"].append({"name": "tauri", "source": "registry+example"})
        with self.assertRaisesRegex(ValueError, "desktop boundary"):
            checker.check_metadata(metadata, allow_mcp=True)

    def test_unrecognized_local_package_is_rejected(self):
        metadata = self.metadata()
        metadata["packages"].append({"name": "shared-app-types", "source": None, "manifest_path": str(checker.ROOT / "src-tauri/Cargo.toml")})
        with self.assertRaisesRegex(ValueError, "external local package"):
            checker.check_metadata(metadata)

    def test_changed_sdk_license_is_rejected(self):
        metadata = self.metadata()
        metadata["packages"][0]["license"] = "MIT"
        with self.assertRaisesRegex(ValueError, "retain its GPL"):
            checker.check_metadata(metadata)

    def test_package_requires_original_license_and_readme(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sdk.crate"
            for license_bytes, include_readme, error in (
                ((checker.ROOT / "LICENSE").read_bytes(), True, None),
                (b"different license", True, "license differs"),
                ((checker.ROOT / "LICENSE").read_bytes(), False, "missing files"),
            ):
                with tarfile.open(path, "w:gz") as archive:
                    contents = {"Cargo.toml": b"", "src/lib.rs": b"", "docs/contracts.md": b"Contracts", "LICENSE": license_bytes}
                    if include_readme:
                        contents["README.md"] = b"SDK usage"
                    for name, data in contents.items():
                        member = tarfile.TarInfo(f"kivio-agent-sdk-0.1.0-alpha.1/{name}")
                        member.size = len(data)
                        archive.addfile(member, io.BytesIO(data))
                if error:
                    with self.assertRaisesRegex(ValueError, error):
                        checker.check_package(path)
                else:
                    checker.check_package(path)

    def package(self, path, *, omit=None, extra=None):
        contents = {
            "Cargo.toml": b'[package]\nname = "kivio-agent-sdk"\nversion = "0.1.0-alpha.1"\n',
            "LICENSE": (checker.ROOT / "LICENSE").read_bytes(),
            "README.md": b"SDK usage",
            "docs/contracts.md": b"Contracts",
            "src/lib.rs": b"pub fn packaged() {}",
        }
        with tarfile.open(path, "w:gz") as archive:
            for name, data in contents.items():
                if name == omit:
                    continue
                member = tarfile.TarInfo(f"kivio-agent-sdk-0.1.0-alpha.1/{name}")
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
            if extra:
                archive.addfile(extra)

    def test_package_requires_each_delivery_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sdk.crate"
            for name in ("Cargo.toml", "LICENSE", "README.md", "src/lib.rs", "docs/contracts.md"):
                with self.subTest(name=name):
                    self.package(path, omit=name)
                    with self.assertRaisesRegex(ValueError, "missing files"):
                        checker.check_package(path)

    def test_package_rejects_unsafe_paths_and_links_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sdk.crate"
            for name, kind in (
                ("sdk/../../escaped", tarfile.REGTYPE),
                ("/absolute", tarfile.REGTYPE),
                ("sdk/C:/escaped", tarfile.REGTYPE),
                ("sdk/..\\escaped", tarfile.REGTYPE),
                ("sdk/.. /escaped", tarfile.REGTYPE),
                ("sdk/link", tarfile.SYMTYPE),
                ("sdk/link", tarfile.LNKTYPE),
                ("other-root/file", tarfile.REGTYPE),
            ):
                with self.subTest(name=name, kind=kind):
                    member = tarfile.TarInfo(name)
                    member.type = kind
                    member.linkname = "../../escaped"
                    self.package(path, extra=member)
                    destination = Path(directory) / "extracted"
                    with self.assertRaises(ValueError):
                        checker.check_package(path, extract_to=destination)
                    self.assertFalse(destination.exists())

    def test_sdk_path_rewrite_preserves_version_features_and_other_dependencies(self):
        manifest = '''[dependencies]
kivio-agent-sdk = { version = "0.1.0-alpha.1", path = "../../../crates/kivio-agent-sdk", features = ["skills", "mcp"] }
other = { path = "../other", features = ["keep"] }
'''
        destination = Path(tempfile.gettempdir()) / "SDK with spaces"
        rewritten = checker.rewrite_sdk_path(manifest, destination)
        expected = checker.tomllib.loads(manifest)
        expected["dependencies"]["kivio-agent-sdk"]["path"] = destination.resolve().as_posix()
        self.assertEqual(checker.tomllib.loads(rewritten), expected)
        self.assertIn('version = "0.1.0-alpha.1"', rewritten)
        self.assertIn('other = { path = "../other", features = ["keep"] }', rewritten)
        self.assertNotIn("\\", checker.tomllib.loads(rewritten)["dependencies"]["kivio-agent-sdk"]["path"])

    def test_sdk_path_rewrite_rejects_unsupported_manifest_instead_of_changing_another_dependency(self):
        for manifest in (
            '[dependencies]\nkivio-agent-sdk = "0.1.0-alpha.1"\n',
            '[dependencies.kivio-agent-sdk]\npath = "../sdk"\n',
        ):
            with self.subTest(manifest=manifest):
                with self.assertRaises(ValueError):
                    checker.rewrite_sdk_path(manifest, Path("sdk"))

    def test_packaged_consumer_uses_extracted_crate_and_preserves_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            package = directory / "sdk.crate"
            self.package(package)
            consumer = directory / "fixture"
            consumer.mkdir()
            manifest = consumer / "Cargo.toml"
            original = '[dependencies]\nkivio-agent-sdk = { path = "../sdk", features = ["skills", "mcp"] }\n'
            manifest.write_text(original, encoding="utf-8")
            (consumer / "Cargo.lock").write_bytes(b"locked dependencies\n")
            (consumer / "tests").mkdir()
            (consumer / "tests" / "existing.rs").write_bytes(b"existing consumer test")
            (consumer / "target").mkdir()
            (consumer / "target" / "do-not-copy").write_bytes(b"build output")

            def run(command, *, check):
                self.assertEqual(command[:4], ["cargo", "test", "--locked", "--manifest-path"])
                self.assertTrue(check)
                copied = Path(command[4])
                dependency = checker.tomllib.loads(copied.read_text(encoding="utf-8"))["dependencies"]["kivio-agent-sdk"]
                sdk = Path(dependency["path"])
                self.assertEqual((sdk / "src/lib.rs").read_bytes(), b"pub fn packaged() {}")
                self.assertEqual(dependency["features"], ["skills", "mcp"])
                self.assertEqual((copied.parent / "tests/existing.rs").read_bytes(), b"existing consumer test")
                self.assertEqual((copied.parent / "Cargo.lock").read_bytes(), b"locked dependencies\n")
                self.assertFalse((copied.parent / "target").exists())

            with patch.object(checker, "CONSUMER", manifest), patch.object(checker.subprocess, "run", side_effect=run) as cargo:
                checker.test_packaged_consumer(package)
                self.assertEqual(cargo.call_count, 1)

            def change_lock(command, *, check):
                run(command, check=check)
                (Path(command[4]).parent / "Cargo.lock").write_bytes(b"changed lock")

            with patch.object(checker, "CONSUMER", manifest), patch.object(checker.subprocess, "run", side_effect=change_lock):
                with self.assertRaisesRegex(ValueError, "changed Cargo.lock"):
                    checker.test_packaged_consumer(package)
            self.assertEqual(manifest.read_text(encoding="utf-8"), original)
            self.assertEqual((consumer / "Cargo.lock").read_bytes(), b"locked dependencies\n")

    def test_test_consumer_requires_package_argument(self):
        error = io.StringIO()
        with redirect_stderr(error), self.assertRaises(SystemExit) as result:
            checker.main(["--test-consumer"])
        self.assertEqual(result.exception.code, 2)
        self.assertIn("--test-consumer requires --package", error.getvalue())


if __name__ == "__main__":
    unittest.main()
