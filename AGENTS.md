# Repository Guidelines

## Project Structure & Module Organization

- `src/`: React + TypeScript renderer. Entry `src/main.tsx`, layout in `src/App.tsx`, key views in `src/Settings.tsx`, `src/ScreenshotResult.tsx`, and `src/ScreenshotExplain.tsx`.
- `src/api/`: Tauri bridge wrappers (keep `invoke` calls centralized in `src/api/tauri.ts`). `src/services/` hosts UI helpers.
- `src-tauri/src/`: Rust backend modules such as `main.rs`, `screenshot.rs`, `settings.rs`, `windows.rs`, `utils.rs`.
- `src-tauri/tauri.conf.json` for app configuration; `src-tauri/icons/` for app icons; `public/` and `src/assets/` for static assets.
- Build artifacts (do not edit/commit): `dist/`, `src-tauri/target/`, `node_modules/`.

## Build, Test, and Development Commands

Use `npm` (lockfile: `package-lock.json`).

- `npm install`: install dependencies.
- `npm run dev`: run the full Tauri app (Rust backend + Vite UI).
- `npm run dev:ui`: run the Vite UI only.
- `npm run build`: bundle the desktop app via Tauri.
- `npm run build:ui`: build the UI bundle only.
- `npm run lint`: run ESLint on `ts`/`tsx`.
- `npm run preview`: preview the built UI bundle.

## Coding Style & Naming Conventions

- TypeScript + React, ESM (`"type": "module"`).
- Follow existing style: 2-space indentation, single quotes, no semicolons.
- Components use `PascalCase.tsx`; utilities/services use `camelCase.ts`.
- Prefer Tailwind utility classes for UI; keep shared styles in `src/index.css` and component-specific styles in `src/App.css`.

## Testing Guidelines

- No automated test runner is configured.
- Manual smoke test after changes: `npm run dev`, verify hotkeys, translation flow, screenshot translation/explain windows, and settings persistence.

## Commit & Pull Request Guidelines

- Git history follows Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`). Use short, imperative subjects.
- PRs should include a concise summary, testing notes, and screenshots/GIFs for UI changes.

## Security & Configuration Tips

- Do not commit API keys or base URLs; configure them through the app settings UI or local environment only.
- If you add new config, update `src-tauri/tauri.conf.json` and document defaults in the PR.
