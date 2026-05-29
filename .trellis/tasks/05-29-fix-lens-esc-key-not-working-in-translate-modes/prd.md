# fix lens esc key not working in translate modes

## Goal

截图翻译（translate）和选中文本翻译（translateText）完成后，按 Esc 无法关闭 Lens 窗口，必须先点一下窗口才行。原因是 macOS accessory app 下无边框 overlay 不是 key window，全局 keydown 收不到事件。

## Root Cause Analysis

### 现有修复方案（Lens.tsx 工作区改动）

已添加 `keyboardCaptureRef`（隐藏文本 input）+ 在 translate/translateText 完成后调用 `focusLensInput`，逻辑上正确，但 **隐藏 input 尺寸为 `w-0 h-0`（0×0px）**。

### 核心问题：0×0px 元素无法触发 WebKit key window

macOS WebKit 的行为：调用 `element.focus()` 能让 DOM focus 生效，但 **触发 WebKit 主动请求 key window（使 overlay 成为接收全局键盘事件的窗口）** 需要元素有真实的渲染尺寸（至少 1×1px）。0×0px 元素的 focus 不会触发这个 key window 请求，因此 keydown listener 仍收不到事件。

### translate 模式（截图翻译）调用链验证

```
handleCaptureWindow/Region
  capturingRef = true
  await flyBarToAnchor(...)
    → animateFullscreenBarToAnchor
      → focusLensInput([TRANSITION_MS+20, ...])  ← 注册延迟回调
  capturingRef = false  (finally)
  ↓ 延迟后 run() 执行：
    canFocusWindow() → capturingRef=false ✓, stage='translating'/'translated' ✓
    keyboardCaptureRef.current.focus()  ← 但元素 0×0，不触发 key window
```

### translateText 模式（选中文本翻译）调用链验证

```
flushSync → stage = 'translating'
await takeLensSelection()
  focusLensInput([0, 80, 200])  ← delay=0 立即执行
    canFocusWindow() → capturingRef=false ✓, stage='translating' ✓
    keyboardCaptureRef.current.focus()  ← 同上，0×0，不触发 key window
```

## Fix

`w-0 h-0` → `w-px h-px`（1×1px，仍不可见）：

```tsx
className="fixed opacity-0 pointer-events-none w-px h-px"
style={{ left: -9999, top: -9999 }}
```

同时改 `absolute` → `fixed`，避免在某些渲染路径下受父级 `overflow: hidden` 裁剪（Lens overlay 虽全屏，防御性更强）。

## Requirements

- translate 和 translateText 模式完成后 Esc 直接关闭，无需先点窗口
- 隐藏 input 视觉上完全不可见、不可点击
- 不影响 chat 模式的现有行为

## Acceptance Criteria

- [ ] 截图翻译完成后按 Esc 关闭 Lens
- [ ] 选中文本翻译出现后按 Esc 关闭 Lens
- [ ] chat 模式 Esc 行为不变

## Out of Scope

- Windows 平台（Win 上焦点机制不同，待独立验证）
- 其他 Lens 功能

## Technical Notes

- 文件：`src/Lens.tsx`，line 1631–1648
- 只需改隐藏 input 的 className：`w-0 h-0` → `w-px h-px`，`absolute` → `fixed`
- `keyboardCaptureRef`、`focusLensInput` 逻辑本身正确，不需要改
