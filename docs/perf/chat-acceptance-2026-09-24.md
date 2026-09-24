# Chat 分阶段加载与过程区验收（2026-09-24）

## 本次交付

- 首次侧栏选择及启动恢复路由使用尾部窗口；取消打开后立即全量补读。
- 尾窗和补页最多 60 条，并使用 512 KiB 序列化消息预算。单条超大消息必须可达，多答组最多额外回溯 4 条及其提问，因而这是软预算，不是内存硬上限。原始消息不截断。
- 首屏窗口可进入已有 30 秒、4 项、24 MiB 保温缓存；完整读取不能误用部分窗口。旧阅读位置不在窗口时仍允许完整读取。
- 搜索当前会话的未加载消息、导航目录跳转按需读取；补页的去重、导航权限、失败重试归现有 navigation controller。A→B→A 可发新请求，旧请求不能提交或解除新请求的锁。
- 历史过程区跨分组合计先挂载最近 20 项，每次展开更早 20 项；最终正文和显式交付产物保持完整。实时输出不移除用户已见步骤，用户显式保持展开时延续到完成；完成后关闭再打开恢复 20 项预算。
- 输入框 ↑ 按需取得完整输入历史，仅提取用户正文，不把完整会话提交给消息列表。失败可用当前已加载输入，键入、发送和切会话撤销迟到回填。
- 增量发布分别以 1、2、7、31 次事件为刷新间隔，验证工具替换、旧投影删除、新运行和取消终态等价。实际事件仍按序应用，未新增另一套合并协议。

后端仍读取完整 Kivio JSON 再裁切，未引入磁盘分页或重读 CLI 原生历史。输入历史的按需请求目前也读取完整 Conversation；其独立轻量接口须有测量依据再增加。Markdown static 是按需试验项，本次保留现有跨块语法兼容模式。

## 自动验证

- 全量：287 个测试文件、2426 项通过；之后最终收尾的 3 个文件、115 项定向回归通过。
- 类型检查、协议生成一致性、lint、architecture:check、build:ui 通过。
- Rust `history_window_bounds_payload_and_keeps_oversized_message_reachable` 通过。
- 缺陷用例观察过修复前失败：历史过程全挂载、重内容窗口不受大小限制、已打开会话搜索未加载消息。

## 浏览器验证

Windows、Edge、开发模式、1280×900；真实 MessageList/Markdown/工具组件、合成数据，无模型请求。宽度在 1280 和 1020px 间切换。测量脚本在资源载入后运行，**切换耗时仅包含同步 UI 提交，不含读盘、IPC、遮罩结束或完整可交互耗时**。

最终采样结果保存在 [原始摘要](./chat-acceptance-2026-09-24.json)。耗时是观察值，不设机器相关硬阈值。

| 场景 | 同步切换 ms | 最大 React commit ms | 宽度变化后挂载 rows | 底部偏差 px | >50ms long tasks |
| --- | ---: | ---: | ---: | ---: | ---: |
| F1：200 条文本 | 30.3 | 15.9 | 16 | 0 | 0 |
| F2：200 个代码块 | 32.1 | 24.1 | 3 | 0 | 0 |
| F3：结构化混合 | 88.5 | 40.5 | 9 | 0 | 2（最大 89ms） |
| F4：20k 流式正文 | 4.9 | 21.1 | 2 | 0 | 0 |

另验证 500 项工具过程：首次展开只挂载 20 项，点击一次增加到 40 项。主动展开会解除跟随，因此不对这一阅读场景断言贴底。

既有快速滚动探针：

| 场景 | 反向跳帧 | 空白帧 | P95 帧间隔 ms | 最大帧间隔 ms | DOM 峰值 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 普通代码负载 | 0 | 0 | 12.7 | 42.2 | 1023 |
| 压力代码负载 | 0 | 0 | 29.9 | 56.6 | 7478 |

F3 长任务和压力负载长帧仍存在。没有同环境旧版本的完整基线，不能声称 M1 的 50% 降幅达标，也不能拿旧研究中的更低帧间隔作本次收益结论。

复现入口：启动 `npm run dev:ui`，然后运行：

```powershell
playwright-cli -s=chat-acceptance open http://localhost:5713/scripts/fixtures/chat-performance.html --browser=msedge
playwright-cli -s=chat-acceptance run-code --filename=scripts/probe-chat-acceptance.playwright.js
playwright-cli -s=chat-acceptance eval "window.chatAcceptanceReport"
playwright-cli -s=chat-acceptance run-code --filename=scripts/probe-chat-scroll.playwright.js
playwright-cli -s=chat-acceptance eval "window.chatScrollReport"
playwright-cli -s=chat-acceptance close
```

## 尚未完成的验收

Tauri 原生窗口的滚动条、跨窗口归属、真实附件，以及冷读/IPC/首屏 paint 的同机对照未测。浏览器组件测试不能替代这些验收；本轮不将整个性能 PRD 标为全部达标。
