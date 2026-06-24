# Release Packaging

This document is the required release checklist for Kivio installers. Do not publish a new release only from memory; follow this file.

## Current Packaging Flow

Kivio is packaged by Tauri.

Local packaging:

```bash
npm ci
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

`npm run build` runs:

1. `npm run build:swift`
   - Builds the macOS Swift sidecars.
   - On non-macOS platforms, creates stub binaries so Tauri `externalBin` validation passes.
2. `tauri build`
   - Runs `beforeBuildCommand` from `src-tauri/tauri.conf.json`, currently `npm run build:ui`.
   - Vite writes the production frontend to `dist/`.
   - Tauri packages `dist/`, configured `externalBin` files, configured `resources`, and platform icons into platform release bundles.
   - Linux automatically merges `src-tauri/tauri.linux.conf.json`, so `npm run build` produces the AppImage target without a manual `--bundles appimage` override.

GitHub release packaging:

1. Bump versions in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`.
2. Update README release notes.
3. Run the local quality gate:
   ```bash
   npm run lint
   npm run typecheck
   cargo test --manifest-path src-tauri/Cargo.toml
   ```
4. Commit and push `main`.
5. Create or move the release tag, for example:
   ```bash
   git tag -f vX.Y.Z
   git push origin main
   git push origin -f vX.Y.Z
   ```
6. Build and upload the Apple Silicon macOS DMG locally from Apple Silicon hardware.
7. `.github/workflows/release.yml` builds release assets for:
   - `macos` on `macos-latest` with `--bundles dmg`
   - `linux` on `ubuntu-22.04`; Linux merges `tauri.linux.conf.json` and builds AppImage
   - manual `workflow_dispatch` supports `platform=macos`, `platform=linux`, or `platform=all`
8. If building Linux locally instead of through GitHub Actions, use Ubuntu 22.04 or a compatible Linux runner:
   ```bash
   npm ci
   npm run typecheck
   cargo test --manifest-path src-tauri/Cargo.toml
   npm run build
   ```
9. Windows NSIS is still built locally and uploaded separately.
10. Watch the workflow and inspect the release assets:
   ```bash
   gh run watch <RUN_ID> --repo ZMGID/kivio --exit-status
   gh release view vX.Y.Z --repo ZMGID/kivio --json url,assets
   ```

## Resources That Must Be Packaged

`src-tauri/tauri.conf.json` controls app resources. At minimum, document Skill releases must include:

```json
"resources": {
  "resources/skills": "skills"
}
```

The final installed app must contain:

- `skills/pdf/SKILL.md`
- `skills/docx/SKILL.md`
- `skills/xlsx/SKILL.md`
- Pyodide core runtime files
- `python_stdlib.zip`
- local Pyodide wheels for common document/data packages

## Mandatory Python / Pyodide Offline Bundle

Bundled document Skills are not complete unless their Python execution runtime is bundled too.

When `pdf`, `docx`, and `xlsx` are shipped, the installer must also include an offline Pyodide package set for normal document analysis. Do not rely on the CDN path as the normal runtime path.

Required local Pyodide files:

- `pyodide.asm.js`
- `pyodide.asm.wasm`
- `pyodide-lock.json`
- `python_stdlib.zip`

Required local package wheels:

- `numpy`
- `pandas`
- `matplotlib`
- `pillow`
- `seaborn`
- `micropip`
- `openpyxl`
- `xlrd`
- `et_xmlfile`
- `pypdf`

Required local Python font assets:

- `NotoSansCJKsc-Regular.otf` for CJK text rendering in Python-generated matplotlib / Pillow images

Implementation requirement:

- Run `npm run prepare:pyodide` before the frontend build. It creates the reproducible Python sandbox runtime resources in `resources/python-sandbox/pyodide/`.
- Update the Vite Pyodide asset plugin in `vite.config.ts` so it emits the sandbox runtime core files, required local wheels, and required local font assets into `dist/pyodide/`; Tauri packages that single frontend asset copy through `frontendDist`.
- Update `src/chat/pyodideRunner.ts` so `run_python` package loading prefers the bundled local `dist/pyodide/` package index and wheels.
- CDN package loading may remain as a fallback, but the app must be able to run normal `pdf` / `docx` / `xlsx` analysis without downloading those common packages at runtime.
- Do not package a host machine virtual environment or host `site-packages` as a substitute for Pyodide wheels. The runtime used by `run_python` is Pyodide in the WebView sandbox.

## Release Verification

Before publishing or announcing installers, inspect the final artifact contents.

For macOS DMG:

```bash
hdiutil attach "src-tauri/target/release/bundle/dmg/Kivio_X.Y.Z_aarch64.dmg"
find "/Volumes/Kivio/Kivio.app/Contents/Resources" -maxdepth 5 -type f | sort
hdiutil detach "/Volumes/Kivio"
```

For the local `.app` bundle before DMG:

```bash
find "src-tauri/target/release/bundle/macos/Kivio.app/Contents/Resources" -maxdepth 5 -type f | sort
```

For Linux AppImage:

```bash
npm run build
find "src-tauri/target/release/bundle/appimage/Kivio.AppDir/usr/lib/Kivio" -maxdepth 5 -type f | sort
tmp_home="$(mktemp -d)"
env HOME="$tmp_home" \
  XDG_CONFIG_HOME="$tmp_home/config" \
  XDG_DATA_HOME="$tmp_home/data" \
  XDG_CACHE_HOME="$tmp_home/cache" \
  NO_AT_BRIDGE=1 \
  KIVIO_DESKTOP_SMOKE=1 \
  KIVIO_DESKTOP_SMOKE_EXIT_AFTER_MS=6000 \
  "src-tauri/target/release/bundle/appimage/Kivio_X.Y.Z_amd64.AppImage"
```

The AppDir file listing verifies bundled Skills under `usr/lib/Kivio/skills`.
Pyodide files are frontend assets embedded into the AppImage, so verify them
from the packaged desktop smoke `resources.pyodideAssets` output. Do not expect
a `usr/lib/Kivio/pyodide` directory.

For GitHub Releases:

```bash
gh release view vX.Y.Z --repo ZMGID/kivio --json url,assets
```

The release is not complete until the final installer resources show both:

- `skills/pdf|docx|xlsx`
- Pyodide runtime plus the required local package wheels and font assets

## Common Failure To Avoid

Do not treat "Skill files are bundled" as equivalent to "document analysis is bundled." `SKILL.md` only tells the model what to do. The Python/Pyodide runtime and common packages are the execution environment and must be packaged separately.
