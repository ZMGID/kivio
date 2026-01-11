# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

KeyLingo is a macOS menu bar translation and AI vision utility built with Electron + React + Vite + TailwindCSS. It provides global hotkey-triggered translation, screenshot OCR translation, and AI-powered screenshot explanation with multi-turn conversation.

## Common Commands

```bash
# Install dependencies
npm install

# Build macOS System OCR helper (requires Xcode Command Line Tools)
npm run build:ocr

# Start development server (Electron + Vite hot reload)
npm run dev

# Production build (builds OCR helper, compiles TypeScript, bundles with Vite, packages with electron-builder)
npm run build

# Lint
npm run lint
```

## Architecture

### Process Model (Electron)

- **Main Process** ([electron/main.ts](electron/main.ts)): Window management, global hotkey registration, IPC handlers, translation/OCR/AI API calls, tray menu, persistent settings via `electron-store`
- **Preload** ([electron/preload.ts](electron/preload.ts)): Exposes `window.api` bridge using `contextBridge` for secure renderer-to-main communication
- **Renderer** ([src/](src/)): React UI components

### Window Modes

The app uses a single React entry point ([src/App.tsx](src/App.tsx)) that renders different components based on URL mode:
- Default: `Translator` - main translation bar (360x120, frameless, always-on-top)
- `?mode=screenshot`: `ScreenshotResult` - OCR translation results
- `#explain?imageId=...`: `ScreenshotExplain` - AI vision chat interface

### Key Components

| File | Purpose |
|------|---------|
| [electron/main.ts](electron/main.ts) | Main process: hotkeys, windows, IPC, API calls |
| [src/App.tsx](src/App.tsx) | Router for window modes |
| [src/Settings.tsx](src/Settings.tsx) | Settings panel UI |
| [src/ScreenshotResult.tsx](src/ScreenshotResult.tsx) | Screenshot translation result display |
| [src/ScreenshotExplain.tsx](src/ScreenshotExplain.tsx) | AI vision chat with conversation history |

### Native Code

- [native/ocr/keylingo_ocr.swift](native/ocr/keylingo_ocr.swift): Swift CLI using macOS Vision framework for offline OCR
- Built via [scripts/build-ocr-helper.mjs](scripts/build-ocr-helper.mjs) to `resources/ocr/keylingo-ocr`

### Translation/AI Flow

1. **Bing Translate**: Uses `bing-translate-api` package (default, free)
2. **AI Translation**: OpenAI-compatible API (DeepSeek, Zhipu, etc.)
3. **Screenshot OCR**: System OCR (Vision framework) or GLM-4V API
4. **Screenshot Explain**: GLM-4V or OpenAI Vision API with multi-turn conversation

### Settings Storage

Uses `electron-store` with typed schema in main process. Key settings paths:
- `hotkey`: Main translation shortcut
- `screenshotTranslation.hotkey`: Screenshot OCR shortcut
- `screenshotExplain.hotkey`: AI explain shortcut
- `openai.*`: AI translation config
- `screenshotExplain.model.*`: Vision model config

### IPC Security

- All IPC handlers validate sender URL via `isAllowedIpcSender()`
- External URLs sanitized to HTTPS-only via `sanitizeExternalUrl()`
- Settings input sanitized via `sanitizeSettings()`
- Explain images tracked by UUID, paths validated against temp directory

## Development Notes

- The app hides from Dock (`app.dock.hide()`) for menu bar app behavior
- Windows use `setVisibleOnAllWorkspaces(true)` for full-screen compatibility
- Screenshot capture uses macOS `screencapture -i` command
- Auto-paste uses AppleScript to simulate Cmd+V
