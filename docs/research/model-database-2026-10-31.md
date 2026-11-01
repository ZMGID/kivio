# 模型库更新 — 2026-10-31

## 本地修复：容量一致性（2026-11-01.1）

内置快照修正两条容量矛盾：[Meta Llama 3 官方模型卡](https://github.com/meta-llama/llama3/blob/main/MODEL_CARD.md)确认 8K 窗口，将 `meta-llama/llama-3-8b-instruct` 的 `maxOutput` 从 16,384 恢复为 8,192；[Ministral 3 3B 官方资料](https://docs.mistral.ai/models/ministral-3-3b-25-12)确认 256K 窗口，将 `ministral-3b-2512` 的 `contextWindow` 从 131,072 修正为 262,144，保留已有输出上限。其他模型和价格不变。

发布脚本及原生下载校验拒绝「已知正数上下文容量小于最大输出」的数据；容量为 0 的未知窗口不据此拒绝。非法下载不覆盖当前快照或缓存，用户覆盖仍保留。内置版本递增为 `2026-11-01.1`；本次没有向独立模型库仓库发布，`.source.json` 继续记录补丁前的上游来源，而非声称存在新的远端提交。

## 此前：仅修正 OpenAI 上下文（2026-10-31.3）

近期 OpenAI 型号及已确认别名的 contextWindow 统一采用 Codex 当前窗口：GPT-6.1 Terra / Sol 为 500K；GPT-6 Astra / Sol / Luna 与 GPT-5.6 Sol / Terra / Luna 为 272K。本机 Codex 0.166.0 在 2026-10-31T08:11:50.995338700Z 刷新的目录确认这些值，仅提取型号与窗口保存到模型库 sources/codex-models-2026-10-31.json。发布数据仅变更 38 条记录的 contextWindow 及版本号，其他参数保留 .2；没有新增模型或计费改动。用户覆盖仍优先，已有客户端更新模型库即可取得这些默认值。

以下小节保留历史，其 API/OAuth 窗口区别已由当前库默认值覆盖。

## 后续：以 OMP 为准对齐（2026-10-31.2）

按本次指定的优先级，以 [OMP 固定快照](https://github.com/can1357/oh-my-pi/blob/c29f7c56f5530d805bb2887ade964aa55970d8e1/packages/catalog/src/models.json) 对照 910 条记录，其中 791 条有来源信息以外的字段变化；加入 8 条 Codex 订阅记录，共 2,914 条。完整差异和边界见模型库仓库的 `OMP-COMPARISON-2026-10-31.json` 与 `OMP-ALIGNMENT-2026-10-31.md`。此节与 OMP 记录优先于下面保留的首轮原厂核验结果。

已直接发布到模型库 `main`，提交 [`1d6d70b20cd0e6f789fc6efc1ff9f579453605cb`](https://github.com/ZMGID/kivio-models/commit/1d6d70b20cd0e6f789fc6efc1ff9f579453605cb)。正式下载通道和默认同步命令均已确认返回 `2026-10-31.2`；101 项前端、35 项 Rust、10 项模型库发布测试通过，TypeScript、定向 ESLint、架构及版本校验通过，[远端 Validate](https://github.com/ZMGID/kivio-models/actions/runs/41182875987) 通过。

GPT-5.4 改为 1,000,000，Mini/Nano 为 400,000；GPT-5.5 Pro 档位为 low/medium/high/xhigh。OpenAI API 的 GPT-6 系列为 1,050,000，而 Codex 订阅的 Astra/Sol/Luna 默认 272,000、6.1 Sol 默认 500,000，输出均为 128,000。价格按 OMP 的具体渠道记录，不把原厂、OpenRouter 和订阅的卡片混为同一报价；使用 `kind: "catalog"` 标注来源，继续由原价格表维护。

现有前后端 `providerModelDatabaseId` / `provider_model_database_id` 扩展为 Codex OAuth 选择已收录的 `openai-codex/…` 记录；其他模型保留已有查找路径。用户覆盖依然优先，实际发给供应商的模型 ID 不变。未新增公开命令、协议或运行时层。已安装客户端能读取通用资料更新，自动识别 Codex 专属默认值需要带本次代码的应用版本。

OMP 的原始思考元数据与计费阶梯均保留；预算预设、额外 effort 值、虚拟 Pro ID 和自动长期计费不是此次新增的应用能力。未做付费调用或桌面实机验证。

## 首轮更新（2026-10-31.1）

内置快照由 `2026-10-16.2` 更新到 `2026-10-31.1`，从 2,756 增至 2,906 个模型 ID：新增 186 条、移除 36 条到期记录，包含 958 个兼容别名。ID 数量包含原厂限定 ID、日期和托管变体，不等于底层独立模型数量。

数据沿用 [kivio-models](https://github.com/ZMGID/kivio-models) 的既有导入器、v1 格式和官方价格表。此次重新获取 Models.dev、OpenRouter 全模态目录和 LiteLLM，源文件 SHA-256 与增删清单记录在上游 `IMPORT-REPORT.json`；逐字段核验记录在 `REFRESH-VERIFICATION-2026-10-31.json`。

上游已发布到 `main`，提交为 [`1ff909bcd2eca7df19038280e39cab34bc69ee5b`](https://github.com/ZMGID/kivio-models/commit/1ff909bcd2eca7df19038280e39cab34bc69ee5b)。本地通过现有 `npm run models:sync` 同步并校验，正式通道返回 `2026-10-31.1`、2,906 条记录；应用内「更新模型库」与默认发版同步均可取得此版本。

主要补充 GPT-6.1 Terra、Kimi K3.1 Pro / Highspeed、MiniMax M3.1 Flash、Mercury 3、Gemini 4 Argon，以及近期开放权重、图像、视频和向量模型。原有 Claude Haiku 5.5 补齐限定 ID 的思考档位和流式能力。

- [OpenAI Terra](https://developers.openai.com/api/docs/models/gpt-6.1-terra)：上下文 1,050,000、输出 128,000，允许关闭思考，档位 low/medium/high/xhigh/max，参考价为输入 1、输出 5、缓存输入 0.05 美元／百万 token；超过 272K 输入的价格阶梯保留在模型库。工具调用仍要求 Responses。
- [Kimi Pro](https://platform.kimi.ai/docs/guide/kimi-k3-1-pro-quickstart)：始终思考，low/high/max；[API](https://platform.kimi.ai/docs/api/chat) 明确默认输出预算 131,072、最大 1,048,576，不能把默认值当上限。[官方价格](https://platform.kimi.ai/docs/pricing/chat)分别为 Pro 2/10/0.20、Highspeed 4/20/0.20。
- [MiniMax API](https://platform.minimax.io/docs/api-reference/text-chat-openai)：正式 Flash 的上限为 524,288，纠正聚合目录的 512,000；不能关闭思考，支持五档。[报价](https://platform.minimax.io/docs/pricing/overview)为 0.15/0.60/0.015；保留旧 Preview 的独立身份。
- [Mercury 3](https://docs.inceptionlabs.ai/get-started/models)：1M 上下文、131,072 输出、low/medium/high，温度默认 1.0，首发五折报价 0.10/0.35/0.01，尚无促销结束日期；其 `instant` 档位不在现有枚举内，此次未增加协议。
- [Gemini Argon](https://ai.google.dev/gemini-api/docs/models/gemini-4-argon)：普通同步输出 65,536，1M 长输出需额外协议，未启用。[当前价格](https://ai.google.dev/gemini-api/docs/pricing)为 2/10/0.10，12 月起翻倍；现有客户端不会按日期自动换价。
- [Haiku 5.5](https://platform.claude.com/docs/en/models/haiku-5-5/overview)：保留普通输出 128K 与 100K 输入价格阶梯，未把 Batch 的 300K beta 上限混入普通请求。

价格继续以 `sources/official-pricing.json` 为唯一维护入口，779 个发布 ID 有可追溯报价。价格表日期是最近一次局部修订日期，并非重审全部历史价格；无关报价和 10 月 16 日固定人民币汇率保留。未核实价格继续缺省，不把免费路由或开源模型的占位零价视为官方报价。用户自定义资料、价格和供应商启用列表保持优先。

清理沿用既有退役策略；核对了 [OpenAI](https://developers.openai.com/api/docs/deprecations) 与 [Google](https://ai.google.dev/gemini-api/docs/deprecations) 的近期关闭日期，未删除仅预告未来关闭的 GPT-4.1 等型号。额外排除三个新导入的可疑候选：未核实的 `gpt-6-rosalind`、1000-token Together 占位记录和 Pareto 自动路由；未来导入重新提出时应复核。

验证：97 项前端相关测试、34 项 Rust 模型资料/生命周期测试、10 项上游导入/发布测试通过；TypeScript、定向 ESLint、架构边界、模型库格式、官方价格一致性、别名目标完整性、版本递增与差异检查通过，上游 PR 的 Validate CI 通过。新增前端回归用例已在旧快照上失败、更新后通过。此次没有新增运行时层、依赖或传输实现；未做付费推理或原生桌面实机验证。
