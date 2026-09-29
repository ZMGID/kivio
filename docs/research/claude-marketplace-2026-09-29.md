# Claude Code 市场接入核验

依据：[官方市场格式参考](https://code.claude.com/docs/en/plugins/marketplace-reference)、[官方市场仓库](https://github.com/anthropics/claude-plugins-official)。核验日期：2026-09-29。

市场索引 `.claude-plugin/marketplace.json` 和插件包 `.claude-plugin/plugin.json` 是不同层级。索引列出身份、展示信息、来源；组件明细需要进一步读取插件源码。相对来源从仓库根开始，不从 `.claude-plugin` 开始。Git 来源含仓库文件，远程 JSON 来源只有索引。

已对照 `E:/ZM database/ZCode-reference/`：

- `apps/zcode-cli/packages/adapters/src/plugins/marketplace.ts` 负责来源解析、暂存校验、激活缓存、安装来源解析，市场和安装记录独立。
- `packages/ui/src/settings/PluginStorePage.tsx` 添加成功切个人市场，`PluginAddMenu.tsx` 提供添加市场入口；Kivio 沿用这个交互，表单在弹窗内。
- `packages/shared/src/plugin-marketplaces.ts` 区分默认公开市场和用户市场；Kivio 自定义来源同样归个人页。

没有照搬 ZCode 的宿主调用和持久化布局。Kivio 的 `plugins/marketplaces.rs` 管市场目录生命周期，`packages.rs` 继续唯一负责插件副本及启停；页面通过 `src/api/market.ts` 提交意图。来源记录不存另一份安装状态，安装状态由 package 的市场身份派生。

官方现行规范与这份 ZCode 代码的旧兼容逻辑存在差异：无插件清单时，当前官方允许任意 strict 值使用条目作为清单；有清单时，strict 为 true 的组件追加、hooks 按事件覆盖，strict 为 false 且两处声明组件时报冲突。实现按当前官方语义归一化托管副本，不改用户仓库。

本次支持 GitHub、HTTPS Git、本地目录/文件、HTTPS JSON 市场；支持相对路径、github、url、git-subdir 插件来源。npm/archive/command、认证辅助脚本、自动更新和依赖安装暂不支持。移除来源保留已安装插件；这与 Claude CLI 的级联卸载行为不同，是 Kivio 管理来源的明确约定。完整兼容边界见 [插件格式](../agents/kivio-plugin-format.md)。

图标补充：官方索引未提供 icon 字段；ZCode 本机的增强目录包含 244 个 HTTPS 图标地址，指向 `cdn-zcode.z.ai/zcode/official-plugin/assets/<name>/icon.png`。Kivio 将这份地址映射保存为 `src/chat/market/claude-market-icons.json`，仅匹配 `anthropics/claude-plugins-official` 的真实 GitHub 来源，避免同名第三方条目误用品牌。当前 314 个条目中匹配 244 个，全部地址联网验证成功。列表、详情、已安装插件共用解析；移除市场后已安装条目仍可由记录的来源身份显示图标。图片在线加载，保留原色、懒加载、不发送 referrer；缺失或加载失败回退默认图标。

详情加载补充：对照 ZCode 的 `describeMarketplacePlugin`、`readComponentsAtRoot` 和 `plugin-components.ts`，打开详情后按需解析技能 frontmatter、命令、智能体、MCP 配置及 Hook 事件。Kivio 的 `packages/details.rs` 统一负责枚举；已安装插件读托管副本，未安装条目复用原安装流程的来源解析、受限复制与清单归一化，在独立临时目录读取后清理，不创建安装记录、不启动 MCP、不执行 Hook。前端共用 `PluginContents`，显示加载、失败重试、部分文件诊断、分组名称/说明和作者/版本/网站/许可证；离开页面后忽略迟到结果。

批量兼容修正：按当前已添加官方目录的固定 revision 检查 314 个条目的 Claude/Codex/Kivio 清单，98 个同时包含 Claude 和 Codex，读取清单请求无网络错误。市场条目必须按 Claude 格式处理，不能套用直接导入时的格式优先级。已移除混合清单拒绝逻辑，`manifest_location` 统一负责预览、安装和后续加载选择；直接导入仍保持原优先级，不合并不同格式的能力。同时修复 `url`/`github` 的可选 `path` 被忽略的问题：目录中的 atomic-agents、rc、revenuecat、zilliz 共 4 个条目受影响。路径仍受相对目录和包含性检查约束。此批量核查验证清单结构，不等于全部插件的运行时能力测试。
