# Design — 模型收藏置顶

## 架构

三层小改动,数据流单向清晰:

```
ModelSelector (点星标)
  → api.setFavoriteModels(models)         // 前端 api 绑定
  → set_favorite_models 命令 (Rust)        // 轻量:改内存 settings + persist_settings
  → settings.json 的 favoriteModels 持久化
ModelSelector 下次 getSettings → 读回 favoriteModels → 渲染"收藏"置顶组
```

收藏是**用户级全局**状态(不按会话),存 settings。

## 后端

### 1. Settings 字段(`settings.rs`)

在 `obsidian_vault_path` 附近加:

```rust
/// 收藏并置顶的模型键（"providerId:model"），列表顺序即置顶顺序。
#[serde(default)]
pub favorite_models: Vec<String>,
```

`#[serde(rename_all="camelCase")]` 使其暴露为 `favoriteModels`。所有 Settings 初始化点(default 派生即可,Vec 默认空)——确认无手写全字段初始化遗漏(sanitize_settings 无需特殊处理:字符串列表,顶多 trim/去重,可选)。

### 2. 轻量命令(`commands.rs`)

```rust
#[tauri::command]
pub(crate) fn set_favorite_models(
    app: AppHandle,
    state: State<AppState>,
    models: Vec<String>,
) -> Result<(), String> {
    // 归一:trim、去空、去重、保序
    let cleaned = dedup_preserve_order(models);
    {
        let mut s = state.settings_write();   // 确认 settings_write 存在
        s.favorite_models = cleaned;
        persist_settings(&app, &s)?;          // 只写盘,不重应用运行时
    }
    Ok(())
}
```

- 复用 `persist_settings`(commands.rs 已 import),**不**调 `apply_settings`——避免热键/托盘/自启重注册。
- 在 `lib.rs` 的 `invoke_handler![...]` 注册该命令。

### 3. 前端 api 绑定(`tauri.ts`)

- `Settings` 类型加 `favoriteModels?: string[]`;`normalizeSettings` 补 `favoriteModels: current.favoriteModels ?? []`。
- `prepareSettingsForSave` 若按字段白名单序列化,需带上 favoriteModels(核实其实现;若是整体透传则无需改)。
- 新增 `setFavoriteModels: (models: string[]) => invoke<void>('set_favorite_models', { models })`。

## 前端 UI(`ModelSelector.tsx`)

### 状态

- 组件已 `getSettings()` 载入 providers;改为同时存 `favoriteModels`(从 settings)到本地 state。
- 切换收藏:本地乐观更新 `favoriteModels` + 调 `api.setFavoriteModels(next)`;失败回滚 + console.error。

### 渲染

- **收藏组(置顶)**:在 provider 分组之前渲染。数据 = `favoriteModels` 过滤出仍有效的(provider 启用且模型在其 enabled/available 列表)→ 解析 `providerId:model` → 展示 `ModelIcon + model`(可加 provider 名副标)。点击行 = `onModelChange`。
- **每行星标**:provider 分组内每个模型行、以及收藏组每行,右侧加星标按钮。`★`=已收藏 / `☆`=未收藏。onClick `stopPropagation` + toggle。
- key 约定:`favoriteKey = `${providerId}:${model}``。

### 交互细节

- 星标 toggle:`isFav = favoriteModels.includes(key)`;toggle 生成 `next`(加到末尾 / 移除),setState + 持久化。
- 收藏组为空时不渲染该组(不占位)。
- 与当前选中态样式共存(选中高亮 + 星标各自独立)。

## 兼容性 / 回滚

- 纯增量:新增字段(默认空)+ 新命令 + UI 增强;旧 settings.json 无该字段 → serde default `[]`,无迁移。
- 失效收藏只在**展示时过滤**,不删存储——用户删/禁用 provider 后再恢复,收藏仍在。
- 回滚:删字段 + 命令 + UI 改动;无 schema 破坏。

## 风险

- `settings_write()` 是否存在 / 用法:实现时核实(state.rs 有 settings RwLock,get 用 settings_read)。
- `prepareSettingsForSave` 白名单:若它只挑部分字段序列化,`save_settings`(设置页整体保存)可能覆盖掉 favoriteModels——需确保白名单含该字段,或 set_favorite_models 与 save_settings 各自独立不互相覆盖(收藏走独立命令 + 独立字段,设置页保存时也应带上该字段以免回写空)。**这是最需要注意的点**。
- 测试:Rust 侧加 favorite_models 序列化/默认值用例;命令逻辑(去重保序)单测。
