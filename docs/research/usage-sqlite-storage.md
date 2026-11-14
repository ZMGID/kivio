# 用量统计存储调查与迁移

2026-11-14。范围：Kivio 自身已记录的模型调用，不新增外部 CLI 用量采集。

## 源码依据

- [OpenCode v1.2.0 数据库入口](https://github.com/anomalyco/opencode/blob/v1.2.0/packages/opencode/src/storage/db.ts)：SQLite、WAL、busy timeout、版本化迁移；[会话表](https://github.com/anomalyco/opencode/blob/v1.2.0/packages/opencode/src/session/session.sql.ts)使用主键和会话索引，消息详情仍可用 JSON 字段。这说明重点是查询与事务，不是彻底消灭 JSON。
- [Pi SessionManager](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/src/core/session-manager.ts)：会话条目（包括 usage）写入 JSONL，失败时撤回内存里的追加。这里核验的是调查当日 main，非固定发布版。

## Kivio 的实现

原路径：按月 JSONL → 全量解析/排序 → Rust 汇总 → 内存分页，两份内存缓存分别服务统计页与会话费用。

新路径：usage 模块 → SQLite `usage/usage.sqlite3` → SQL 筛选、聚合和分页。唯一持久化负责人仍为 `UsageLedger`；数据库实现封装在 `usage/store.rs`，今日用量共用它。字段直接落关系表，不将整条记录封装为 JSON。时间与会话索引分别服务列表和会话费用；统计各部分在同一个读事务内取得一致快照。写入使用 WAL 和 FULL 同步，清空使用事务。未新增依赖，未改变 IPC 类型或费用估算公式。

首次打开库时，在同一事务内创建结构、逐行导入原目录 JSONL、按记录 ID 去重并设置版本。I/O 或 SQL 错误回滚，后续可重试；无效 JSON 行计入 skipped_records，原文件保留。迁移完成后只读写数据库，不再扫描旧文件。清空删除数据库记录和跳过计数，保留迁移版本，因此重启不会恢复旧记录。

旧 JSONL 留在原处作为迁移备份，清空统计不删除这份备份。不要在升级后继续用旧版本写入同一数据目录；迁移后的旧文件新增内容不会再次导入。

## 验证范围

真实临时数据库测试覆盖旧数据导入、重复 ID、损坏行、读取失败后重试、重启、清空后不重新导入、写入与清空失败回滚、并发追加、分页及原统计口径对照。桌面页面与真实用户数据迁移需另作实机验证；本次不触碰用户已有用量目录。
