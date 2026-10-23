# Zeron UI → Kivio

本目录从 Zeron `crates/ui` 整体复制，包含 src、assets、examples、tests、build.rs 和 Cargo.toml。来源提交和原文件校验值见 UPSTREAM.json，许可和第三方声明原样保留。原文件哈希描述导入基线，不代表适配后文件仍字节相同。

当前已加入 Kivio native workspace，主启动入口直接调用本目录 `run_app`。GPUI import 和 Cargo 依赖统一为 GPUI Kit 0.8.0；Shell、composer、transcript、settings 等原始组件实现作为修改基底。源代码所依赖的支持 crate 复制到 `../zeron-support/`，均来自同一锁定 Zeron 提交。Kivio 的模型与存储业务通过 `src/kivio.rs` 接入，来源业务源码继续复用，不复制一套后端。

`browser/` 保留追溯，实际编译的是 `browser_external.rs`，只打开系统浏览器。Wry、WebKit、WebView2 不在当前应用依赖闭包。原始 Zeron engine 仍因 UI 类型/帮助函数而参与编译，但其启动流程被 KivioBackend 替换；原云认证、后台 daemon 和自动更新均不启动。后续需继续收敛这批兼容依赖。

Kit 不支持的 fork 渲染扩展采用明确的视觉降级，未补造同名空实现来声称完全兼容。接入范围、运行命令、测试及剩余限制统一见 [原生客户端说明](../README.md)。
