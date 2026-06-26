# Memory Surface Snapshot

## Resolved Memory Surface

- Native conversation goal plus `docs/progress/MASTER.md`.
- No repo-local fallback memory file was created.

## Durable Decisions

- Preserve the existing stack: Tauri v2, Rust, React 18, TypeScript, Vite, and TailwindCSS v4.
- Use AppImage as the Linux distributable on Ubuntu 22.04.
- Keep Linux capture explicitly unsupported until a real Linux capture backend is implemented.
- Expose Linux RapidOCR independently from System OCR.
- Use `tauri.linux.conf.json` for Linux AppImage bundling so macOS/Windows targets remain unchanged.
- Treat AppDir file listing and packaged desktop smoke as complementary artifact checks.

## Verification Evidence

- Automated gates: `npm run lint`, `npm run typecheck`, `npm test`, and `cargo test --manifest-path src-tauri/Cargo.toml`.
- AppImage runtime smoke: `KIVIO_DESKTOP_SMOKE=1` on `Kivio_2.7.2_amd64.AppImage`.
- Smoke host: Ubuntu 22.04.5 LTS, kernel 6.8.0-40-generic, X11.
