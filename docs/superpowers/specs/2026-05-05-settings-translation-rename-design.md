# 设置页面 — 输入翻译与截图翻译命名/归类整理 — 设计文档

> 状态:Pending · 日期:2026-05-05

## 一句话目标

把"输入翻译"的快捷键从`基础` tab 搬到`输入翻译` tab,同时把 tab 名称从模糊的`翻译`/`截图`改成`输入翻译`/`截图翻译`,让第一次接触的用户一眼能区分两条链路。

## 背景

当前(v2.5.3)`Settings.tsx` 的 tab 划分:

| Tab id | zh / en | 内容 |
|---|---|---|
| `general` | 基础 / General | 外观、**快捷键(实为输入翻译热键)**、行为、截图自动归档、权限 |
| `translate` | 翻译 / Translate | 目标语言、翻译引擎、输入翻译提示词(无热键) |
| `screenshot` | 截图 / Screenshot | 截图翻译开关/热键/模型/原文/思考/流式/系统OCR/全屏/自定义提示词 |
| `lens` | (Lens) | Lens 开关/热键/... |
| `providers` / `about` | — | 不在此次范围 |

**痛点:**

1. 输入翻译的热键放在`基础`,跟`翻译` tab 的提示词、模型、目标语言被人为切开;新用户到`翻译` tab 找热键找不到。
2. `t.hotkey = '翻译快捷键'` 这条 i18n 字符串在`截图`tab 和 `Lens` tab 当 label 复用,用户在"截图翻译热键"那一行看到"翻译快捷键"会觉得跑题。
3. tab 标签`翻译`和`截图`本身是模糊的——三个 tab(翻译 / 截图 / Lens)其实都在做"翻译/AI 处理",`翻译`是"哪一种翻译"看不出来,`截图`是"截图做什么"看不出来。

**这次解的范围:** 仅做命名与位置的微整理,不改持久化字段,不改翻译质量/UI 视觉/交互。

## 决策摘要

| 维度 | 选择 | 替代方案 | 否决理由 |
|---|---|---|---|
| 整理深度 | B(移动 + 重命名) | A(只移动) / C(B + 目标语言提到全局) | A 没解决 tab 名歧义;C 涉及目标语言三方共享,改动面比"基础整理"大,留作下次 |
| 持久化字段 | 不动 | 同步重命名 `settings.hotkey` | 重命名会破坏已发布版本的 settings.json 兼容,且无收益 |
| `t.hotkey` 字符串 | 改成通用"快捷键" | 拆成 `inputTranslateHotkey` / `screenshotHotkey` / `lensHotkey` 三条 | 拆分多此一举,tab/section 上下文已经能去歧义 |
| 是否同步本次去打磨翻译质量 | 否 | 同时打磨 prompt/语言检测/流式/UX | 用户明确说"先聚焦设置归类",其他留给后续轮次 |

**Out of scope(本次不做):**

- 翻译质量(prompt 调优、auto 语言检测扩到 ja/ko/fr/de、glossary)
- 输入翻译流式 / 重试按钮 / 复制按钮 / 历史
- 截图翻译卡片视觉重做
- 把目标语言提到全局
- 删除/重排其他 tab(Lens / 模型 / 关于)

## 架构

### 改动文件

```
src/settings/i18n.ts        — 3 条 i18n key 改值
src/Settings.tsx            — 1 段 section 删除 + 1 段 section 新增
```

### i18n 字符串改动

| key | 旧 zh | 新 zh | 旧 en | 新 en |
|---|---|---|---|---|
| `tabTranslate` | 翻译 | **输入翻译** | Translate | **Input Translation** |
| `tabScreenshot` | 截图 | **截图翻译** | Screenshot | **Screenshot Translation** |
| `hotkey` | 翻译快捷键 | **快捷键** | Hotkey | Hotkey(已是中性,不改) |

`hotkey` 这条改完,会同步影响:

- `Settings.tsx:809` `<SectionTitle>{t.hotkey}</SectionTitle>` —— 这块整段会被搬走,源位置删除
- `Settings.tsx:1033` 截图 tab `<span>{t.hotkey}</span>` —— 显示从"翻译快捷键"变"快捷键",更准
- `Settings.tsx:1149` Lens tab `<span>{t.hotkey}</span>` —— 同上

### Settings.tsx 结构搬迁

**删除**(`general` tab 内):大约 808–821 行的 `<section>` 块:

```tsx
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

**新增**(`translate` tab 内,作为该 tab 的第一个 section,在`目标语言`之前):同上一段 JSX 原样插入,变量、handler、`recordingTarget` 枚举值 `'main'` 全部不变。

搬迁后的 tab 顺序:

```
基础 tab                          输入翻译 tab                       截图翻译 tab
─────────                         ───────────                        ───────────
外观                              快捷键        ← 新搬来             开关 / 快捷键 / 模型 ...
快捷键        ← 删除              目标语言                            (内容不变,仅 tab 名变)
行为                              翻译引擎
截图自动归档                       输入翻译提示词
权限 (macOS)
```

## 数据流 & 兼容性

- 持久化键 `settings.hotkey` **不变**,跟 backend `register_hotkey`、`save_settings` 全链路无关联改动
- `recordingTarget` 联合类型 `'main' | 'screenshotTranslation' | 'lens'` 不变
- 老用户升级:UI 内容相同,位置不同,无数据迁移
- 旧版本回滚:同样兼容(没动 schema)

## 风险

| 风险 | 缓解 |
|---|---|
| 搬迁过程中 `recordingTarget === 'main'` 那条键盘录制逻辑跟着搬走时漏改 handler | 搬的是 JSX,handler `toggleRecording('main')` 在父作用域已定义,无需动 |
| `hotkey` i18n 改完忘了再有第四处复用 | 已 grep 全仓:仅 3 处使用 `t.hotkey`(General / Screenshot / Lens),全部已知 |
| tab 名变长导致 sidebar 排版换行 | tab 是 sidebar 形式,中文`输入翻译`/`截图翻译`各 4 字,英文`Input Translation`最长 18 字符,与 Lens / Models 长度类似,实测无溢出风险(若有,后续 PR 调样式) |

## 测试 & 验证

- [ ] `npm run lint` 无错
- [ ] `npm run typecheck` 无错
- [ ] `npm run dev` 手动:
  - [ ] `基础` tab 不再有"快捷键"section
  - [ ] `输入翻译` tab 顶部出现"快捷键",录制能正常写入 `settings.hotkey`
  - [ ] 录制后保存,关闭/重开设置,值仍在
  - [ ] 截图 tab 那一行 label 现在写"快捷键"而不是"翻译快捷键"
  - [ ] tab 标签显示`输入翻译`/`截图翻译`(中文)和`Input Translation`/`Screenshot Translation`(英文,切设置语言验证)
  - [ ] 触发输入翻译 hotkey,主窗口正常弹出(回归)

## 不做 / 留给下一轮

- 把目标语言提到全局(C 方案)
- 输入翻译 / 截图翻译的"打磨"本身(质量、流式、UX、错误恢复)—— 等本次结构整理上线后,在干净基础上单独评估
