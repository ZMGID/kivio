# 设置页面 — 输入翻译/截图翻译命名与归类整理 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把"输入翻译"快捷键从`基础` tab 搬到(改名后的)`输入翻译` tab,并把模糊的 tab 名`翻译`/`截图`改成`输入翻译`/`截图翻译`,让首次接触的用户一眼能区分两条链路。

**Architecture:** 纯 UI/i18n 整理。后端不动。持久化字段 `settings.hotkey` 不动,只改它在 UI 中的归属位置和外露字符串。涉及 2 个文件:`src/settings/i18n.ts`(3 条 zh + 2 条 en 字符串改值)和 `src/Settings.tsx`(删一个 section,在另一处插同样 section)。

**Tech Stack:** TypeScript, React 18, Vite, lucide-react icons。验证手段:`npm run typecheck` / `npm run lint` / 手动 smoke(项目无前端单测/e2e 框架,见 CLAUDE.md)。

参考设计文档: `docs/superpowers/specs/2026-05-05-settings-translation-rename-design.md`。

---

## File Structure

| File | 改动 |
|---|---|
| `src/settings/i18n.ts` | 改 zh 块的 `tabTranslate` / `tabScreenshot` / `hotkey` 三条;改 en 块的 `tabTranslate` / `tabScreenshot` 两条(en `hotkey: 'Hotkey'` 已是中性,不动)。 |
| `src/Settings.tsx` | 删`general` tab 内的`{/* 快捷键 */}` section(当前 807-821 行);在`translate` tab `<div>` 内(当前 949 行后),作为第一个子节点插入相同 JSX,放在`{/* 目标语言 */}` 之前。 |

**不改的:** 后端、`settings.hotkey` 字段名、`recordingTarget` 联合类型、`HotkeyInput` 组件、tray 注册逻辑、provider/lens tab 内部内容。

---

### Task 1: 更新 i18n 字符串

**Files:**
- Modify: `src/settings/i18n.ts`

- [ ] **Step 1: 改 zh `tabTranslate`**

在 `src/settings/i18n.ts` 中:

OLD:
```ts
    tabTranslate: '翻译',
```
NEW:
```ts
    tabTranslate: '输入翻译',
```

- [ ] **Step 2: 改 zh `tabScreenshot`**

OLD:
```ts
    tabScreenshot: '截图',
```
NEW:
```ts
    tabScreenshot: '截图翻译',
```

- [ ] **Step 3: 改 zh `hotkey`**

OLD:
```ts
    hotkey: '翻译快捷键',
```
NEW:
```ts
    hotkey: '快捷键',
```

- [ ] **Step 4: 改 en `tabTranslate`**

OLD:
```ts
    tabTranslate: 'Translate',
```
NEW:
```ts
    tabTranslate: 'Input Translation',
```

- [ ] **Step 5: 改 en `tabScreenshot`**

OLD:
```ts
    tabScreenshot: 'Screenshot',
```
NEW:
```ts
    tabScreenshot: 'Screenshot Translation',
```

(英文 `hotkey: 'Hotkey'` 已经是中性词,无需改。)

- [ ] **Step 6: typecheck 与 lint 通过**

Run:
```bash
npm run typecheck
npm run lint
```
Expected: 两条都成功退出(`i18n` 是带 `Lang` 类型的 typed object literal,key 拼错或中英文 keyset 不一致会被 `tsc` 拦下)。

- [ ] **Step 7: Commit**

```bash
git add src/settings/i18n.ts
git commit -m "$(cat <<'EOF'
refactor(settings): rename translate/screenshot tabs and generalize hotkey label
EOF
)"
```

---

### Task 2: 把快捷键 section 从`基础`tab 搬到`输入翻译`tab

**Files:**
- Modify: `src/Settings.tsx`

- [ ] **Step 1: 删除`general` tab 内的`{/* 快捷键 */}` section**

在 `src/Settings.tsx` 中找到下面这段(当前位于 807-821 行,后面跟一个空行):

OLD:
```tsx
            {/* 快捷键 */}
            <section>
              <SectionTitle icon={Keyboard}>{t.hotkey}</SectionTitle>
              <div className="settings-card overflow-hidden px-4 py-3">
                <HotkeyInput
                  value={settings.hotkey}
                  placeholder={t.hotkeyPlaceholder}
                  recording={recordingTarget === 'main'}
                  onToggleRecording={() => toggleRecording('main')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                />
              </div>
            </section>

```

NEW: (整段删掉,包括尾随空行;结果是 `</section>`(外观)→ 空行 → `{/* 行为 */}`)

```
```

具体可用 Edit 工具:`old_string` 为上面 OLD 的整段(含尾随空行 + 缩进),`new_string` 为空字符串。

- [ ] **Step 2: 在`translate` tab 顶部插入同样的 section**

找到 translate tab 入口(当前 947-950 行):

OLD:
```tsx
        {/* ===== 翻译设置标签页 ===== */}
        {activeTab === 'translate' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 目标语言 */}
```

NEW(在 `<div>` 与 `{/* 目标语言 */}` 之间插入完整 section,保持 12 空格缩进与上下空行风格一致):

```tsx
        {/* ===== 翻译设置标签页 ===== */}
        {activeTab === 'translate' && (
          <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-300">
            {/* 快捷键 */}
            <section>
              <SectionTitle icon={Keyboard}>{t.hotkey}</SectionTitle>
              <div className="settings-card overflow-hidden px-4 py-3">
                <HotkeyInput
                  value={settings.hotkey}
                  placeholder={t.hotkeyPlaceholder}
                  recording={recordingTarget === 'main'}
                  onToggleRecording={() => toggleRecording('main')}
                  recordLabel={t.hotkeyRecord}
                  recordingLabel={t.hotkeyRecording}
                  recordingPlaceholder={t.hotkeyRecordingPlaceholder}
                />
              </div>
            </section>

            {/* 目标语言 */}
```

- [ ] **Step 3: 验证依赖标识符仍在作用域**

被搬动的 JSX 引用了:`Keyboard`(lucide 图标)、`SectionTitle`、`HotkeyInput`、`t`、`settings.hotkey`、`recordingTarget`、`toggleRecording`。这些标识符在 `Settings.tsx` 顶部 import / 函数顶层均已声明,搬动只是改 JSX 位置,不需要新加 import。

```bash
grep -n "Keyboard,\|SectionTitle,\|HotkeyInput," src/Settings.tsx | head
```
Expected: 找到原有 import 语句,行号未消失。例如:
```
12:import { buildHotkey, getPlatform } from './settings/utils'
16:  SectionTitle, PermissionItem, HotkeyInput, DefaultPrompt,
```
(具体行号可能微变,关键是仍然存在。)

- [ ] **Step 4: typecheck 与 lint 通过**

```bash
npm run typecheck
npm run lint
```
Expected: 两条都成功退出。

- [ ] **Step 5: Commit**

```bash
git add src/Settings.tsx
git commit -m "$(cat <<'EOF'
refactor(settings): move input translation hotkey to Input Translation tab
EOF
)"
```

---

### Task 3: 手动 smoke 验证

**Files:** 不改代码(仅验证)

按 `CLAUDE.md`:"There is no frontend unit/e2e test runner configured. Manual smoke testing is required after changes that affect app flows."

按用户的持久化记忆:测试期间**不要**清空 settings 或 providers,保留已配置的 API key。

- [ ] **Step 1: 启动开发版**

```bash
npm run dev
```
等到 Tauri shell 与 Vite 都打印就绪后再继续。

- [ ] **Step 2: 中文界面验证(`settingsLanguage = 'zh'`)**

打开设置(翻译器小窗右上角的齿轮),逐项核对:
- 左侧 tab 列表显示:`基础` / `输入翻译` / `截图翻译` / `Lens` / `模型` / `关于`
- `基础` tab 内容顺序:外观 → 行为 → 截图自动归档 → 权限(macOS)。**没有**快捷键 section。
- `输入翻译` tab 内容顺序:**快捷键**(新增) → 目标语言 → 翻译引擎 → 输入翻译提示词
- `截图翻译` tab 中,原来写"翻译快捷键"的 label 现在写`快捷键`
- `Lens` tab 中,同上,label 写`快捷键`

- [ ] **Step 3: 切到英文界面再验证**

在`基础` → `Language` 下拉里选 `English`。重新打开设置(若 UI 不立即重渲染),核对:
- Sidebar tabs:`General` / `Input Translation` / `Screenshot Translation` / `Lens` / `Models` / `About`
- 三处 hotkey label 都仍读 `Hotkey`(英文 i18n 未改,符合预期)

- [ ] **Step 4: 在新位置重录一次输入翻译热键并保存**

`Input Translation` tab → `Hotkey` section → 点 `Record` → 按一个新组合(例如 `Cmd+Alt+T`)→ 退出录制 → 点 `Save`。

关闭设置,重新打开 → 新组合应仍显示在 `Hotkey` 字段。

退出设置回到任意其它 app,按下新组合 → translator 小窗弹出。

- [ ] **Step 5: 回归未触动的链路**

- 按下截图翻译热键 → Lens overlay 进 translate 模式,正常截图 + 翻译。
- 按下 Lens 热键 → Lens overlay 进 chat 模式,可正常对话。

(这一步确认 `settings.hotkey` 字段语义未变,也没有遗留的孤立绑定。)

- [ ] **Step 6: 控制台无新报错**

打开 DevTools(`npm run dev` 自带,或 `npm run dev:ui` 用浏览器 DevTools),`Console` 选项卡应保持干净,无新增 React/runtime 报错或 warn。

- [ ] **Step 7: 验证完成,不需要 commit**

(本任务仅验证,无代码改动。)

---

## 完成判据

- 两次 commit(Task 1、Task 2 各一)进入主分支
- typecheck / lint 通过
- Task 3 全部勾选完成
- `settings.json` 中已存在的 `hotkey` 字段值在升级前后不变(回归保证)
