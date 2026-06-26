# Linux Feasibility Findings

## Scope

本文件记录 Phase 2 在 Ubuntu 22.04 / kernel `6.8.0-40-generic` 上的真实验证结果。目标是判断是否能直接进入 AppImage 细节适配。

## Environment

```text
Linux zk 6.8.0-40-generic #40~22.04.3-Ubuntu SMP PREEMPT_DYNAMIC Tue Jul 30 17:30:19 UTC 2 x86_64 x86_64 x86_64 GNU/Linux
Ubuntu 22.04.5 LTS (jammy)
node v22.22.1
npm 10.9.4
rustc 1.94.0
cargo 1.94.0
tauri-cli 2.10.0
```

当前会话：

```text
XDG_SESSION_TYPE=x11
XDG_CURRENT_DESKTOP=ubuntu:GNOME
DESKTOP_SESSION=ubuntu
DISPLAY=:1
```

## System Build Dependencies

`pkg-config` 探测结果：

```text
webkit2gtk-4.1 2.50.4
javascriptcoregtk-4.1 2.50.4
gtk+-3.0 3.24.33
gdk-3.0 3.24.33
libsoup-3.0 3.0.7
ayatana-appindicator3-0.1 0.5.90
openssl 3.0.2
```

系统包确认：

```text
libwebkit2gtk-4.1-dev 2.50.4-0ubuntu0.22.04.1
libgtk-3-dev 3.24.33-1ubuntu2.2
libayatana-appindicator3-dev 0.5.90-7ubuntu2
librsvg2-dev 2.52.5+dfsg-3ubuntu0.2
patchelf 0.14.3-1
```

构建工具确认：

```text
cc /usr/bin/cc
gcc /usr/bin/gcc
g++ /usr/bin/g++
make /usr/bin/make
cmake /usr/bin/cmake
ninja /usr/bin/ninja
patchelf /usr/bin/patchelf
dpkg /usr/bin/dpkg
dpkg-deb /usr/bin/dpkg-deb
ar /usr/bin/ar
file /usr/bin/file
```

## Dependency Restore

第一次执行：

```bash
timeout 120s npm ci
```

失败：

```text
npm error code ECONNRESET
npm error network aborted
process exited with code 124
```

第二次执行：

```bash
timeout 300s env NPM_CONFIG_FETCH_RETRIES=5 \
  NPM_CONFIG_FETCH_RETRY_MINTIMEOUT=20000 \
  NPM_CONFIG_FETCH_RETRY_MAXTIMEOUT=120000 \
  npm ci --prefer-offline
```

成功：

```text
added 599 packages, and audited 600 packages in 7s
15 vulnerabilities (1 low, 7 moderate, 6 high, 1 critical)
```

说明：

- 本机依赖恢复可行，但 npm registry 网络偶发中断。
- 未执行 `npm audit fix`，因为它会修改依赖树，超出当前 Linux feasibility 范围。

## Tauri Linux Bundle Capability

`tauri build --help` 显示 Linux bundle 支持：

```text
--bundles [<BUNDLES>...]
[possible values: deb, rpm, appimage]
```

`tauri info` 环境摘要：

```text
OS: Ubuntu 22.4.0 x86_64 (X64) (ubuntu on x11)
webkit2gtk-4.1: 2.50.4
framework: React
bundler: Vite
```

## AppImage Build Probe

直接执行：

```bash
timeout 900s npm exec tauri -- build --bundles appimage --ci
```

第一次失败在 externalBin：

```text
resource path `binaries/kivio-ocr-helper-x86_64-unknown-linux-gnu` doesn't exist
```

原因：直接调用 `tauri build` 绕过了项目脚本里的 `npm run build:swift`。

补救执行：

```bash
timeout 60s npm run build:swift
```

成功生成 Linux stub：

```text
[build-swift-sidecar] 写空 stub → src-tauri/binaries/kivio-ocr-helper-x86_64-unknown-linux-gnu
```

再次执行：

```bash
timeout 900s npm exec tauri -- build --bundles appimage --ci
```

已通过：

- Pyodide 资源准备。
- Vite production build。
- GTK/WebKit/libsoup/appindicator 相关 crate 编译。
- `externalBin` 校验。

失败在 Rust 代码编译：

```text
error[E0425]: cannot find value `DYLIB_NAME` in this scope
error[E0425]: cannot find value `DYLIB_URL` in this scope
error[E0425]: cannot find value `DYLIB_ARCHIVE_PATH` in this scope
error[E0425]: cannot find type `HashMap` in this scope
error[E0061]: this function takes 7 arguments but 8 arguments were supplied
error[E0433]: failed to resolve: use of unresolved module or unlinked crate `libc`
```

Main failure locations:

- `rapidocr.rs`: ONNX Runtime dylib constants and archive extraction helpers are only defined for macOS/Windows, but Linux compiles the generic RapidOCR path.
- `lens_commands.rs`: non-macOS/non-Windows `capture_region_image` stub accepts 7 args, but caller passes `exclude_self_pid` as an eighth arg.
- `shell.rs` and `interactive/mod.rs`: Unix code uses `libc`, but `libc` is only declared under the macOS target dependency section.

## Phase 3 Compile-Gate Update

The first AppImage probe exposed Linux compile blockers. The minimum compile-boundary fixes were:

- Move `libc` to Unix target dependencies so Linux shell/kivio-code paths can use `setsid`, `kill`, and `poll`.
- Move `flate2`/`tar` to macOS+Linux target dependencies because ONNX Runtime official macOS/Linux packages are `.tgz`.
- Add Linux ONNX Runtime constants in RapidOCR:
  - `libonnxruntime.so.1.24.4`
  - `onnxruntime-linux-x64-1.24.4.tgz`
  - `onnxruntime-linux-aarch64-1.24.4.tgz`
- Align the Linux `capture_region_image` stub signature with the shared caller by accepting `exclude_self_pid`.

Validation:

```bash
timeout 300s cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-unknown-linux-gnu
```

Result:

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 40s
```

AppImage build:

```bash
timeout 900s bash -lc 'npm run build:swift && npm exec tauri -- build --bundles appimage --ci'
```

Result:

```text
Finished 1 bundle at:
    src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage
```

Artifact check:

```text
Kivio_2.7.2_amd64.AppImage: 135M
ELF 64-bit LSB pie executable, x86-64, static-pie linked, stripped
```

AppImage content evidence:

```text
squashfs-root/usr/lib/Kivio/skills/pdf/SKILL.md
squashfs-root/usr/lib/Kivio/skills/docx/SKILL.md
squashfs-root/usr/lib/Kivio/skills/xlsx/SKILL.md
squashfs-root/usr/bin/kivio-ocr-helper
```

Frontend/Pyodide evidence:

`dist/pyodide/` is not visible as plain squashfs files because Tauri embeds frontend assets into the main binary. The following strings are present in both `target/release/kivio` and the AppImage:

```text
/pyodide/pyodide.asm.wasm
/pyodide/python_stdlib.zip
/pyodide/pyodide-package-manifest.json
/pyodide/numpy-1.26.4-cp312-cp312-pyodide_2024_0_wasm32.whl
/pyodide/pandas-2.2.0-cp312-cp312-pyodide_2024_0_wasm32.whl
/pyodide/matplotlib-3.5.2-cp312-cp312-pyodide_2024_0_wasm32.whl
/pyodide/NotoSansCJKsc-Regular.otf
```

Rust regression after Linux OCR routing patch:

```bash
timeout 300s cargo test --manifest-path src-tauri/Cargo.toml
```

Result:

```text
1126 passed; 0 failed; 8 ignored
```

AppImage rebuild after the same patch:

```text
Finished 1 bundle at:
    /home/jn/kivio/src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage
```

Artifact recheck:

```text
-rwxr-xr-x 1 jn jn 135M Jun 24 16:54 src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage
ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), static-pie linked, stripped
```

Non-GUI AppImage runtime checks:

```bash
timeout 30s src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage --appimage-help
timeout 30s src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage --appimage-offset
```

Result:

```text
--appimage-help printed AppImage runtime options
--appimage-offset printed 944632
```

## Decision

Current decision: **AppImage is a viable packaging path for this repository on Ubuntu 22.04 x86_64**.

What is proven:

- Tauri CLI supports `appimage`.
- Ubuntu 22.04 system libraries are sufficient for the Linux build.
- The project now compiles for `x86_64-unknown-linux-gnu`.
- Rust unit tests pass after the Linux OCR routing patch.
- Tauri produced `Kivio_2.7.2_amd64.AppImage`.
- AppImage runtime metadata commands execute without launching the GUI.
- Skills are present as package files.
- Pyodide asset names are embedded in the application binary.
- RapidOCR is preserved as a selectable Linux OCR mode at settings/routing level.

What is not proven yet:

- GUI runtime smoke test.
- Lens overlay behavior.
- Region screenshot behavior on X11/Wayland.
- RapidOCR runtime download/init on Linux.
- Global shortcut/tray/autostart behavior on Linux desktop.

Required next step:

1. Preserve the AppImage path.
2. Define user-visible Linux capability status for screen-level features.
3. Run desktop smoke tests on Ubuntu 22.04 X11, then Wayland if available.
4. Verify RapidOCR download/init with Linux ONNX Runtime.
