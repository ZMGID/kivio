# Kivio 文献库：Zotero 调研与实现取舍

## 核验来源

- [Zotero 条目模型](https://www.zotero.org/support/kb/library_items)：书目条目是主体，PDF 与笔记附属于条目，条目本身可以没有全文。
- [分类与标签](https://www.zotero.org/support/collections_and_tags)：分类可嵌套，一个条目可属于多个分类；移出或删除分类不等于删除文献；回收站可以恢复条目。
- [PDF 元数据识别](https://www.zotero.org/support/retrieve_pdf_metadata)：PDF 导入后匹配书目信息，识别失败允许手动录入；不是所有 PDF 都有可靠资料。
- [重复检测](https://www.zotero.org/support/duplicate_detection)：Zotero 根据标题、DOI 等字段提示可能重复，合并需保留分类和标签。
- [参考文献生成](https://www.zotero.org/support/creating_bibliographies)：文献管理延伸到写作输出；书目交换与格式化引用是不同能力。
- [Crossref REST API](https://www.crossref.org/documentation/retrieve-metadata/rest-api/) 与 [DataCite 单 DOI 查询](https://support.datacite.org/docs/api-get-doi)：使用公开 DOI 注册数据取得书目资料，DataCite 覆盖 arXiv DOI。

## 关键结论

上一版只有阅读器，没有足够的文献管理入口。应把默认入口改为文献库：左侧分类与标签、中间文献列表、右侧可编辑资料；从条目进入原文与对话，再回到原列表。保留 Kivio 的全局控件和主题，复用已接好的真实聊天，不复制 Zotero 的桌面外观。

本次交付闭环：

1. 收集：批量 PDF、DOI、手动条目、UTF-8 RIS 导入；无 PDF 条目可后续关联全文。
2. 整理：标题、类型、作者、年份、出版物、DOI、网址、摘要、标签；嵌套分类、多分类归属、收藏与阅读状态；批量加入/移出分类、状态和标签调整。
3. 找回：书目字段与阅读笔记搜索、标签交集筛选、标题/作者/年份/加入时间排序、未分类/未读/在读/已读/收藏视图。
4. 阅读：双击条目或使用阅读入口打开现有原文与真实聊天；返回文献库保留当前筛选和选择。
5. 整理错误：相同 PDF 字节指纹或 DOI 导入复用原条目；可能重复列表供人工核验；移入回收站与恢复不删除 PDF、笔记和聊天历史。
6. 交换：选中条目或当前筛选结果导出 RIS，可导回 Zotero。RIS 包含当前支持的书目字段与标签，不包含 PDF、分类、笔记或聊天。

## 所有权与兼容

`reading.rs` 持有文献与本地 PDF 副本，原有每文档目录不变；新增字段以 serde 默认值读取旧记录。分类独立保存在 `collections.json`，软移除分类不删除文献。`readingLibrary.ts` 是当前窗口唯一写入负责人，序列化写入并保留失败草稿供重试。聊天消息仍由原聊天存储持有，文献只关联对话 ID。

DOI 查询对 `10.48550/` 直接使用 DataCite；其他 DOI 先查询 Crossref，仅在 Crossref 返回 404 时回退到 DataCite。arXiv 条目标记为预印本，RIS 使用 `UNPB` 类型。资料更新仅用非空返回字段覆盖已有书目值，缺失的作者、年份、出版物、网址或摘要保留原有手填内容。

旧路径：阅读路由 → 单 PDF 工作台。新路径：同一路由 → 文献库 → 条目 → 原 PDF 工作台；删除“打开 PDF 就是首页”的入口，保留既有阅读与聊天链路。

## 明确边界

这不是完整 Zotero 替代品。本次不提供账号同步、浏览器采集扩展、Word 引文插件、CSL 样式引擎、附件多版本合并或 OCR。可能重复仅提示，不自动合并不同 PDF 版本；附件目前每条目一个 PDF，拒绝覆盖已有 PDF 以保护页码锚点。元数据识别为显式操作：读取前两页识别 DOI/arXiv ID，或读取 PDF 内嵌标题作者；联网只查询标识符，不上传全文。RIS 采用常见核心字段，未宣称完整无损支持 Zotero 的全部类型和专用字段。
