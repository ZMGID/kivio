import argparse
import json
from pathlib import Path
import subprocess
import sys
import tarfile


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


def check_package(path):
    with tarfile.open(path, "r:gz") as archive:
        files = {
            member.name.split("/", 1)[1]: member
            for member in archive.getmembers()
            if member.isfile() and "/" in member.name
        }
        required = {"Cargo.toml", "LICENSE", "README.md", "src/lib.rs"}
        missing = required - files.keys()
        if missing:
            raise ValueError(f"SDK package is missing files: {sorted(missing)}")
        if archive.extractfile(files["LICENSE"]).read() != (ROOT / "LICENSE").read_bytes():
            raise ValueError("SDK package license differs from the repository license")


def main():
    parser = argparse.ArgumentParser(description="Check standalone SDK dependencies and packaging")
    parser.add_argument("--package", type=Path)
    args = parser.parse_args()
    if args.package:
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
    except (ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
