# Screenshot Arrow Annotation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users drag red arrows on the Lens overlay after capture, then send the arrow-annotated PNG (composed in-browser) to the vision model.

**Architecture:** Frontend Canvas composition. New backend command `lens_register_annotated_image` receives the composed PNG (base64), writes it to `temp_dir`, archives it, and registers a fresh `image_id` so the existing `lens_ask` path keeps working unchanged. Annotation is a sub-mode of `stage='ready'` controlled by a toolbar toggle; if no arrows are drawn, send goes through the original path with zero overhead.

**Tech Stack:** Tauri v2 (Rust), React 18 + TypeScript, OffscreenCanvas API, SVG for live preview.

**Spec:** [`docs/superpowers/specs/2026-05-03-screenshot-arrow-annotation-design.md`](../specs/2026-05-03-screenshot-arrow-annotation-design.md)

---

## File Structure

| File | Type | Responsibility |
|---|---|---|
| `src-tauri/src/main.rs` | Modify | Add `lens_register_annotated_image` command + register in `invoke_handler` |
| `src/api/tauri.ts` | Modify | Add `lensRegisterAnnotatedImage` invoke binding |
| `src/settings/i18n.ts` | Modify | Add `lensArrowToggle` strings (zh/en) |
| `src/Lens.tsx` | Modify | Arrow type + state + SVG layer + interactions + toolbar button + `composeAnnotatedImage` + `handleSend` integration |

The Lens.tsx changes are split across 5 tasks (state → helpers → SVG layer → toggle button → handleSend wire) so each commit is small and independently verifiable.

**Note on testing:** Per spec, no Rust unit tests or frontend tests are added (project has no frontend test infrastructure; the new Rust command is IO + state mutation, same pattern as existing `lens_capture_*`). Manual smoke testing in Task 7 is the verification gate.

---

### Task 1: Backend command + frontend API binding

**Files:**
- Modify: `src-tauri/src/main.rs` (add function near line 2010, register at line 2464 block)
- Modify: `src/api/tauri.ts` (add binding near line 251)

**Why both files in one task:** The Rust command and TypeScript binding are a single contract — committing them together prevents a half-broken state where one side knows about the command but the other doesn't.

- [ ] **Step 1.1: Add `lens_register_annotated_image` command in `src-tauri/src/main.rs`**

Insert this function immediately before the `archive_captured_image` function (which is currently at line 2014):

```rust
/// 接收前端合成的带箭头标注 PNG（base64 编码），落盘到 temp_dir、归档、注册新 image_id。
/// 原 image_id 对应的临时文件保留（由 lens_close / 下次截图 cleanup 路径回收）。
#[tauri::command]
fn lens_register_annotated_image(
  app: AppHandle,
  state: State<AppState>,
  base64_png: String,
) -> Result<serde_json::Value, String> {
  let bytes = general_purpose::STANDARD
    .decode(base64_png.as_bytes())
    .map_err(|e| format!("base64 decode failed: {e}"))?;

  let temp_path = std::env::temp_dir().join(format!("lens-{}.png", Uuid::new_v4()));
  std::fs::write(&temp_path, &bytes)
    .map_err(|e| format!("write png failed: {e}"))?;

  let image_id = Uuid::new_v4().to_string();
  archive_captured_image(&app, &temp_path, &image_id);

  {
    let mut map = state.images_lock();
    map.insert(image_id.clone(), temp_path);
  }
  {
    let mut current = state.current_id_lock();
    *current = Some(image_id.clone());
  }

  Ok(serde_json::json!({ "success": true, "imageId": image_id }))
}
```

Verify imports already present (no changes needed — confirmed):
- `use base64::{engine::general_purpose, Engine as _};` at line 29
- `use uuid::Uuid;` at line 35
- `AppHandle`, `State`, `AppState` already used by neighboring commands

- [ ] **Step 1.2: Register command in the invoke handler block**

In `src-tauri/src/main.rs`, the `tauri::generate_handler![]` block starts at line 2464. Add `lens_register_annotated_image` to the list. After the change, the block reads (look for `lens_capture_region,` and add the new entry on the line after it):

```rust
    .invoke_handler(tauri::generate_handler![
      get_settings,
      get_default_prompt_templates,
      save_settings,
      translate_text,
      commit_translation,
      open_external,
      explain_read_image,
      fetch_models,
      test_provider_connection,
      get_permission_status,
      open_permission_settings,
      lens_request,
      lens_request_translate,
      lens_list_windows,
      lens_capture_window,
      lens_capture_region,
      lens_register_annotated_image,
      lens_ask,
      lens_translate,
      lens_cancel_stream,
      lens_close,
      lens_set_floating,
      take_lens_selection,
      lens_commit_image_to_history,
      lens_delete_history_image,
      check_github_latest_release,
      download_update_asset,
      install_update_and_quit,
      apple_intelligence_available
    ])
```

- [ ] **Step 1.3: Verify Rust build**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: builds with no errors. Existing warnings (if any) are unchanged.

- [ ] **Step 1.4: Add `lensRegisterAnnotatedImage` binding in `src/api/tauri.ts`**

In `src/api/tauri.ts`, locate the `lensCaptureRegion` block (lines 243-251) and add the new binding immediately after it (before `lensRequestTranslate` at line 252):

```typescript
  lensRegisterAnnotatedImage: (base64Png: string) =>
    invoke<{ success: boolean; imageId?: string; error?: string }>(
      'lens_register_annotated_image', { base64Png }
    ),
```

- [ ] **Step 1.5: Verify TypeScript typecheck**

Run: `npm run typecheck`
Expected: passes with no errors.

- [ ] **Step 1.6: Commit**

```bash
git add src-tauri/src/main.rs src/api/tauri.ts
git commit -m "feat: add lens_register_annotated_image backend command

Receives a base64-encoded PNG composed in the frontend (with arrow
annotations burned in), writes it to temp_dir, archives it, and
registers a fresh image_id so the existing lens_ask path can use it
unchanged."
```

---

### Task 2: i18n strings for arrow toggle button

**Files:**
- Modify: `src/settings/i18n.ts` (zh block ending at line 172, en block ending at line 341)

- [ ] **Step 2.1: Add Chinese strings**

In `src/settings/i18n.ts`, the zh language block ends with `lensKeepFullscreenHint` at line 171. Replace that line with the original line plus three new keys (preserve the trailing comma + closing brace):

```typescript
    lensKeepFullscreen: '截图后保持全屏覆盖',
    lensKeepFullscreenHint: '关闭后截图完成时窗口缩小为浮动,可在 Lens UI 外操作桌面',
    lensArrowToggle: '画箭头',
    lensArrowToggleOff: '退出画箭头',
    lensArrowDisabledHint: '截图后可用',
  },
```

- [ ] **Step 2.2: Add English strings**

Mirror the same three keys at the end of the `en` block (after `lensKeepFullscreenHint` at line 340):

```typescript
    lensKeepFullscreen: 'Keep fullscreen after capture',
    lensKeepFullscreenHint: 'When off, Lens shrinks to a floating window after capture so you can interact with the desktop',
    lensArrowToggle: 'Draw arrow',
    lensArrowToggleOff: 'Exit draw mode',
    lensArrowDisabledHint: 'Available after capture',
  }
```

(Note: the `en` block has no trailing comma after the closing `}` because it's the last entry in the `i18n` object — preserve that.)

- [ ] **Step 2.3: Verify TypeScript typecheck**

Run: `npm run typecheck`
Expected: passes. The exported `I18n = typeof i18n.zh` automatically picks up the new keys, and `en` is type-checked against it.

- [ ] **Step 2.4: Commit**

```bash
git add src/settings/i18n.ts
git commit -m "feat: add i18n strings for lens arrow annotation toggle"
```

---

### Task 3: Lens.tsx — Arrow type, state, geometry constants

**Files:**
- Modify: `src/Lens.tsx` (state cluster around line 252-300, plus a top-level Arrow type)

This task adds the data plumbing only. No UI is wired up yet — the new state is unused, but the file remains compilable and behavior unchanged.

- [ ] **Step 3.1: Add the `Arrow` type**

In `src/Lens.tsx`, find the `CapturedFrame` type definition (search for `capturedFrame: CapturedFrame | null` at line 33 to locate the surrounding `HistoryItem` block). Just below the existing top-level type definitions but before the `export default function Lens()` (line 244), add:

```typescript
type Arrow = {
  x1: number
  y1: number
  x2: number
  y2: number
}

const ARROW_COLOR = '#ff3b30'
const ARROW_MIN_DRAG_PX = 8
const ARROW_HEAD_ANGLE_DEG = 30
```

If the existing types are clustered near the top (line ~25-50) following the imports, add the snippet there to keep the type cluster together.

- [ ] **Step 3.2: Add component state**

Inside `export default function Lens()`, find the existing state declarations (line 244 onward). Locate the line `const [capturedFrame, setCapturedFrame] = useState<CapturedFrame | null>(null)` at line 291. Immediately after it, add:

```typescript
  // 箭头标注:仅 stage==='ready' 子模式
  // arrows / draftArrow 坐标系 = capturedFrame 逻辑像素 (左上角为原点)
  const [drawMode, setDrawMode] = useState(false)
  const [arrows, setArrows] = useState<Arrow[]>([])
  const [draftArrow, setDraftArrow] = useState<Arrow | null>(null)
```

- [ ] **Step 3.3: Add stage-transition reset effect**

In `src/Lens.tsx`, the existing reset effects live in the cluster around line 400-650. Find a spot near the existing `useEffect` blocks that respond to `stage` changes (search for `if (stage !== ` or `setImagePreview('')` to locate them). Add this new effect — it can go right after the state declarations at line ~294 (any location works as long as it's inside the component body):

```typescript
  // 任何 stage 切换时强制清掉 draw 子模式 + 已落箭头
  useEffect(() => {
    if (stage !== 'ready') {
      setDrawMode(false)
      setArrows([])
      setDraftArrow(null)
    }
  }, [stage])
```

- [ ] **Step 3.4: Verify lint + typecheck**

Run these in parallel:

```bash
npm run typecheck
npm run lint
```

Expected: both pass. ESLint may warn about unused state setters — that's expected since later tasks consume them. If lint fails on `no-unused-vars` for `setDrawMode`/`setArrows`/`setDraftArrow` (it shouldn't because they're inside a component, but if the project has stricter rules), add `// eslint-disable-next-line` only as a temporary hold and remove it in Task 4.

- [ ] **Step 3.5: Commit**

```bash
git add src/Lens.tsx
git commit -m "feat: add arrow annotation type and state to Lens

Adds Arrow type, drawMode/arrows/draftArrow state, and a
stage-transition guard that clears them whenever Lens leaves
the ready stage. No UI is wired yet."
```

---

### Task 4: Lens.tsx — Compose helpers (drawArrow + composeAnnotatedImage)

**Files:**
- Modify: `src/Lens.tsx` (top-level helpers near other utilities like `makeThumbnail`)

This task adds the pure functions. They are not called by anything yet but compile cleanly and can be reasoned about in isolation.

- [ ] **Step 4.1: Add `drawArrow` helper**

In `src/Lens.tsx`, find `makeThumbnail` (line 66) — that's the canvas utility cluster. Add the following helpers in the same area (after `makeThumbnail`):

```typescript
function drawArrow(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  lineWidth: number,
) {
  const dx = x2 - x1
  const dy = y2 - y1
  const len = Math.hypot(dx, dy)
  if (len < 1) return

  const headSize = lineWidth * 4
  const angle = Math.atan2(dy, dx)
  const headAngle = (ARROW_HEAD_ANGLE_DEG * Math.PI) / 180

  // 箭杆终点回退一格,避免三角覆盖时尾端有缺口
  const shaftEndX = x2 - Math.cos(angle) * (headSize * 0.6)
  const shaftEndY = y2 - Math.sin(angle) * (headSize * 0.6)

  ctx.save()
  ctx.strokeStyle = ARROW_COLOR
  ctx.fillStyle = ARROW_COLOR
  ctx.lineWidth = lineWidth
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'

  ctx.beginPath()
  ctx.moveTo(x1, y1)
  ctx.lineTo(shaftEndX, shaftEndY)
  ctx.stroke()

  // 三角箭头
  const wing1X = x2 - Math.cos(angle - headAngle) * headSize
  const wing1Y = y2 - Math.sin(angle - headAngle) * headSize
  const wing2X = x2 - Math.cos(angle + headAngle) * headSize
  const wing2Y = y2 - Math.sin(angle + headAngle) * headSize
  ctx.beginPath()
  ctx.moveTo(x2, y2)
  ctx.lineTo(wing1X, wing1Y)
  ctx.lineTo(wing2X, wing2Y)
  ctx.closePath()
  ctx.fill()

  ctx.restore()
}
```

- [ ] **Step 4.2: Add `composeAnnotatedImage` helper**

Add immediately after `drawArrow`:

```typescript
async function composeAnnotatedImage(
  imageDataUrl: string,
  arrows: Arrow[],
  frameWidth: number,
  frameHeight: number,
): Promise<string> {
  const img = await new Promise<HTMLImageElement>((resolve, reject) => {
    const el = new Image()
    el.onload = () => resolve(el)
    el.onerror = () => reject(new Error('failed to load image for compose'))
    el.src = imageDataUrl
  })

  const canvas = new OffscreenCanvas(img.naturalWidth, img.naturalHeight)
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('OffscreenCanvas 2d context unavailable')

  ctx.drawImage(img, 0, 0)

  // 逻辑像素 → 物理像素的等比缩放
  // capturedFrame.width 是逻辑像素;PNG 是物理像素 → naturalWidth 大于等于 width
  const scaleX = frameWidth > 0 ? img.naturalWidth / frameWidth : 1
  const scaleY = frameHeight > 0 ? img.naturalHeight / frameHeight : 1
  const lineWidth = Math.max(3, img.naturalWidth / 400)

  for (const a of arrows) {
    drawArrow(
      ctx,
      a.x1 * scaleX,
      a.y1 * scaleY,
      a.x2 * scaleX,
      a.y2 * scaleY,
      lineWidth,
    )
  }

  const blob = await canvas.convertToBlob({ type: 'image/png' })
  const buf = await blob.arrayBuffer()
  let binary = ''
  const bytes = new Uint8Array(buf)
  const chunkSize = 0x8000
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize))
  }
  return btoa(binary)
}
```

- [ ] **Step 4.3: Verify typecheck + lint**

Run in parallel:

```bash
npm run typecheck
npm run lint
```

Expected: passes. If lint flags `composeAnnotatedImage`/`drawArrow` as unused, that warning will clear in the next task — leave it for now or use a single-line `// eslint-disable-next-line` (preferred: leave; the project's existing patterns suggest it's tolerant of unused helpers temporarily).

- [ ] **Step 4.4: Commit**

```bash
git add src/Lens.tsx
git commit -m "feat: add Canvas helpers for arrow composition

drawArrow renders one red arrow with the project geometry constants;
composeAnnotatedImage loads imagePreview into an OffscreenCanvas,
draws every arrow scaled to physical pixels, and returns base64 PNG
without the data: prefix."
```

---

### Task 5: Lens.tsx — SVG annotation layer + mouse interactions

**Files:**
- Modify: `src/Lens.tsx` (insert new JSX block right after the existing `capturedFrame` rectangle at line 1236-1260)

This task adds the visible drawing surface. After the commit, draw mode is reachable only via React DevTools (no toolbar button yet), but a manual `setDrawMode(true)` confirms the SVG renders + handlers behave.

- [ ] **Step 5.1: Add the SVG annotation layer**

In `src/Lens.tsx`, find the closing `</>` of the existing `{capturedFrame && stage !== 'select' && keepFullscreen && (...)}` block. After the line that closes that fragment (`)}` after `</> `, currently around line 1260), insert the new block:

```tsx
      {/* drawMode:在 capturedFrame 矩形内画箭头.frozen background = imagePreview;
          用 SVG 叠加 + 自带 mousedown/move/up,pointer-events 仅在 drawMode 启用 */}
      {capturedFrame && stage === 'ready' && keepFullscreen && drawMode && (
        <>
          {/* 截图框外加深(dim),让用户聚焦在截图区) */}
          <div
            className="absolute inset-0 pointer-events-none bg-black/40"
            style={{ zIndex: 10 }}
          />
          {/* capturedFrame 内:背景填充冻结 PNG + SVG 收事件 */}
          <div
            className="absolute"
            style={{
              left: capturedFrame.x,
              top: capturedFrame.y,
              width: capturedFrame.width,
              height: capturedFrame.height,
              backgroundImage: imagePreview ? `url("${imagePreview}")` : undefined,
              backgroundSize: '100% 100%',
              backgroundRepeat: 'no-repeat',
              cursor: 'crosshair',
              zIndex: 11,
            }}
            onMouseDown={(e) => {
              e.stopPropagation()
              const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect()
              const x = e.clientX - rect.left
              const y = e.clientY - rect.top
              setDraftArrow({ x1: x, y1: y, x2: x, y2: y })
            }}
            onMouseMove={(e) => {
              if (!draftArrow) return
              e.stopPropagation()
              const rect = (e.currentTarget as HTMLDivElement).getBoundingClientRect()
              const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left))
              const y = Math.max(0, Math.min(rect.height, e.clientY - rect.top))
              setDraftArrow(d => (d ? { ...d, x2: x, y2: y } : d))
            }}
            onMouseUp={(e) => {
              e.stopPropagation()
              if (!draftArrow) return
              const dx = draftArrow.x2 - draftArrow.x1
              const dy = draftArrow.y2 - draftArrow.y1
              if (Math.hypot(dx, dy) >= ARROW_MIN_DRAG_PX) {
                setArrows(prev => [...prev, draftArrow])
              }
              setDraftArrow(null)
            }}
          >
            <svg
              width={capturedFrame.width}
              height={capturedFrame.height}
              className="absolute inset-0 pointer-events-none"
              style={{ overflow: 'visible' }}
            >
              {arrows.map((a, i) => (
                <ArrowSvg key={i} arrow={a} />
              ))}
              {draftArrow && <ArrowSvg arrow={draftArrow} />}
            </svg>
          </div>
        </>
      )}
```

- [ ] **Step 5.2: Add the `ArrowSvg` sub-component**

In `src/Lens.tsx`, near the top-level helpers (next to `drawArrow` from Task 4), add a small SVG renderer that mirrors the canvas geometry:

```tsx
function ArrowSvg({ arrow }: { arrow: Arrow }) {
  const { x1, y1, x2, y2 } = arrow
  const dx = x2 - x1
  const dy = y2 - y1
  const len = Math.hypot(dx, dy)
  if (len < 1) return null

  // SVG 在逻辑像素坐标系下渲染 → 线宽用屏幕粗细,合成时再按 PNG 物理像素重算
  const lineWidth = 4
  const headSize = lineWidth * 4
  const angle = Math.atan2(dy, dx)
  const headAngle = (ARROW_HEAD_ANGLE_DEG * Math.PI) / 180

  const shaftEndX = x2 - Math.cos(angle) * (headSize * 0.6)
  const shaftEndY = y2 - Math.sin(angle) * (headSize * 0.6)
  const wing1X = x2 - Math.cos(angle - headAngle) * headSize
  const wing1Y = y2 - Math.sin(angle - headAngle) * headSize
  const wing2X = x2 - Math.cos(angle + headAngle) * headSize
  const wing2Y = y2 - Math.sin(angle + headAngle) * headSize

  return (
    <g>
      <line
        x1={x1}
        y1={y1}
        x2={shaftEndX}
        y2={shaftEndY}
        stroke={ARROW_COLOR}
        strokeWidth={lineWidth}
        strokeLinecap="round"
      />
      <polygon
        points={`${x2},${y2} ${wing1X},${wing1Y} ${wing2X},${wing2Y}`}
        fill={ARROW_COLOR}
      />
    </g>
  )
}
```

- [ ] **Step 5.3: Verify typecheck**

Run: `npm run typecheck`
Expected: passes. The `setDrawMode` setter is still unused publicly, but the JSX block consumes `drawMode` so that no longer triggers warnings.

- [ ] **Step 5.4: Manual sanity check (no commit yet)**

Run: `npm run dev`
- Open Lens via the configured hotkey, take a region screenshot
- In React DevTools, find the `Lens` component and toggle `drawMode` to `true`
- Confirm the dim layer + frozen PNG inside the capturedFrame rectangle render with crosshair cursor
- Drag inside the rectangle: a red arrow should follow + lock in on mouseup
- Toggle `drawMode` back to `false`: the SVG layer disappears, arrows state retained

If any rendering looks off (e.g., arrow drawn outside frame, dim covers frame), fix in this step before the commit.

- [ ] **Step 5.5: Commit**

```bash
git add src/Lens.tsx
git commit -m "feat: add SVG arrow drawing layer to Lens overlay

Renders inside capturedFrame when drawMode is on: dims background,
freezes the captured PNG as backdrop, captures mouse drags into
draftArrow / arrows. ArrowSvg mirrors the canvas geometry for live
preview. Toggle is not yet wired to the UI."
```

---

### Task 6: Lens.tsx — Toolbar toggle button + keyboard shortcuts

**Files:**
- Modify: `src/Lens.tsx` (input bar around line 1332-1362, plus a new useEffect for keyboard handling)

This task makes drawMode reachable via the toolbar and adds Cmd+Z undo + Esc exit.

- [ ] **Step 6.1: Import the arrow icon**

In `src/Lens.tsx`, find the `lucide-react` import line (search for `from 'lucide-react'`). Add `MousePointer2` to the import list:

```typescript
import {
  Loader2, Copy, Check, Square,
  Image as ImageIcon, ArrowUp,
  History as HistoryIcon, ChevronDown, Brain,
  MousePointer2,
} from 'lucide-react'
```

(Adjust to match the exact existing import style — single-line vs multi-line. Just add `MousePointer2` to whatever list is already there.)

- [ ] **Step 6.2: Add the toggle button in the input bar**

In `src/Lens.tsx`, find the `selectionLineCount` badge (line 1354-1361). Immediately after the closing `</span>` of that conditional block (the `)}` at line 1361), but still inside the `<div className="shrink-0 flex items-center gap-2">` parent, add the arrow toggle button:

```tsx
              {stage === 'ready' && (
                <button
                  type="button"
                  onClick={() => setDrawMode(m => !m)}
                  disabled={!imagePreview}
                  title={imagePreview
                    ? (drawMode ? t.lensArrowToggleOff : t.lensArrowToggle)
                    : t.lensArrowDisabledHint}
                  className={`shrink-0 w-8 h-8 rounded-lg flex items-center justify-center transition-colors ${
                    drawMode
                      ? 'bg-blue-500 text-white hover:bg-blue-600'
                      : 'text-neutral-600 dark:text-neutral-300 hover:bg-black/[0.05] dark:hover:bg-white/[0.06]'
                  } ${!imagePreview ? 'opacity-40 cursor-not-allowed' : ''}`}
                >
                  <MousePointer2 size={15} strokeWidth={1.75} />
                </button>
              )}
```

- [ ] **Step 6.3: Add Cmd+Z undo + Esc exit keyboard handler**

In `src/Lens.tsx`, find the location where existing keyboard listeners live (search for `addEventListener('keydown'` to locate the cluster). Add a new effect that only listens when `drawMode` is true:

```typescript
  // drawMode 键盘:Cmd+Z 撤销最后一支箭头,Esc 退出 drawMode(arrows 保留)
  useEffect(() => {
    if (!drawMode) return
    const onKey = (e: KeyboardEvent) => {
      // 输入框聚焦时不拦截,让用户继续打字
      const target = e.target as HTMLElement | null
      const isInput = target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA'

      if (e.key === 'Escape' && !isInput) {
        e.preventDefault()
        e.stopPropagation()
        setDrawMode(false)
        setDraftArrow(null)
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z' && !e.shiftKey && !isInput) {
        e.preventDefault()
        e.stopPropagation()
        setArrows(prev => prev.slice(0, -1))
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [drawMode])
```

The `capture` flag (`true` as third arg) ensures we run before the page-level Esc handler (which would close Lens entirely).

- [ ] **Step 6.4: Verify typecheck + lint**

Run in parallel:

```bash
npm run typecheck
npm run lint
```

Expected: both pass.

- [ ] **Step 6.5: Manual sanity check**

Run: `npm run dev`
- Take a screenshot. The arrow icon button should appear next to the thumbnail in the input bar.
- Click it: button turns blue, dim layer + frozen PNG appear inside the captured rectangle. Cursor is crosshair.
- Draw 3 arrows. Press Cmd+Z (or Ctrl+Z on Win): the last arrow disappears.
- Press Esc: drawMode exits, button returns to neutral. Re-click the button: previously-drawn arrows are still there (Esc preserves them).
- Click in the input field, type something, press Cmd+Z: the input's native undo runs (no arrow popped) — confirms the input-focus guard works.

- [ ] **Step 6.6: Commit**

```bash
git add src/Lens.tsx
git commit -m "feat: wire arrow draw toggle button + keyboard shortcuts

Adds a MousePointer2 toggle next to the input thumbnail (only
visible during stage='ready'); disabled while imagePreview is empty.
Cmd+Z pops the last arrow; Esc exits drawMode while keeping arrows.
Input-focus guard prevents conflict with native input undo."
```

---

### Task 7: Lens.tsx — Compose into handleSend

**Files:**
- Modify: `src/Lens.tsx` (handleSend at line 1031)

This task wires composition into the send path: arrows present → compose → register → swap to new image_id.

- [ ] **Step 7.1: Update `handleSend` to compose when arrows exist**

In `src/Lens.tsx`, replace the `handleSend` function (line 1031-1086) with the version below. Most of the body is unchanged — the new logic is the early branch that runs *before* the existing `flushSync` block and decides which `image_id` to send to the model.

Locate the function:

```typescript
  const handleSend = async () => {
    if (!input.trim() || streaming) return
    const question = input.trim()
    const id = imageIdRef.current
    setHistoryOpen(false)
    setInput('')
```

Modify the body so it becomes:

```typescript
  const handleSend = async () => {
    if (!input.trim() || streaming) return
    const question = input.trim()
    setHistoryOpen(false)
    setInput('')

    // 默认沿用当前 image_id;若有箭头则先合成 + 注册新图,把后续 ask 切到合成版
    let effectiveImageId = imageIdRef.current
    if (arrows.length > 0 && imagePreview && capturedFrame) {
      try {
        const base64 = await composeAnnotatedImage(
          imagePreview,
          arrows,
          capturedFrame.width,
          capturedFrame.height,
        )
        const result = await api.lensRegisterAnnotatedImage(base64)
        if (result.success && result.imageId) {
          effectiveImageId = result.imageId
          imageIdRef.current = result.imageId
          setImagePreview(`data:image/png;base64,${base64}`)
          setArrows([])
          setDraftArrow(null)
          setDrawMode(false)
        } else {
          console.warn('[lens-arrow] register annotated image failed:', result.error)
        }
      } catch (err) {
        console.warn('[lens-arrow] compose failed, fallback to original:', err)
      }
    }

    // 首轮 chat 注入:把启动时抓到的选中文本作为 [已选文本] 段前置到 user prompt;
    // 后续轮次(messages.length>0)严格不重复注入.translate 模式不到这里.
    const isFirstTurn = messages.length === 0
    const ctx = (isFirstTurn && mode === 'chat') ? selectionText.trim() : ''
    const userContent = ctx
      ? (lang === 'zh'
          ? `[已选文本]\n${ctx}\n\n[用户问题]\n${question}`
          : `[Selected Text]\n${ctx}\n\n[Question]\n${question}`)
      : question
    const userMsg: ExplainMessage = { role: 'user', content: userContent }
    const placeholder: ExplainMessage = { role: 'assistant', content: '' }
    // sendMessages:发给后端的 history(保留前面对话上下文 + 本次提问,最后一条是 user 提问)
    const sendMessages: ExplainMessage[] = [...messages, userMsg]
    flushSync(() => {
      setMessages([...sendMessages, placeholder])
      setStage('answering')
      setStreaming(true)
    })
    try {
      const result = await api.lensAsk(effectiveImageId || '', sendMessages)
      if (!result.success) {
        const errText = `${t.lensError}: ${result.error}`
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: errText }]
        })
      } else if (result.response) {
        // 非流式:把完整答案塞进占位 assistant;流式情况已在 onLensStream 累积,避免覆盖
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          if (last.content.length > 0) return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: result.response! }]
        })
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      setMessages(prev => {
        const last = prev[prev.length - 1]
        if (!last || last.role !== 'assistant') return prev
        return [...prev.slice(0, -1), { role: 'assistant', content: `${t.lensError}: ${msg}` }]
      })
    } finally {
      // ref 在 setStreaming(false) 之前置 true,让持久化 effect 在本次 rerun 中识别这是"流刚结束"路径
      justFinishedStreamRef.current = true
      setStreaming(false)
    }
  }
```

Key changes vs the original:
- Removed `const id = imageIdRef.current` near top; introduced `let effectiveImageId = imageIdRef.current` lower down
- Added the `arrows.length > 0` branch that composes + registers + updates state
- Replaced `await api.lensAsk(id || '', ...)` with `await api.lensAsk(effectiveImageId || '', ...)`

- [ ] **Step 7.2: Verify typecheck + lint**

Run in parallel:

```bash
npm run typecheck
npm run lint
```

Expected: both pass.

- [ ] **Step 7.3: Manual sanity check (the golden-path smoke)**

Run: `npm run dev`
- Take a region screenshot of a UI with multiple buttons / regions
- Toggle draw mode, draw 1 arrow pointing at a specific element
- Type a question like "what is this?" and press Enter
- The thumbnail in the input bar should immediately swap to the annotated version
- The vision model's response should reference the arrow direction (e.g., "the button you arrowed at...")
- Toggle history dropdown: the entry should show the annotated thumbnail
- Ask a follow-up: should not re-compose (arrows already cleared); should reuse the now-current annotated image_id

- [ ] **Step 7.4: Commit**

```bash
git add src/Lens.tsx
git commit -m "feat: wire arrow composition into Lens handleSend

When arrows are present, handleSend composes a PNG via Canvas,
registers it through lens_register_annotated_image to get a fresh
image_id, swaps imagePreview to the annotated data URL, and clears
arrow state. Failure path falls back to the original image with a
console warning. Follow-up questions reuse the annotated image."
```

---

### Task 8: Manual smoke test sweep

**Files:** none (verification only — no commit unless a fix is needed)

Run through every item in the spec's smoke checklist before declaring done. Each unchecked item is a blocker for the PR.

- [ ] **Step 8.1: macOS region screenshot → 1 arrow → ask → vision model references the arrow**

Run: `npm run dev` (on macOS)
- Lens hotkey → drag region → draw 1 arrow → ask "what does this arrow point to?"
- Expected: model output specifically describes the area the arrow indicates

- [ ] **Step 8.2: macOS window screenshot → 3 arrows → Cmd+Z undo → ask**

- Lens hotkey → click a window → toggle draw mode → draw 3 arrows on different elements
- Press Cmd+Z once → 3rd arrow disappears
- Ask "what are these two arrows pointing at?" → model should describe both remaining arrows

- [ ] **Step 8.3: Regression — no arrows → direct send (must use original path)**

- Lens hotkey → screenshot → type a question → Enter (no arrows)
- Expected: send works exactly as before; no `lens-arrow` console warnings; `imageIdRef` unchanged from capture

- [ ] **Step 8.4: Esc closes Lens → reopen → no residual arrows**

- Lens hotkey → screenshot → draw 2 arrows → Esc (closes whole Lens overlay)
- Trigger Lens again → take a new screenshot
- Expected: no SVG / arrow remnants visible; `arrows` is empty

- [ ] **Step 8.5: Windows region screenshot → arrows → DPI scaling correct**

(If a Windows machine is available)
- Lens hotkey on Windows → drag region → draw an arrow tip exactly on a 10×10 button
- Expected: in the composed PNG (check the archive directory), the arrow tip lands on the same pixel cluster as in the live preview

- [ ] **Step 8.6: 4K + Retina + standard display**

- Same as 8.5 across at least two DPI conditions
- Expected: arrow line width looks proportional to screenshot resolution (thicker on 4K, thinner on standard)

- [ ] **Step 8.7: Archive directory has 2 entries per annotated send**

- Configure image archive directory (Settings → image archive)
- Take a screenshot → draw arrow → send
- Expected: the archive directory contains 2 PNGs with timestamps within a few seconds — original capture (pre-arrow) and annotated version

- [ ] **Step 8.8: Compose-failure fallback**

- Temporarily inject a failure into `composeAnnotatedImage` (e.g., `throw new Error('test')` at the top of the function)
- Take screenshot, draw arrow, send
- Expected: console.warn fires; the original image is sent to the model; UI does not break
- Revert the injected failure before committing

If any item fails, fix the bug, re-run the failing item plus the regression item (8.3), and commit the fix as a separate commit referencing the failure mode.

- [ ] **Step 8.9: (Optional) tag the commit chain**

Once all 8 manual checks pass, the feature is shippable. Do not tag a release here — that's the user's call.

---

## Self-Review

After running through the plan once before execution, fix any of these inline:

**Spec coverage check:** Every section in the spec is now mapped to a task:
- §Architecture / Components → Tasks 1, 3, 5, 6, 7
- §Type definitions → Task 3
- §State machine reset → Task 3 (Step 3.3)
- §Send path data flow → Task 7
- §composeAnnotatedImage / arrow geometry → Task 4
- §Interaction details (toggle, drag, keyboard) → Tasks 5, 6
- §Backend command → Task 1
- §Error handling fallbacks → Task 7 (try/catch + console.warn)
- §Edge cases (history restore, follow-up, Esc, clamp, min-drag, drawMode outside frame, input typing, capture source) → covered by Task 3 reset effect + Task 5 mouse handlers (clamp, min-drag, pointer-events)
- §i18n → Task 2
- §Test plan → Task 8

**Placeholder scan:** No "TBD"/"TODO"/"add appropriate"/"similar to Task N"/etc. Every code block contains literal code, every command has its expected output, every commit has its message.

**Type/method consistency:** `Arrow` type used identically in Tasks 3-7. `composeAnnotatedImage(imagePreview, arrows, capturedFrame.width, capturedFrame.height)` matches its definition in Task 4. `api.lensRegisterAnnotatedImage(base64)` matches the binding in Task 1. `setDrawMode` / `setArrows` / `setDraftArrow` setters from Task 3 used in Tasks 5-7.

---

## Done Criteria

- All 8 tasks committed
- All Step 8.* manual smoke checks pass
- `npm run typecheck` and `npm run lint` clean on the final commit
- `cargo build --manifest-path src-tauri/Cargo.toml` clean on the final commit
- The spec under `docs/superpowers/specs/2026-05-03-screenshot-arrow-annotation-design.md` matches the shipped behavior (no spec drift)
