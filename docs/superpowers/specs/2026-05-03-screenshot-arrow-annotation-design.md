# 截图箭头标注 — 设计文档

> 状态:Approved · 日期:2026-05-03

## 一句话目标

让用户在 Lens 截图后、发给 vision 模型前,在 overlay 原位置拖出红色箭头,箭头烧入 PNG 后再发送。

## 背景

当前(v2.5.0)Lens 流程:

1. Hotkey 触发 Lens overlay
2. 用户点窗口或拖区域 → `lens_capture_window` / `lens_capture_region`
3. 后端用 SCK(macOS)/ xcap(Windows)生成 PNG → `temp_dir/lens-<uuid>.png`
4. 自动归档到用户配置目录(v2.5.0)
5. 注册 `image_id` 进 `AppState.images`
6. 前端进入 `stage='ready'`,显示输入栏 + thumbnail
7. 用户输入问题 → vision API 接收 base64 PNG + question

**痛点:** 截图发给模型时无法"指点"模型看哪里。比如截图里有 3 个按钮,用户问"这个怎么用",模型不知道哪个是"这个"。

**这次解的范围:** 仅"发送前用箭头指点"。

## 决策摘要

| 维度 | 选择 | 替代方案 | 否决理由 |
|---|---|---|---|
| 介入时机 | 发送前(在调 vision API 之前) | 发送后 / 独立工具 / 弹菜单 | 用户场景是"我要让 AI 看这里",B/C/D 都偏离 |
| 工具集 | 仅箭头 | 标准(+ 框/圆/文字)/ 全套(+ 打码裁剪) | YAGNI,先验证主路径 |
| 画布位置 | Lens overlay 原位 | 弹模态编辑器 | 80% 用户不画箭头时,模态多一步操作打扰 |
| 像素合成 | 前端 Canvas | 后端 image crate / 仅传坐标给 AI | 改动最小,不引新 Rust 依赖,坐标合成 vision 模型理解差 |

**Out of scope(YAGNI):**
- 颜色 picker(固定红色)
- 矩形 / 椭圆 / 文字 / 自由画笔 / 打码 / 裁剪
- 重做(Cmd+Shift+Z)
- 已落下箭头的选中 / 拖动 / 单独删除(只有 Cmd+Z 全局撤销)
- 编辑器独立窗口
- 翻译模式(`stage='translating'`)的标注

## 架构

### 组件改动

```
┌────────────────────────────────────────────────┐
│ Lens.tsx                                       │
│                                                │
│  state:                                        │
│   + drawMode: boolean                          │
│   + arrows: Arrow[]                            │
│   + draftArrow: Arrow | null                   │
│                                                │
│  UI 新增:                                      │
│   + 输入栏左侧 toggle 按钮                     │
│   + capturedFrame 内 SVG 层(箭头渲染)         │
│   + drawMode 时 capturedFrame 背景填 PNG       │
│   + drawMode 时 cursor: crosshair              │
│                                                │
│  hooks:                                        │
│   + composeAnnotatedImage(png, arrows, frame)  │
│   + handleSend 注入合成步骤                    │
│   + Cmd+Z 撤销 / Esc 退出 drawMode             │
└────────────────────────────────────────────────┘
                       ↓ invoke
┌────────────────────────────────────────────────┐
│ src-tauri/src/main.rs                          │
│                                                │
│ + lens_register_annotated_image                │
│   (base64_png) → image_id                      │
│                                                │
│   1. base64 decode → PNG bytes                 │
│   2. write to temp_dir/lens-<uuid>.png         │
│   3. archive_captured_image(...)               │
│   4. images_lock().insert(new_id, path)        │
│   5. current_id_lock() = new_id                │
│   6. 不删除 original(由 cleanup 路径管)        │
└────────────────────────────────────────────────┘
```

### 类型定义

```ts
type Arrow = { x1: number; y1: number; x2: number; y2: number }
// 坐标:相对 capturedFrame 左上角的逻辑像素(等同 capturedFrame.width/height 坐标系)
// 颜色固定 #ff3b30,不存到 Arrow 里
```

### Lens 状态机

`stage` 类型不变(`'select' | 'ready' | 'answering' | 'translating' | 'translated'`)。

`drawMode` 是 `'ready'` 阶段的子模式 —— 只有 `stage === 'ready'` 时 toggle 才有意义。任何 stage 切换时强制 reset:

```ts
useEffect(() => {
  if (stage !== 'ready') {
    setDrawMode(false)
    setArrows([])
    setDraftArrow(null)
  }
}, [stage])
```

## 数据流

### Send 路径

```
handleSend()
  ├─ if arrows.length === 0:
  │   └─ effectiveImageId = currentImageId  (现有路径,无变化)
  │
  └─ else:
      ├─ base64 = await composeAnnotatedImage(imagePreview, arrows, capturedFrame)
      ├─ result = await api.lensRegisterAnnotatedImage(base64)
      ├─ if !result.success:
      │   └─ console.warn + effectiveImageId = currentImageId  (回退原图)
      └─ else:
          ├─ effectiveImageId = result.imageId
          ├─ setImagePreview('data:image/png;base64,' + base64)  // 让 thumbnail / history 都用合成版
          ├─ setArrows([])
          └─ setDrawMode(false)

→ 后续 explainStream/vision API 调用使用 effectiveImageId
```

**会话内追问**:第一次合成后 `arrows` 已清空 + `currentImageId` 已切到合成版,后续追问不再走合成路径。

**history 一致性**:`imagePreview` 在合成成功后立即更新为合成版的 data URL。现有 history `useEffect` 监听 `imagePreview` 变化,自动用 `makeThumbnail` 生成缩略图,因此 history 里看到的就是带箭头的版本。

### 前端合成函数

```ts
async function composeAnnotatedImage(
  imageDataUrl: string,
  arrows: Arrow[],
  frame: CapturedFrame
): Promise<string> {
  const img = await loadImage(imageDataUrl)
  const canvas = new OffscreenCanvas(img.naturalWidth, img.naturalHeight)
  const ctx = canvas.getContext('2d')!
  ctx.drawImage(img, 0, 0)

  const scale = img.naturalWidth / frame.width
  const lineWidth = Math.max(3, img.naturalWidth / 400)

  for (const a of arrows) {
    drawArrow(
      ctx,
      a.x1 * scale, a.y1 * scale,
      a.x2 * scale, a.y2 * scale,
      lineWidth
    )
  }

  const blob = await canvas.convertToBlob({ type: 'image/png' })
  return await blobToBase64(blob)  // 返回不带 data: 前缀的 base64 字符串
}
```

### 箭头几何参数(写死)

| 参数 | 值 |
|---|---|
| 颜色 | `#ff3b30` |
| 线宽(物理像素) | `max(3, img.naturalWidth / 400)` |
| 头部三角边长 | `4 × lineWidth` |
| 头部夹角 | 30° |
| `lineCap` / `lineJoin` | `round` |

## 交互细节

### 进入 / 退出 draw 态

- 输入栏左侧新增箭头 toggle 按钮(在 thumbnail 右边、`selectionLineCount` 徽章那一带)
- 按钮点击 → 切 `drawMode`
- `drawMode === true` 时:按钮高亮蓝色;`capturedFrame` 内背景填 PNG;光标 `crosshair`;capturedFrame 外 dim 加深(`bg-black/40`)
- `imagePreview === ''` 时按钮 `disabled`

### 拖出箭头

```
mousedown (in capturedFrame, drawMode=true)
  → setDraftArrow({ x1, y1, x2:x1, y2:y1 })

mousemove
  → setDraftArrow(d => ({ ...d, x2: clamp(...), y2: clamp(...) }))
  // clamp 到 [0, frame.width] × [0, frame.height]

mouseup
  → if hypot(x2-x1, y2-y1) >= 8: arrows.push(draftArrow)
  → setDraftArrow(null)
  // 不关闭 drawMode,允许连续画
```

### 键盘

| 按键 | 行为 | 条件 |
|---|---|---|
| `Cmd+Z` / `Ctrl+Z` | `arrows.pop()` | drawMode 时拦截;输入框聚焦时不拦 |
| `Esc` | `setDrawMode(false)` | 已落下的 `arrows` 保留;再次开 toggle 可继续画 |
| 普通输入 / Enter | 不受影响 | input 元素正常工作,Enter 触发 send |

## 后端

### 新命令 `lens_register_annotated_image`

```rust
#[tauri::command]
async fn lens_register_annotated_image(
  app: AppHandle,
  state: State<AppState>,
  base64_png: String,
) -> Result<serde_json::Value, String> {
  let bytes = BASE64_STANDARD
    .decode(&base64_png)
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

  // 原 image_id 对应文件保留,由现有 cleanup 路径(下次截图 / Lens 关闭)清理。
  // 这样 history 恢复时如果引用了原图 image_id 也不会立刻指向死路径。

  Ok(serde_json::json!({ "success": true, "imageId": image_id }))
}
```

注册到 `tauri::generate_handler![]` 中。

### 归档行为

- **原图归档**(现有行为,保留):capture 完成后 `archive_captured_image` 自动归档
- **合成图归档**(新增):`lens_register_annotated_image` 内再调一次 `archive_captured_image`
- 同一次截图归档 **2 个文件**(原图 + 标注版),文件名时间戳相邻几秒。**有意保留两份**,不去重 — 用户既能看到原图也能看到带标注版。

### 不复用现有 `explain_read_image`

读图函数走的是 `images_lock` 查路径 → 读盘 → base64 → 返回。新命令是反向(写新图 + 注册),独立成一个 command 更清晰。

## 错误处理

| 失败点 | 行为 |
|---|---|
| `composeAnnotatedImage` 抛异常 | 回退原 image_id,`console.warn`,不阻断 send |
| `lens_register_annotated_image` 写失败 | 同上,回退原图 |
| archive 失败 | best-effort(`archive_captured_image` 已是后台 spawn,不阻塞) |
| `imagePreview === ''` 时进入 drawMode | toggle 按钮 `disabled` |

## 边缘情况

1. **history 恢复** — `setHistoryItem` 后 `setArrows([])` + `setDrawMode(false)`。history 记录的是合成后的图(因为 history 在 send 之后写入)
2. **会话内追问** — 第一次 send 已切到合成版 image_id,后续追问不重复合成
3. **Esc 关闭 Lens** — 现有清理路径(`stage='select'` 时 reset)自动清空 arrows
4. **拖动出 capturedFrame 边界** — `mousemove` 把坐标 clamp 到 frame 内
5. **极短拖动**(< 8 px) — 丢弃 draftArrow,防误触
6. **drawMode 时点击 capturedFrame 外** — 不响应,光标保持 crosshair
7. **输入栏在 drawMode 下仍可打字** — SVG `pointer-events` 限定 capturedFrame 矩形内
8. **window 截图 vs region 截图** — `capturedFrame` 字段对两种来源同语义,无差异

## 测试

| 类型 | 范围 |
|---|---|
| Rust 单测 | 不新增 — `lens_register_annotated_image` 是 IO + state 更新,与 `lens_capture_*` 同模式,沿用人工验证 |
| 前端单测 | 项目无前端测试基础设施 — 不新增 |
| 手工冒烟(必跑) | 见下表 |

**手工冒烟清单**

- [ ] macOS 区域截图 → 1 支箭头 → 提问 → vision 模型回应能引用箭头方向
- [ ] macOS 窗口截图 → 3 支箭头 → Cmd+Z 撤销最后一支 → 提问
- [ ] 不画箭头直接 send(回归 — 走原路径,不应触发合成)
- [ ] 画箭头时 Esc 关闭 Lens → 重开一次截图 → 不应有残留 SVG / arrows
- [ ] Windows 区域截图 → 加箭头 → 坐标缩放正确(不同 DPI)
- [ ] 4K 屏 / 普通屏 / Retina 不同 scale_factor 下箭头粗细看起来合理
- [ ] 归档目录:一次截图 + 标注 send 后,归档目录里应有 2 个文件
- [ ] 合成失败注入测试:临时让 `composeAnnotatedImage` 抛异常 → 验证回退到原图发送

## 改动文件清单

| 文件 | 改动类型 | 大致行数 |
|---|---|---|
| `src/Lens.tsx` | 主体改动 | ~150-200 行新增 |
| `src/api/tauri.ts` | 新增 `lensRegisterAnnotatedImage` | ~5 行 |
| `src-tauri/src/main.rs` | 新增命令 + invoke_handler 注册 | ~30 行 |
| `src/settings/i18n.ts` | toggle 按钮 tooltip 文案,4 语言 | ~8 行 |

## 依赖

- 前端:无新增(Canvas API 原生)
- 后端:无新增(`base64` crate 已在,`Uuid` 已在,`std::fs::write` 标准库)

## 不在本次范围内

- 颜色 / 粗细可配
- 多种标注图元
- 撤销以外的编辑(选中已落箭头、拖动、改色等)
- 标注模式独立窗口 / 模态
- 翻译路径标注
- 标注与历史记录的差异化展示(history 里区分原图 vs 标注版)
