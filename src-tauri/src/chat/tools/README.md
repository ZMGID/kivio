# 工具契约

本模块提供 SDK-03 的内部候选接口，尚未作为独立 crate 发布。它不读取应用设置，不发现 MCP 服务，也不自动启用文件或命令工具。

## 接入静态工具

实现 `StaticTool`，声明 `Args`、`Output`、`Error` 和 `HostData`。参数使用 `Deserialize` 和 `JsonSchema`，结构化输出使用 `Serialize`。`execute` 返回模型文本、类型化结构化结果和宿主数据。

注册定义通过 `input_schema::<T>()` 生成参数 schema。执行适配器通过 `call_typed` 解码参数、调用实现并序列化结构化结果。业务错误保留到适配边界，再转换为 `ToolError::Execution`。输出编码失败返回 `ToolError::OutputEncoding`，不会生成成功结果。

生产示例是 `native_tools::WriteFile`。`mcp/native_registry.rs` 的 `write` 条目提供声明、执行函数和策略。它保留阻塞线程池、文件锁、原子替换和宿主产物处理。新增普通工具只需实现及注册，不修改 Agent 循环。

本轮的 `AgentRunConfig.tools` 和调用方提供的 `ToolExecutor` 共同组成显式注册边界。循环只匹配这份列表。即使桌面注册表存在某个同名工具，未选入本轮也不能执行。桌面发现、用户配置与授权仍由应用处理。

## 动态工具与输入校验

静态和动态工具共用 `execute_tool_call` 的校验、审批、超时和取消入口。模型参数解析失败、输出截断或 schema 校验失败时，不调用工具。`call_typed` 负责静态参数反序列化，不重复执行循环已经完成的 schema 检查。

`schema` 使用现有 jsonschema 依赖校验动态 JSON Schema 和 schemars 的嵌套引用。外部文件及网络 schema 引用一律拒绝。兼容性转换保留已有字符串数字、布尔值和可选 null 处理。同一文档的 JSON Pointer 引用参与转换；带独立 `$id` 作用域或 anchor 的引用保留原参数，仍执行完整校验。

原生 `task` 的批量校验继续在审批后由其实现执行。该例外由注册条目的 `ArgumentPolicy::ToolAfterApproval` 表达。

## 结果与生命周期

`ToolOutput<H, S>` 分别保存模型可见文本、结构化结果、宿主数据和类型化后续消息。宿主数据不自动拼入模型文本。当前桌面将 `H` 设为 `Vec<ModelArtifact>`，用于现有工具卡。

动态 MCP 的 wire 数据由 `chat/tool_output.rs` 适配。它保留 MCP 结构化反馈、错误状态和图片后续消息，原始协议负载不进入核心。原生工具保留已格式化的文本，避免重复附加结构化 JSON。

`ToolExecutor::call` 必须等待所需结果准备完成。三个生产宿主都先执行工具，再通过 `tool_output::prepare` 等待产物发布和转换。整个调用仍处于同一个超时与取消范围内。准备失败产生错误记录；模型和工具卡均不会看到成功。

原生工具的超时与并行属性归 `NativeToolEntry`。`ToolTimeout::Managed` 表示由工具管理等待时间，其他策略使用默认、最小或参数决定的时限。并行仍需通过原有授权检查，写操作保持串行。子代理仍等待已经开始的工具完成清理，才释放运行资源。

## 验证

`tools/tests.rs` 验证静态参数、类型化错误和输出编码失败。`schema.rs` 验证嵌套引用、动态约束、兼容性转换及外部引用拒绝。`agent/tool_contract_tests.rs` 通过真实循环检查显式注册、静态与动态工具往返及失败状态。原生写入测试检查实际文件和结构化回执。

运行完整验证使用仓库的 `scripts/win-cargo-test.ps1 --lib`。相关过滤路径包括 `chat::tools`、`chat::agent`、`chat::tool_output`、`mcp` 和 `native_tools`。
