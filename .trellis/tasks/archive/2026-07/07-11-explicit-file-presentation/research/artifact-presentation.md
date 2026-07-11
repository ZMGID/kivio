# Artifact 展示调研摘要

- 当前文件卡统一追加到回答末尾；图片只有通过 Markdown 文件名引用时能进入正文中间。
- artifact 不是一等 segment，且没有稳定 ID。
- 推荐使用独立展示工具表达模型意图，并利用结构化时间线位置持久化展示顺序。
- 本任务采用更小但结构化的实现：`present_artifacts` 工具调用本身就是位置锚点，工具记录的 structured_content 保存 artifact IDs；不使用 Markdown 占位符。
- 新 artifact 显式展示，旧无 ID artifact 保持末尾自动显示。
