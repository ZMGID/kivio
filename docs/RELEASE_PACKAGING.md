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
   - Tauri packages `dist/`, configured `externalBin` files, configured `resources`, and platform icons into DMG / MSI / NSIS bundles.

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
7. `.github/workflows/release.yml` builds the Windows release assets only:
   - `windows` on `windows-latest` with `--bundles msi,nsis`
8. Watch the workflow and inspect the release assets:
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

For GitHub Releases:

```bash
gh release view vX.Y.Z --repo ZMGID/kivio --json url,assets
```

The release is not complete until the final installer resources show both:

- `skills/pdf|docx|xlsx`
- Pyodide runtime plus the required local package wheels

## Common Failure To Avoid

Do not treat "Skill files are bundled" as equivalent to "document analysis is bundled." `SKILL.md` only tells the model what to do. The Python/Pyodide runtime and common packages are the execution environment and must be packaged separately.
