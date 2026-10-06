# 公开插件扩充核验（2026-10-06）

本次新增 30 个 Kivio 原生插件包，保留飞书与企业微信，共 32 个公开条目。每个新包有独立 setup 与按任务拆分的业务 Skills，无固定主 Skill；当前所选能力均通过 CLI/文件工作流完成，未注册 MCP、Hook 或伪造服务地址。依赖安装、OAuth 与具体业务验证由 setup 按实际环境处理。

## 筛选与适配

参考 [ZCode 市场](https://github.com/zai-org/zcode-plugins) 中 GitHub、GitLab、阿里云、钉钉、腾讯会议与 Obsidian 的用途划分和配置验证方式；未复制其 Skill 正文、付费服务配置或宿主专用指令。其他条目以项目维护者仓库及文档为依据，Kivio 独立编写适配说明。

纳入条件：有明确维护者来源、公开安装与用法、可通过现有 Kivio 终端/文件工具调用、能区分只读验证和业务写操作。列表条目不是第三方 CLI 已安装或云账号已授权的证明。没有对 30 个第三方工具逐一安装并完成真实账号操作。

暂不纳入：

- ZCode Video2code、插件创建器与安全 Hook：依赖其 Browser Use、插件 API 或事件协议，不能原样承诺 Kivio 支持。
- 同花顺、万得、天眼查及依赖它们的金融包：有宿主付费服务或未验证凭证/数据源，不能凭公开清单提供同等能力。
- CloudBase：发现旧 GitHub CLI 仓库已归档，当前来源和云端配置需单独核对，未用旧包凑数。
- 不把一个多功能工具拆成多个市场条目；同一工具的业务技能随同一插件安装。

## 条目核验

“验证”列是用户安装后 setup 的验收路径，不表示本次已连接真实账户。仓库 README 均已在线读取；运行时以安装版本帮助及目标账户权限为准。

| 插件 | 组件 | 依赖与验证边界 | 来源 |
| --- | --- | --- | --- |
| GitHub | 配置；审阅 Pull Request；Issue 与 Actions | GitHub Enterprise 必须沿用目标 host；不要将仓库名相同视为同一仓库。 | [仓库](https://github.com/cli/cli) · [文档](https://cli.github.com/manual/) |
| GitLab | 配置；Merge Request 评审；流水线诊断 | 不同 GitLab 实例的权限不互通；不通过改 host 或账号规避 403。 | [仓库](https://github.com/gitlabhq/cli) · [文档](https://docs.gitlab.com/cli/) |
| 阿里云 CLI | 配置；资源查询；云资源变更 | 帮助和业务调用均保留 `--auto-plugin-install false`，避免检查隐式安装产品插件。 | [仓库](https://github.com/aliyun/aliyun-cli) · [文档](https://www.alibabacloud.com/help/en/cli/) |
| Docker | 配置；容器诊断；镜像与 Compose | Docker Desktop 的许可条件与 Engine 不同；遵循用户已有部署方式，不自动替换 Docker context。 | [仓库](https://github.com/docker/cli) · [文档](https://docs.docker.com/reference/cli/docker/) |
| Kubernetes | 配置；工作负载诊断；发布与回滚 | 后续命令显式保留 context 和 namespace；只读权限失败不能通过切换生产集群规避。 | [仓库](https://github.com/kubernetes/kubectl) · [文档](https://kubernetes.io/docs/reference/kubectl/) |
| Helm | 配置；Chart 检查；Release 管理 | values 与渲染结果可能含 Secret；只输出必要字段，不默认跨所有命名空间枚举。 | [仓库](https://github.com/helm/helm) · [文档](https://helm.sh/docs/) |
| Terraform | 配置；配置审阅；计划与变更 | plan 可能访问云 API、持有状态锁并包含敏感值；不读取或展示完整 state、plan JSON。 | [仓库](https://github.com/hashicorp/terraform) · [文档](https://developer.hashicorp.com/terraform/cli) |
| AWS CLI | 配置；AWS 资源查询；日志与操作 | 每次任务保留 profile/region；身份检查不代表所有服务权限，日志和对象访问按最小范围。 | [仓库](https://github.com/aws/aws-cli) · [文档](https://docs.aws.amazon.com/cli/latest/userguide/) |
| Azure CLI | 配置；Azure 资源查询；部署排查 | 不主动 az account set 更改默认订阅；后续显式传 --subscription，避免租户与订阅混用。 | [仓库](https://github.com/Azure/azure-cli) · [文档](https://learn.microsoft.com/cli/azure/) |
| DigitalOcean | 配置；Droplet 查询；应用与集群 | 保留 context；Droplet 创建、扩容、备份及带宽可能计费，不能用于 setup 验证。 | [仓库](https://github.com/digitalocean/doctl) · [文档](https://docs.digitalocean.com/reference/doctl/) |
| Vercel | 配置；部署诊断；预览部署 | link/pull 会改本地文件，环境变量可能包含密钥；不将环境文件提交仓库。 | [仓库](https://github.com/vercel/vercel) · [文档](https://vercel.com/docs/cli) |
| Netlify | 配置；站点与构建；草稿部署 | 部署前区分站点 ID、团队及草稿/生产环境；netlify env 输出可能含密钥。 | [仓库](https://github.com/netlify/cli) · [文档](https://cli.netlify.com/) |
| Cloudflare Wrangler | 配置；Workers 开发与发布；KV 与 R2 | dev 可能连接远端资源，明确 local/remote；Wrangler secret 与绑定值不回显。 | [仓库](https://github.com/cloudflare/workers-sdk) · [文档](https://developers.cloudflare.com/workers/wrangler/) |
| Supabase | 配置；数据库迁移；Edge Functions | supabase status 可能输出本地 API Key；只读取必要字段。db reset/db push 不属于只读验证。 | [仓库](https://github.com/supabase/cli) · [文档](https://supabase.com/docs/reference/cli/) |
| Firebase | 配置；本地模拟器；Hosting 发布 | 模拟器与生产端点必须区分；部署明确 --project 与 --only，避免发布不相关资源。 | [仓库](https://github.com/firebase/firebase-tools) · [文档](https://firebase.google.com/docs/cli) |
| Sentry CLI | 配置；Release 管理；Source Map | 本插件面向发布与符号映射，不声称 sentry-cli 可以读取所有 Issue 或事件 API。 | [仓库](https://github.com/getsentry/sentry-cli) · [文档](https://docs.sentry.io/cli/) |
| Stripe CLI | 配置；Webhook 调试；API 查询 | 本插件用于集成调试；不以验证为由创建真实付款，不默认 --live。 | [仓库](https://github.com/stripe/stripe-cli) · [文档](https://docs.stripe.com/stripe-cli) |
| 钉钉 Workspace | 配置；消息与日程；文档与表格 | 不能仅凭 success:true 判定已登录；多组织明确 corpId:userId，不自动切换。 | [仓库](https://github.com/DingTalk-Real-AI/dingtalk-workspace-cli) · [文档](https://github.com/DingTalk-Real-AI/dingtalk-workspace-cli) |
| 腾讯会议 | 配置；会议管理；录制与参会信息 | 不通过创建测试会议验证认证；录制访问和下载需要对应会议权限。 | [仓库](https://github.com/TencentCloud/tencentmeeting-cli) · [文档](https://github.com/TencentCloud/tencentmeeting-cli) |
| Google Workspace CLI | 配置；Drive 文件；邮件与日历 | 命令由 Discovery API 动态生成，必须用 gws schema/--help 获取当前参数；组织仓库不等于官方支持承诺。 | [仓库](https://github.com/googleworkspace/cli) · [文档](https://github.com/googleworkspace/cli) |
| Obsidian | 配置；笔记与属性；Canvas 与 Bases | 保留双链、frontmatter、附件和 Canvas 格式；不要假设默认活跃笔记就是用户目标。 | [仓库](https://github.com/kepano/obsidian-skills) · [文档](https://help.obsidian.md/cli) |
| Pandoc | 配置；文档转换；引用与 PDF | 复杂格式转换可能丢失布局；转换成功不等于版面一致，必须打开产物核对。 | [仓库](https://github.com/jgm/pandoc) · [文档](https://pandoc.org/MANUAL.html) |
| Quarto | 配置；报告渲染；网站与幻灯片 | render 可执行代码块及 pre/post-render 脚本；preview 会启动常驻服务。 | [仓库](https://github.com/quarto-dev/quarto-cli) · [文档](https://quarto.org/docs/guide/) |
| FFmpeg | 配置；媒体分析；转码与剪辑 | 禁止默认 -y 覆盖原件；剪切精度、字幕和色彩信息需单独验证。 | [仓库](https://github.com/FFmpeg/FFmpeg) · [文档](https://ffmpeg.org/documentation.html) |
| ImageMagick | 配置；图片转换；拼图与批处理 | 不修改系统 policy.xml 绕过限制；保存到新文件，mogrify 原地覆盖需要明确授权。 | [仓库](https://github.com/ImageMagick/ImageMagick) · [文档](https://imagemagick.org/command-line-processing/) |
| yt-dlp | 配置；格式与字幕检查；下载与校验 | 不自动读取浏览器 Cookies、不绕过 DRM；只有用户授权时才使用其登录态。 | [仓库](https://github.com/yt-dlp/yt-dlp) · [文档](https://github.com/yt-dlp/yt-dlp#usage-and-options) |
| Rclone | 配置；云盘查询与核对；复制与同步 | sync、move、purge 与 copy 含义不同；默认先预览，不把同步当作普通复制。 | [仓库](https://github.com/rclone/rclone) · [文档](https://rclone.org/docs/) |
| SQLite | 配置；结构检查；查询与导出 | 连接不存在的路径可能创建新库；不要复制正在写入的主文件却忽略 WAL。 | [仓库](https://github.com/sqlite/sqlite) · [文档](https://sqlite.org/cli.html) |
| DuckDB | 配置；文件分析；SQL 与导出 | DuckDB 扩展可能自动安装/加载；先确认外部访问需求，本地分析不自动启用远程源。 | [仓库](https://github.com/duckdb/duckdb) · [文档](https://duckdb.org/docs/current/clients/cli/overview.html) |
| HTTPie | 配置；请求构造；接口诊断 | 不使用 --verify=no 绕过 TLS；认证信息不放在聊天或日志，session 文件可能保存敏感信息。 | [仓库](https://github.com/httpie/cli) · [文档](https://httpie.io/docs/cli) |

## 工程接入

原路径：公开目录只安装全局 setup/entry Skills，并把会话绑定到 mainSkillId。新增路径：公开目录 → 通用包导入/启停 → 按任务发现包内 Skills；包内容与启用状态仍唯一归 `plugins/packages.rs`，市场只保存已安装包 ID 与目录 revision。原有两个条目保持兼容。

详情从实际插件包读取组件与说明，沿用 PluginContents 展示；新包 mainSkillId 为 null，“使用”创建不绑定固定技能的对话。公开目录拥有的包不重复显示在个人插件与已安装图标中。

资源是随应用发布的独立 Kivio 说明，版本 1.0.0，不是上游二进制版本，也没有自动下载安装 CLI。各包使用 GPL-3.0-or-later，与仓库一致；上游软件各自的许可证仍适用。图标使用项目官网、项目组织、ZCode 市场与 Devicon 原色资源，按原始字节随应用发布；Pandoc 为其维护者提供的社区 Logo，并非官方商标。来源、文件校验和及许可见 [Logo 来源](../licenses/plugin-logo-sources.md)。页面直接显示图像，不再通过遮罩统一染色。

## 验证记录

- `npx vitest run src/chat/market`：3 个文件、30 项测试通过，含无主 Skill 的组件展示、使用回调与已安装包去重。
- `cargo test --manifest-path src-tauri/Cargo.toml --lib market::tests --no-fail-fast`：7 项测试通过，覆盖 30 个真实资源包导入、配置技能缺损检测与已有两个条目的兼容。
- `npm run protocol:check` 通过。
- 定向 ESLint、`npm run architecture:check`（12 项检查）、`npm run package:check`、`git diff --check` 通过。
- `npx tsc --noEmit`：受已有 `src/chat/markdownUtils.ts` 中 `MarkdownNode.value` 类型错误阻塞，本次插件文件没有新增类型诊断；未改无关实现。
- 上游 README 在线读取 30/30；文档链接修正 Quarto 失效地址，并将 ImageMagick、DuckDB 更新为实际文档地址。
- `npm run tauri -- build --debug --bundles app` 通过；已重启构建产物，实机看到 32 个条目和 6 个分类，包内确有 30 份原生 manifest，图标与源码资源一致。
- 实机 HTTPie：详情显示 setup 与两个业务 Skill；安装后“已加载”，停用后“未加载”，离开再打开状态保持，重新启用成功，卸载后恢复“安装”。测试安装已移除，没有安装 HTTPie CLI 或发出业务请求。
- 未逐一连接云账号/执行 30 个第三方 CLI 的业务操作；文档与包结构验证不等于全部上游业务已跑通。

### 原色 Logo 更新

- 32 个条目的图标均随包保存（13 PNG、19 SVG），保留来源原始字节，清除了新增条目的临时分类图；原有飞书和企业微信也改用彩色资源。
- 列表、详情和已安装区域统一通过图片元素显示，取消 mask 染色；加载失败仍回退通用插件图标。
- 图标回归断言在旧渲染方式下失败，修复后市场 30 项前端测试通过；最终资源下后端 7 项测试通过。
- 定向 ESLint、架构检查、资源检查和桌面 debug 构建通过。重启新构建后，实机确认列表、GitLab 详情、飞书已安装图标与列表下方资源正常显示；32 份打包图标与源码逐字节一致。
