# 对话切换检查与 T3 Code 对照

本次检查覆盖选择入口、窗口所有权、缓存校验、磁盘读取、IPC、列表渲染、滚动恢复和切换完成状态。下方“桌面实测补充”是后续真实 WebView 证据，优先于前期源码检查的推测。用户要求先查清楚，当前不实施修复。

## 已确认并修改

`conversationWarmCache.ts` 原先在离开后 30 秒释放快照。正常查看另一条对话一分钟再返回时，即使数据没变化，也失去复用机会。保留时间改为 5 分钟，4 条和 24 MiB 序列化体积估算上限不变，每次复用仍校验后端 revision。缓存体积估算不是 JS 堆内存上限。正在生成的对话仍按既有路径处理。

新增回归用例在修改前失败：离开一分钟后无法取得相同快照。修改后覆盖复用、版本失效和五分钟到期释放；导航、历史窗口和首屏完成测试一起通过，共 62 项。

T3 的共享客户端 [threadRetention.ts](https://github.com/pingdotgg/t3code/blob/0f0c1ca196943505ef454c0002a3a9918919e2ed/packages/client-runtime/src/state/threadRetention.ts) 使用五分钟闲置快照期限。此次只采用适合现有缓存负责人的保留策略，没有引入另一套状态管理。

## 整条路径的发现

- **选择和并发保护**：`chatNavigationController.ts` 先确认弹出窗口所有权，再读取目标对话；请求代次阻止旧读取覆盖新选择。这是正确性约束，本次保留。
- **缓存命中**：前端保留已加载的显示快照，后端 revision 查询只读索引，不传完整正文。缓存登记在空闲时执行，但会 `JSON.stringify` 整个快照；它仍是主线程工作，尚未测出是否构成长任务。
- **冷打开**：`catalog.rs::chat_get_conversation_window` 经 repository 读入整份会话，外置旧内嵌图片、对账工具分段、剥离模型转录，然后才选择显示窗口。分页减少 IPC 和渲染负担，不能消除整份 JSON 的读盘与反序列化成本。
- **窗口边界**：已有最多 60 条、约 512 KiB 的窗口选择；单条超大消息仍完整保留，首条回答会带上相邻用户消息/多答组。它不是绝对 512 KiB 的传输限制。
- **滚动恢复**：`Chat.tsx::readConversationWindow` 在缓存未命中且记住的位置不在底部时改读完整历史。这样能恢复旧位置，但对大对话存在额外成本。后续若改为围绕消息锚点读取，需要一起覆盖滚动定位、历史目录和分页合并，不能直接删除这个分支。
- **渲染**：MessageList 已有虚拟列表、行高缓存；完成态工具过程通过折叠组件按需挂载。并非全部工具记录都会变成 DOM。
- **遮罩更正**：`ChatConversationPane.tsx` 仅在目标数据未到达时遮挡。MessageList 最长两秒的布局等待仍存在，但不能据此声称页面必然被遮两秒。该等待检查所有已挂载图片与异步块，包含屏幕外内容，值得在实机追踪中区分。

T3 的 [threads.ts](https://github.com/pingdotgg/t3code/blob/0f0c1ca196943505ef454c0002a3a9918919e2ed/packages/client-runtime/src/state/threads.ts) 分开管理快照、增量订阅、历史游标和较早页面；保留快照可以带序列位置恢复。其 [threadDetail.ts](https://github.com/pingdotgg/t3code/blob/0f0c1ca196943505ef454c0002a3a9918919e2ed/packages/client-runtime/src/state/threadDetail.ts) 将结构、可见项和正文订阅拆开，桌面端由 `apps/web/src/state/entities.ts` 使用。Kivio 也有分页、缓存和流式隔离，主要差距是冷读取仍依赖整份 JSON，以及旧阅读位置回退到完整历史。此次没有足够实机测量依据启动存储重构。

## 验证边界

读取用户指定对话做了临时离线组件诊断，没有修改聊天记录。分别传入最后两条和全部十四条消息，均只有约 170–180 个 DOM 节点，列表报告约四个挂载行。该检查运行于 jsdom，不能代表 WebView 布局、磁盘、IPC 或真实点击延迟；也不能拿两次挂载时间作优化前后比较。临时诊断文件已删除，私人对话内容未加入仓库。

以上是前期检查边界：当时仅调整缓存期限和对应测试，尚未完成桌面切换实测。不能将缓存期限调整称为卡顿问题已解决。

## 桌面实测补充：两个独立问题

### 查看动作推进版本，导致失效与额外全量读取

已在运行中的 Kivio Dev 上复现，不发送消息，仅在两个现有对话间切换：

1. `Chat.tsx` 在当前对话 ID 变化时调用 `refreshContextStats` 与 `chatSyncState`。
2. `chat_get_context_stats` 计算统计后调用 `persist_context_state_best_effort` → `repository.update_context`。`apply_context_state_update` 每次递增 conversation revision，并持久化整份对话与索引；统计含 `last_measured_at`。
3. `useConversationContext.ts` 接到结果仅更新上下文统计展示，没有把返回的 conversation revision 合入当前显示快照。协议模块也独立维护自己的 revision。
4. 临时监听协议通知时，连续四次切换均观测到 `resync_required`，紧接着 `readConversation` → `reloadConversation` → `getConversation` 全量读取。通知栈精确落在运行模块 `chatProtocol.ts:365`，对应源码 `validated.conversationRevision > knownRevision` 分支，而非 missing run 分支。
5. 另一次原生侧栏点击前后读取磁盘元数据：revision 从 139 到 140，消息数组 SHA-256 均为 `4cd63e2a836db9dc005ea49775e5284b39fa5954935e50a3ef4574ac97e27640`，`updated_at` 不变，上下文统计哈希变化。没有新增聊天内容，单纯查看已经写盘并推进版本。

这是比缓存 TTL 更上游的问题：统计刷新推进内容缓存和协议同步使用的同一版本，而显示快照与协议版本的更新时点不同。其结果可能是下一次缓存校验失效，也可能是首轮显示后再全量读取；具体先后受统计和同步的完成顺序影响。不得简单取消 revision/CAS 校验，因为它也保护真实内容变更与并发写入。

### 缓存命中后仍有同步渲染长任务

原生点击录制文件：`../../.scratch/conversation-switch-baseline.json.gz`。目标 renderer PID 18828、TID 77588；切回大对话的 click 时间戳为 737732384830 μs。

- 点击后约 2 ms 发出 revision 查询，约 8 ms 后完成；首轮显示前没有全量读取，符合缓存命中路径。
- 约 10 ms 后开始一个 182.068 ms 的主线程任务。CPU 采样归因约 170 ms 位于 `showConversation` → `flushSync`，其中同步 React render 约 135 ms、commit 约 35 ms。
- render 内 Streamdown `Ct` 的 Markdown parse/transform/生成 React 节点累计约 79 ms。该值包含解析器约 51 ms，不能相加。CPU 采样是近似归因，不是每个函数的精密计时。
- `settledMarkdownCache` 缓存的是规范化 Markdown 源文本和标题提纲，没有复用 Markdown AST；重新挂载仍进入解析链。
- 点击后约 329 ms 另发起全量 `chat_get_conversation`，请求至完成约 102 ms。此时长包含 IPC 与前端调度，不能直接称为后端磁盘耗时。后续独立监听确认了上述 resync → reload 路径，但不把两次采集混称为同一条轨迹。

缓存避免读取，并没有保留已挂载消息树；切换仍同步重建可挂载的历史内容。这是实际主线程阻塞证据。`flushSync` 同时承载对话和流式预览的一致性约束，不能未经验证直接删除。

## 数据规模与证据边界

目标对话 JSON 约 8.15 MiB，14 条消息、453 次工具调用。该值是磁盘文件体积，不是该对话独占内存，也不是每次 IPC 的体积。工具记录、推理和模型历史占主要空间；折叠工具记录不等于全部挂载 DOM。

- 本轮预渲染范围试改已经撤回；它尚未证实为主要根因。其试验前后加载消息数不同，且遇到热重载，不能报告加速比例。原有缓存期限调整保留。
- 录制启动时约 883 ms 的 profiler 任务已排除，不计入切换卡顿。`conversation-visible` 指标记录生命周期，不能当作首屏耗时。
- 调用来源监听通过临时包装已加载模块与临时协议订阅完成；脚本使用 finally 恢复。随后页面发生热重载，临时运行时探针随旧页面释放。没有继续修改功能代码。
- 后续调用来源复查期间开发页面发生热重载，且工作区有并行变化；这些样本只用于核实调用路径，不用于速度比较。尚未测正式发布构建，也未隔离单个对话的 JS 堆占用。
- T3 对照仍以本文所链接提交为准：其快照保留、正文订阅与历史分页分开管理。不能仅对齐五分钟 TTL 就认为两者切换路径等价，也未做两款应用的同机性能跑分。

后续修复应先处理统计写入与显示/协议版本之间的失效关系，再针对已确认的同步 Markdown 挂载成本验证最小改动。验收需同时保留真实编辑后的失效、并发写保护、旧阅读位置恢复和流式预览一致性。当前仅记录诊断，不实施这些修复。

## 后续修改（同日）

上文两处结论已被后续修改取代：正在生成的对话不再按完整历史读取，旧阅读位置也不再一律回退到完整历史。

- **气泡挂载时的引用解析**：`artifactReferences.ts::referencedArtifactIds` 原先把正文与全部步骤文字拼成一个字符串整体解析。用指定对话的真实数据在无头 Chrome、开发模式下测量，切到完整历史约 520 ms，其中约 470 ms 在这一处；改为只解析提到 `artifact:` 的文本并缓存最近 64 段后约 80 ms，末尾窗口由约 85 ms 降到约 27 ms。
- **正在生成的对话走首屏窗口**：`chatNavigationController.ts` 的选择与路由恢复不再因为对话在运行而读取完整历史。运行期间草稿只追加到独立日志（`draft_journal.rs`），主会话文件不变，窗口的切取依据与未运行时相同。按消息定位的搜索跳转仍读完整历史。
- **正在生成的对话也进入切换缓存**：`Chat.tsx::prepareSelection` 离开时一并登记。复用前仍校验后端 revision，运行结束写入终态后 revision 变化，缓存自然失效。运行中登记的快照另带一条有效条件：只在同一次运行仍在进行时复用（`conversationWarmCache.ts` 的 `stillCurrent`），避免运行失败且未写入任何内容时沿用旧快照。
- **阅读位置只在需要时读完整历史**：`chatReadingPosition.ts::readingPositionNeedsFullHistory` 先看首屏窗口是否仍包含离开时所在的行；包含则直接用窗口，停在实时回复行时同样用窗口。行在窗口之外时会多一次窗口读取，再读完整历史。

验证：`src/chat` 全部 258 个测试文件、2475 项通过；`tsc --noEmit`、ESLint、架构检查通过。新增用例在修改前失败，覆盖运行中对话的窗口读取、离开后返回不再读取、阅读位置判断和分段引用解析。

未执行：桌面端实机切换测量。Rust 端读取、IPC 传输的耗时没有实测；冷读取仍要读入并反序列化整份会话 JSON，这一点未改。运行结束时仍会回载完整历史，窗口被完整历史替换时，未跟随底部的读者可能看到小幅位移（组件测量中约 100 px）。

## 打开对话触发的重读与重写（同日）

打开对话会调用 `chat_get_context_stats` 刷新上下文统计。原实现每次都把统计写回会话文件并让 revision 加一，即使只有测量时间变了。后果有三处：切换缓存按 revision 校验，返回时必然失效；`chatProtocol.ts` 的同步发现 revision 变大，上报 `resync_required`，`Chat.tsx` 随即读取完整历史；命令本身读两遍会话文件、重写整份文件和索引，并把完整会话随响应传回，而前端只用其中的 `contextState`。

修改：

- `repository.rs::context_state_changed` 比较统计时忽略测量时间。`context.rs::persist_context_state` 在统计未变时不写文件、不动 revision，也不再做第二次读取。
- 统计确实变化并写入时，`chat_get_context_stats` 发出带新 revision 的 `ContextUpdated` 会话事件，发起刷新的窗口据此更新已知 revision，不把自己的刷新当成外部修改。写入因版本冲突让位时不发事件，真实的外部修改仍会触发重新同步。
- 响应只返回 `contextState`，`chatApi.getContextStats` 的返回类型随之收窄。

统计真的变化时内存中的会话 revision 仍落后一次，下次返回会多读一次首屏窗口，不读完整历史。

## Rust 端读取耗时实测

用指定对话的 8.5 MB 文件，在与 Dev 应用相同的优化级别下测量四次：读文件约 3 ms，反序列化约 19 ms，转成 `serde_json::Value` 约 9 ms，再序列化约 10 ms。冷读取合计约 25 ms，不构成切换瓶颈，因此没有为窗口读取增加后端缓存或流式解析。

## 首次显示的渲染

修复引用解析后重新采样：首次显示该对话的末尾窗口约 100 ms（开发模式），其中约 40 ms 是各行挂载时的同步测高触发的布局，Markdown 相关约 12–20 ms；再次显示约 30 ms。剩余部分是新内容本身的排版，没有再改列表测量逻辑。另一份分析里的 182 ms / Markdown 79 ms 来自不同的测量环境，本次没有在同一环境复测，两组数字不能直接比较。

验证：`src/chat` 与 `src/api` 共 270 个测试文件、2559 项通过；Rust `chat::repository`、`chat::commands` 测试通过；类型检查、ESLint、架构检查通过。仍未执行桌面端实机切换测量和正式版测量。
