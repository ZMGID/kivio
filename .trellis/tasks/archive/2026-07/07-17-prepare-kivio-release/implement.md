# Implementation Plan

1. Update all version sources to `2.8.0`.
2. Replace English/Chinese README `v2.7.9` sections with the `v2.8.0` highlights.
3. Add `docs/releases/v2.8.0.md` using the established bilingual format and `v2.7.9...v2.8.0` compare link.
4. Update tracked website version labels and release workflow defaults; do not deploy the website.
5. Run release quality gate:
   - `npm run lint`
   - `npm run typecheck`
   - frontend tests (with the Node 26 localStorage compatibility flag locally)
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   - targeted version consistency search
6. Build macOS release:
   - `npm run build:swift`
   - `npx tauri build --bundles dmg`
7. Mount/inspect the DMG and verify Skill/Pyodide assets per `docs/RELEASE_PACKAGING.md`.
8. Commit release metadata as `chore(release): v2.8.0` using an explicit file list.
9. Fast-forward `main`, push `main`, create and push Tag `v2.8.0`.
10. Watch the release workflow to success and verify the Windows `.exe` asset.
11. Upload the local macOS DMG without overwriting an existing asset unexpectedly.
12. Replace GitHub Release body from `docs/releases/v2.8.0.md` and verify final URL/assets.
13. Complete Trellis checks, archive the release task, record journal, and push bookkeeping commits to `main` without moving the release Tag.

## Validation Gates

- No release Tag is created until local macOS DMG build and inspection pass.
- No push occurs while unrecognized tracked changes exist.
- `git status` may retain only the two known untracked website deployment files plus Trellis task artifacts before release commit.
- Final Release has exactly the expected versioned Windows and macOS installer assets.
