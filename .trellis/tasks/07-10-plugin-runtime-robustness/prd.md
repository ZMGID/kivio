# PRD: 插件即插即用 — 补齐 Kivio 运行时健壮性并去除 OfficeCLI 专项适配

## 背景

OfficeCLI 插件接入后暴露出一批问题(实测于 conv_c69151cf / conv_4f0dfeb4 / conv_87d4e9a6 三轮 PPT 生成):

1. officecli 命令走 bash → Windows 每条冷启 PowerShell,慢且不触发预览。
2. MCP 工具结果里的截图被拍扁成 `[image: image/png]` 占位符,模型看不到图 → Gate 3 视觉审查假 PASS(slide2 字面 `\n` 缺陷漏检)。
3. 辅助视觉模型只做"描述",不报告缺陷。
4. 中间产物(batch JSON、审查 PNG)写进交付目录 → 聊天界面刷屏;跑完不清理。
5. skill 的 bash 例子(heredoc/`$VAR`/管道/seq)在 Windows PowerShell 里全是语法错误,模型被迫绕路。

前三轮用 `plugins/catalog.rs` 的 system_hint 打了厚补丁(强制 MCP、绝对路径、截图 `-o` 落盘+read 绕行),**方向错了**:把 Kivio runtime 的通用缺口当成 officecli 专项适配来补。Claude Code / opencode / pi 不需要任何适配就能用 officecli,因为它们的环境天然满足:bash 是母语、工具结果原生带图、Read 天生读图。

## 目标

**补 Kivio 通用健壮性,让任何插件/MCP server 即插即用;officecli 专项 hint 瘦身到不可再删的几行。**

## 需求(五项)

### R1 — MCP 工具结果图片直达模型
MCP 工具返回的 image 内容块,当主模型支持 vision 时作为图片直接喂给模型(复用 `read` 已走通的 `follow_up_user_messages` 管子,`commands.rs::read_image_as_tool_result` 是参照实现);不支持 vision 时走辅助视觉模型(与 read 读图一致)。带护栏:单图大小上限、单结果图片数上限,超限降级为占位符+落盘路径提示。

### R2 — 辅助视觉模型改"审查向"
`analyze_chat_images_with_auxiliary_model` 的提示词从"客观描述"改为"审查+描述":必须显式报告文字截断/溢出/重叠/字面转义符(如 `\n`)/元素错位/对比度问题,无缺陷才写"未见异常"。惠及所有"纯文本主模型读图"场景。

### R3 — 通用行为规范进系统提示通用段
在 `chat/agent/prepare.rs` 的通用段(所有对话生效,非插件 hint)补三条:
- 多步任务的中间产物写系统临时目录,不写交付目录;
- 任务结束前清理中间产物,交付目录只留最终交付物;
- 调 stdio MCP 工具传文件路径一律绝对路径(server cwd 不可预测)。

### R4 — Windows `run_command` 优先 Git Bash(bash-first, PowerShell 回落)
调研结论见 `research/shell-execution-survey.md`(pi 源码分析 + opencode/codex issue 挖掘)。要点:pi/Claude Code 在 Windows 是 bash-only(Git Bash);codex 硬编 PowerShell 被用户诟病;opencode 自动检测但对模型不透明,产生大量静默命令腐蚀 issue。

Kivio 采用 **pi 的解析顺序 + PowerShell 回落 + 动态工具描述**:
- 解析顺序:Git Bash 已知安装位(`%ProgramFiles%\Git\bin\bash.exe`、x86、LocalAppData)→ `where bash.exe` + existsSync 复核(`where` 会返回幽灵路径)→ 都无则回落现有 PowerShell 路径(与 pi 不同,不硬报错,保护无 Git 用户,零回归)。
- **排除 WSL bash**(`System32|sysnative\bash.exe`):其文件系统视图(/mnt/c)与 Kivio 传的 Windows 路径语义不符。
- **工具 description 按选中 shell 动态生成**(opencode #16479 教训:shell 选择对模型不可见 → 语法腐蚀):bash 时写明"bash 语法、Windows 路径用正斜杠 C:/…"(防 opencode #15810 的 `\U` 转义路径腐蚀);PowerShell 时保留现有 PS 描述。模型永远知道面对哪个 shell。
- 不做命令内容静默改写(opencode 踩坑)。检测结果进程级缓存(OnceLock)。
- 可选 settings 字段 `shell_path` 显式指定(pi 同款),MVP 后置。

### R5 — OfficeCLI hint 瘦身
`plugins/catalog.rs` system_hint 只留真正 officecli 专属的:
- 优先 MCP `officecli` 工具而非 bash(常驻进程性能 + Kivio 预览联动);
- 禁 `officecli watch`/`unwatch`(MCP 编辑不驱动 watch,Kivio 有自己的预览);
- 禁 `officecli mcp claude|cursor|vscode|…`(Kivio 已注册 plugin-officecli);
- 保留 skill 激活指引(load_skill 映射表)。
删除:batch 教学、截图落盘+read 绕行指引、绝对路径规则(→R3)、清理规则(→R3)、效率细则。

## 非目标 / 红线

- **不改 officecli 二进制、官方 skill、MCP server**(它们是上游,Kivio 只做宿主)。
- 不动 `chat/model/` 适配器对 provider 协议 wire 格式约定之外的行为(R1 走现有 follow_up 机制,不新增协议分支)。
- 不删 `MessageBubble.tsx` 的"最后一轮截图画廊"逻辑(保留作通用兜底)。
- 不动 `plugins/preview.rs` 的 live preview 机制。

## 验收标准

- [x] **AC1(图直喂)**:vision 主模型对话中,MCP 工具(officecli `view screenshot`)返回的截图作为图片内容块出现在下一条模型请求中(request_debug 或 conv JSON 的 model_messages 可证);非 vision 主模型得到审查向文字分析。
- [x] **AC2(审查向)**:构造带字面 `\n`/文字溢出的幻灯片截图,辅助视觉分析文本中显式指出该缺陷。
- [x] **AC3(通用规范)**:新对话做 PPT,中间 batch JSON 不出现在交付目录/聊天卡片;结束后中间产物被清理,交付目录只剩 .pptx。
- [x] **AC4(bash)**:Windows 上 `run_command` 执行 `cat <<EOF`、`for i in $(seq 1 3)`、管道命令成功;无 Git Bash 的机器回落 PowerShell 不报错。
- [x] **AC5(瘦身)**:catalog.rs hint 显著缩短(目标 ≤15 行);重跑 5 页 PPT 测试,officecli 全走 MCP、模型真实看图审查(能发现植入缺陷)、无中间文件刷屏。
- [x] 无回归:`npm run lint`、`npm run typecheck`、`npm test`、Rust 测试(经 `scripts/win-cargo-test.ps1`,对照已知基线失败集)。

## 约束

- Windows 优先验证(用户主力平台);macOS 路径不破坏。
- R1 图片进上下文注意 token 成本:默认仅当轮工具结果的图直喂,不回填历史轮。
- 验证方式:chat_probe 通道 + 扒 conv JSON tool_calls/model_messages(已有成熟脚本方法)。
