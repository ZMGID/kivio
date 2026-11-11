# 模型库更新 2026-11-11

内置快照更新为 `2026-11-11.1`，从 2,914 增至 2,973 个模型 ID，新增 66 条、移除 7 条。独立模型库已通过 [PR #3](https://github.com/ZMGID/kivio-models/pull/3) 发布，固定来源为提交 [`0e1aadb6a0e8cd164f72ae7e49cfa5f346786eaa`](https://github.com/ZMGID/kivio-models/commit/0e1aadb6a0e8cd164f72ae7e49cfa5f346786eaa)。正式下载通道与 `npm run models:sync` 均已确认返回相同快照。

新增 Claude Fable 5.2、Mythos 5.2、Gemini 4 Carbon、GPT Image 2.6 Flare 与 Sunburst，以及 LongCat 2.5、Inkling 2 Small、Muse Ember 和近期托管、向量型号。ID 数量包含渠道、别名与批量变体，不等于独立模型数量。来源摘要、逐项增删、排除项和价格边界见模型库的[更新记录](https://github.com/ZMGID/kivio-models/blob/0e1aadb6a0e8cd164f72ae7e49cfa5f346786eaa/REFRESH-2026-11-11.md)。

本次仅更新数据。所有保留的旧模型参数与价格数值均逐条核对，继续保留 Llama 3 8B 的 8,192 输出上限和 Ministral 3B 的 262,144 上下文修正。用户资料和供应商启用列表沿用既有覆盖规则。独立库补齐 8 条 Codex 补充资料的来源字段，修复批量导入中断，并加入来源完整性回归用例。

Fable 和 Mythos 5.2 不能关闭思考，支持五档 effort。Carbon 采用 65,536 普通同步输出上限；需要额外协议的长输出不在此次实现范围内。Carbon 当前首发价在 12 月 1 日起调整，后续价格保存在资料中，客户端不会按日期自动换价。GPT Image 2.6 的文字与图像 token 价格继续分开保存，未将图像输出报价混入普通文本费用。价格表日期表示本次维护日期，并非重审全部旧报价。

沿用退役策略移除 Gemini Embedding 2 Preview 的两个 ID 与 Grok Imagine Image Quality 的五个 ID；旧 API ID 目前转向替代型号，目录清理不删除用户配置。未核实的 GPT-6 Rosalind、Cyber、Astra Law、Together 占位记录和 Pareto 自动路由未收录。

验证通过 38 项前端相关测试、34 项 Rust 模型资料与更新生命周期测试、12 项独立模型库测试、3 项发布脚本测试、TypeScript、定向 ESLint、Rust 格式、架构边界、容量一致性、别名完整性及价格表一致性检查。新增前端用例在旧快照上失败，更新后通过，覆盖新型号识别、关键参数和用户覆盖优先。Rust 匹配用例将已退役的 Grok Image Quality 样例替换为 Image 2.0，并补充新型号样例。未执行付费推理或原生桌面实机验证。
