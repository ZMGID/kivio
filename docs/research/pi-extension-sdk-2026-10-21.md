# Pi 扩展机制分析与 Kivio 首版边界

2026-10-21 查阅官方仓库（`badlogic/pi-mono` 当前重定向到 `earendil-works/pi`）：

- [扩展文档](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md)
- [类型与 SDK](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts)
- [运行时派发](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/runner.ts)

Pi 的扩展工厂接收统一 API，事件处理器接收事件及上下文。工具调用可返回阻止结果，结果事件可以返回修改字段；运行时定义各类事件的合并与短路规则。工厂注册与会话资源生命周期分开，重载后旧上下文失效。Pi 的扩展在进程内执行，不是权限沙箱。

Kivio 采用版本化清单、统一 SDK、带类型的双向生命周期 Hook、延迟加载、串行启停和失效上下文。Study 用 beforeDisable 返回保存失败原因，宿主据此保留启用状态；原生租约阻止启停与新任务竞争。与 Pi 的终端 Agent 不同，Kivio 还需处理 React 页面、Tauri 命令与后台进程，因此页面卸载和扩展关闭是不同生命周期。

首版不复制 Pi 的全部事件，也不新增通用字符串事件总线。只提供实际接入的 activated / beforeDisable / deactivated。第三方源码作者可以按相同目录与 SDK 编写扩展；原生包仍需显式编译注册。在线包安装、权限隔离和独立更新另需真实需求与安全边界。

实现入口：仓库 [extensions](../../extensions/README.md)。工程规范仍以 [统一工程规范](../engineering-standards.md) 为唯一规则入口。
