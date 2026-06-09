# Settings Contracts

## Scenario: Appearance Theme Color Presets

### 1. Scope / Trigger

- Trigger: a settings field affects both persisted Rust settings and frontend rendering.
- Scope: `themeColor` controls the light-appearance surface tint while `theme` continues to control `system | light | dark` appearance mode.

### 2. Signatures

- Rust settings field: `Settings.theme_color: String`, serialized as camelCase `themeColor`.
- Frontend settings field: `Settings.themeColor: string`.
- Frontend preset source: `src/themeColors.ts`.
- Runtime application point: `document.documentElement.dataset.themeColor`.

### 3. Contracts

- Valid `themeColor` ids: `neutral`, `warm`, `cool`.
- Default value: `neutral`.
- `warm` maps to the reference warm white `#FAF9F5`.
- `themeColor` must store a preset id, not an arbitrary hex color.
- Add new preset ids in all three places:
  - `src/themeColors.ts` type/preset list.
  - `src/index.css` `:root[data-theme-color="<id>"]` variables and local `.kv[data-theme-color="<id>"]` preview variables.
  - `src-tauri/src/settings.rs` `sanitize_settings` allowlist.

### 4. Validation & Error Matrix

- Missing `themeColor` from older settings -> normalize to `neutral`.
- Unknown `themeColor` from hand-edited settings -> normalize to `neutral`.
- Unknown `theme` mode -> normalize to `system`.
- Dark appearance mode -> keep dark variables authoritative; theme color should not reduce dark-mode contrast.

### 5. Good/Base/Bad Cases

- Good: user selects `warm`; settings persists `themeColor: "warm"`; root gets `data-theme-color="warm"` and light surfaces use warm variables.
- Good: full-window chat surfaces such as `.chat-main-pane`, `.chat-empty-hero`, loading fallbacks, and embedded view roots use theme variables instead of hard-coded `bg-white`.
- Base: old settings without `themeColor`; app loads with `neutral` and existing visual appearance remains unchanged.
- Bad: settings file contains `themeColor: "#FAF9F5"` or `themeColor: "mint"`; sanitize/normalize must fall back to `neutral`.

### 6. Tests Required

- Frontend: run `npm run typecheck` and `npm run lint`.
- Backend: add/update a `sanitize_settings` unit test when changing theme/themeColor validation.
- Build smoke: run `npm run build:ui` after changing CSS theme variables or lazy-loaded settings code.

### 7. Wrong vs Correct

#### Wrong

```typescript
// Stores arbitrary colors and bypasses the preset contract.
updateSettings({ themeColor: '#FAF9F5' })
```

#### Correct

```typescript
// Stores stable preset ids and lets CSS own the actual palette values.
updateSettings({ themeColor: 'warm' })
```

#### Wrong

```tsx
// Covers the themed root with a local hard-coded surface.
<div className="flex-1 bg-white dark:bg-[#212121]" />
```

#### Correct

```tsx
// Lets the selected preset control the light surface tint.
<div className="chat-themed-surface flex-1" />
```
