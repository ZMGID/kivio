# OpenCode 模型档位

## 来源与协议

- T3 Code [`openCode2Catalog.ts`](https://github.com/pingdotgg/t3code/blob/d2c25950b4d9183ea342d5d5fc14c326b9e0172a/packages/provider-opencode/src/server/openCode2Catalog.ts) 从 `model.list` 读取每个模型的 `variants[].id`；[`status.ts`](https://github.com/pingdotgg/t3code/blob/d2c25950b4d9183ea342d5d5fc14c326b9e0172a/packages/provider-opencode/src/server/status.ts) 将其作为 Reasoning 选项，没有 variants 的模型不显示该选项。
- Kivio 保留 ADR-0003 的 ACP 会话协议。OpenCode 2 的 CLI `api model.list --param location[directory]=…` 提供同一目录数据；ACP 的 `session/set_config_option` 接受 `configId: "effort"`。不引入另一套 HTTP 会话、进程或私有数据库读取。
- [OpenCode CLI 文档](https://opencode.ai/docs/cli/)说明 CLI 默认复用其后台服务；本机 `opencode api --help` 确认支持 OpenAPI operation ID 和查询参数。

## 改动

原路径只解析 `opencode models` 的模型 ID，档位信息为空。现在优先读取结构化目录，按完整 `providerID/modelID` 保存真实档位与默认选项；无档位模型保留空列表，自定义档位原样传递。仅持久化模型 ID 和档位，不保留 API 结果中的供应商设置或凭据。旧 CLI 保留原模型列表与 ACP 回退，不硬编码猜测档位；此次未增加 OpenCode 1 的档位发现。

发现仍归 `external_agents/detection.rs`，沿用现有模型快照与后台刷新。选择器复用已有 `reasoningByModel`，无需增加按钮或 IPC 契约。

ACP 的配置应用仍归 `AcpSession`。连接及续聊切换模型后读取新 `configOptions`，再设置该模型的 effort；失败和超时不再被当成已应用。返回默认档位发送 `default`，切到无档位模型清除旧选择。

## 验证与限制

- 本机 OpenCode 2.2.6：`model.list` 与 `models` 均返回 80 个模型。直接 ACP 验证旧模型不支持的 `max` 返回错误；先切到支持该档位的模型后，`max`、`default`、切至无档位模型、再切回并设置 `high` 均成功。未发送生成请求；探测创建的两条空会话已通过 ACP 删除。
- 选择器 21 项测试通过，新增覆盖自定义档位、默认档位、切至无档位模型。
- 从生产文件提取目录解析函数及仓库测试，用已缓存 Rust 依赖独立编译运行：5 项目录测试及 1 项本机真实目录检查通过。这不等同于完整应用编译。
- 未执行完整 Rust 测试或桌面端到端验收；未验证 OpenCode 1 或其他 ACP 客户端实机行为。
