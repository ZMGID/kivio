# Settings 前端缓存契约（settingsCache）

> 读写 `Settings` 的前端约定。违反会重演“旧值覆盖后端刷新”的 bug。

来源：`src/api/settingsCache.ts`（任务 `07-06-chat-window-first-open`）。

---

## 为什么存在

后端 `get_settings` 是纯内存读，但每次 `invoke` 都要在 Rust 端整体 clone `Settings` + 序列化 + 前端 `normalizeSettings`。一次 chat 冷启动会有 5-6 个组件各自独立读取。缓存把首读之外的都变成即时返回，并让 `SettingsShell` 用缓存做首帧渲染（SWR，不转圈）。

---

## 硬性约定

1. **读 settings 一律走 `settingsCache`，不要直接 `api.getSettings()`。**
   - 纯展示读 → `getSettingsCached()`（命中缓存即时返回；并发首读去重为一次 invoke）。
   - 首帧要立即渲染、可容忍片刻旧值 → `peekSettings()`（同步）+ 后台 `refreshSettings()` 校准（SWR）。

2. **“读-改-写整个 Settings”必须用 `refreshSettings()`（现读），不能用缓存快照。**
   典型：Chat 的审批策略切换 / MCP 服务器开关、SkillCenter 保存。
   原因：后端 OAuth 令牌刷新（`src-tauri/src/mcp/manager.rs` `persist_refreshed_server`）会**绕过前端**直接改写 `chat_tools.servers[].auth/headers` 并落盘，缓存无对应失效。若用缓存快照整体保存，会把刚刷新的 token 覆盖回旧值（下次连接才自愈）。
   - 只改 servers 之外字段、但要整体保存的写方（如 SkillCenter 只改技能字段）：保存前 `refreshSettings()`，并让 `servers` 取自 fresh、其余字段用本地编辑值。

3. **所有写 settings 的路径必须走缓存写通入口**，否则缓存变旧：
   - `saveSettingsCached()` / `importSettingsCached()` / `setFavoriteModelsCached()`。
   - 严禁在组件里直接 `api.saveSettings/importSettings/setFavoriteModels`。

4. **失败语义**：读失败不写缓存（下次重试）；保存失败不动缓存。与“加载失败不合成默认值、避免错误态 Save 覆盖磁盘真实数据”一致。

5. **返回值视为只读**：`getSettingsCached()` 返回共享缓存引用，修改请用展开生成新对象。

---

## 生命周期与豁免

- 缓存是 **per-webview 模块单例**，随 webview 销毁而丢弃（Kivio 窗口关闭即 destroy）。
- 跨窗口一致性刻意豁免：translator/lens 是短命窗口且不写 settings；settings 编辑入口只在 chat 窗口内。
- 已知残余竞态：`SettingsShell` 可编辑 servers，其长驻草稿与后端 OAuth 刷新存在字段级竞态——属既有问题，靠 SWR pristine 校准缓解，非缓存新引入。

---

## 相关

- `src/settings/SettingsShell.tsx` — SWR 首帧 + 草稿 pristine 保护。
- `src/App.tsx` / `src/chat/Chat.tsx` — chat 窗口首次 reveal 门控（内容就绪才 show + 3s 兜底）；与本缓存同任务落地。
