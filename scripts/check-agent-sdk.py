import argparse
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib


ROOT = Path(__file__).resolve().parent.parent
SDK = ROOT / "crates/kivio-agent-sdk/Cargo.toml"
CONSUMER = ROOT / "tests/fixtures/agent-sdk-consumer/Cargo.toml"
FORBIDDEN = {
    "kivio", "wry", "tao", "rusqlite", "libsqlite3-sys",
    "sqlite-vec", "ort", "ort-sys", "oar-ocr", "arboard", "enigo",
    "xcap", "window-vibrancy", "screencapturekit",
}


def check_metadata(metadata, *, allow_mcp=False):
    packages = metadata["packages"]
    sdk = [package for package in packages if package["name"] == "kivio-agent-sdk"]
    if len(sdk) != 1 or sdk[0]["license"] != "GPL-3.0-or-later":
        raise ValueError("SDK must retain its GPL-3.0-or-later license")
    allowed_local = {SDK.resolve(), CONSUMER.resolve()}
    nodes = metadata.get("resolve", {}).get("nodes", [])
    active_ids = {node["id"] for node in nodes}
    sdk_node = next((node for node in nodes if node["id"] == sdk[0].get("id")), None)
    mcp_enabled = sdk_node is not None and "mcp" in sdk_node.get("features", [])
    if mcp_enabled and not allow_mcp:
        raise ValueError("Default SDK dependency graph must not enable MCP")
    for package in packages:
        name = package["name"]
        if name in FORBIDDEN or name.startswith("tauri"):
            raise ValueError(f"SDK dependency crosses the desktop boundary: {name}")
        if name == "rmcp" or name.startswith("rmcp-") or name == "process-wrap":
            active = not nodes or package.get("id") in active_ids
            if active and not (allow_mcp and mcp_enabled):
                raise ValueError(f"MCP dependency requires the SDK mcp feature: {name}")
        if package["source"] is None:
            manifest = Path(package["manifest_path"]).resolve()
            if manifest not in allowed_local:
                raise ValueError(f"SDK depends on an external local package: {manifest}")


def check_package(path, *, extract_to=None):
    with tarfile.open(path, "r:gz") as archive:
        members = archive.getmembers()
        files = {}
        roots = set()
        for member in members:
            parts = PurePosixPath(member.name).parts
            if (not parts or member.name.startswith("/") or "\\" in member.name
                    or any(part in {".", ".."} or part.endswith((".", " ")) or ":" in part for part in parts)
                    or not (member.isfile() or member.isdir())):
                raise ValueError(f"SDK package has an unsafe member: {member.name}")
            roots.add(parts[0])
            if member.isfile() and len(parts) > 1:
                name = "/".join(parts[1:])
                if name in files:
                    raise ValueError(f"SDK package has a duplicate file: {name}")
                files[name] = member
        if len(roots) != 1:
            raise ValueError("SDK package must have one root directory")
        required = {"Cargo.toml", "LICENSE", "README.md", "src/lib.rs", "docs/contracts.md"}
        missing = required - files.keys()
        if missing:
            raise ValueError(f"SDK package is missing files: {sorted(missing)}")
        if archive.extractfile(files["LICENSE"]).read() != (ROOT / "LICENSE").read_bytes():
            raise ValueError("SDK package license differs from the repository license")
        if extract_to is not None:
            for name, member in files.items():
                destination = extract_to / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source, destination.open("wb") as target:
                    shutil.copyfileobj(source, target)


def rewrite_sdk_path(manifest, sdk):
    original = tomllib.loads(manifest)
    dependency = original["dependencies"]["kivio-agent-sdk"]
    if not isinstance(dependency, dict) or "path" not in dependency:
        raise ValueError("Consumer must have an inline SDK path dependency")
    rewritten, count = re.subn(
        r'(?m)^([ \t]*kivio-agent-sdk[ \t]*=[ \t]*\{[^\n]*?\bpath[ \t]*=[ \t]*)"[^"\n]*"',
        lambda match: match[1] + json.dumps(sdk.resolve().as_posix()),
        manifest,
    )
    dependency["path"] = sdk.resolve().as_posix()
    if count != 1 or tomllib.loads(rewritten) != original:
        raise ValueError("Consumer SDK path rewrite must preserve all other manifest values")
    return rewritten


def test_packaged_consumer(package):
    with tempfile.TemporaryDirectory(prefix="kivio-agent-sdk-") as directory:
        directory = Path(directory)
        sdk = directory / "sdk"
        check_package(package, extract_to=sdk)
        consumer = directory / "consumer"
        shutil.copytree(CONSUMER.parent, consumer, ignore=shutil.ignore_patterns("target"))
        manifest = consumer / "Cargo.toml"
        manifest.write_text(rewrite_sdk_path(manifest.read_text(encoding="utf-8"), sdk), encoding="utf-8")
        lock = (consumer / "Cargo.lock").read_bytes()
        subprocess.run(["cargo", "test", "--locked", "--manifest-path", str(manifest)], check=True)
        if (consumer / "Cargo.lock").read_bytes() != lock:
            raise ValueError("Packaged consumer test changed Cargo.lock")


def main(argv=None):
    parser = argparse.ArgumentParser(description="Check standalone SDK dependencies and packaging")
    parser.add_argument("--package", type=Path)
    parser.add_argument("--test-consumer", action="store_true", help="test the independent consumer against --package")
    args = parser.parse_args(argv)
    if args.test_consumer and not args.package:
        parser.error("--test-consumer requires --package")
    if args.package:
        if args.test_consumer:
            test_packaged_consumer(args.package)
            print("Independent consumer passed against the extracted SDK package")
        else:
            check_package(args.package)
        print("SDK package files and license passed")
        return
    for manifest, features, allow_mcp in (
        (SDK, ["--no-default-features"], False),
        (SDK, ["--all-features"], True),
        (CONSUMER, [], True),
    ):
        result = subprocess.run(
            ["cargo", "metadata", "--locked", "--format-version", "1", "--manifest-path", str(manifest), *features],
            check=True, stdout=subprocess.PIPE, encoding="utf-8",
        )
        check_metadata(json.loads(result.stdout), allow_mcp=allow_mcp)
    print("SDK and independent consumer dependency boundaries passed")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, tarfile.TarError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
