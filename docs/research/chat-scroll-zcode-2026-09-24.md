# 长对话滚动与 ZCode 对照（2026-09-24）

## 范围与证据

用户报告长聊天滚动卡顿、快速滚动抽动、定位不准。使用真实 MessageList、Markdown 和应用样式，在 Edge 浏览器驱动合成对话复现；不调用模型、不读取用户聊天数据。本次没有运行 ZCode 的相同数据基准，也没有完成 Tauri 原生窗口的人工验收，因此不声称两款应用存在某个确定性能倍数。

ZCode 固定参考提交：`328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f`。

## ZCode 的实现与可借鉴部分

- [ConversationTimeline](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/packages/ui/src/v4/ConversationTimeline.tsx)：使用 TanStack Virtual、稳定的行 key、实测高度缓存、ResizeObserver；历史区虚拟化，正在生成的尾部单独放在文档流中。Kivio 已有相近结构，不需要更换虚拟列表库。
- 同一文件保留虚拟列表默认同步更新。Kivio 原来 `useFlushSync: false`，只有手动展开动画才显式同步测高。屏外行高度变化时，scrollTop 已补偿而 DOM transform 尚未提交，能直接产生一帧错位。
- [timelineScrollAnchor](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/packages/ui/src/v4/timelineScrollAnchor.ts)：区分用户滚动权和布局补偿，阅读历史时只补偿位于视口上方的行。Kivio 已有这一规则，问题在补偿与渲染的提交时序。
- [code-block](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/packages/ui/src/components/ai-elements/code-block.tsx)：代码容器使用 `content-visibility: auto` 与 `contain-intrinsic-size: auto 200px`。不能直接照搬。Kivio 的延迟渲染岛原来用固定 intrinsic size，实际会把屏外长代码块测成 112px，再恢复到真实高度。
- [message](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/packages/ui/src/components/ai-elements/message.tsx)：完成态 Markdown 使用 static 模式，自定义代码块渲染，关闭正文流式淡入；这些方向与 Kivio 的现有缓存、完成态分块渲染基本一致。

## 已落地的修复

1. **同步提交位置和测量结果。** 滚动事件同步挂载新可见范围；ResizeObserver 测高同步更新兄弟行位置。React ref 挂载阶段单独降为普通通知，避免在 commit 内递归 flushSync。保留已有导航、跟随和宽度锚点的职责。
2. **按像素限制屏外渲染。** 每侧以两屏高度为预算，最多六条；长回复通常仅需一条相邻回复，短消息仍保留原有缓冲。导航和宽度恢复的强制挂载邻域继续保留。
3. **保持代码占位的真实布局。** 延迟高亮时继续显示完整纯文本；移除渲染岛的浏览器 intrinsic-size 替代布局，代码块不再受 112px 最小高度约束。Mermaid、HTML 等未知高度内容仍可保留各自的最小占位高度。

## 复现与测量

普通负载为 F2：20 轮问答、200 个代码块。压力负载在同一数据中将每个代码块扩为约 52 行。每 40ms 向上滚动一次，共 30 次，步长分别为 750px 和 9000px。逐 rAF 记录视口内锚点、反向移动、空白帧、挂载节点数量及帧间隔。

原始普通负载可见约 470px 的反向跳动，缩到 8 轮也能复现。单独减少预渲染虽然减少节点，但极快滚动仍暴露出占位高度失真和空白帧，因此最终同时修复布局与提交时序。

| 测试 | 修改前/隔离对照 | 最终复测 |
| --- | --- | --- |
| 普通负载反向跳帧 | 可重复复现，原始一次扫描 19 帧 | 0 |
| 普通负载空白帧 | 未记录完整原始基线 | 0 |
| 普通负载挂载节点峰值 | 2043 | 1023 |
| 压力负载挂载节点峰值 | 21897（仅修同步、固定六条缓冲） | 11088 |
| 压力负载 P95 帧间隔 | 20.8ms（同上） | 8.4ms |
| 压力负载最大帧间隔 | 37.5ms（同上） | 33.3ms |
| 压力负载反向跳帧 / 空白帧 | 中间版本仍会出现空白 | 0 / 0 |

以上是本机开发模式、带测量探针的合成数据结果，受调度、缓存和硬件影响；P95 改善不代表完全消除冷挂载的长帧。连续向下滚动普通负载也通过，未见反向跳帧或空白。6 个消息导航落点误差 0px；另加标题的临时负载中，5 次标题跳转距预期 16px 顶部留白的误差均为 0.5px。未复现独立的导航目标选错问题。

## 回归入口

单元/组件回归 `src/chat/MessageList.scrolling.test.tsx` 使用真实虚拟列表：验证同一 observer delivery 中滚动补偿与 transform 一致、快速滚动同一 delivery 挂载目标范围，以及长短消息不同的缓冲预算。同步位置和缓冲预算用例均验证过修改前失败。

浏览器回归保留为 `scripts/fixtures/chat-scroll.html` 和 `scripts/probe-chat-scroll.playwright.js`。真实布局不能由 jsdom 的虚拟尺寸可靠覆盖，浏览器探针直接断言反向跳帧和空白帧为零；耗时仅记录，不设置依赖机器性能的硬阈值。

先启动 `npm run dev:ui`（已有桌面开发服务时无需再启动），再运行：

```powershell
playwright-cli -s=chat-scroll open http://localhost:5713/scripts/fixtures/chat-scroll.html --browser=msedge
playwright-cli -s=chat-scroll run-code --filename=scripts/probe-chat-scroll.playwright.js
playwright-cli -s=chat-scroll eval "window.chatScrollReport"
playwright-cli -s=chat-scroll close
```

此探针依赖独立安装的 playwright-cli，不是 npm test 的隐式依赖。

最终检查：16 个相关测试文件、206 项测试通过；typecheck（含协议检查）、lint、architecture:check 和 git diff --check 通过。首次高并发运行中 F3 渲染测试超过 5 秒，单独重跑通过；随后限制为两个 worker 重跑全部 16 个文件，206 项全部通过，未放宽测试超时阈值。
