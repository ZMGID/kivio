# Paseo 检测/缓存架构（调研归档）

## 可用性层（cwd 无关，未缓存于底层）
- `executable-resolution.ts`：`findExecutable(name)`(108) 只吃命令名（PATH 全局，无 cwd）；`probeExecutable`(61) 跑 `--version`，`PROBE_TIMEOUT_MS=2000`；超时/任意退出码=存在，仅 ENOENT/EACCES/ENOEXEC=不存在。本文件无缓存。
- 单飞缓存在上层 `services/forge-cli-command.ts:179 createCachedCliPathResolver`：memo 路径、共享 in-flight promise、miss 时逐出（后装的 CLI 免重启可见）。

## 模型层（ProviderSnapshotManager，真正的缓存）
- `provider-snapshot-manager.ts`：`snapshots: Map<snapshotCwd, Map<provider, ProviderSnapshotEntry>>`(174)。外层 key = `GLOBAL_PROVIDER_SNAPSHOT_KEY="paseo:global"`(41) 或解析 cwd（空 cwd⇒global，937）。
- `ProviderSnapshotEntry`(agent-sdk-types.ts:104)：`status(loading|ready|error|unavailable)/enabled/models?/modes?/fetchedAt?`——可用性+目录同体。
- 懒 + 先返旧值：`getSnapshotForTarget`(485) 立即返当前 entry，后台 `warmUp`(void,488)。`loadProvider`(708) 是 single-flight 闸门：force=false 返 in-flight promise(715)，已 resolved(status!=loading) 短路(719)。
- 失效**事件驱动、无 TTL**：无 setInterval/ttl/expires；`fetchedAt`(808) 仅信息性。重探仅 `refreshSnapshotForCwd/refreshSettingsSnapshot`(force=true,214/223) 或 `applyMutableProviderConfig`(391 配置变更)。
- `fetchCatalog` 懒 + 逐 provider，绝不 eager-for-all。

## 触发（provider-catalog-session.ts）
- get_providers_snapshot(356)→懒后台 warm；list_provider_models(141)→`getProviderSnapshotEntryForRead`(252) 仅 missing/loading 才 warm **单个** provider(267)；refresh(374)=显式强刷；start(74) 订阅 change 事件推 `providers_snapshot_update`。
- **无启动 eager warm、无定时器、无 per-conversation/per-keystroke。**

## 廉价 vs 昂贵解耦
- `refreshProvider`(750)：先 `client.isAvailable()`(783, PATH resolve, 廉价)；仅 available 才 `fetchCatalog`(794, 起 CLI/ACP 握手/模型列)。不可用者标 unavailable(789)、不付目录成本。可用性 PATH 全局 / 目录 cwd-scoped，非对称。

## 给 kivio 的教训
1. 别按会话 cwd 存缓存 → 按工作区根/全局。
2. 拆可用性(PATH 全局) vs 目录(工作区)，昂贵探测 gated on 廉价可用性。
3. resolved 项无限复用，事件失效非 TTL。
4. 全部 single-flight。
5. 先返旧值/loading、后台 warm、change 推更新。
6. 只懒查选中的 provider，别探 8 个。
