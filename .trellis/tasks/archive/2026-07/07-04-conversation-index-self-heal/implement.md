# Implement — 对话索引自愈防丢失

上下文顺序:本文件 + `prd.md` + `design.md`。全部在 `src-tauri/src/chat/storage.rs`。Windows Rust 测试用 `scripts/win-cargo-test.ps1`(见 [[windows-rust-test-manifest]])。

## 执行清单(有序)

### A. R2 atomic_write 去缺失窗口(先做,独立且低风险)
- [ ] A1 `atomic_write`(storage.rs:54):把 `fs::rename(...).or_else(|_| { remove_file; rename })` 改为直接 `fs::rename(&tmp_path, path)`;其余(外层重试循环、tmp 清理、NotFound 建目录)不动。
- [ ] A2 `cargo check`。

### B. R1 索引对账自愈
- [ ] B1 新增 `conversation_file_ids(app) -> Result<Vec<String>, String>`:read_dir → 仅 .json + `validate_conversation_id` 通过的 file_stem 收集为 id(不读内容)。
- [ ] B2 新增 `rebuild_and_heal_index(app)`:`load_conversation_list_from_files` 组装 index + best-effort `save_index`(失败仅 eprintln)。
- [ ] B3 改写 `load_index_or_scan`:先取 `conversation_file_ids`(失败降级空集);`load_index` Ok 时用 id 集合判 `covers_all`(file_ids ⊆ indexed),覆盖则信任、否则 `rebuild_and_heal_index`;Err 时 `rebuild_and_heal_index`。
- [ ] B4 `cargo check`。

### C. 测试
- [ ] C1 storage 层测试(用临时目录 + headless AppState 或直接测纯函数):
  - index 缺失 + 存在 N 个 conv 文件 → load_index_or_scan 返回 N 条,且 index.json 被写回(covers all)。
  - index 只含 1 条但磁盘有 N 个文件 → 返回 N 条(重扫)。
  - index 覆盖全部文件(可含 1 个多余幽灵条目)→ 原样返回,不重扫(可用文件数不变+条目含幽灵验证)。
  - atomic_write 覆盖已存在文件:写 A → 写 B → 读回 B;失败路径保留旧文件(可选,视可测性)。
  - **注**:AppHandle 依赖使这些函数不易纯测。若 headless 构造 AppHandle 困难,则把可测逻辑(id 集合 covers_all 判定、conversation_file_ids 的过滤)抽成接受 `dir: &Path` 的纯函数并测之;load_index_or_scan 的编排靠手动 smoke。实现时按可测性决定抽取粒度。
- [ ] C2 `powershell -File scripts/win-cargo-test.ps1 --lib storage`(或相应过滤)。

### D. 验证
- [ ] D1 `cargo check --no-default-features` 干净。
- [ ] D2 Rust 测试通过(对基线,忽略既有环境类失败)。
- [ ] D3 dev app 手动 smoke(复现场景):
  - 起 app,确认侧栏对话齐全。
  - 手动把 index.json 改成只剩 1 条(或删除),重启 app → 侧栏**自动恢复全部**,且 index.json 被写回完整。
  - 新建对话并发消息 → 历史不丢。
  （chat-probe 不驱动侧栏 UI;此步手动。）

## 验证命令
- `cd src-tauri && cargo check --no-default-features`
- `powershell -File scripts/win-cargo-test.ps1 --lib storage`

## 风险 / 回滚
- 正常路径只多一次 `read_dir` + id 集合比较(廉价);全量重扫仅在检测到索引残缺时发生(罕见)且随后 heal。
- 回滚:还原 storage.rs 的 atomic_write + load_index_or_scan 两处;无 schema/迁移。
