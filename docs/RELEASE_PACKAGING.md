# Release Packaging

This document is the required release checklist for Kivio Desktop installers. Do not publish a new release only from memory; follow this file.

## Current Packaging Flow

Kivio Desktop is packaged by Tauri.

Local packaging (debug / inspect only — published installers come from GitHub Actions):

```bash
npm ci
npm run lint
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

`npm run build` runs:

1. `npm run package:check`
   - Verifies app/lockfile versions, MSI version inheritance, and resource mappings.
2. `npm run icons:check`
   - Verifies Windows assets against their unpadded source without changing macOS icons.
3. `npm run build:swift`
   - Builds the macOS Swift sidecars.
   - On non-macOS platforms, creates stub binaries so Tauri `externalBin` validation passes.
4. `npm run protocol:check`
   - Verifies committed chat protocol artifacts.
5. `tauri build`
   - Runs `beforeBuildCommand` from `src-tauri/tauri.conf.json`, currently `npm run build:ui`.
   - Vite writes the production frontend to `dist/`.
   - Tauri packages `dist/`, configured `externalBin` files, configured `resources`, and platform icons into DMG / MSI / NSIS bundles.

GitHub release packaging (this is the official path — do not build installers locally):

1. Bump versions in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`.
2. Write the bilingual GitHub body under `docs/releases/vX.Y.Z.md`. On the READMEs, **only** bump the version pointer in 功能 / Features (see [README format](#readme-format)). Do not restyle or replace `README.md` / `README.en.md`.
3. Commit and push `main`. Wait for `.github/workflows/ci.yml` on `main` to pass (lint / typecheck / frontend tests / Rust tests).
4. Create or move the release tag, for example:
   ```bash
   git tag -f vX.Y.Z
   git push origin main
   git push origin -f vX.Y.Z
   ```
   Pushing the `v*` tag is what starts packaging. To rebuild an existing tag after a workflow change:
   ```bash
   gh workflow run release.yml --repo ZMGID/kivio --ref main -f tag=vX.Y.Z -f ref=vX.Y.Z
   ```
5. `.github/workflows/release.yml` builds **both** installers on GitHub Actions and uploads them to the tag's release:
   - `macos-latest` (Apple Silicon / aarch64) with `--bundles dmg` → `Kivio.Desktop_X.Y.Z_aarch64.dmg`
   - `windows-latest` (x64) with `--bundles nsis` → `Kivio.Desktop_X.Y.Z_x64-setup.exe`
   - After the NSIS build, Windows also packs `scripts/package-windows-portable.ps1` → `Kivio.Desktop_X.Y.Z_x64-portable.zip` (unzip and run `Kivio Desktop.exe`; no Start Menu). In-app update still downloads the NSIS installer.
   - GitHub normalizes spaces in `productName` to dots in the asset file names.
   - The macOS DMG is **unsigned** (no signing secrets configured); first launch needs right-click → Open, or `xattr -cr "/Applications/Kivio Desktop.app"`.
6. Watch the workflow and inspect the release assets:
   ```bash
   gh run watch <RUN_ID> --repo ZMGID/kivio --exit-status
   gh release view vX.Y.Z --repo ZMGID/kivio --json url,assets
   ```
7. **Replace the CI-generated release body with hand-written bilingual notes.** The
   workflow publishes the release with a boilerplate body ("Automated macOS…");
   overwrite it to match the prior `v2.7.x` release format — title, a `## 下载 / Downloads`
   block (Windows NSIS + Windows portable zip + macOS DMG, plus the macOS "unsigned / first launch" note), a
   `## 新版本亮点 / What's New` bilingual bullet list (中文 + English inline per bullet,
   matching `docs/releases/vX.Y.Z.md`, not an inline README changelog), and a `完整变更 / Full changelog: …compare/vPREV...vX.Y.Z`
   link:
   ```bash
   gh release edit vX.Y.Z --repo ZMGID/kivio --notes-file docs/releases/vX.Y.Z.md
   ```

## Resources That Must Be Packaged

`src-tauri/tauri.conf.json` controls app resources.

After Tauri copies resources, `src-tauri/build.rs` prunes files no longer present
in the source from the profile's `skills/` and `licenses/` directories. Keep
these directory mappings aligned with `bundle.resources`; this prevents retired
skills from surviving incremental builds. Resource roots are watched for additions
and removals. The cleanup does not touch user-installed skills or app data.

Validate this behavior with `cargo test --test bundled-resources` (on Windows,
use `scripts/win-cargo-test.ps1 --test bundled-resources`).

At minimum, document Skill releases must include:

```json
"resources": {
  "resources/skills": "skills",
  "../docs/licenses": "licenses"
}
```

The final installed app must contain:

- `skills/pdf/SKILL.md`
- `skills/docx/SKILL.md`
- `skills/xlsx/SKILL.md`
- `skills/obsidian-markdown/SKILL.md` (+ `references/`)
- `skills/obsidian-bases/SKILL.md` (+ `references/`)
- `skills/json-canvas/SKILL.md` (+ `references/`)
- `skills/obsidian-cli/SKILL.md`
- `licenses/` (all files from `docs/licenses/`)

`npm run package:check` checks all five release-version files (including the npm
lockfile root package) and resource sources. `npm run test:packaging` covers version
drift, old MSI overrides, missing licenses, modified resources and retired skills.
Resource test fixtures copy every mapping from `bundle.resources`, including plugins,
so adding a bundled resource directory does not leave the fixture incomplete.
Release jobs also pass `--version "$RELEASE_TAG"` to reject a tag/source mismatch.
Do not pin `bundle.windows.wix.version`: Tauri derives MSI versions from the app
version. This follows the [Tauri WiX configuration contract](https://v2.tauri.app/reference/config/#wixconfig).

To check an extracted installer or portable layout, run:

```bash
node scripts/check-desktop-package.mjs --resources-dir /path/to/app/resources
```

This compares every mapped directory's file list and SHA-256 content against
source, including unexpected old files. Release jobs check extracted NSIS/DMG
contents; portable packaging copies all configured resource mappings and checks
the staged contents before compression. Portable packaging also rejects an EXE
whose embedded product version differs from the requested ZIP version.

> The four `obsidian-*` / `json-canvas` skills (adapted from kepano/obsidian-skills, MIT —
> see `resources/skills/NOTICE.md`) are gated at runtime on the Obsidian connector (a
> configured vault path), so they only surface to the model once the user sets an Obsidian vault.

## Release Verification

### Windows desktop baseline (2026-09-18 audit)

This is a source/configuration audit, not a completed interactive certification.
The icon work and packaging/autostart fixes are not installed on the user's PC yet.

| Area | Evidence / status | Required interactive check |
| --- | --- | --- |
| Identity and version | Stable `com.zmair.kivio`; debug EXE reports Kivio Desktop / 2.9.9. Removed stale MSI 2.8.2 override; version checks run before builds. | Installed Apps, EXE properties, shortcut target after upgrade. |
| Installation scope | NSIS uses `currentUser`; portable creates no Start Menu or uninstall entry. | Clean user install, upgrade and uninstall using CI artifacts; preserve user data. |
| Resources | Installer and portable mappings include skills and licenses; full-content verification replaces presence-only confidence. | Inspect final ZIP as well as staged files. |
| Startup | Windows now reads the OS startup state on launch; unrelated settings saves and rollback do not re-enable a Task Manager-disabled entry. Explicit preference changes still apply. | Enable, disable in Task Manager, reopen app and save an unrelated setting; entry must remain disabled. |
| Single instance / tray | Activation restores the existing visible window without replacing its route; tray has Open/Settings/Quit and a tooltip. | Second launch while minimized, hidden, editing Settings and using an overlay; Quit actually exits. |
| Taskbar / window controls | Chat is resizable, not always-on-top and appears in the taskbar. Min/max/restore/close buttons and native maximized-state synchronization exist. | Alt+Tab, Alt+F4, Win+Arrow, title-bar drag/double-click; verify close-to-tray behavior matches preference. |
| DPI | Tao initializes Per-Monitor V2 awareness, with older-OS fallbacks. This does not prove every custom overlay scales correctly. | 100/150/200% scaling, mixed-DPI monitors, unplug/reconnect a monitor, no inaccessible windows. |
| Windows 11 Snap hover | **Remaining gap:** custom maximize button has no native `WM_NCHITTEST` / `HTMAXBUTTON` integration. | Implement and verify the native hover menu; do not infer support from Win+Arrow alone. |
| Update mode | **Remaining gap:** portable uses the NSIS updater and becomes an installed edition. Portable README now states this and explains manual ZIP replacement. | Separate portable update UX; interrupted download, installer failure and rollback. |
| Update trust | **Remaining gap:** current updater downloads via HTTPS but does not independently verify an artifact signature before execution. Installed Windows EXE is unsigned. | Set up code-signing credentials and authenticated update verification; signing is not solved by setting a publisher string. |
| Accessibility / appearance | Button labels and keyboard-focus styling exist; reduced-motion rules exist. Not a full accessibility pass. | Narrator, keyboard-only use, Windows contrast themes, light/dark mode and text scaling. |

Microsoft's [Snap layout guidance](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-snap-layout-menu)
requires native hit testing for a custom maximize button. Implement that with a
dedicated native-window change and Windows 11 testing, not a CSS imitation.

### Application icons

Windows and macOS use different outer margins. Keep their asset generation separate:

- Windows source: `public/icon.png`, the full-canvas rounded artwork. Run
  `npm run icons:generate` to update `icons/icon.ico`, `windows-tray.png`,
  `Square*Logo.png` and `StoreLogo.png` under `src-tauri/`.
- `icon.ico` supplies the Windows executable, default window/taskbar icon and
  NSIS installer icon. It includes 16, 24, 32, 48, 64 and 256 px layers.
  Windows tray rendering uses the colored `windows-tray.png`, not the macOS template.
  `build.rs` explicitly watches `icons/` so an incremental build recompiles the
  executable's Windows icon resource after artwork changes.
- macOS retains `source-rounded.png`, `icon.icns`, the shared PNG size variants
  and `icon.png` with the existing ~80% artwork footprint. `tray-icon.png` is its
  monochrome template; the system handles menu-bar light/dark appearance.
- `npm run icons:check` regenerates into a temporary directory and verifies the
  checked-in Windows assets and required ICO sizes. CI and both release platforms
  run this check. Do not run an unscoped icon generation into `src-tauri/icons`:
  it would overwrite macOS assets with Windows margins, or vice versa.

The [Windows icon construction guide](https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction)
defines the baseline ICO sizes. After a Windows build, inspect the icon embedded
in the actual executable and check desktop/taskbar rendering at 100% and 150% scaling.
An existing desktop shortcut still refers to the installed executable, not a dev
build; install the new build before checking it. Refresh or recreate that shortcut
if Explorer retains an old cached icon after the executable has been updated.

### Installer contents

Before publishing or announcing installers, inspect the final artifact contents.

For macOS DMG:

```bash
mkdir -p /tmp/kivio-release-check
hdiutil attach -nobrowse -readonly -mountpoint /tmp/kivio-release-check \
  "src-tauri/target/release/bundle/dmg/Kivio Desktop_X.Y.Z_aarch64.dmg"
find "/tmp/kivio-release-check/Kivio Desktop.app/Contents/Resources" -maxdepth 5 -type f | sort
hdiutil detach /tmp/kivio-release-check
rmdir /tmp/kivio-release-check
```

For the local `.app` bundle before DMG:

```bash
find "src-tauri/target/release/bundle/macos/Kivio Desktop.app/Contents/Resources" -maxdepth 5 -type f | sort
```

For GitHub Releases:

```bash
gh release view vX.Y.Z --repo ZMGID/kivio --json url,assets
```

The release is not complete until the final installer contains loose `Contents/Resources/skills/pdf|docx|xlsx` Skill files.

## README format

GitHub's default landing page is **Chinese-first**, in the CC Switch README shape. Agents and humans updating the README for a release **must keep this layout**. Do not revert to the old short bilingual page (English-first header, inline “What's New” bullets, LINUX DO footer, no sponsor block, no star history).

### Files

| File | Role |
|---|---|
| `README.md` | Default. Chinese. |
| `README.en.md` | English. Same section order, same images, same links. |
| `README.zh-CN.md` | Stub that points at the two files above. Do not duplicate the body. |

Keep `README.md` and `README.en.md` in lockstep. A release bump that edits one must edit the other.

### Section order (do not reorder)

1. Centered header: `public/icon.png`, title, one-line tagline, badges (release / platform / Tauri / downloads / license), language switcher, download + 功能/帮助 + QQ **1104450740**, QQ group image.
2. Two-paragraph pitch (tray / agent / bring-your-own-key). No LINUX DO or other 友链.
3. **❤️ 赞助 / Sponsor** — `<details open>`. Table: logo 150px in the left cell (`docs/sponsors/…`), sponsor-provided copy in the right cell. Copy is the sponsor's; do not append in-app setup steps (“设置 → 供应商 → 添加驱动…”). Contact line stays GitHub Issues + QQ.
4. 为什么用 Kivio / Why Kivio
5. 截图 / Screenshots (`docs/screenshots/`)
6. 功能 / Features — link [Releases](https://github.com/ZMGID/kivio/releases) **and** `docs/releases/vX.Y.Z.md`. **This version pointer is the only README line a release should change.** Do not paste the changelog into README.
7. 热键 / Hotkeys
8. 下载安装 / Download
9. 帮助 / Help — Releases + Issues + QQ only. **Do not** list PRDs, architecture drafts, Chat Probe, packaging checklists, perf baselines, `CLAUDE.md`, or the model-adapter contract. Those stay in the repo for contributors (see 开发).
10. 快速开始 / Quick start
11. 常见问题 / FAQ (`<details>`)
12. 开发 / Development (`<details>`)
13. 贡献 / Contributing
14. **Star History** at the end — embed the committed chart `docs/star-history.svg` (refreshed by `.github/workflows/star-history.yml`). Do not use `api.star-history.com`; GitHub locked the public stargazers API and that URL now renders a “restricted access” placeholder. Then License.

### Release bump (example)

In `README.md` 功能:

```markdown
完整记录见 [Releases](https://github.com/ZMGID/kivio/releases) · 当前版本说明：[vX.Y.Z](docs/releases/vX.Y.Z.md)
```

In `README.en.md` Features:

```markdown
Full history: [Releases](https://github.com/ZMGID/kivio/releases) · current notes: [vX.Y.Z](docs/releases/vX.Y.Z.md)
```

Badges already resolve to `releases/latest`; do not hard-code the version in badge URLs.

A README-only bump is docs; `.github/workflows/ci.yml` skips `**.md` / `docs/**` / `LICENSE` and will not run the quality job. That is expected.

## Common Failure To Avoid

Do not treat "Skill files are bundled" as equivalent to "the host can parse those documents." `SKILL.md` only tells the model to use host `read`/`bash` tools. If Python or a PDF/Office CLI is missing, the agent should say so rather than inventing contents.
