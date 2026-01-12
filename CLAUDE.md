# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

KeyLingo is a **cross-platform** (macOS/Windows) menu bar translation and AI vision utility built with Electron + React + Vite + TailwindCSS. It provides global hotkey-triggered translation, screenshot OCR translation, and AI-powered screenshot explanation with multi-turn conversation.

**Current Version**: 1.3.6

## Common Commands

```bash
# Install dependencies
npm install

# Build OCR helper (auto-detects platform: swiftc on macOS, dotnet on Windows)
npm run build:ocr

# Start development server (Electron + Vite hot reload)
npm run dev

# Production build for current platform
npm run build

# Windows-specific build (run on Windows only)
npm run build:win

# Lint
npm run lint
```

## Architecture

### Cross-Platform Support (Adapter Pattern)

The app uses a **PlatformAdapter** pattern to abstract OS-specific operations:

```
electron/platforms/
├── interface.ts    # PlatformAdapter interface definition
├── mac.ts          # MacAdapter implementation
└── windows.ts      # WindowsAdapter implementation
```

**Platform-specific operations abstracted:**
- `captureScreenshot()`: macOS uses `screencapture`, Windows uses PowerShell + Snipping Tool
- `performSystemOCR()`: macOS uses Swift/Vision, Windows uses C#/Windows.Media.Ocr
- `pasteText()`: macOS uses AppleScript, Windows uses PowerShell SendKeys
- `registerHotkey()`: Windows auto-maps `Command` → `Ctrl`
- `getModifierKey()`: Returns `Command` (Mac) or `Ctrl` (Win)

### Process Model (Electron)

- **Main Process** ([electron/main.ts](electron/main.ts)): Platform adapter initialization, window management, global hotkey registration, IPC handlers, translation/OCR/AI API calls, tray menu, persistent settings via `electron-store`
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
| [electron/main.ts](electron/main.ts) | Main process: platform init, hotkeys, windows, IPC, API calls |
| [electron/platforms/interface.ts](electron/platforms/interface.ts) | PlatformAdapter interface |
| [electron/platforms/mac.ts](electron/platforms/mac.ts) | macOS-specific implementations |
| [electron/platforms/windows.ts](electron/platforms/windows.ts) | Windows-specific implementations |
| [src/App.tsx](src/App.tsx) | Router for window modes |
| [src/Settings.tsx](src/Settings.tsx) | Settings panel UI |
| [src/ScreenshotResult.tsx](src/ScreenshotResult.tsx) | Screenshot translation result display |
| [src/ScreenshotExplain.tsx](src/ScreenshotExplain.tsx) | AI vision chat with conversation history |

### Native Code

**macOS:**
- [native/ocr/keylingo_ocr.swift](native/ocr/keylingo_ocr.swift): Swift CLI using macOS Vision framework for offline OCR
- Built via [scripts/build-ocr-helper.mjs](scripts/build-ocr-helper.mjs) to `resources/ocr/keylingo-ocr`

**Windows:**
- [native/win-ocr/Program.cs](native/win-ocr/Program.cs): C# console app using `Windows.Media.Ocr` API
- [native/win-ocr/KeyLingo.Ocr.csproj](native/win-ocr/KeyLingo.Ocr.csproj): .NET 6.0 project targeting Windows 10/11
- [resources/scripts/win-screenshot.ps1](resources/scripts/win-screenshot.ps1): PowerShell script for clipboard-based screenshot capture
- Built via [scripts/build-ocr-helper.mjs](scripts/build-ocr-helper.mjs) to `resources/ocr/keylingo-ocr.exe`

### Translation/AI Flow

1. **Bing Translate**: Uses `bing-translate-api` package (default, free)
2. **AI Translation**: OpenAI-compatible API (DeepSeek, Zhipu, etc.)
3. **Screenshot OCR**: 
   - macOS: System OCR (Vision framework) via Swift binary
   - Windows: System OCR (Windows.Media.Ocr) via C# binary
   - Or GLM-4V / OpenAI Vision API (cloud)
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

### macOS
- The app hides from Dock (`app.dock.hide()`) for menu bar app behavior
- Windows use `setVisibleOnAllWorkspaces(true)` for full-screen compatibility
- Screenshot capture uses macOS `screencapture -i` command
- Auto-paste uses AppleScript to simulate Cmd+V

### Windows
- Screenshot uses PowerShell to invoke `ms-screenclip:` and monitor clipboard
- Auto-paste uses PowerShell `SendKeys` to simulate Ctrl+V
- Requires .NET 6.0 SDK for building OCR helper
- Hotkeys with `Command` are auto-mapped to `Ctrl`

## Build Configuration

`package.json` contains platform-specific `electron-builder` configurations:

**macOS (`build.mac`):**
- Target: DMG
- Extra resources: `resources/ocr/keylingo-ocr` → `ocr/keylingo-ocr`

**Windows (`build.win`):**
- Target: NSIS installer
- Extra resources:
  - `resources/scripts/win-screenshot.ps1` → `scripts/win-screenshot.ps1`
  - `native/win-ocr/bin/Release/.../keylingo-ocr.exe` → `ocr/keylingo-ocr.exe`

## Testing on Windows

If developing on macOS but need to test Windows:
1. Use a Windows VM or physical machine
2. Clone repo, run `npm install`
3. Install .NET 6.0 SDK
4. Run `npm run build:win`
5. Test `release/win-unpacked/KeyLingo.exe`

See the [Windows Verification Guide](/.gemini/antigravity/brain/.../windows_verification_guide.md) for detailed testing steps.
