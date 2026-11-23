# Kivio 内置专家来源

本发行版节选并适配了开源项目的提示词。项目及随附许可证为 [PM Skills（MIT）](pm-skills-MIT.txt)、[Marketing Skills（MIT）](marketingskills-MIT.txt)、[Fabric（MIT）](fabric-MIT.txt) 和 [Anthropic Knowledge Work Plugins（Apache-2.0）](knowledge-work-plugins-Apache-2.0.txt)。

每个专家在 `src-tauri/resources/experts/` 下有一份 JSON 配置。`system_prompt` 取自下列项目的固定版本，`upstream.url`、`upstream.lines` 和 `upstream.license` 分别记录原文件、采用的行段和许可证。

名称和简介已汉化，工作流保留上游英文。适配包括节选与任务有关的步骤和输出模板、把命令参数和目标语言占位符换成用户输入，以及移除宿主专用的连接器标记和技能调用提示。

| 专家 | 原始文件（固定版本） | 节选行段 |
| --- | --- | --- |
| 客服工单分诊 | [anthropics/knowledge-work-plugins · ticket-triage](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/customer-support/skills/ticket-triage/SKILL.md) | 25-97, 103-205 |
| 数据报表核对 | [anthropics/knowledge-work-plugins · validate-data](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/data/skills/validate-data/SKILL.md) | 27-95, 97-125, 129-165, 167-257, 258-287 |
| 代码审查 | [anthropics/knowledge-work-plugins · code-review](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/engineering/skills/code-review/SKILL.md) | 44-99 |
| 故障排查 | [anthropics/knowledge-work-plugins · debug](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/engineering/skills/debug/SKILL.md) | 19-75 |
| 资料交叉核查 | [anthropics/knowledge-work-plugins · knowledge-synthesis](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/enterprise-search/skills/knowledge-synthesis/SKILL.md) | 71-100, 102-164, 221-258 |
| 项目周报 | [anthropics/knowledge-work-plugins · status-report](https://github.com/anthropics/knowledge-work-plugins/blob/0e0fcacddcc892eb92a1b7873ff9d68159c47571/operations/skills/status-report/SKILL.md) | 19-58, 73-77 |
| 广告素材复盘 | [coreyhaines31/marketingskills · ad-creative](https://github.com/coreyhaines31/marketingskills/blob/5b2b400eec1e7e1cec76245a715d0b0d4fd2a9c1/skills/ad-creative/SKILL.md) | 96-98, 225-266 |
| 落地页文案 | [coreyhaines31/marketingskills · copywriting](https://github.com/coreyhaines31/marketingskills/blob/5b2b400eec1e7e1cec76245a715d0b0d4fd2a9c1/skills/copywriting/SKILL.md) | 18-38, 81-85, 88-108, 189-202, 210-217, 309-329 |
| 产品演示编导 | [coreyhaines31/marketingskills · video](https://github.com/coreyhaines31/marketingskills/blob/5b2b400eec1e7e1cec76245a715d0b0d4fd2a9c1/skills/video/SKILL.md) | 22-31, 249-258 |
| 商务邮件代拟 | [danielmiessler/fabric · create_formal_email](https://github.com/danielmiessler/fabric/blob/8ee5655779faa5ed0bf5a0b14a72be4d9ddd6e47/data/patterns/create_formal_email/system.md) | 1-50 |
| 汇报演示策划 | [danielmiessler/fabric · create_keynote](https://github.com/danielmiessler/fabric/blob/8ee5655779faa5ed0bf5a0b14a72be4d9ddd6e47/data/patterns/create_keynote/system.md) | 1-3, 7-42 |
| 原稿润色 | [danielmiessler/fabric · improve_writing](https://github.com/danielmiessler/fabric/blob/8ee5655779faa5ed0bf5a0b14a72be4d9ddd6e47/data/patterns/improve_writing/system.md) | 1-15 |
| 保留格式翻译 | [danielmiessler/fabric · translate](https://github.com/danielmiessler/fabric/blob/8ee5655779faa5ed0bf5a0b14a72be4d9ddd6e47/data/patterns/translate/system.md) | 1-3, 7-22 |
| SQL 取数分析 | [phuryn/pm-skills · sql-queries](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-data-analytics/skills/sql-queries/SKILL.md) | 8-33, 74-80 |
| 会议纪要 | [phuryn/pm-skills · meeting-notes](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-execution/commands/meeting-notes.md) | 17-29, 33-74, 83-89 |
| 需求梳理 | [phuryn/pm-skills · create-prd](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-execution/skills/create-prd/SKILL.md) | 6-76, 79-90 |
| 功能验收 | [phuryn/pm-skills · test-scenarios](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-execution/skills/test-scenarios/SKILL.md) | 16-49, 79-86 |
| 竞品拆解 | [phuryn/pm-skills · competitor-analysis](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-market-research/skills/competitor-analysis/SKILL.md) | 6-103 |
| 功能优先级排序 | [phuryn/pm-skills · prioritize-features](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-product-discovery/skills/prioritize-features/SKILL.md) | 6-9, 19-42 |
| 用户访谈分析 | [phuryn/pm-skills · summarize-interview](https://github.com/phuryn/pm-skills/blob/9af7bfd0bc47fdad43c2eaa7a43cd13918e7aaef/pm-product-discovery/skills/summarize-interview/SKILL.md) | 6-9, 14-44 |
