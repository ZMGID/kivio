# Implement — 模型收藏置顶

上下文顺序:本文件 + `prd.md` + `design.md`。Windows Rust 测试用 `scripts/win-cargo-test.ps1`（见记忆 [[windows-rust-test-manifest]]）。前端可 Vite HMR 实时看效果。

## 执行清单（有序）

### A. 后端 Settings 字段
- [ ] A1 `settings.rs`:在 `obsidian_vault_path` 附近加 `#[serde(default)] pub favorite_models: Vec<String>,`。
- [ ] A2 编译确认无"缺字段初始化"报错(Settings 若有手写全字段构造点需补;default 派生/`..Default::default()` 无需)。跑 `cargo check`。

### B. 轻量持久化命令
- [ ] B1 `commands.rs`:新增 `set_favorite_models(app, state, models: Vec<String>) -> Result<(),String>`:去空/trim/去重保序 → `state.settings_write().favorite_models = cleaned` → `persist_settings(&app, &settings)`(不走 apply_settings)。
- [ ] B2 `lib.rs`:`invoke_handler` 注册 `set_favorite_models`。
- [ ] B3 去重保序小工具 + 单测(空、重复、trim)。

### C. 前端 api 绑定
- [ ] C1 `tauri.ts`:`Settings` 加 `favoriteModels?: string[]`;`normalizeSettings` 加 `favoriteModels: current.favoriteModels ?? []`(**关键**:保证 getSettings→saveSettings 往返不丢,`prepareSettingsForSave` 是 `...settings` 透传)。
- [ ] C2 `tauri.ts`:`api.setFavoriteModels = (models) => invoke<void>('set_favorite_models', { models })`。

### D. ModelSelector UI
- [ ] D1 载入时把 `settings.favoriteModels` 存入本地 state(与 providers 一起)。
- [ ] D2 `favoriteKey(providerId, model) = ` `${providerId}:${model}` ``;`toggleFavorite`:乐观更新本地 + `api.setFavoriteModels(next)`,失败回滚 + console.error。
- [ ] D3 每个模型行(provider 分组内)右侧加星标按钮:`★`(已收藏)/`☆`,onClick `stopPropagation` + toggle。
- [ ] D4 下拉顶部渲染"收藏"分组:按 `favoriteModels` 顺序,过滤出仍有效的(provider 在 activeProviders 且 model 在其 models 列表)→ 行 = `ModelIcon + model`(+ 可选 provider 名),点击 `onModelChange`,右侧实心星标可取消。空则不渲染该组。
- [ ] D5 样式与既有选中高亮共存;星标用 lucide `Star`（描边/填充切换）。

### E. 验证
- [ ] E1 `cargo check` → `powershell -File scripts/win-cargo-test.ps1 --lib favorite`(命令去重单测 + settings 序列化默认值)。
- [ ] E2 `npm run lint && npm run typecheck`。
- [ ] E3 dev app 手动 smoke:收藏一个模型→顶部出现→重启 app 仍在→点收藏项能切换→取消收藏消失→删/禁用对应 provider 后收藏项不显示不报错。（chat-probe 只能测生成,不能驱动这个 UI;手动 smoke。）

## 验证命令
- `cd src-tauri && cargo check --no-default-features`
- `powershell -File scripts/win-cargo-test.ps1 --lib favorite`
- `npm run lint && npm run typecheck`

## 风险 / 回滚
- 已澄清:`settings_write()` 存在;`prepareSettingsForSave` 透传 → 只要 normalizeSettings 带 favoriteModels 就不会被设置页保存覆盖(C1 是关键)。
- 回滚:删 settings 字段 + 命令 + 注册 + UI/api 改动;无 schema 破坏、无迁移。
