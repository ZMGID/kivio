---
name: wecom-cli-setup
description: Install the official wecom-cli and its published skills, then verify scan-code login with one read-only identity call.
kivio-market-managed: true
---

# 设置企业微信 CLI

只安装官方 CLI 和官方技能，并做只读验证。市场只管理 `wecom-cli` 和 `wecom-cli-setup` 两个 Skill。业务技能用企业微信自己发布的安装命令，不把仓库里的 `wecomcli-*` 复制进 `~/.kivio/skills`，也不改写那些技能。已有 CLI、技能和授权直接复用。如果这两个市场文件损坏，使用市场的“重新配置”修复。

## 0. 核对 Kivio 接入

先确认当前 Kivio 对话能加载 `wecom-cli` 和 `wecom-cli-setup`。本插件通过这两个 Skill、官方 `wecom-cli`，以及官方安装到本机的 `wecomcli-*` 技能工作，没有命令或 MCP 组件，不要为它创建空的 MCP 配置。若市场显示“需修复”或这两个 Skill 缺失，使用市场的安装/修复入口恢复，不手工改官方技能。

## 1. 安装官方 CLI 和官方技能

需要 Node.js 18 或更新版本。运行 `command -v wecom-cli` 和 `wecom-cli --version`。缺少 CLI 时，先展示 `npm install -g @wecom/cli` 并取得同意，然后执行并核对版本。

官方技能用企业微信给出的命令安装，不要自己写技能内容：

```bash
npx skills add WeComTeam/wecom-cli -y -g
```

装完后确认官方技能已出现，例如 `wecomcli-shared` 能被读到。命令失败就停止，并把错误告诉用户。不要把这些技能再复制一份到 `~/.kivio/skills`。

## 2. 复用或完成授权

运行 `wecom-cli auth show --status`。不要读取或输出 Bot Secret、Token、Cookie 或配置文件内容。

- 输出 `authorized`：进入第 3 步，不重新登录。
- 输出 `unauthorized`：在能持续读取输出的终端运行 `wecom-cli auth init --noninteractive`。先把命令展示的授权链接和二维码原样给用户，再等待用户用企业微信扫码。若当前工具看不到输出，让用户在自己的终端运行。完成后重新运行 `wecom-cli auth show --status`，只有输出 `authorized` 才继续。
- 其他输出或命令失败：停止，并把错误告诉用户。不要让用户把密钥发到聊天。`--manual` 只在用户自己选择时使用。

## 3. 只读身份确认

授权通过后运行 `wecom-cli identity whoami`。以退出码 0 且能读到当前身份为成功。setup 不发送消息，不改文档、日程、会议、待办、邮件或微盘。

分别报告 Kivio 的两个 Skill、CLI 版本、官方技能是否已安装、授权状态和身份确认结果。市场“已安装”只表示组件就位；未完成 `identity whoami` 时只报告“CLI 已就绪，身份未验证”。

完整完成后，只把已安装的 `~/.kivio/skills/wecom-cli-setup/SKILL.md` 的 YAML `description` 行尾追加一次 ` [setup completed once]`；原有描述和其他内容不变。验证未完成就不要标记。这个标记只表示曾跑通；用户要求重跑或调用失败时仍可重跑，不重复追加标记。
