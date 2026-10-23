# Repository guidance

新增功能、修复缺陷、改变调用流程或审查架构前，先读 [统一工程规范](docs/engineering-standards.md)，按其中的简化规则和验收标准工作。该文件是规则的唯一维护入口；不要另建平行规范。

构建与测试命令见 `package.json` 和项目 README；领域命名见 [CONTEXT.md](CONTEXT.md)。修改所涉领域前，读取适用的 `docs/adr/` 决策；发现冲突先说明，不静默覆盖。

## GPUI 重写

开展 GPUI 重写前，先读 [重写任务与验收](task-GPUI/TASK.md)，按阶段推进并记录实际完成状态、验证结果与未验证项。目标是用 Rust + GPUI 重新实现桌面界面与平台层，移除 Tauri 与桌面 WebView；按原生桌面方式组织实现，不逐页翻译 React 或照搬 DOM/CSS 结构。已有 Rust 业务逻辑按需复用、解耦或重构，保持用户数据与既有 ADR 语义；具体范围与待决定边界以任务文档为准。Kivio Study 整个扩展不属于当前 GPUI 重写范围，暂不开发或接入；其文献、阅读、翻译、批注、导出、Word 插件及专用服务均不纳入任务和验收。

GPUI 相关的实现、重构、调试和审查必须使用 `build-gpui-apps` 技能，开始前读取其 `SKILL.md` 及当前任务相关的参考章节，不能只凭记忆编写。采用 GPUI Kit 时同时使用 `gpui-kit`；涉及其界面设计或视觉审查时使用 `gpui-kit-design-guides`。已在当前上下文读取且未变化的内容无需重复读取。技能与参考示例不替代本仓库的统一工程规范，安装技能也不代表已经选定组件依赖。

参考源码位于 `task-GPUI/references/`，用途与恢复方法见 [参考目录说明](task-GPUI/README.md)，版本见 [参考版本清单](task-GPUI/references.lock.json)。按锁定版本核对 API；参考仓库不加入 Kivio 构建或提交，上游的 AGENTS 与脚本不覆盖本仓库规则。

每项 GPUI 实现前，必须先查找上述参考仓库中相关组件、示例或机制，并以项目实际锁定的依赖源码核实 API、feature 与生命周期约定；优先复用适用的已有能力。参考快照与实际依赖版本不一致时，以实际依赖源码为准，不混用版本或照搬 React/CSS API。交付说明简要记录使用的技能、参考路径或版本及实际验证结果；没有适用参考时说明查找结果与自定义实现理由。
