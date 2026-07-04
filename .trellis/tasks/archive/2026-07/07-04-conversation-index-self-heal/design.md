# Design — 对话索引自愈防丢失

## 原则

`index.json` 是**缓存**,`conv_<id>.json` 是**真相源**。读取时缓存必须能被真相源纠正;写入时绝不制造"缓存暂时消失"的窗口。全部改动在 `src-tauri/src/chat/storage.rs`。

## R1 索引对账自愈(`load_index_or_scan`)

### 新增:廉价列出对话文件 id(不读内容)

```rust
/// 扫描 conversations 目录，仅按文件名收集有效对话 id（不读文件内容，廉价）。
fn conversation_file_ids(app: &AppHandle) -> Result<Vec<String>, String> {
    let dir = conversations_dir(app)?;
    let mut ids = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("read conversations dir: {e}"))? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        if validate_conversation_id(stem).is_ok() {
            ids.push(stem.to_string());
        }
    }
    Ok(ids)
}
```

`validate_conversation_id` 要求 `conv_` 前缀 → 天然排除 index.json / projects.json / assistants.json。

### 改写 load_index_or_scan

```rust
fn load_index_or_scan(app: &AppHandle) -> Result<ConversationIndex, String> {
    let file_ids = conversation_file_ids(app).unwrap_or_default();
    match load_index(app) {
        Ok(index) => {
            // 索引是否覆盖了磁盘上每个对话文件？缺任一 → 索引残缺/过期 → 以文件为准重建。
            let indexed: std::collections::HashSet<&str> =
                index.conversations.iter().map(|c| c.id.as_str()).collect();
            let covers_all = file_ids.iter().all(|id| indexed.contains(id.as_str()));
            if covers_all {
                Ok(index)
            } else {
                rebuild_and_heal_index(app)
            }
        }
        Err(e) => {
            eprintln!("conversation index unavailable, rebuilding from files: {e}");
            rebuild_and_heal_index(app)
        }
    }
}

/// 从对话文件重扫重建列表，并尽力写回修复 index.json（失败不影响返回）。
fn rebuild_and_heal_index(app: &AppHandle) -> Result<ConversationIndex, String> {
    let index = ConversationIndex {
        conversations: load_conversation_list_from_files(app)?,
    };
    if let Err(e) = save_index(app, &index) {
        eprintln!("heal index write failed (non-fatal): {e}");
    }
    Ok(index)
}
```

要点:
- **覆盖判定用 id 集合**(`file_ids ⊆ indexed`),不是数量比较——精准命中"有文件但不在索引"(即数据被孤立)的真实故障,同时容忍索引里的多余幽灵条目(无害,不触发重扫)。
- 只在**检测到缺失**时才 `load_conversation_list_from_files`(读全部文件),正常路径零额外读。
- 检测到缺失时**顺手 save_index 修复**,使残缺 index 在首次读取后就持久化补全,避免每次读都重扫。
- `conversation_file_ids` 失败降级为空集 → `covers_all` 对空 file_ids 恒 true → 回退到"信任 index"(与旧行为一致,不会因目录读失败而误重建)。

## R2 atomic_write 去缺失窗口

当前(71-74)危险片段:
```rust
fs::rename(&tmp_path, path).or_else(|_| {
    if path.exists() { fs::remove_file(path)?; }  // ← 删掉后若下面 rename 失败，目标就没了
    fs::rename(&tmp_path, path)
})
```
改为:
```rust
let write_result = fs::write(&tmp_path, content).and_then(|_| fs::rename(&tmp_path, path));
```
- Windows `fs::rename` 原子替换已存在目标,无需先删。
- 瞬时失败(锁/杀软)由既有外层 `for attempt in 0..WRITE_RETRY_ATTEMPTS` 循环 sleep 后重试整次写(78-91),**目标文件全程保留旧内容**,绝不出现"中途缺失"。
- 保留 tmp 文件命名(带 attempt 后缀)与失败清理逻辑不变。

## 数据流(修复后)

新建对话生成(高频保存)中即便 index.json 一度缺失:
- 下一次 `save_conversation` → `load_index_or_scan` → load_index 得空/报错 → **重扫 25 个文件** → 得到完整列表 → 插入新对话 → save_index 写出 26 条完整索引。历史对话**永不丢**。
- atomic_write 不再制造 index.json 缺失窗口,从源头降低触发概率。

## 兼容性 / 回滚

- 纯后端加固:index schema、对话文件、前端契约均不变。
- 旧数据无需迁移;残缺/正常 index 都被正确处理。
- 回滚:还原 storage.rs 两处改动即可。

## 风险

- `load_conversation_list_from_files` 在大量对话时读全部文件——但仅在检测到索引缺失时触发(罕见),且随后 heal 写回,后续读不再触发。正常路径只多一次廉价 `read_dir` + id 集合比较。
- `rebuild_and_heal_index` 内 save_index 走 atomic_write;若并发另一实例也在写,atomic_write 已原子替换,最坏是两次都写完整列表(结果正确)。
