# Repository Guidelines

## Project Structure & Module Organization

- `src/`: React + TypeScript renderer UI (main window, settings, screenshot UI).
- `src/services/`: renderer-side services (e.g. translation helpers).
- `electron/`: Electron main + preload (`electron/main.ts`, `electron/preload.ts`).
- `public/`: static assets (icons, SVGs).
- Build outputs (do not edit/commit): `dist/`, `dist-electron/`, `release/`.

## Build, Test, and Development Commands

Use `npm` (lockfile: `package-lock.json`).

- `npm install`: install dependencies.
- `npm run dev`: start the Vite dev server and Electron app (via `vite-plugin-electron`).
- `npm run build`: typecheck (`tsc`), build renderer (`vite build`), then package the app (`electron-builder`, output to `release/`).
- `npm run lint`: run ESLint on `ts`/`tsx`.
- `npm run preview`: preview the built renderer bundle.

## Coding Style & Naming Conventions

- Language: TypeScript (`.ts`, `.tsx`), ESM (`"type": "module"`).
- Formatting: follow existing style (2-space indentation, single quotes, no semicolons).
- Components: `PascalCase.tsx` (e.g. `src/ScreenshotResult.tsx`); utilities/services `camelCase.ts`.
- Keep Electron code split: privileged logic in `electron/main.ts`, UI logic in `src/`, and only safe APIs exposed via `electron/preload.ts`.

## Testing Guidelines

There is no automated test runner configured yet. For changes, do:

- Manual smoke test: `npm run dev`, verify hotkeys, translation flow, settings persistence, and screenshot features.
- If you add non-trivial logic, consider introducing a lightweight unit test setup (e.g. Vitest) in the same PR.

## Commit & Pull Request Guidelines

- Commit messages follow a Conventional Commits style: `feat: …`, `fix: …`, `chore: …`, `release: …`.
- PRs should include: a short summary, steps to verify, and screenshots/GIFs for UI changes.
- Do not include secrets (API keys/base URLs) in code or screenshots; keep them in local settings only.
