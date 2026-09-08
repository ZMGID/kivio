# macOS Kivio 卡死排查（v2.9.7）

## 已知现象与范围

- 用户确认：卡住的是 Kivio；Windows 正常；问题始于本版或最近一两个版本。
- 用户补充对照结果：测试者在旧版关闭通知后未再出现卡死。这是通知路径的触发证据，不是修复版已通过实机验证，也尚不能区分窗口查询与原生发送中的具体阻塞点。
- 尚缺：macOS 版本、触发动作、卡死时线程堆栈。当前开发环境是 Windows，不能把静态排查当作 Mac 实机复现。
- 对比范围：v2.9.5 → v2.9.6 → v2.9.7，另检查 v2.9.4 → v2.9.5 的弹出窗口变更。

## 确认的问题与修复

1. `a97aa82c` 在 v2.9.7 加入聊天回复通知；`79b3fc71` 加入同步窗口状态 / URL 查询。Tauri runtime 2.10.0 的 getter 在工作线程上发送主线程消息后执行无超时 `recv()`。通知位于 `complete_assistant_reply` 返回之前，而 `ChatSendReservation` 到发送命令返回才释放。因此通知查询受阻会把回复收尾和下一次发送一起拖住。第一轮修复将判断投递到主线程，但仍保留原生窗口接口。此次进一步彻底移除通知路径中的窗口状态 / URL 查询：页面在焦点、可见性、路由变化时报告正在查看的对话；原生失焦、隐藏关闭、销毁时清理缓存。回复收尾只读取缓存，设置或缓存锁忙时不等待，不再向主线程投递通知判断。
2. 原 macOS 发送端使用 `osascript display notification`，不绑定 Kivio 的原生通知身份，不检查退出结果，也不回收 Child。替换为 `UNUserNotificationCenter`：以实际 .app bundle 身份请求授权，授权 / 发送均使用异步回调，记录拒绝和发送错误，不再创建脚本进程。此次将原生调用入口移至独立工作线程，队列上限 16 条，通过 `try_send` 投递，队列满时跳过，排队超过 15 秒则丢弃；系统通知调用阻塞时，回复和 UI 无需等待该线程。Apple 文档允许从任意应用线程使用共享通知中心；授权和发送结果由系统异步回调。前台 delegate 允许显示其他对话的提醒；正在查看的对话仍由聊天层抑制。
3. 原生通知在未打包的 `tauri dev` / `cargo run` 环境不能冒充已安装的 Kivio。调用通知中心前核对 `.app` 路径及 bundle identifier，避免无 bundle 环境的 Objective-C 异常；开发模式会明确记录跳过原因。通知弹窗必须用打包后的 Mac 应用验证。
4. Windows 的注册身份、XML 转义、PowerShell 发送内容保留，进程启动使用 blocking pool。

## 其他回归路径的检查结果

- v2.9.5 到 v2.9.7 的 Tauri / Wry 依赖版本没有变化；不能把它归因于这两版升级了 WebKit 封装库。
- 同一区间 `windows.rs`、`shortcuts.rs` 和 `chat/popout.rs` 没有改动。v2.9.5 新增的弹出窗 / activation policy 路径仍是关闭或切换窗口时需要实机核查的候选，尚未找到能够证明死锁的锁等待链。
- 最近的消息宽度测量、live → history 交接和滚动布局确有改动。现有纯逻辑 / jsdom 测试不能排除 macOS WebKit 独有的渲染问题，没有凭猜测回滚这些功能。
- 没有证据证明 osascript 未显示通知本身会把整个 UI 锁死；原发送端使用 spawn，未同步等待脚本退出。

## 卡死现场的取证

macOS 增加独立原生主线程 watchdog：每 5 秒最多投递一个心跳，连续两次 10 秒未获响应时调用系统 `/usr/bin/sample` 采集 Kivio 的 1 秒线程堆栈。同一次卡死只采一次；采样器超过 10 秒则终止并回收；只保留 3 个循环覆盖的文件；睡眠或严重调度延迟后重新计时。

文件位置由 Tauri `app_log_dir()` 决定，macOS 为：

```text
~/Library/Logs/com.zmair.kivio/hangs/kivio-hang-0.txt
~/Library/Logs/com.zmair.kivio/hangs/kivio-hang-1.txt
~/Library/Logs/com.zmair.kivio/hangs/kivio-hang-2.txt
```

如新包再次卡住，等待约 25 秒后再强退，并保留上述文件。线程栈可用来区分原生互斥锁等待、AppKit / WebKit 同步调用阻塞和其他路径。它不会上传数据。

**边界：** watchdog 检查的是原生主线程。若仅网页渲染进程卡住、原生主线程仍有响应，可能没有自动采样文件。此时在「活动监视器」对 Kivio 执行「取样进程」，并检查其对应 WebKit WebContent 进程；没有采样文件不能当作“没有卡死”。

## Mac 实机验收

1. 安装并打开打包的 `.app`，在设置中手动开启「回复完成通知」（默认关闭），切到其他应用后完成一条回复：首次出现 Kivio 通知授权，允许后应收到预览。
2. 留在当前对话完成回复：无多余提示，马上可以发送下一条；主窗查看其他对话时应有提醒。
3. 拒绝通知权限：回复和下一次发送仍正常。随后在系统通知设置允许 Kivio，下一次应恢复提醒。
4. 覆盖主窗前台、切换其他应用、最小化、隐藏保活、关闭重开和弹出对话窗口；分别跑单模型及多模型回答。
5. 对长对话进行回复收尾、窗口缩放和侧栏 / Dock 展开收起；如果再卡死，以现场堆栈继续定位，不预先认定通知是唯一根因。

## 本次验证结果

- 本轮 Windows Rust：`scripts/win-cargo-test.ps1 --lib notification` 共 19 项通过，包含工作线程模拟通知服务阻塞 / 队列满、查看状态缓存更新及锁忙不等待测试。
- 本轮前端：`notificationView`、`tauri.normalizeSettings`、`GeneralTab` 三份测试共 30 项通过，覆盖焦点、隐藏、路由、卸载及 IPC 未返回 / 拒绝；`tsc --noEmit` 和修改组件 / 新模块的 ESLint 检查通过。
- 本轮 macOS 通知模块及其测试再次通过下述隔离工程的目标平台类型检查，尚未在 Mac 实机执行。
先前验证记录：

- Windows Rust：`scripts/win-cargo-test.ps1 --lib notification`（16 项）、`--lib automation::notify::tests`（4 项）、`--lib macos_hang_watchdog`（2 项）均通过；去掉重复项共 20 项。其中通知 XML / 注册测试不实际发送通知。
- 前端：`useChatWidthLayout`、`useLiveRowMeasurement`、`useScrollFollow` 和 `messageListVirtualization` 四份测试共 57 项通过。
- 新增的原生通知和 watchdog 模块，连同 Mac 专属请求构造测试，已用 `aarch64-apple-darwin` 执行 `cargo check --tests`。为避开 Windows 无 Apple SDK 的限制，使用隔离检查工程并对少量 Tauri AppHandle 方法提供编译桩；Objective-C / UserNotifications 绑定使用真实依赖。这是新增模块的目标平台类型检查，不等同于整个 Mac 应用构建或实机测试。
- 修改的 Rust 文件格式检查及 `git diff --check` 通过。尚未在 Mac 执行原生通知测试、展示授权弹窗或复现窗口卡死。
