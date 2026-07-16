# Product Identity and Packaging

## 1. Scope / Trigger

This contract applies whenever the desktop product name, application title, installer name, release documentation, or public marketing copy changes. The user-facing product name is **Kivio Desktop**, while compatibility-sensitive identifiers remain **kivio** or **Kivio** as documented below.

## 2. Signatures

- `src-tauri/tauri.conf.json`: `productName = "Kivio Desktop"`, `identifier = "com.zmair.kivio"`.
- `package.json`: `name = "kivio"`; only the human-readable description uses Kivio Desktop.
- `src-tauri/Cargo.toml`: package and binary names remain `kivio`; only the description uses Kivio Desktop.
- macOS application bundle: `Kivio Desktop.app`.
- CLI entry points: `kivio` and `kivio code`.

## 3. Contracts

- Use **Kivio Desktop** in window titles, tray tooltip, onboarding, About/settings branding, HTML titles, release names, installer documentation, README, and website copy.
- Keep Bundle ID `com.zmair.kivio` unchanged so Tauri continues to resolve the existing settings and application-data directory.
- Keep npm package, Rust crate, main binary, CLI, repository, updater User-Agent, MCP/server identifiers, and internal protocol names unchanged unless a separate migration is designed.
- Keep existing user paths such as `~/Kivio/workspace` and `~/Kivio/outputs` unchanged. They are persisted user data, not display branding.
- macOS screen-window filtering must recognize both `Kivio Desktop` and legacy `Kivio` owner/title values during upgrades and development runs.
- Release paths containing the product name must be quoted, for example `"/Applications/Kivio Desktop.app"`.

## 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Existing user upgrades after the rename | Settings, conversations, Skills, and application data remain discoverable through `com.zmair.kivio` |
| User invokes the terminal command | `kivio` and `kivio code` continue to work |
| macOS installer or manual quarantine command | Paths include and quote `Kivio Desktop.app` |
| Lens enumerates Kivio windows | New and legacy owner/title names are treated as self-owned; the primary Chat window remains selectable |
| Release asset selection | Match platform/architecture suffixes rather than assuming the old `Kivio_*` prefix |

## 5. Good / Base / Bad Cases

- Good: Finder, Dock, tray tooltip, window titles, installer, and release page say Kivio Desktop while existing settings load without migration.
- Base: Internal logs or protocol metadata still say Kivio because their identifiers are compatibility-sensitive and not product display surfaces.
- Bad: A repository-wide replacement renames `com.zmair.kivio`, the `kivio` binary, or `~/Kivio`; existing users appear to lose settings or CLI integrations.
- Bad: Release scripts use an unquoted `/Applications/Kivio Desktop.app` path and split it at the space.

## 6. Tests Required

- Run `cargo test --manifest-path src-tauri/Cargo.toml` and assert the macOS Lens owner/title compatibility tests pass.
- Run `npm test`, `npm run lint`, `npm run typecheck`, and `npm run build:ui`.
- Run `cargo check --manifest-path src-tauri/Cargo.toml` and `git diff --check`.
- Before publishing, build the platform bundles and inspect that the app/installer display name is Kivio Desktop while the executable command remains `kivio`.
- On an upgrade test machine, launch with an existing `com.zmair.kivio` data directory and verify settings and conversations are present.

## 7. Wrong vs Correct

### Wrong

```json
{
  "productName": "Kivio Desktop",
  "identifier": "com.zmair.kivio-desktop"
}
```

```text
~/Kivio Desktop/workspace
```

### Correct

```json
{
  "productName": "Kivio Desktop",
  "identifier": "com.zmair.kivio"
}
```

```text
CLI: kivio
Data: ~/Kivio/workspace
App: /Applications/Kivio Desktop.app
```
