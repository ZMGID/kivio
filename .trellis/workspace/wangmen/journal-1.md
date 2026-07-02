# Journal - wangmen (Part 1)

> AI development session journal
> Started: 2026-07-02

---



## Session 1: 上下文压缩三路统一 + R-3/R-4 优化

**Date**: 2026-07-02
**Task**: 上下文压缩三路统一 + R-3/R-4 优化
**Branch**: `main`

### Summary

调研 Codex/Claude Code 压缩后对照优化 Kivio：统一三路压缩到 compaction.rs 核心，L2 落盘 summary；R-3 摘要输出上限 4096->8192 且 L2 用模型真实上限（实测修 9 段摘要截断）；R-4 加 decay_warning_for（3 次压缩后告警）；质量兜底加截断拒绝。R-1 microcompact 拆子任务暂缓。cargo test 受测试二进制 DLL 版本不匹配阻塞，用 cargo check + 纯函数单测 + ZEN 真实流式测试覆盖。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `97dfb8a` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: Microcompact 增量降级 (R-1) + 缓存保留调研否决 (R-2)

**Date**: 2026-07-02
**Task**: Microcompact 增量降级 (R-1) + 缓存保留调研否决 (R-2)
**Branch**: `main`

### Summary

调研确认 R-2(前缀缓存保留)是伪命题（Kivio 全仓无 cache_control，Anthropic 缓存未启用，OpenAI 自动缓存在压缩点天然 miss），砍掉。实现 R-1 Microcompact：maybe_compact_send_view 在 LLM 摘要前先把 old_segment 工具结果降级成标记，够了就跳过摘要；纯函数 microcompact_send_view + 4 单测。cargo test 二进制受 0xC0000139 阻塞，用独立项目原样执行纯函数 4/4 通过。用户将手动端到端测试（WebView2 CDP attach 尝试过、可行但环节多，改手动）。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `ffff7c3` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: 修复上下文压缩卡死、boundary 错位与压缩动画位置

**Date**: 2026-07-02
**Task**: 修复上下文压缩卡死、boundary 错位与压缩动画位置
**Branch**: `main`

### Summary

修复 97dfb8a 压缩统一后的回归：1) chat-compaction 事件配对（新增 failed phase，compact_conversation 单出口保证 started 必有终止事件，前端压缩中状态不再卡死）；2) runtime→UI boundary 映射改为 _ui_message_id 标注（弃用条数推算，修复静默丢上下文）；3) 手动压缩小对话保底切分（>4 条）；4) 保留链式压缩衰减警告；5) 用户反馈后二轮改造：divider 语义改为时间线锚点（display_after_message_id，压缩发生在哪就显示在哪），动画槽位=当前最后一条消息，锚点被删时回退切分点。spec 落盘 .trellis/spec/chat/compaction-contracts.md。cargo test 仍受 0xC0000139 环境问题，Rust 逻辑用独立 harness 验证（7 项全过）。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `080817d` | (see git log) |
| `2198319` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: 用户消息编辑并重新生成 + 压缩阈值标签修正 + 降级调用可取消

**Date**: 2026-07-02
**Task**: 用户消息编辑并重新生成 + 压缩阈值标签修正 + 降级调用可取消
**Branch**: `main`

### Summary

1) 修复三处降级/恢复模型调用不接取消信号导致停止按钮无效（planning 流式断包降级、synthesis 超窗恢复、中立精简恢复，包 tokio::select）；2) 前端自动压缩阈值标签 85%→90% 与后端同步；3) 新功能：用户消息编辑并重新生成——chat_regenerate_message 加 new_content 参数（原子编辑+截断+重生成，附件保留，摘要 stale 用 idx），用户气泡加编辑按钮与原地编辑态，流式中入口禁用。trellis-check 修了 6 个前端问题（streaming 门控缺口、frozen 窗口、静默吞编辑等）。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `0191277` | (see git log) |
| `6af88c0` | (see git log) |
| `d86a763` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: 对齐 opencode 请求格式：稳定提示词/工具健壮性/会话亲和

**Date**: 2026-07-03
**Task**: 对齐 opencode 请求格式：稳定提示词/工具健壮性/会话亲和
**Branch**: `main`

### Summary

基于 opencode 真实流量对比改造 Kivio 的 OpenAI-compatible 请求（抓包代理逐字段验证）。根因发现：web_search 工具名撞 Cursor 上游保留名被吞→空响应（20+ 次二分定位）。五项改造：1) 系统提示词日期级时钟，前缀字节稳定（缓存命中+会话亲和前提）；2) 工具名大小写自愈（Grep→grep）+ 未知工具清单喂回；3) planning 空响应重试一次；4) web_search→search_web wire 别名（仅 wire/prompt，内部逻辑仍见 web_search，match_tool_call 映射回执行）；5) 会话亲和三件套（x-session-id/affinity 头 + prompt_cache_key/promptCacheKey）+ tool_choice auto + stream_options.include_usage。真实会话验证 search_web 闭环+多轮工具循环正常。spec 落 .trellis/spec/chat/request-shape-contracts.md。cargo test 受 0xC0000139 环境问题，Rust 逻辑用 harness 验证 12/12。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `a79cb28` | (see git log) |
| `648df36` | (see git log) |
| `ca7be76` | (see git log) |
| `4051e14` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete
