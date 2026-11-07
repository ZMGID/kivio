import importlib.util
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest


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
        for name in ("kivio", "tauri", "tauri-plugin-store", "wry", "rmcp", "rusqlite", "ort"):
            with self.subTest(name=name):
                metadata = self.metadata()
                metadata["packages"].append({"name": name, "source": "registry+example"})
                with self.assertRaisesRegex(ValueError, "desktop boundary"):
                    checker.check_metadata(metadata)

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
                    contents = {"Cargo.toml": b"", "src/lib.rs": b"", "LICENSE": license_bytes}
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


if __name__ == "__main__":
    unittest.main()
