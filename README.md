<div align="center">

<img src="public/icon.png" width="120" height="120" alt="Kivio Desktop">

# Kivio Desktop

### macOS / Windows 屏幕级 AI 助手：Agent 客户端，加上翻译、截图 OCR 与视觉问答

[![Release](https://img.shields.io/github/v/release/ZMGID/kivio?style=flat-square&color=4f46e5&label=release)](https://github.com/ZMGID/kivio/releases/latest)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey?style=flat-square)](https://github.com/ZMGID/kivio/releases)
[![Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange?style=flat-square)](https://tauri.app/)
[![Downloads](https://img.shields.io/github/downloads/ZMGID/kivio/total?style=flat-square)](https://github.com/ZMGID/kivio/releases/latest)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue?style=flat-square)](LICENSE)

**中文** · [English](README.en.md) · [更新日志](https://github.com/ZMGID/kivio/releases)

[下载](https://github.com/ZMGID/kivio/releases/latest) · [功能](#功能) · [帮助](#帮助) · QQ 群 **1104450740**

<img src="docs/screenshots/qq-group.png" width="220" alt="Kivio QQ 群 1104450740">

</div>

常驻托盘。热键翻译输入、选中文字或屏幕内容，框选后直接问 AI。打开客户端则是完整 Agent：工具、子代理、Skills、MCP、知识库、多模型并排。

自带 Key，无账号、无中转；数据只留本机，无遥测。

## ❤️ 赞助

> 想出现在这里？欢迎通过 [GitHub Issues](https://github.com/ZMGID/kivio/issues) 或 QQ 群 **1104450740** 联系。

<details open>
<summary>点击折叠</summary>

<table>
<tr>
<td width="180" align="center" valign="middle">
<a href="https://hezubus.cc"><img src="docs/sponsors/hezubus.png" alt="合租巴士 Hezubus" width="150"></a>
</td>
<td>
感谢 <a href="https://hezubus.cc">合租巴士</a> 赞助本项目。<a href="https://hezubus.cc">合租巴士</a> 提供 GPT / Claude 等多款模型的官方稳定极速 API 中转服务，支持企业级定制、报销开票、7×16h 专属技术支持，更有独家适配的 WebSocket 连接方式，畅享极速首字速度。Codex 补贴倍率低至 0.08。<a href="https://hezubus.cc">点此注册</a>。
</td>
</tr>
</table>

</details>

## 为什么用 Kivio

屏幕上的字、框选的画面、本机已装的编码 CLI，不必拆成三套软件。Kivio 把它们收进同一个托盘应用：热键随时呼出，对话走你自己的供应商。

- **一个循环，多种宿主** — 内置 Agent、只读研究用的 Kivio Chat、以及本机外部 CLI（Claude Code、Codex、Cursor、OpenCode、Gemini、Kimi、Pi、Hermes、Grok、DeepSeek Harness），共用同一套聊天界面
- **屏幕级工具** — 快速翻译、选中翻译、截图翻译、替换翻译（译文画回原位置）；Lens 冻结屏幕后提问、标注，并可把对话交回客户端
- **自带 Key** — OpenAI Chat Completions / Responses、Anthropic、Gemini、Grok（xAI Responses）；翻译、截图、Lens、每条对话可各用不同模型
- **扩展** — Skills、MCP、知识库、子代理、插件、连接器；右侧 Dock 带文件树、Git 与终端
- **跨平台** — macOS（Apple Silicon）与 Windows 10/11，Tauri 2 原生应用

## 截图

| 聊天客户端 | 设置 |
| :--------: | :--: |
| ![聊天客户端](docs/screenshots/chat-client.png) | ![设置](docs/screenshots/settings.png) |

<p align="center">
  <img src="docs/screenshots/lens-formula-extraction.gif" width="760" alt="Lens 公式提取">
</p>

<p align="center">
  <img src="docs/screenshots/screenshot-translation.png" width="760" alt="截图翻译">
</p>

## 功能

完整记录见 [Releases](https://github.com/ZMGID/kivio/releases) · 当前版本说明：[v3.1.0](docs/releases/v3.1.0.md)

### 聊天与 Agent

- 工具循环、子代理、Skills、MCP、知识库、附件；一问多模型对比
- 三种运行时：Kivio Agent（完整工具）、Kivio Chat（检索 / 抓取 / 知识库等只读能力）、外部 CLI
- 对话可交给本机已装的 Claude Code、Codex、Cursor、OpenCode、Gemini、Kimi、Pi、Hermes、Grok 或 DeepSeek Harness
- 可从本机 CLI 导入原生会话（钉在原 CLI 与原工作目录上续聊）
- 媒体站：生成图片与视频；视频结果和创作记录详情在播放前加载首帧画面，不自动播放
- 聊天 SVG 渐进预览：`svg` 代码块和以 SVG 为根的纯 `html` 图形在生成中逐步显示；可切换源码，标签、属性或字符实体未完成时保留已有画面，包括 Markdown 补入末尾换行的情况。普通 HTML 页面仍在生成结束后隔离预览

### 翻译与 Lens

- 快速翻译、选中翻译、截图 OCR 翻译、替换翻译
- Lens：热键冻结屏幕，框选（macOS 可点窗口）后提问，可画箭头 / 矩形 / 马赛克，可把对话交回客户端
- OCR：系统引擎（Apple Vision / Windows OCR）或可选 RapidOCR 离线包

### 系统

- 全局热键（可改、带冲突检测）、托盘常驻、浅色 / 深色
- 用量统计、生命周期钩子、可选聊天窗口保活（关闭后隐藏而不是销毁）
- 墨墨桌宠：默认关闭，可从侧栏账号菜单「界面语言」下方的「桌面宠物」开关或托盘「显示墨墨桌宠」开启；两个入口与右键隐藏实时同步，切换失败保留原状态并提示重试。在应用现有进程中原生绘制，不新增桌宠 WebView 或外部进程。跟随思考、执行、回答、等待确认、完成和失败状态；单击戳一下、拖动移动、双击打开对应聊天、右键隐藏。开关和位置保存在本机，关闭后停止动画；没有喂养、打卡或死亡机制。
  - 桌面版固定为朝左的蓝色正圆，不随状态、点击或拖动变形。八种状态分别使用慢呼吸、思考三点、搜索放大镜、紧凑键盘、回答气泡、确认问号、完成对勾和失败感叹号；只有执行工作时显示键盘。完成短暂反馈后回到闲置，失败提示与对应聊天入口保留到下一次运行；等待确认优先于其他运行状态。原生几何和动画由 `src-tauri/src/desktop_pet/sim.rs` 维护，不影响聊天页墨墨。
  - macOS 与 Windows 共用平面几何；身体和道具可见部分接收鼠标，透明空隙穿透。闲置时鼠标靠近会关注并停止闲逛；拖动不拉伸，松手保存并停留在拖放位置，不自动下落或吸附底边。只有放在工作区底边时才会空闲短距离活动，任务进行中或等待确认时不闲逛。系统「减少动态效果」停用动画但保留状态标记；隐藏停止更新，再次显示恢复保存的位置。
  - 陪伴行为不改变工作状态：空闲时偶尔东张西望、打盹或轻轻蹦一下，鼠标靠近或点击就清醒；减少动态效果时不播放这些动作。单击会搭一句话，持续空闲时约每四分钟说一句本地短句，不调用模型、不增加 Token 费用。文字使用贴近墨墨、按内容收缩的半透明深色原生小气泡，十秒后收起；拖动、隐藏、锁屏和任务状态变化会撤下旧话，不排队补播。
  - 今日 Token 与费用分开用一句话播报，复用用量统计的本地日历日和计费口径，仅包含 Kivio 已记录调用，外部 CLI 未回传的用量不在其中。费用可能含估算，缺失费用不按免费展示；读取失败或记录不完整会说明，不编造数字。异步读取不阻塞动画，过期结果不会重新弹出气泡。
- 电脑操控：同一窗口内切换页面保留安装 / 更新进度；后台完成后刷新工具状态，失败提示保留并允许重试
- 页面任务：技能与插件安装、模型下载、媒体提交、原生会话导入、对话库批量处理及归档 / 删除 / 导出在同一窗口内持续执行；返回可查看进度、结果或失败提示，同一运行中的操作不会重复提交。技能与媒体站保留当前视图；技能卸载后商店恢复安装入口，旧预览、媒体引用和 Pi 安装回调不覆盖更新的草稿或反馈。前端状态保留限于当前窗口，不是应用重启恢复机制。
- 编辑草稿：笔记与自动化保存跨页面继续；笔记导航前同步读取编辑器正文，避免延迟回传丢字。助手只在手动保存时写入，旧删除或复制回调不清除、替换另一编辑会话。返回任务页恢复原自动化画布；加载远端新版本时同步更新画布，迟到读取不覆盖期间产生的编辑，包括已经保存成功的版本。
- TinyFish 授权：离开页面取消当前授权；再次进入只接受新授权的结果，不保存旧流程迟到的凭据。

### 主题

- 设置 → **主题**：独立管理显示模式、半透明侧边栏和主题库；内置中性、暖白、冷白，以及参考 T3 Code 配色适配的石墨、樱粉、森林、海湾、余烬、鸢尾，另有纯白、北欧 Nord、日晒 Solarized，共十二套主题，均包含浅色与深色色板。纯白的浅色主背景为 `#FFFFFF`，区别于中性的奶白；Nord 使用冷灰蓝与冰蓝强调，Solarized 使用米黄 / 深青底色与青绿强调。原有九套配色保持不变。
- 主题卡片同时展示明暗缩略图，支持方向键选择预览。复制内置主题创建自定义主题，颜色按表面、文字与边框、强调色、危险状态分组，可用取色器或 HEX 输入编辑；编辑时显示实时预览，窄窗口自动改为单列。预览不影响当前界面；保存后点击「应用主题」切换。正在使用的自定义主题保存修改后立即更新。
- 编辑草稿在设置页之间切换时保留，保存失败可重试；删除正在使用的主题后回到中性主题。主题随本机设置及设置备份保存。
- JSON 分享：导出弹窗显示完整 JSON，可直接复制；导入弹窗粘贴 JSON 后读取、预览并保存，Escape 关闭并恢复原按钮焦点。格式为 `version: 1`，`theme` 包含 `name`、`light` 和 `dark`；每个色板必须包含全部 17 个颜色字段，值为 `#RRGGBB`。导入分配新 ID，不覆盖同名主题；不支持任意 CSS。
- 界面表面与文字使用语义主题颜色：聊天消息、审批与提问卡、运行时菜单、Markdown 图表、自动化画布、用量统计、Lens 控件、图片查看器和知识库设置均随主题切换；正在使用的自定义色板修改后同步更新。截图和图片像素、品牌图标、语法高亮及状态标识保留各自颜色，不做全局白色替换。
- 刷新首屏在应用脚本和后端设置加载前同步恢复上次已应用的明暗色板，加载背景随主题显示；跟随系统模式会重新读取当前系统明暗。首屏缓存只保存颜色，不保存供应商或凭据，后端设置读取后覆盖缓存；内容外壳挂载后恢复透明画布，保留原生材质和截图浮窗透明性。浏览器存储不可用时仍正常应用后端主题。

## 热键

都是开关，可在设置里改。

| 功能 | macOS | Windows |
|---|---|---|
| 打开聊天 | `⌘⇧K` | `Ctrl+Shift+K` |
| 快速翻译 | `⌘⌥T` | `Ctrl+Alt+T` |
| 截图翻译 | `⌘⇧A` | `Ctrl+Shift+A` |
| 选中翻译 | `⌘⇧T` | `Ctrl+Shift+T` |
| 替换翻译 | `⌘⇧R` | `Ctrl+Shift+R` |
| Lens | `⌘⇧G` | `Ctrl+Shift+G` |

## 下载安装

### 系统要求

- **macOS**：Apple Silicon（`.dmg` 未签名）
- **Windows**：Windows 10 / 11（需要 Edge WebView2，系统通常已带）

从 [Releases](https://github.com/ZMGID/kivio/releases/latest) 下载：

- macOS：`Kivio.Desktop_*_aarch64.dmg`
- Windows 安装包：`Kivio.Desktop_*_x64-setup.exe`
- Windows 便携版：`Kivio.Desktop_*_x64-portable.zip`（解压即用）

macOS 首次请右键打开，或：

```bash
xattr -cr "/Applications/Kivio Desktop.app"
```

macOS 需要**辅助功能**与**屏幕录制**权限。启动后按引导填供应商即可。

## 帮助

- [更新日志](https://github.com/ZMGID/kivio/releases) — 各版本下载与亮点
- [问题反馈](https://github.com/ZMGID/kivio/issues)
- QQ 群 **1104450740**

## 快速开始

1. 安装并打开 Kivio，完成首次引导（供应商 + 热键）。
2. **添加供应商**：设置 → 供应商 → 添加驱动 → 选预设（或自定义），填入 API Key。
3. **聊天**：`⌘⇧K` / `Ctrl+Shift+K` 打开客户端，选模型后发送。需要本机 CLI 时，在运行时选择器里切换。
4. **屏幕**：用热键翻译或打开 Lens；需要的权限在系统设置里打开。

## 常见问题

<details>
<summary><strong>需要注册账号吗？数据存在哪？</strong></summary>

不需要账号。Key 写在本机 `settings.json` 里，对话、知识库、笔记都在应用数据目录：

- Windows：`%APPDATA%\com.zmair.kivio`
- macOS：`~/Library/Application Support/com.zmair.kivio`

无遥测，请求只发往你配置的供应商。

</details>

<details>
<summary><strong>支持哪些外部 CLI？</strong></summary>

本机已安装即可作为对话后端：Claude Code、Codex、Cursor、OpenCode、Gemini、Kimi、Pi、Hermes、Grok、DeepSeek Harness。可用性按本机探测；模型列表按所选 CLI 懒加载。

</details>

<details>
<summary><strong>macOS 提示已损坏 / 无法打开？</strong></summary>

安装包未签名。请右键 → 打开，或执行 `xattr -cr "/Applications/Kivio Desktop.app"`。截图与替换翻译还需要屏幕录制权限。

</details>

<details>
<summary><strong>导入的 CLI 对话为什么不能换模型续聊？</strong></summary>

导入只做显示快照，历史真身仍在原 CLI。续聊走该 CLI 的原生会话，并钉在原来的工作目录。详见 [ADR-0001](docs/adr/0001-imported-cli-conversations-stay-on-their-cli.md)、[ADR-0002](docs/adr/0002-imported-history-is-a-snapshot.md)。

</details>

## 开发

<details>
<summary><strong>环境与命令</strong></summary>

需要 Node.js、npm、Rust，以及 Tauri 2 的平台依赖。macOS 上 OCR sidecar 还需要 Swift。

```bash
npm install
npm run dev          # Rust + UI（macOS 会编 Swift sidecar）
npm run dev:ui       # 只开 Vite
npm run lint
npm run typecheck
npm test
```

Rust 测试：

```bash
# macOS / Linux
cargo test --manifest-path src-tauri/Cargo.toml

# Windows（请用脚本；直接 cargo test 的测试二进制可能起不来）
powershell -File scripts/win-cargo-test.ps1
```

协议类型由 Rust 生成，改协议后需要：

```bash
npm run protocol:generate
npm run protocol:check
```

</details>

<details>
<summary><strong>架构总览</strong></summary>

```
┌─────────────────────────────────────────────────────────────┐
│                 前端（React 18 + Vite + Tailwind）            │
│   同一套 bundle，四个窗口：main / chat / lens / translate     │
└────────────────────────┬────────────────────────────────────┘
                         │ Tauri IPC · chat-protocol
┌────────────────────────▼────────────────────────────────────┐
│                  后端（Tauri 2 + Rust）                       │
│   AgentHost 循环（prepare → planning → rounds → synthesis）  │
│     ├─ GUI 聊天                                              │
│     ├─ 外部 CLI 代理                                         │
│     └─ 子代理                                                │
└─────────────────────────────────────────────────────────────┘
```

- 实时聊天走版本化 `chat-protocol`（`npm run protocol:check` 校验生成物）
- 供应商适配器：OpenAI Chat、Anthropic Messages、Gemini、OpenAI Responses（含 xAI Grok）
- 设置存在 `settings.json`（含 API Key），对话在 `conversations/`，崩溃草稿走 JSONL journal

开发约定见[统一工程规范](docs/engineering-standards.md)。

</details>

## 贡献

欢迎 [Issues](https://github.com/ZMGID/kivio/issues) 与 PR。提交前请确保：

- `npm run lint`
- `npm run typecheck`
- `npm test`

较大的功能请先开 issue 讨论。议题约定见 [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md)。

## Star History

[![Star History Chart](docs/star-history.svg)](https://github.com/ZMGID/kivio/stargazers)

## License

[GPL-3.0-or-later](LICENSE) © ZM
