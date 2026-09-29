---
name: feishu-cli-setup
description: Check Kivio integration and Lark CLI, reuse existing profiles or guide first-time app and OAuth setup, then verify identity, scopes, and a minimal read-only call.
kivio-market-managed: true
---

# 设置飞书 CLI

只准备本机 CLI、应用配置与身份，并做只读验证。市场安装 `feishu-cli` 和 `feishu-cli-setup` 两个 Skill；setup 负责查漏补缺。已有环境直接复用，不因重复运行而新建应用、重新授权或覆盖全局 Skills。如果市场管理的两个 Skill 文件损坏，使用市场的“重新配置”修复，让安装状态保持准确。

## 0. 核对 Kivio 接入

先确认当前 Kivio 对话能加载 `feishu-cli` 和 `feishu-cli-setup`；不能只凭磁盘上有文件就认定接入成功。本插件通过这两个 Skill 和外部 `lark-cli` 工作，没有命令或 MCP 组件，不要为它创建空的 MCP 配置。若市场显示“需修复”或 Skill 缺失，使用市场的安装/修复入口恢复市场管理的文件，不手工复制到 `~/.agents/skills`。若终端能找到 CLI、Kivio 对话却不能调用，检查 Kivio 实际运行环境中的 PATH，再做下面的 CLI 验证。

## 1. 检查或安装 CLI

运行 `command -v lark-cli` 和 `lark-cli --version`。缺少 CLI 或用户明确要求更新时，先用 `npm view @larksuite/cli@latest version` 查当前版本。若本次尚未授权修改全局 CLI，展示 `npm install -g @larksuite/cli@latest` 并取得同意，然后执行并核实实际版本。需要隔离时可用 npm 的 `--prefix` 安装到专用目录，随后直接调用其中的二进制，不改现有 PATH。安装后用 `lark-cli skills list` 检查内置领域说明可读取；任一步失败就停止业务操作。

不要默认使用 `npx @larksuite/cli@latest install` 完整向导或 `lark-cli update`：它们可能同步全局 Skills。只安装 CLI；不要写入 `~/.agents/skills`，不要覆盖用户已有的 Skills。

## 2. 复用当前 profile 和身份

用户指定 profile 时，把 `--profile <name>` 带到后续每个 CLI 命令；否则沿用当前 profile，不主动切换。运行 `lark-cli auth status --json --verify`，只读取身份、验证状态、用户名、open ID 和 token 状态等必要字段，不读取或输出 App Secret、Token、Cookie 或配置文件内容。

- 未配置应用：进入第 3 步。用户身份缺失或失效：进入第 4 步。
- 已有用户身份且服务端验证成功：进入第 5 步，不重新登录。`needs_refresh` 先让正常只读请求使用 CLI 自带的刷新机制，不能直接推断必须重新授权。
- 只有 bot 可用不代表用户已登录。用户明确要求 bot 操作时保留 `--as bot`；bot 的权限问题不通过用户 OAuth 修复。
- 网络或 scope 错误按具体错误处理，不靠重建应用、扩大权限或换账号试探。

## 3. 首次应用配置

只在确实没有可用配置、且用户同意初始化时运行 `lark-cli config init --new`。该命令会等待浏览器操作：在能持续读取输出的终端启动，先把新鲜的授权 URL 原样展示给用户，再等待完成。可以用 `lark-cli auth qrcode <url> --output <cwd 内相对路径.png>` 生成二维码。若工具不能及时展示输出，让用户在自己的终端运行，不在不可见的阻塞调用里等待。不要猜填 App ID/Secret；完成后回到第 2 步。

## 4. 缺少用户授权时分步 OAuth

先从目标命令的 `--help` 或 `lark-cli schema` 确定最小 scope。无具体任务时不默认请求所有权限；`--recommend` 在当前 CLI 中等同所有已知业务域，只能在用户明确选择这个范围后使用。发起非阻塞流程：`lark-cli auth login --scope <required-scopes> --no-wait --json`，或使用用户选定的 `--domain`。

把 verification URL 原样展示，并可用 `lark-cli auth qrcode` 生成二维码。让用户完成浏览器授权，本轮到此结束；不要紧接着阻塞轮询。用户确认后，用**同一次**流程的 device code 运行 `lark-cli auth login --device-code <device-code>`，再回到第 2 步验证。过期就重新发起同一范围的流程。不要让用户把 Token、Secret 或验证码发到聊天。

## 5. 验证范围和最小只读请求

有具体任务和实际所需 scope 时，运行 `lark-cli auth check --scope <required-scopes> --json`，检查输出中的缺失范围；不能只看退出码。先看目标命令的 `--help`，再做一个与任务相关的最小只读请求。默认用户资源显式传 `--as user`，例如日历任务可用 `lark-cli calendar +agenda --as user --format json`。业务调用以退出码 0 且 JSON 的 `ok == true` 为成功；`auth status` 的诊断 JSON 按第 2 步字段判断。`--help`、`--dry-run` 和缓存状态都不能代替真实请求。

分别报告 Kivio 组件接入、实际 CLI 版本、profile 来源、有效身份、scope 检查和只读结果。市场“已安装”只表示组件就位；未完成真实请求时只报告“基础认证通过，业务未验证”。setup 不发送消息、不创建文档、不修改飞书数据。

完整完成身份、所需权限和真实只读请求验证后，只把已安装的 `~/.kivio/skills/feishu-cli-setup/SKILL.md` 的 YAML `description` 行尾追加一次 ` [setup completed once]`；原有描述和其他内容不变。只要验证未完成，就不要标记。这个标记只表示曾跑通，不能代替当前 profile、scope 或业务请求的检查；用户要求重跑、身份变化或调用失败时仍可重跑，不重复追加标记。
