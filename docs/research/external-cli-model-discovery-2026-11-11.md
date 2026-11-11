# 外部 CLI 模型目录的非阻塞刷新

本轮对照 T3 Code 的模型目录生命周期，保持 Kivio 各 CLI 原有发现协议，优化共同的展示与刷新流程。

## 参考实现

- [providerStatusCache.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/providerStatusCache.ts)：恢复磁盘快照，按 provider 身份合并目录；自定义模型由当前配置提供，不从旧快照复活。
- [ProviderRegistry.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/provider/ProviderRegistry.ts)：保留适用的旧模型，刷新失败不直接清空，阻止已替换实例的迟到结果。
- [ProviderModelPicker.tsx](https://github.com/pingdotgg/t3code/blob/main/apps/web/src/components/chat/ProviderModelPicker.tsx)：打开菜单时异步刷新，展示已有选项。
- [模型目录说明](https://github.com/pingdotgg/t3code/blob/main/docs/internals/model-manifest.md)：Claude 有随包目录，Codex 的实际目录仍来自 app-server。

## Kivio 的改动

旧路径：页面挂载 → 内存缓存未命中 → 启动 CLI / 握手 / 枚举 → 返回模型列表。

新路径：页面挂载 → 返回新鲜缓存、磁盘快照或默认列表 → 必要时后台刷新 → 更新当前页面。

`external_agents::commands` 仍是模型发现入口；`ExternalDiscoveryState` 持有缓存、探测锁、失效代次与快照读写。共享前端 API 顺序调用同一命令的快照与刷新阶段，聊天与自动化都收到初始快照。无需额外广播、订阅或每个 provider 各写缓存。

- 快照最多保留 64 个 CLI / 工作目录 / Kivio CLI 配置组合，原子落盘。配置仅以 SHA-256 摘要进入快照索引，不写入凭据。用户手填模型仍从当前配置合并。
- 磁盘快照只供先展示，不恢复为新鲜缓存，也不据此自动修改对话模型或推理档位。刷新失败保留成功目录与明确错误，使用已有短负缓存避免重复探测。
- 内存缓存与同一目录的探测锁继续复用；并发强制刷新复用请求开始后完成的结果。配置变更会作废在途探测，普通偏好修改不再清空 CLI 发现缓存。
- 选择器切换 CLI / 对话时清空旧列表，离开后忽略迟到结果，刷新不覆盖用户刚作出的选择。重新打开菜单检查缓存；仅在失败时提供重试，重试绕过 TTL。
- Codex 的两个发现入口均失败时明确返回降级状态，避免把静态兜底误记成成功目录。首次无快照仍有默认选项。

Claude 与 dsh 仍使用已有本地目录 / 配置读取；Codex、Pi、ACP、OpenCode、Antigravity 的协议发现继续保留。ACP 的迟到模型通知窗口未缩短；它不再阻塞已有目录的展示。

## 验证边界

回归测试覆盖快照先显示、新鲜缓存不重复探测、用户选择保护、卸载及切换对话后的迟到结果、快照重启恢复、目录隔离、容量、损坏缓存和配置失效。

这是等待路径的改进，不是 CLI 本身启动速度的测量。未以源码中的超时上限宣称实际耗时或提速倍数；真实 CLI 的权限和模型可用性仍以刷新及发送时的实际结果为准。
