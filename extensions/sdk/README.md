# Extension SDK v1

入口是 [`index.ts`](index.ts)，宿主能力分别从 `chat.ts`、`ui.ts`、`files.ts` 导入。静态链接的原生包通过 `native.rs` 使用公开的存储、模型、进程与执行租约能力，不直接引用宿主私有实现路径。SDK 与应用源码一起发布，`apiVersion` 不匹配会拒绝载入；扩展业务版本由 `version` 独立描述。

```tsx
import { defineExtension } from '../sdk'
import { Button } from '../sdk/ui'

export default defineExtension({
  Page: () => <Button onClick={() => { /* 用户操作 */ }}>开始</Button>,
  activate(api) {
    api.on('beforeDisable', async (event, context) => {
      // event.extensionId / event.reason 定位请求；context.signal 标记实例生命周期。
      // await savePendingChanges()
      // 保存失败可以 return { cancel: true, reason: '尚有内容未保存' }
    })
    api.on('activated', (event, context) => {
      // event.reason 为 enable 或 restore；此时原生启用已经确认。
      // 如有监听器、定时器，必须在 context.signal 的 abort 回调中释放。
    })
  },
})
```

## 双向 Hook

宿主调用 `(event, context)`，扩展通过结构化返回值回应，不通过第二个全局事件偷偷修改流程。

| Hook | 输入 | 返回及失败语义 |
| --- | --- | --- |
| `activated` | 扩展 ID，`enable` / `restore` | 原生启用成功后通知；主动启用时失败会回退启用并销毁实例 |
| `beforeDisable` | 扩展 ID，`user` | `void` 允许；`{ cancel: true, reason }` 阻止。抛错同样阻止，错误显示在管理页 |
| `deactivated` | 扩展 ID，`user` | 原生停止与设置保存成功后的通知；抛错仍执行实例释放，不伪造已回滚 |

同一实例的 Hook 按注册顺序串行执行，第一个取消或错误停止该次派发。`on` 返回可重复调用的退订函数。宿主串行化启停，关闭完成或激活失败后 abort 旧上下文并清除全部订阅；旧 API 不允许再注册 Hook。所有异步任务都应检查 signal，不可在失效后写入新实例。正常切换页面只卸载页面；关闭扩展才结束扩展实例和后台服务。

目前 Hook 只覆盖实际已接入的扩展生命周期，不宣称支持 Pi 全套工具调用与模型请求拦截。将来新增 Hook 应明确输入、结果合并顺序、取消和异常语义，并接入真实宿主操作。

## 页面和能力

`Page` 接收 `ExtensionPageProps`：宿主消息列表/输入框状态、模型选择，以及创建、激活和发送嵌入式对话的方法。消息、流式输出和草稿仍由宿主持有。使用 SDK 提供的组件和草稿操作，扩展只保存自己领域的数据及关联 ID。传入 `onCreateConversation` 的标题由扩展决定。这三个宿主操作绑定当前激活实例；关闭后旧回调不能借相同包 ID 操作重新开启的实例。

`chat.ts` 是现有宿主聊天能力的公开接口；API v1 暂时复用宿主组件的类型，因此属于同版本源码 SDK，不承诺跨桌面版本的二进制兼容。`ui.ts` 复用主题、控件、通知与可访问性。

## 启停事务与数据

前端：注册模块 → 原生启用确认 → activated → 显示入口。注册失败不会启动原生服务。关闭：beforeDisable（Study 在这里等待写入）→ 原生关闭确认 → deactivated → AbortSignal/释放实例/隐藏入口。保存失败保留启用状态。

原生：持久设置在应用数据目录 `extensions.json`，缺失字段/文件默认为全部关闭；已有 Study 数据不使它自动开启。命令执行租约和启停写锁隔离并发，新命令不能越过关闭。正在执行的命令结束后，停止安装、翻译和 Word 服务。关闭不删除数据或已安装的依赖。

## 参考

调研依据及 Kivio 取舍见 [Pi 扩展机制分析](../../docs/research/pi-extension-sdk-2026-10-21.md)。
