# 桌面端「产品壳窗口」与工具/浮层窗口的一手结构调查

研究日期：2026-09-19。范围只核验官方文档、官方仓库源码和第一方 API，不引用博客或二手复述。本轮没有改产品代码。

决策背景：Kivio 是 Tauri v2 桌面应用，从翻译工具长成完整 AI 客户端。`src-tauri/tauri.conf.json` 仍声明唯一静态窗口 `label: "main"`（392×152、无边框、透明、`skipTaskbar`、`visible: false`、`create: false`），实际只作输入翻译浮层模板；`chat` / `lens` / `translate` 已在 Rust 里按需创建。讨论中的两条路：

- **A**：把 `main` 改名为 `translator`，仍保留一份配置模板
- **B**：把 `app.windows` 设成 `[]`，翻译窗也像 chat/lens 一样从 Rust 创建

下文把「配置里有没有静态窗口」和「产品有没有壳窗口」分开写。前者是实现，后者是产品。

## 结论摘要

1. Tauri 官方把 `app.windows` 的默认值写成 `[]`；`create: false` 是官方提供的「配置里留模板、启动时不创建」开关。空数组合法，不会单独让 capabilities 或 `get_webview_window` 崩溃。
2. 成熟 AI 客户端把 **chat / workbench 当作产品壳**。Electron 族（Cherry Studio、Chatbox、LobeHub Desktop、Open WebUI Desktop、5ire）没有 `tauri.conf.json` 那种静态窗口表，主进程里创建大壳，浮层另开。
3. 成熟工具把 **调色板 / 翻译浮层做成按需窗口，且通常不叫 `main`**。pot-desktop 的静态窗是隐藏 `daemon`；翻译窗 label 是 `translate`。CopyQ / Alfred / Raycast 的「主界面」是可隐藏的面板或命令栏，不是常驻 workbench。Coco AI 是反例：它的 `main` 就是 680×590 的跳过任务栏浮层，因为它本身是启动器产品。
4. 空 `windows: []` 在 **Tauri 契约里常见、在热门 Tauri 聊天客户端里少见**。热门聊天客户端多数是 Electron，本来就没有这份数组。
5. 对 Kivio：**产品上 chat 已经是壳，翻译窗不该再叫 `main`。实现上 A、B 都合法；B 更贴近 Kivio 现有的 chat/lens 工厂，也更贴近 Electron 成熟客户端「壳在代码里创建」的做法。空配置 ≠ 没有产品壳。**

## 1. Tauri 官方契约

### 1.1 `app.windows` 默认就是空数组

官方配置参考写明：`windows` 是 `WindowConfig[]`，「The app windows configuration」，**Default: `[]`**。创建启动窗的示例是往数组里放一项；未写 `label` 时标识符默认为 `"main"`，之后用 `app.get_webview_window` / `WebviewWindow.getByLabel` 取窗。多窗必须各有唯一 label。([Configuration · windows](https://v2.tauri.app/reference/config/))

同一页还写了第三条路径：把某项设成 `"create": false`，再在 Rust 里用 `WebviewWindowBuilder::from_config(app.handle(), &app.config().app.windows[0])?.build()?`。这正好对应 Kivio 现在对 `main` 的用法。

### 1.2 `WindowConfig.create` 的字面含义

`create` 是 boolean：「Whether Tauri should create this window at app startup or not.」设为 `false` 时，「you must manually grab the config object via `app.config().app.windows` and create it with `WebviewWindowBuilder::from_config`」。**Default: `true`**。([WindowConfig.create](https://v2.tauri.app/reference/config/#create))

`label`：「The window identifier. It must be alphanumeric.」**Default: `"main"`**。([WindowConfig.label](https://v2.tauri.app/reference/config/#label))

因此：

- 空 `windows: []` = 启动时不自动建任何窗，官方默认。
- 数组里有一项且 `create: true`（默认）= 启动时立刻建窗，哪怕 `visible: false` 也会加载页面。
- 数组里有一项且 `create: false` = 只当模板，必须自己 `from_config`。

### 1.3 PR #11032 / Issue #10950：这个开关为什么存在

Issue #10950 的诉求是：写在 `tauri.conf.json` 的窗会在 setup 阶段自动弹出；`visible: false` 仍会创建隐藏 WebView 并跑页面代码。作者不想把 40+ 个 `WindowConfig` 字段抄进 Rust。([Issue #10950](https://github.com/tauri-apps/tauri/issues/10950))

同 issue 里 martpie 写：「I think creating a window manually instead of defining it in the conf file. … My `windows` section in `tauri.config.json` is empty.」维护者 amrbashir 随后用 [PR #11032](https://github.com/tauri-apps/tauri/pull/11032) 加上 `create`，并说明不必新 API：`app.config().app.windows` 是 vector，可用 `find(|w| w.label == "main")`。该 PR 于 2024-09-16 合并。

官方因此同时承认两条实现：

- **空数组 + `WebviewWindowBuilder::new`**（martpie / 插件示例）
- **留模板 + `create: false` + `from_config`**（#11032 的设计意图）

Kivio 的 `ensure_main_window` 走第二条：找不到已有 `main` 时，`find(|w| w.label == "main")` 再 `from_config`。([`src-tauri/src/windows.rs`](../../src-tauri/src/windows.rs))

### 1.4 空数组会不会单独弄坏 capabilities / `get_webview_window` / macOS activation

**Capabilities 按 label 授权，不要求该 label 出现在 `app.windows`。** 官方 Capability 写：「If a webview or its window is not matching any capability then it has no access to the IPC layer at all。」窗口可用精确名或 glob（`*`、`admin-*`）加入 capability。示例里 `"windows": ["main"]` 只是举例。([Capability](https://v2.tauri.app/reference/config/#capability)) 安全教程同样用 `first` / `second` 这类任意 label，并说明 capability 的 `windows` 字段匹配的是窗口 label。([Capabilities for windows and platforms](https://v2.tauri.app/learn/Security/capabilities-for-windows-and-platforms/))

Kivio 现有 `src-tauri/capabilities/default.json` 已列出 `main`、`chat`、`chat-popout-*`、`lens`、`translate`。后四个本来就不在 `tauri.conf.json` 的 `windows` 里，IPC 仍可用。所以清空 `app.windows` **不会**因为「配置里没有这个 label」而拆掉 capability；只要创建时仍用这些 label，权限表不用跟着变空。

**`get_webview_window("main")` 在窗不存在时返回 `Option`，不是启动失败。** 官方 tray 教程的示例是 `if let Some(window) = app.get_webview_window("main") { window.unminimize(); window.show(); … }`，明确按「可能没有」处理。([System Tray](https://v2.tauri.app/learn/system-tray/)) 官方还写：重复同一个 label 会冲突，需要先关掉旧窗或换 label。([WindowBuilder::from_config](https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html)) 插件维护者对 `WindowLabelAlreadyExists("main")` 的官方答复是：要么从 conf 删掉静态窗只在 Rust 建，要么删掉 setup 里再建一次。([plugins-workspace#597](https://github.com/tauri-apps/plugins-workspace/issues/597))

**macOS activation 与 `app.windows` 是否为空无关。** Electron 官方把 `app.setActivationPolicy` 分成 `regular`（普通 Dock 应用）、`accessory`（不出现在 Dock / 菜单栏，可被程序或点击自己的窗激活）、`prohibited`（不可建窗、不可激活）。([Electron app.setActivationPolicy](https://electronjs.org/docs/latest/api/app)) Tauri 侧同样用 `ActivationPolicy`。Kivio 已按产品身份切换：chat 需要 Dock，所以注释写明本 app 为 `ActivationPolicy::Regular`，lens/翻译才改成非激活 NSPanel；关 chat 可回到 `Accessory`。([`src-tauri/src/windows.rs`](../../src-tauri/src/windows.rs) 注释；[`lib.rs` / `shortcuts.rs` / `chat/popout.rs`](../../src-tauri/src/lib.rs) 的 `set_activation_policy` 调用) 官方 tray 教程只示范「点托盘则 `get_webview_window("main")` 再 show」，没有要求启动时必须已有名为 `main` 的窗。([System Tray](https://v2.tauri.app/learn/system-tray/))

Coco AI 在 setup 里对 macOS 调用 `set_activation_policy(ActivationPolicy::Accessory)`，再 `get_webview_window("main")`——前提是它的 `main` **已经在 conf 里创建**（`visible: false`）。这是「隐藏静态窗 + Accessory」的启动器写法，不是空数组写法。([coco-app `src-tauri/src/lib.rs`](https://github.com/infinilabs/coco-app/blob/bc0f4f39/src-tauri/src/lib.rs))

**会断的是依赖「conf 里必须有名为 `main` 的模板」的代码**，不是 Tauri 运行时本身。Kivio 的 `ensure_main_window` 在找不到 `label == "main"` 时返回 `"Main window config not found"`。选 B 等于放弃这条 `from_config` 路径；选 A 则要把查找键改成新 label。这是本地耦合，不是官方空数组不支持。

## 2. Tauri / 桌面 AI 客户端

热门「Tauri 聊天客户端」实际很少。Cherry Studio、Chatbox、LobeHub Desktop、Open WebUI Desktop、5ire 都是 Electron。真正带 `tauri.conf.json` 的是 pot-desktop、lencx/ChatGPT、Coco AI、NextAI Translator。下面按源码写，不把 Electron 说成 Tauri。

### 2.1 Cherry Studio（Electron，仓库 CherryHQ/cherry-studio）

- **有没有静态 main 配置表：** 没有。Electron 没有 `app.windows`。
- **壳是什么：** `WindowService.createMainWindow()` 用 `windowStateKeeper` 恢复几何，新建 `BrowserWindow`（`show: false`，mac 隐藏标题栏，Win/Linux 无框），再 `setupMainWindow`。这是产品壳。([`src/main/services/WindowService.ts`](https://github.com/CherryHQ/cherry-studio/blob/b8485805/src/main/services/WindowService.ts)) 更新的 `MainWindowService` 把创建交给 `windowManager.open(WindowType.Main, …)`，仍然是「主窗 = 壳」。([`src/main/services/MainWindowService.ts`](https://github.com/CherryHQ/cherry-studio/blob/9ea7e850/src/main/services/MainWindowService.ts))
- **浮层：** `createMiniWindow` 另建一扇：`frame: false`、`alwaysOnTop: true`、`skipTaskbar: true`、mac 上 `type: 'panel'`、`setVisibleOnAllWorkspaces`。快捷助手开启时在创建主窗后预载。([同一 `WindowService.ts`](https://github.com/CherryHQ/cherry-studio/blob/b8485805/src/main/services/WindowService.ts))
- **托盘启动：** `index.ts` 在 `app.whenReady` 里读 `getLaunchToTray()`，为真则 `app.dock?.hide()`，然后仍调用 `createMainWindow()` 并建 `TrayService`。是「可藏 Dock 的壳」，不是无窗启动。([`src/main/index.ts`](https://github.com/CherryHQ/cherry-studio/blob/b8485805/src/main/index.ts))

### 2.2 Chatbox（Electron，仓库 chatboxai/chatbox）

- **静态配置表：** 无。
- **壳：** `createWindow()` 建 `mainWindow = new BrowserWindow({ show: false, titleBarStyle: 'hidden', frame: false, … })`，开发载 renderer URL，生产载 `renderer/index.html`；`ready-to-show` 后按持久化状态 maximize / fullscreen / `show()`。注释写窗口几何「很大程度参考了 VSCODE」。([`src/main/main.ts`](https://github.com/chatboxai/chatbox/blob/main/src/main/main.ts)、[`src/main/window_state.ts`](https://github.com/chatboxai/chatbox/blob/main/src/main/window_state.ts))
- **浮层：** 这份 main 里没有独立 overlay 窗；托盘存在（`createTray` / `ensureTray`），与主窗并列。
- **启动：** `app.whenReady` 路径调用 `createWindow()` 再 `ensureTray()`。产品壳在启动时创建。

### 2.3 LobeHub Desktop（Electron，lobehub/lobe-chat `apps/desktop`）

- **静态配置表：** 无。窗口表是 TypeScript：`BrowsersIdentifiers.app = 'app'`，`appBrowsers.app` 为 `keepAlive: true`、`showOnInit: true`、`path: '/'`、1200×800。([`apps/desktop/src/main/appBrowsers.ts`](https://github.com/lobehub/lobe-chat/blob/2eb9e34f/apps/desktop/src/main/appBrowsers.ts))
- **壳：** `BrowserManager.initializeBrowsers()` 对 `identifier === app` 的项（或 `keepAlive`）调用 `retrieveOrInitialize`，内部 `new BrowserWindow({ show: false, frame: false, … })`。([`BrowserManager.ts`](https://github.com/lobehub/lobe-chat/blob/2eb9e34f/apps/desktop/src/main/core/browser/BrowserManager.ts)、[`Browser.ts`](https://github.com/lobehub/lobe-chat/blob/2eb9e34f/apps/desktop/src/main/core/browser/Browser.ts)) `App.ts` 注释：Electron ready 后「create the main BrowserWindow before native menus, … tray and updater」。([`App.ts`](https://github.com/lobehub/lobe-chat/blob/e816a83e/apps/desktop/src/main/core/App.ts))
- **浮层 / 多实例：** `windowTemplates.chatSingle` / `topicPopup` 设 `keepAlive: false`、`allowMultipleInstances: true`，按需开，不是启动必建。

### 2.4 Open WebUI Desktop（Electron，open-webui/desktop）

- **静态配置表：** 无。
- **壳：** `createMainWindow` 建 1280×800 的 `mainWindow`（`show: false`，随后 `ready-to-show` 才 `show`）。另有 `createContentWindow` 承载实际 WebUI。关主窗时若 `runInBackground !== false` 则 `preventDefault` + `hide()`。([`src/main/index.ts`](https://github.com/open-webui/desktop/blob/d475bde0/src/main/index.ts)) 默认配置 `runInBackground: true`。([`src/main/utils/index.ts`](https://github.com/open-webui/desktop/blob/d475bde0/src/main/utils/index.ts))
- **浮层：** `createSpotlightWindow` 按当前显示器全屏建透明、`alwaysOnTop`、`skipTaskbar` 的 spotlight；blur / close 后把 `mainWindow` 再 `show()`。`toggleSpotlight` 没有则新建。这是独立 overlay，不是壳。
- **启动：** 配置好默认连接或否则都 `createMainWindow()`；`activate` 时若没有任何窗则再建主窗。

### 2.5 5ire（Electron，nanbingxyz/5ire）

- **不是 Tauri。** 主进程是 Electron：`getMainWindow()` 取 `Renderer.state.window`；`close-app` 对当前 `BrowserWindow` `destroy()`，非 darwin 再 `app.quit()`。([`src/main/main.ts`](https://github.com/nanbingxyz/5ire/blob/810ff2b5/src/main/main.ts)) 开发文档写 `npm start`「Open the application window」。([`DEVELOPMENT.md`](https://github.com/nanbingxyz/5ire/blob/810ff2b5/DEVELOPMENT.md))
- 没有在本次核验到独立 overlay 窗源码；能确定的是产品壳是那一扇主 `BrowserWindow`。

### 2.6 lencx/ChatGPT（Tauri v1/v2 混合期，约 54k star）

- **`tauri.conf.json` 没有 `windows` 数组。** 已抓到的 `src-tauri/tauri.conf.json` 含 allowlist、systemTray、bundle、security.dangerousRemoteDomainIpcAccess（列出 `core` / `main` / `tray` 等 **webview 名**），没有启动窗几何。([该文件](https://raw.githubusercontent.com/lencx/ChatGPT/main/src-tauri/tauri.conf.json))
- **壳在 Rust 创建：** `WindowBuilder::new(&handle, "core").title("ChatGPT")`，再往这扇窗里放 `main` / `titlebar` / `ask` 多个 webview。前端按 `getCurrentWebview().label` 选 Titlebar / Ask / Settings。([`setup.rs` 检索摘要](https://github.com/lencx/ChatGPT/blob/a6de9a8b/src-tauri/src/core/setup.rs)、[`src/App.tsx`](https://github.com/lencx/ChatGPT/blob/a6de9a8b/src/App.tsx))
- **产品壳 label 是 `core`，不是 conf 默认的 `main`。** `main` 在这里是壳内部的内容 webview。这是「空/缺失 windows 数组 + Rust 建壳」的现存 Tauri 聊天客户端。

### 2.7 Coco AI（Tauri v2，infinilabs/coco-app）

- **声明多扇静态窗，全部 `visible: false`。** `main`：680×590、无装饰、`skipTaskbar: true`、`alwaysOnTop: true`、`visibleOnAllWorkspaces: true`、`url: "/ui"`。另有 `settings`、`check`、`selection`。([`src-tauri/tauri.conf.json`](https://raw.githubusercontent.com/infinilabs/coco-app/main/src-tauri/tauri.conf.json))
- **`main` 就是启动器浮层，不是 chat workbench。** bundle `category` 为 `"Utility"`。Rust 用 `MAIN_WINDOW_LABEL` 取这扇窗，macOS 设 `ActivationPolicy::Accessory`，`CloseRequested` 时 `hide()` + `prevent_close()`。([`src-tauri/src/lib.rs`](https://github.com/infinilabs/coco-app/blob/bc0f4f39/src-tauri/src/lib.rs))
- **这是「产品本身是调色板」时把浮层命名为 `main` 的例子。** 不能用来证明完整 AI 客户端也应把翻译浮层叫 `main`。

### 2.8 NextAI Translator（Tauri v2，nextai-translator/nextai-translator）

- **`"windows": []` 是生产配置，不是示例。** `app.windows` 为空，另有 `trayIcon`。([`src-tauri/tauri.conf.json`](https://raw.githubusercontent.com/nextai-translator/nextai-translator/main/src-tauri/tauri.conf.json))
- **所有可见窗都在 Rust 按需创建，没有叫 `main` 的窗。** 常量：`translator`、`quick_translator`、`inline_lookup`、`settings`、`screenshot`、`writing_indicator`。`get_translator_window` 用 `WebviewWindowBuilder::new(..., TRANSLATOR_WIN_NAME, …)`，默认 `visible(false)`。([`src-tauri/src/windows.rs`](https://raw.githubusercontent.com/nextai-translator/nextai-translator/main/src-tauri/src/windows.rs))
- **Windows 上不要启动时预创建隐藏 WebView2。** 隐藏窗的 renderer 会继续合成、空烧 CPU；快捷翻译改为首次使用再创建。([PR #1887](https://github.com/nextai-translator/nextai-translator/pull/1887)、[commit 96c62a0](https://github.com/nextai-translator/nextai-translator/commit/96c62a0ddc7235069a29db201a5ec6d8579791c8))
- 它仍是翻译产品，所以产品壳就是 `translator`。对 Kivio 有用的是实现：空数组 + 语义 label + 按需创建，已被同一技术栈的生产应用采用。

### 2.9 社区 Tauri 端口

[reecelikesramen/open-webui-desktop](https://github.com/reecelikesramen/open-webui-desktop/) README 自称「A native desktop app for Open WebUI built with Tauri v2」，并列出 Main window / Chatbar / Chat Companion。这是第三方 WIP，不是 Open WebUI 官方桌面；官方桌面是上一节的 Electron 仓库。本次未再把它的 conf 当作产品先例。

## 3. 翻译器 / 调色板工具

### 3.1 pot-desktop（Tauri，pot-app/pot-desktop）

- **静态窗只有隐藏 `daemon`，不叫 `main`。**  
  `"windows": [{ "title": "Daemon", "label": "daemon", "visible": false, "url": "daemon.html", … }]`，另有 `systemTray`。([`src-tauri/tauri.conf.json`](https://raw.githubusercontent.com/pot-app/pot-desktop/master/src-tauri/tauri.conf.json))
- **翻译 / OCR / 设置 / 截图全是 Rust 按需建。** `build_window(label, title)`：已有则 `set_focus`，否则 `WindowBuilder::new(app, label, WindowUrl::App("index.html"))`，先 `visible(false)`。`translate_window()` 用 label `"translate"`，`set_skip_taskbar(true)`，默认约 350×420。`config_window` / `recognize` / `screenshot` / `updater` 同工厂。([`src-tauri/src/window.rs`](https://raw.githubusercontent.com/pot-app/pot-desktop/master/src-tauri/src/window.rs))
- **托盘常驻，启动可以没有可见窗。** 静态 daemon 只为取 monitor 列表；找不到还会再补一扇隐藏 daemon。
- 这是与 Kivio 翻译血统最接近的开源对照：工具浮层 **不占用 `main` 这个默认名**。

### 3.2 Bob

未找到可核验的应用源码。`ripperhe/Bob` 在本次检索中没有可用的窗口管理实现。**源码不可用，不推断其窗口模型。**

### 3.3 Raycast（第一方手册，无开源主程序）

- 官方不把 Root Search 写成「没有窗口」：Search Bar 是「the Raycast window」顶部的唯一输入；Compact Mode 下空查询时窗口收成只有搜索栏，失焦再收起。热键打开后搜索栏默认聚焦。([Search Bar](https://manual.raycast.com/search-bar))
- **AI Chat「lives in its own window」**，可与当前工作并排；可 Always on Top。Quick AI 走 Root Search，完整会话进独立 AI Chat 窗。([AI Chat](https://manual.raycast.com/ai/ai-chat))
- 没有公开主进程源码，不能断言「启动时零 BrowserWindow」。能断言的是产品模型：**命令栏是主交互，完整 chat 是另一扇窗**，不是把 chat 塞进常驻小翻译条。

### 3.4 Alfred（第一方帮助）

- Appearance 写：默认 **non-activating panel**，「operating much like Spotlight's window when in Standard mode」；Compatibility mode 才「operates like an application and takes focus」。可关菜单栏图标。([Appearance](https://www.alfredapp.com/help/appearance/))
- Workflow 工具里有 **「Show the Alfred main window」**，说明官方仍把这层面板叫 main window，但是按需显示的面板，不是 workbench。([Workflow Utilities](https://www.alfredapp.com/help/workflows/utilities/))
- 主程序未开源。

### 3.5 CopyQ（开源，hluk/CopyQ）

- 官方 FAQ：主窗在「无法最小化到任务栏且没有托盘」时会保持打开，作为首次设置的安全网；可在 Preferences → Layout 开 **Hide main window**，或 `copyq config hide_main_window true`。全局快捷键命令是 `toggle()`「Show/hide main window」。([docs/faq.rst](https://github.com/hluk/CopyQ/blob/master/docs/faq.rst)、[FAQ](https://copyq.readthedocs.io/en/stable/faq.html))
- 脚本 API：`show` / `hide` / `toggle` / `visible` 都针对 main window；另有 tray menu。([Scripting API](https://copyq-docs.readthedocs.io/en/latest/scripting-api.html))
- 模式：**有名为 main 的管理窗，默认可藏，托盘可独立活着。** 这扇 main 是剪贴板管理器，不是 392×152 翻译条。

### 3.6 uTools

官方只开了插件 API 类型（`uTools-Labs/utools-api-types`），其中的 `BrowserWindow` 声明是给插件用的宿主窗 API，**不是 uTools 主程序窗口管理源码**。([ubw.d.ts](https://github.com/uTools-Labs/utools-api-types/blob/main/ubw.d.ts)) **主程序窗口结构无法从第一方源码核验。**

## 4. 带调色板 / 浮层的完整桌面客户端

### 4.1 Claude Desktop

- 官方学院：桌面 app 的 **quick entry 是「a small Claude window that opens on top of whatever app you're in」**；Mac 双击 Option，Win/Linux 用设置里的快捷键；也可点菜单栏 / 系统托盘图标。([Navigating the Claude desktop app](https://academy.claude.com/tutorials/navigating-the-claude-desktop-app))
- 同一页把「关窗后任务继续」写成产品行为，并指向独立的 Claude Code workbench 文档。这是 **壳 + 快捷小窗**，但 **没有公开窗口创建源码**，不能写 BrowserWindow 字段。
- Linux 安装文档只谈发行版与快捷键门户，不谈窗结构。([Claude Desktop on Linux](https://code.claude.com/docs/en/desktop-linux))

### 4.2 ChatGPT 桌面 / Slack / Discord / Notion

本次没有找到这些闭源产品的第一方窗口创建源码或支持文档来描述「main vs overlay」的实现。**不编造。**

### 4.3 VS Code（microsoft/vscode，开源）

- **壳：** `WindowsMainService` 在没有现成窗时 `instantiationService.createInstance(CodeWindow, { state, … })`。([`windowsMainService.ts`](https://github.com/microsoft/vscode/blob/4c055a03/src/vs/platform/windows/electron-main/windowsMainService.ts)) `CodeWindow` 是 Electron 主进程里的 workbench 窗。
- **命令面板不是独立 Electron 窗。** 扩展菜单把未声明的 command 隐式挂到 `commandPalette`；这是 workbench 内 overlay / QuickPick，不是第二扇 `BrowserWindow`。([`menusExtensionPoint.ts`](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/actions/common/menusExtensionPoint.ts))
- Cursor 未提供对等的第一方窗口源码；能核验的是 VS Code 这条「真 main workbench + 窗内 palette」。

### 4.4 Electron 托盘优先 / 无窗启动的官方契约

Electron `app` 文档：

- 若不监听 `window-all-closed`，关光所有窗会退出；监听后由应用决定是否退出。([Event: window-all-closed](https://electronjs.org/docs/latest/api/app))
- 主进程教程：主进程的首要职责是用 `BrowserWindow` **创建和管理应用窗口**；示例是主动 `new BrowserWindow`。([Process Model](https://github.com/electron/electron/blob/main/docs/tutorial/process-model.md))
- `setActivationPolicy('accessory')` 允许「不出现在 Dock、可以没有常规 UI，仍可被程序或自己的窗激活」。([setActivationPolicy](https://electronjs.org/docs/latest/api/app))

官方允许托盘常驻、启动不 show 窗（`show: false` 或根本不 `new`），但成熟聊天客户端（上文 Cherry / Chatbox / Lobe / Open WebUI）都在 ready 时创建壳，再用 hide / tray 做后台。

## 5. 逐产品对照

| 产品 | 静态 main 配置？ | 那扇窗是壳还是工具残留？ | 浮层是否另窗按需？ | 无窗/托盘启动？ | 一手出处 |
| --- | --- | --- | --- | --- | --- |
| Tauri 官方 | 数组默认 `[]`；单项默认 label `main`、`create: true` | 脚手架习惯叫 main，不是产品语义 | `create: false` 或 `new` 都支持按需 | tray 教程用 `if let Some(window)` | [config](https://v2.tauri.app/reference/config/)、[#11032](https://github.com/tauri-apps/tauri/pull/11032) |
| Kivio 现状 | 有，`main` + `create: false` | **工具残留**（翻译条模板） | chat / lens / translate 已是 Rust 按需 | 翻译条不自动创建；chat 冷启动隐藏再 reveal | 本地 `tauri.conf.json`、`windows.rs` |
| Cherry Studio | 无（Electron） | 代码里的 main = 壳 | miniWindow 独立 panel | launch-to-tray 仍创建主窗，可藏 Dock | [WindowService.ts](https://github.com/CherryHQ/cherry-studio/blob/b8485805/src/main/services/WindowService.ts) |
| Chatbox | 无 | `mainWindow` = 壳 | 未见独立 overlay | ready 时 createWindow + tray | [main.ts](https://github.com/chatboxai/chatbox/blob/main/src/main/main.ts) |
| LobeHub Desktop | 无；TS 表 `app` | `app` keepAlive 壳 | chatSingle / topicPopup 按需 | ready 后 initializeBrowsers | [appBrowsers.ts](https://github.com/lobehub/lobe-chat/blob/2eb9e34f/apps/desktop/src/main/appBrowsers.ts) |
| Open WebUI Desktop | 无 | `mainWindow` = 连接/设置壳，content 另窗 | spotlight 全屏 overlay | `runInBackground` 时关窗 hide | [index.ts](https://github.com/open-webui/desktop/blob/d475bde0/src/main/index.ts) |
| 5ire | 无（Electron，不是 Tauri） | Renderer 主窗 = 壳 | 本次未见 overlay 源码 | 开发启动即开窗 | [main.ts](https://github.com/nanbingxyz/5ire/blob/810ff2b5/src/main/main.ts) |
| lencx/ChatGPT | conf **无 windows 数组** | Rust 建 `core` 壳；`main` 是内部 webview | settings 等按 label | 有 systemTray | [tauri.conf.json](https://raw.githubusercontent.com/lencx/ChatGPT/main/src-tauri/tauri.conf.json)、[setup.rs](https://github.com/lencx/ChatGPT/blob/a6de9a8b/src-tauri/src/core/setup.rs) |
| NextAI Translator | **`"windows": []`** | 无 `main`；产品壳是 Rust 建的 `translator` | `quick_translator` / `inline_lookup` 按需 | 托盘 + 自启不显示翻译窗 | [tauri.conf.json](https://raw.githubusercontent.com/nextai-translator/nextai-translator/main/src-tauri/tauri.conf.json)、[windows.rs](https://raw.githubusercontent.com/nextai-translator/nextai-translator/main/src-tauri/src/windows.rs) |
| Coco AI | 有，`main` 是 680×590 浮层 | **`main` 就是启动器产品** | settings / selection 等同表、默认隐藏 | Accessory + hide-on-close | [tauri.conf.json](https://raw.githubusercontent.com/infinilabs/coco-app/main/src-tauri/tauri.conf.json) |
| pot-desktop | 有，但 label=`daemon` | daemon 是隐藏辅助，不是产品壳 | translate / screenshot / config 按需 | 托盘 + 隐藏 daemon | [tauri.conf.json](https://raw.githubusercontent.com/pot-app/pot-desktop/master/src-tauri/tauri.conf.json)、[window.rs](https://raw.githubusercontent.com/pot-app/pot-desktop/master/src-tauri/src/window.rs) |
| Bob | 源码不可用 | — | — | — | — |
| Raycast | 无开源主程序 | 第一方把 Search Bar 写成「the Raycast window」 | AI Chat 官方写「own window」 | 手册描述热键唤起，非常驻 workbench | [Search Bar](https://manual.raycast.com/search-bar)、[AI Chat](https://manual.raycast.com/ai/ai-chat) |
| Alfred | 无开源主程序 | 官方称 Alfred main window，默认非激活 panel | Preferences 另面 | 可关菜单栏图标 | [Appearance](https://www.alfredapp.com/help/appearance/) |
| CopyQ | Qt，非 Tauri | 有可隐藏的 main window（管理器） | tray menu 可独立 | `hide_main_window` | [faq.rst](https://github.com/hluk/CopyQ/blob/master/docs/faq.rst) |
| uTools | 主程序未开源 | 不可核验 | 不可核验 | 不可核验 | 仅有 [api-types](https://github.com/uTools-Labs/utools-api-types) |
| Claude Desktop | 无公开创建源码 | 官方有完整桌面壳 + quick entry 小窗 | quick entry 官方称 small window | 菜单栏/托盘可开 | [Academy](https://academy.claude.com/tutorials/navigating-the-claude-desktop-app) |
| VS Code | 无 tauri 表 | CodeWindow = workbench 壳 | Command Palette 是窗内 overlay | 启动即开 workbench | [windowsMainService.ts](https://github.com/microsoft/vscode/blob/4c055a03/src/vs/platform/windows/electron-main/windowsMainService.ts) |
| Slack / Discord / Notion / 官方 ChatGPT 桌面 | 无第一方窗源码 | — | — | — | 不推断 |

## 6. 直接回答四个决策问题

### 空 `windows: []` 是常见 Tauri 模式还是少见？

**契约上常见，产品样本里少见。**

- 官方 Default 就是 `[]`。([config](https://v2.tauri.app/reference/config/))
- 官方 issue 里有应用声明自己的 windows 段为空。([#10950](https://github.com/tauri-apps/tauri/issues/10950))
- 官方插件答复把「从 tauri.conf 删掉窗、只在 Rust spawn」列为合法解。([#597](https://github.com/tauri-apps/plugins-workspace/issues/597))
- 实际抓到的热门 Tauri 应用：pot 留一扇 **daemon**；Coco 留多扇隐藏窗且 `main`=启动器；lencx/ChatGPT **没有 windows 数组**；NextAI Translator **显式 `"windows": []`**。没有发现「热门 Tauri 聊天客户端普遍写 `windows: []`」的集群——因为热门聊天客户端大多不是 Tauri。空数组的生产先例在翻译工具（NextAI）和个别聊天客户端（lencx）里，不在「必须留一个叫 main 的静态项」里。

### 成熟客户端会不会让 chat / workbench 成为 THE main window？

**会。产品壳是 chat/workbench，不是翻译条。**

Cherry / Chatbox / Lobe / Open WebUI / 5ire / VS Code / lencx 的 `core` 都把大壳当主身份。Claude 官方把完整桌面和 quick entry 小窗分开。Raycast 把完整 AI Chat 放进「own window」。

Coco / Alfred / Raycast Root Search / pot 证明的是另一类产品：启动器或翻译工具，主交互本来就是浮层。Kivio 已经不是这类产品的主叙事。

### 成熟工具会不会保留一扇不叫 main 的常顶小窗？

**会把小窗做成按需、并且经常不叫 `main`。**

- pot：静态 `daemon`，翻译窗 `translate`，`skip_taskbar`。
- Cherry：miniWindow 是独立 panel，`skipTaskbar`，不是 `createMainWindow`。
- Open WebUI：spotlight 独立全屏 overlay。
- CopyQ：可以没有可见 main，只留托盘。
- Alfred：默认非激活 panel。
- Coco 把浮层叫 `main`，因为它的产品就是这个浮层。

「常顶小窗」本身常见；**把这扇窗继续叫 `main`，在「已经长出 chat 壳」的应用里，没有一手先例。**

### 对 Kivio 的建议（只到决策，不到实现步骤）

先分开两层：

| 层 | 含义 | Kivio 现在 | 证据指向 |
| --- | --- | --- | --- |
| 产品壳 | 用户认为「打开 Kivio」看到的那扇窗 | 已经是 `chat`（Rust 创建、几何持久化、冷启动隐藏再 reveal） | 成熟客户端一致 |
| 配置数组 | `tauri.conf.json` 里有没有静态 `WindowConfig` | 只剩翻译条模板，还叫 `main` | 这是脚手架残留，不是产品壳 |

**空配置（实现）≠ 没有产品壳（产品）。** 选 B 不会让 Kivio 变成「没有主窗的托盘工具」；chat 壳已经在 Rust 里。选 A 也不会让翻译条变成产品壳；只是换掉错误的默认名并留下几何模板。

在 A / B 之间：

- **A 合法，且正好是 #11032 的设计意图**（JSON 留模板，`create: false`，Rust `from_config`）。Kivio 的 `ensure_main_window` 已经走这条路，只是 label 还叫 `main`。
- **B 也合法，且更贴近 Kivio 已经对 chat / lens / translate 采用的工厂，以及 Electron 成熟客户端「壳和浮层都在主进程代码里创建、没有静态窗表」的现实。** pot 证明翻译工具甚至不必把翻译窗写进 conf；lencx 证明 Tauri 聊天客户端可以完全不写 `windows`。
- 两条都要求：**翻译浮层不再使用 `main` 这个默认身份。** capabilities 按 label 授权，与数组是否为空无关；`get_webview_window("main")` 对不存在的窗是 `None`。macOS Regular/Accessory 由 chat 身份驱动，不由 conf 里是否写着 `main` 驱动。

**建议：产品上按成熟客户端处理——chat 是壳，翻译/Lens 是按需浮层。实现上倾向 B**（空 `windows`，翻译窗与现有 overlay 同一条创建路径），因为当前唯一静态项已经 `create: false`，并不能提供「启动即有壳」的价值，却把默认 label 占用在遗留工具上。**A 可接受**，若更想保留官方 `from_config` 模板；那只是实现偏好，不是「更像成熟客户端」。  
不要为了「看起来有 main」去把 chat 再写回 `app.windows`：那会和现有的 Rust 冷启动 / hide-reuse 打架，也不是 Electron 同行的做法。
