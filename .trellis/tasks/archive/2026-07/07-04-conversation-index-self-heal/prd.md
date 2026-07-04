# 对话索引自愈防丢失

## Goal

根治"新建对话/生成时侧栏历史对话全部消失"的数据完整性 bug。让 `index.json` 成为**可自愈的缓存**:任何时候它缺失/写坏/写少,读取时都以磁盘上的对话文件(真相源)对账重建,绝不再用残缺缓存覆盖掉真实数据的引用。

## Background / 已核实的根因

对话存储:`{app_data}/conversations/` 下**每对话一个 `conv_<uuid>.json`(真相源)+ 一个 `index.json`(侧栏列表缓存)**(storage.rs)。

丢失链(与用户复现"新建对话生成时旧对话全没"完全吻合):
1. `load_index`(storage.rs:194):index.json **缺失时返回空 `Ok(default)`**(非错误)。
2. `load_index_or_scan`(136):**只在 `load_index` 报【错】时**才 `load_conversation_list_from_files` 重扫;"缺失→空"这条不触发重扫。
3. `atomic_write`(54):rename 覆盖失败分支里**先 `remove_file(path)` 再 `rename`**(71-74)——中途 index.json 不存在;高频流式保存 / 并发实例 / force-kill / 杀软占用时,那次 rename 一旦失败,index.json 就此消失。
4. 下一次 `save_conversation`(388)→ `load_index_or_scan` 得到空 → 只 insert 当前新对话 → 写出**只 1 条**的 index.json,其余 24 个对话文件被孤立。
5. 侧栏读 index.json 只剩 1 条。**对话文件全在**(已据此秒恢复 25 条)。

关键事实:
- `load_index_or_scan` 被读/写多处使用(get_conversations 586、save_conversation 395、delete 438 等)。
- 对话文件 id 可从**文件名**廉价获取(`validate_conversation_id`:`conv_` 前缀 + 字母数字/_/-),无需读文件内容。
- Rust `std::fs::rename` 在 Windows 上原子替换已存在目标(MoveFileExW REPLACE_EXISTING),`remove_file` 那步非必需且有害。

## Requirements

- R1 **索引对账自愈**(主修复):`load_index_or_scan` 改为——先廉价列出磁盘上所有对话文件 id;若 `load_index` 报错,**或**索引未覆盖全部文件 id(存在"有文件但不在索引"的对话),则以文件为准重扫重建列表(`load_conversation_list_from_files`),并**尽力写回**修复后的 index.json(best-effort,失败不影响返回)。索引覆盖全部文件 id 时照常信任索引(允许其含额外条目,无害)。
- R2 **消除 atomic_write 的"文件缺失窗口"**(次修复):去掉"先 `remove_file` 再 rename",改为直接 `rename(tmp, path)`(Windows 会原子替换);瞬时失败交由既有外层重试循环(sleep 后重试整次写),**任何时刻都不让目标文件中途消失**,失败时保留旧文件。
- R3 不改对话文件格式、不改前端契约、不改 index.json schema。纯后端存储层加固。
- R4 单测覆盖:索引缺失→重扫、索引残缺(少于文件)→重扫、索引完整→信任;atomic_write 覆盖已存在文件成功且不产生缺失窗口。

## Acceptance Criteria

- [ ] AC1 index.json 被删/写空/写少(少于实际对话文件)后,调用 get_conversations 返回**全部**对话(从文件重扫),并把 index.json 修回完整。
- [ ] AC2 新建对话时:即便 index 一度缺失,save_conversation 内部的 load 会先重扫出全部,再插入新对话保存 → 历史对话**不丢**。
- [ ] AC3 index 完整(覆盖所有文件)时不触发重扫(不读全部文件,保持轻量)。
- [ ] AC4 atomic_write 覆盖已存在文件成功;写失败时旧文件仍在(无缺失窗口)。
- [ ] AC5 既有 storage/删除/重命名等测试不回归;新增自愈/atomic_write 用例通过(Windows 用 `scripts/win-cargo-test.ps1`)。

## Out of Scope

- 不改前端 / index schema / 对话文件格式。
- 不做跨实例文件锁(并发只是诱因;自愈已让并发下最坏结果=下次读自动补全)。R1+R2 足够根除数据丢失表现。
- 不处理"index 含多余幽灵条目"(点开会失败但非数据丢失;可后续清理)。

## Open Questions

（无阻塞;根因与修复已明确。）
