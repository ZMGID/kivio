# Design: 普通对话默认工作台

## Overview

将当前分散的 `workspace_roots`、普通对话相对路径和 `~/Kivio/outputs/<conversation>` 收敛为一个运行时概念：`NativeToolWorkspace` 的默认工作目录。

- 普通对话：`<settings.working_directory>/<conversation-id>`
- 项目对话：项目绑定根目录
- 显式绝对路径或 `~/` 路径：始终按用户指定路径处理

默认工作目录仅用于路径缺省和相对路径解析，不承担沙箱或访问控制职责。

## Configuration Contract

`ChatNativeToolsConfig` 新增：

```rust
working_directory: String
```

默认值由后端统一生成：`<user_home>/Kivio/workspace`。旧 `workspace_roots` 仅保留反序列化兼容；当新字段为空时取旧列表第一项，随后不再作为运行时配置或前端设置暴露。

前端对应 `workingDirectory: string`，设置页用单路径输入框、目录选择按钮和恢复默认按钮。

## Runtime Data Flow

```text
Settings.working_directory
  -> resolve_native_workspace(app, conversation)
  -> project conversation ? project root : working_directory/conversation_id
  -> NativeToolWorkspace.default_directory
  -> relative file paths / missing command cwd / run_python artifact export
```

`NativeToolWorkspace` 负责区分：

- `standalone`: 无对话上下文，保持用户主目录兼容行为；
- `conversation`: 普通对话的按需工作台；
- `project`: 项目根目录。

路径解析规则：

1. 绝对路径或 `~/` 路径不改写，不触发工作台限制。
2. 相对路径基于 `default_directory`；普通对话仅在真正使用相对路径或默认 `cwd` 时创建目录。
3. 项目根目录不存在时继续明确报错。

## Prompt Contract

系统提示词注入当前默认工作目录，并明确：

- 它是默认临时工作台，不是访问限制；
- 用户明确指定路径时优先使用指定路径；
- 未指定产出位置时写入默认工作目录；
- 相对文件路径和缺省命令 `cwd` 从这里解析。

项目对话保留现有项目绑定说明，但统一“显式路径优先”的措辞。

## Artifact Delivery

`sandbox_exports.rs` 不再拥有固定 outputs 根目录。`SandboxExportContext` 携带本次运行解析出的目标目录：

- 普通对话：对话工作台；
- 项目对话：项目根目录；
- 无对话上下文：后端兼容目录。

`run_python` 生成物直接写入目标目录并生成文件卡。普通对话中 `write_file` 写入工作台的文件继续生成文件卡；项目代码写入维持“不自动为每次编辑生成文件卡”的现有体验。

工作台是用户工作目录，不能沿用 outputs 的“最多 16 个文件自动删除”策略；移除交付目录自动裁剪，防止删除用户工作文件。

生成文件打开/定位命令改为验证“绝对且存在的文件”，不再依赖固定 outputs 根目录。只有后端生成的 artifact 卡会调用这些命令。

## Migration

### Legacy configuration

后端 sanitize 时：

1. 使用非空 `working_directory`；
2. 否则取 `workspace_roots[0]`；
3. 否则使用 `<user_home>/Kivio/workspace`。

### Legacy outputs

首次解析普通对话工作台时，如存在 `~/Kivio/outputs/<conversation-id>`：

- 无冲突合并到当前工作台；
- 成功后删除旧目录；
- 更新该对话消息和工具调用中的 artifact path；
- 有冲突或复制失败时保留旧目录并返回明确错误，不覆盖文件。

### Global root change

`save_settings` 在替换运行时设置和持久化之前执行迁移：

- 只处理现存的合法普通对话目录；
- 从旧根目录迁往新根目录；
- 同时吸收尚未迁移的 legacy outputs；
- 预检目标冲突，任何同名目标均阻止迁移；
- 失败时旧设置与源文件保持有效；
- 成功后更新 artifact path 并持久化新设置。

跨卷迁移采用“无覆盖复制 -> 校验成功 -> 删除源目录”，同卷可优先 rename；目标已有非冲突内容时执行安全合并。

## Deletion

删除对话前加载其项目归属：

- 普通对话：删除当前配置根目录下由合法 conversation ID 派生的目录；
- 项目对话：绝不删除项目根目录；
- 始终清理 legacy outputs/runs 残留。

删除 helper 通过规范化后的根目录 + 合法 ID 构造目标，不接受任意路径输入。

## Error Handling

- 配置根目录创建失败：工具返回明确错误，不回退用户主目录。
- 迁移冲突：设置保存失败，指出冲突目标；不覆盖、不删除源文件。
- legacy outputs 懒迁移失败：本次需要工作台的工具失败并提示用户处理；显式外部绝对路径仍可独立使用。

## Compatibility

- Kivio Code/CLI 的项目工作区构造保持项目语义。
- 无 native conversation context 的测试/独立调用保持用户主目录兼容行为。
- 不改变工具审批和磁盘访问权限模型。
