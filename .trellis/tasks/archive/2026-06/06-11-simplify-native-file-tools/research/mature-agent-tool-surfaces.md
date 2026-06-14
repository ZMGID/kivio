# Research: 成熟编码 Agent 的文件工具面设计（pi agent + Claude Code）

- **Query**: 一手核实 pi agent 与 Claude Code 的文件工具面，作为 Kivio 收敛内置文件工具（11 个 → 极简、删除分段写入协议）的设计基准
- **Scope**: external（GitHub 源码 raw 抓取 + 官方文档抓取 + 当前 Claude Code 会话直接观察）
- **Date**: 2026-06-11
- **检索方式**: smart-search CLI（`smart-search fetch` 抓 raw.githubusercontent.com 源码与 code.claude.com 文档；`smart-search exa-search` 定位文档页）。所有引文均来自抓取原文，非训练记忆。

## Sources Reviewed

### pi agent（badlogic/pi-mono，已迁移至 earendil-works/pi，redirect 仍有效）

- 仓库树（GitHub API，确认 redirect 与文件清单）: https://api.github.com/repos/badlogic/pi-mono/git/trees/main?recursive=1 → 实际指向 `earendil-works/pi`
- 工具注册入口: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/index.ts
- write 工具: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/write.ts
- read 工具: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/read.ts
- edit 工具: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/edit.ts
- edit 匹配引擎: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/edit-diff.ts
- 文件互斥队列: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/file-mutation-queue.ts
- 截断常量: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/truncate.ts
- bash 工具: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/src/core/tools/bash.ts
- grep/find/ls 工具: 同目录 `grep.ts` / `find.ts` / `ls.ts`
- README（默认工具集声明）: https://raw.githubusercontent.com/badlogic/pi-mono/main/packages/coding-agent/README.md

### Claude Code

- 官方工具参考（含 Read/Edit/Write/Bash 行为章节）: https://code.claude.com/docs/en/tools-reference.md
- 当前 Claude Code 会话内可见的 Read/Write 工具描述原文（2026-06-11 实测观察，本研究会话即运行在 Claude Code harness 上）

### 本仓库（用于映射）

- `src-tauri/src/mcp/types.rs` — Kivio 当前原生工具定义（name 字段清单）
- `src-tauri/src/native_tools/files.rs` — 工具实现

---

## 1. pi agent

### 1.1 完整内置工具清单

`tools/index.ts` 定义了全部 7 个内置工具，但**默认给模型的只有 4 个**：

```ts
export type ToolName = "read" | "bash" | "edit" | "write" | "grep" | "find" | "ls";

export function createCodingToolDefinitions(cwd, options?) {
  return [
    createReadToolDefinition(...),
    createBashToolDefinition(...),
    createEditToolDefinition(...),
    createWriteToolDefinition(...),
  ];
}
```

README 原文佐证：

> "Then just talk to pi. By default, pi gives the model four tools: `read`, `write`, `edit`, and `bash`."

`grep` / `find` / `ls` 存在但属于 read-only 扩展集（`createReadOnlyToolDefinitions`），不在默认编码工具集内。**写侧工具总数 = 2（write + edit），加上 bash 可以做任意文件操作。没有 delete / move / copy / mkdir / patch 等独立工具——全部交给 bash。**

各工具参数 schema（TypeBox，全部抓自源码）：

| 工具 | 参数 | 必填 |
|---|---|---|
| `read` | `path: string`、`offset?: number`（1-indexed 起始行）、`limit?: number`（最大行数） | path |
| `write` | `path: string`、`content: string` | 全部 |
| `edit` | `path: string`、`edits: [{oldText, newText}]`（数组，支持一次多处） | 全部 |
| `bash` | `command: string`、`timeout?: number`（秒，无默认超时） | command |
| `grep` | `pattern`、`path?`、`glob?`、`ignoreCase?`、`literal?`、`context?` | pattern |
| `find` | `pattern`（glob）、`path?`、`limit?`（默认 1000） | pattern |
| `ls` | `path?`、`limit?`（默认 500） | 无 |

### 1.2 write 工具：大文件如何处理 → **没有任何分段/草稿机制**

`write.ts` 的 description 原文（130 字符，3 句人话）：

> "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories."

promptGuidelines 仅一条：`"Use write only for new files or complete rewrites."`

execute 全部逻辑就是 mkdir + 一次性写入：

```ts
await ops.mkdir(dir);                       // 递归建父目录
await ops.writeFile(absolutePath, content); // fsWriteFile(path, content, "utf-8") 一次写完
return { content: [{ type: "text", text: `Successfully wrote ${content.length} bytes to ${path}` }] };
```

- 无分段、无草稿、无 chunk 协议、无大小上限校验、无 read-before-write 要求。
- 源码里唯一与"大内容"相关的常量 `WRITE_PARTIAL_FULL_HIGHLIGHT_LINES = 50` 纯属 TUI 渲染层（流式参数到达时的增量语法高亮缓存），模型完全不可见。
- 非原子写：直接 `fs.writeFile` 覆盖，无 temp+rename，无备份。

### 1.3 read 工具：有 offset/limit 窗口化，截断时给续读提示

description 原文（模板字符串展开后约 340 字符）：

> "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete."

截断常量（`truncate.ts`）：`DEFAULT_MAX_LINES = 2000`、`DEFAULT_MAX_BYTES = 50 * 1024`，先到者生效，从不返回半行。

超大文件的三层兜底（`read.ts` execute 内）：

1. 截断发生 → 输出尾部追加可执行的续读提示：
   `[Showing lines {start}-{end} of {total}. Use offset={next} to continue.]`
2. 用户 limit 提前截止但文件还有内容 → `[{remaining} more lines in file. Use offset={next} to continue.]`
3. 单行就超过 50KB → 指引模型换 bash：
   `[Line {n} is {size}, exceeds 50.0KB limit. Use bash: sed -n '{n}p' {path} | head -c 51200]`
4. offset 越界 → `Offset {offset} is beyond end of file ({n} lines total)` 报错。

图片自动缩放（默认 2000×2000 内），非视觉模型读图时附加说明文本。

### 1.4 edit 工具的匹配契约

description 原文（约 330 字符）：

> "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting overlapping edits. Do not include large unchanged regions just to connect distant changes."

契约要点（`edit-diff.ts` `applyEditsToNormalizedContent`）：

- **唯一匹配，无 replace_all**。多于 1 处命中直接报错。
- 一次调用支持多个不相交 edit；全部针对**原始文件**匹配（非增量），按位置倒序应用；重叠则报错。
- **两级匹配**：先 exact `indexOf`；失败后 fuzzy 匹配（NFKC 归一化 + 去行尾空白 + 智能引号/Unicode 连字符/特殊空格转 ASCII）。
- runtime 默默处理：strip BOM、CRLF→LF 归一化匹配、写回时恢复原始行尾和 BOM。
- 兼容垫片 `prepareArguments`：模型把 `edits` 发成 JSON 字符串会被 parse；发旧版顶层 `oldText/newText` 会被折叠进数组（注释注明 "Some models (Opus 4.6, GLM-5.1) send edits as a JSON string"）。

失败时的恢复提示原文（全部含下一步动作）：

- 未命中: `"Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines."`
- 多处命中: `"Found {n} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique."`
- 替换后无变化: `"No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."`
- 重叠: `"edits[i] and edits[j] overlap in {path}. Merge them into one edit or target disjoint regions."`

成功消息：`Successfully replaced {n} block(s) in {path}.`，另携带 display diff + unified patch 给 UI（details 字段，非模型主输出）。

### 1.5 描述风格

确实是"一句人话"风格：1-4 句陈述行为 + 截断规则 + 一条使用指引。两个原文样例：

- write: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories."
- bash: "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds."

另有独立的 `promptSnippet`（一行能力摘要，进系统提示词）和 `promptGuidelines`（1-4 条要点），与 description 分离。

### 1.6 pi runtime 层的脏活清单

- **per-file 互斥队列**（`file-mutation-queue.ts`）：write 和 edit 的 execute 都包在 `withFileMutationQueue(absolutePath, fn)` 里，按 `realpath` 解析后的 key 串行化同一文件的并发变更，不同文件仍并行。abort 信号只在 await 间隙检查、不提前释放队列（注释明确解释了原因）。
- **BOM / 行尾保真**（edit）：匹配前 strip BOM + 归一化 LF，写回时还原。
- **fuzzy 匹配兜底**（edit）：Unicode 归一化挽救"看起来一样但字节不同"的 oldText。
- **没有的东西**：原子写（temp+rename）没有；备份没有；read-before-edit 状态追踪没有（edit 只检查文件存在可读写）；写入大小限制没有。

---

## 2. Claude Code

### 2.1 工具面（官方 tools-reference）

文件相关核心工具：`Read`、`Write`、`Edit`、`NotebookEdit`、`Glob`、`Grep`、`Bash`。**通用写侧工具 = 2（Write + Edit）+ 1 个特化（NotebookEdit）**。无 delete/move/copy/mkdir 工具，全走 Bash。

工具表中的一行式描述（doc 原文）：

> `Read` — "Reads the contents of files." / `Write` — "Creates or overwrites files." / `Edit` — "Makes targeted edits to specific files."

### 2.2 Write 的契约

doc 原文（Write tool behavior 全文要点）：

> "The Write tool creates a new file or overwrites an existing one with the full content provided. It does not append or merge. If the target path already exists, Claude must have read that file at least once in the current conversation before overwriting it. A Write to an unread existing file fails with an error. This constraint does not apply to new files. ... For partial changes to an existing file, Claude uses Edit instead of Write."

- 参数面：`file_path` + `content`（本会话工具面直接可见）。
- **read-before-overwrite 是工具层强制（fails with an error），不是 prompt 约定**；新文件豁免。
- **没有任何模型可见的大文件写入保护**：无分段、无草稿、无 chunk、一次 Write 写全部。doc 通篇无任何 size 限制描述。

本会话观察到的 Write description 原文（验证篇幅风格，约 60 词）：

> "Writes a file to the local filesystem, overwriting if one exists. When to use: creating a new file, or fully replacing one you've already Read. Overwriting an existing file you haven't Read will fail. For partial changes, use Edit instead."

### 2.3 Edit 的契约

doc 原文：

> "The Edit tool performs exact string replacement. It takes an `old_string` and a `new_string` and replaces the first with the second. It does not use regex or fuzzy matching. Three checks must pass for an edit to apply:
> - **Read-before-edit**: Claude must have read the file in the current conversation, and the file must not have changed on disk since that read. This check runs first, before any string matching.
> - **Match**: `old_string` must appear in the file exactly as written. A single character of whitespace or indentation difference is enough to miss.
> - **Uniqueness**: `old_string` must appear exactly once. When it appears more than once, Claude either supplies a longer string with enough surrounding context to pin down one occurrence, or sets `replace_all: true` to replace them all."

- 参数面：`file_path` + `old_string` + `new_string` + `replace_all?: bool`。
- read-before-edit 是**工具层校验**且附带**磁盘新鲜度检查**（读后文件被外部改过 → 拒绝）；用 Bash `cat/head/tail/sed -n/grep` 单文件无管道查看也算"已读"。
- 与 pi 的差异：Claude Code 严格 exact（"does not use regex or fuzzy matching"）但提供 `replace_all` 出口；pi 提供 fuzzy 兜底但无 replace_all。

### 2.4 Read 的窗口化与大文件

doc 原文：

> "By default, Read returns the file from the start. When a whole-file read exceeds the token limit, Read returns the first page with a `PARTIAL view` notice that tells Claude how much of the file it received and how to read more with `offset` and `limit`. A read that passes an explicit `offset` or `limit` and still exceeds the token limit returns an error."

本会话观察到的 Read description 确认默认窗口："Reads up to 2000 lines by default"、"Results are returned using cat -n format, with line numbers starting at 1"。与 pi 的 2000 行默认一致。图片自动缩放重压缩、PDF 超 10 页强制分页（`pages`，每次最多 20 页）、ipynb 按 cell 返回。

### 2.5 Bash 大输出处理（与 write 无关但同款思路）

> "Output length: 30,000 characters by default. When a command produces more than that, Claude Code saves the full output to a file in the session directory and gives Claude the file path plus a short preview from the start."

即：大输出落盘 + 给路径，而不是发明分段协议。pi 的 bash 同样"truncated, full output is saved to a temp file"。

### 2.6 描述篇幅风格

- 工具表内一行描述均 ≤ 15 词。
- 会话内实际 description：Write 约 4 句；Read 约 10 条短 bullet（含格式、截断、PDF/图片行为）；总体每个工具 50-150 词，主体是行为陈述 + "when to use"指引，没有冗长协议说明。

---

## 3. 综合结论

### 3.1 写侧工具总数与模型可见保护机制

| | pi agent | Claude Code |
|---|---|---|
| 默认工具总数 | 4（read/write/edit/bash） | ~10 核心（文件相关 7） |
| 写侧文件工具 | **2**：write + edit | **2**：Write + Edit（+NotebookEdit 特化） |
| 大文件**写入**分段/草稿机制（模型可见） | **0 个** | **0 个** |
| 写入大小上限（模型可见） | 无 | 无 |
| read-before-write/edit 约束 | 无（edit 仅查文件存在） | 有，**工具层报错**（含磁盘新鲜度检查） |
| edit 多处修改 | edits[] 数组（一次调用多个不相交替换） | 单 old/new + replace_all；多处分多次调用 |
| 大文件**读取**保护 | offset/limit + 2000 行/50KB 截断 + 续读提示 | offset/limit + 2000 行默认 + PARTIAL view 提示 |

两者对"模型可能写超长内容"的态度完全一致：**不设防**。写不动是模型/上下文的问题，不是工具协议要解决的问题；唯一的窗口化都放在**读侧**。

### 3.2 runtime 层各自默默做的脏活

- **pi**：per-file 互斥队列（realpath 键控串行化）；edit 的 BOM/行尾保真 + Unicode fuzzy 兜底 + 旧参数形态垫片。**不做**原子写、不做备份。
- **Claude Code**：harness 维护"已读文件状态"用于 read-before-edit/write 与磁盘新鲜度校验；Bash 大输出自动落盘。原子写/备份在公开文档中**无记载**（不能声称有）。

共同点：所有脏活对模型零暴露——工具参数面里没有任何 lock/draft/chunk/transaction 概念。

### 3.3 对 Kivio 收敛方案的启示

Kivio 当前 `mcp/types.rs` 写侧暴露 11+ 个工具：`write_file`、`write_file_chunk`、`begin_file_write`、`append_file_write`、`finish_file_write`、`abort_file_write`、`edit_file`、`patch`、`create_dir`、`delete_path`、`move_path`、`copy_path`。对照两个基准：

1. **write_file + edit_file 双工具确实是行业共识形态**。pi 和 Claude Code 的通用写侧都恰好是这两个，且参数面几乎相同（path+content；path+old/new）。旧研究覆盖的 OpenCode 也是 write+edit 为主。
2. **整个 chunk/draft 五件套（write_file_chunk、begin/append/finish/abort_file_write）在两个基准里都不存在对应物**，删除有直接证据支撑。大写入失败的兜底是"模型重试/改用 edit"，不是分段协议。
3. `create_dir` 无需独立工具：两家 write 都自动建父目录（pi description 明示 "Automatically creates parent directories"）。
4. `delete_path` / `move_path` / `copy_path`：两家都没有，都交给 bash。Kivio 若无 bash 等价物可保留，但这是 Kivio 自己的取舍，不是基准形态。
5. 可借鉴的低成本细节：
   - read 截断时返回"Use offset=N to continue"式**可执行续读提示**（pi 原文模式）。
   - edit 失败消息带恢复动作（"provide more context to make it unique"）。
   - edit 的 replace_all（Claude Code）或 edits[] 数组（pi）二选一解决多处修改；pi 的 BOM/CRLF 保真值得在 Rust 实现里保留。
   - description 控制在 1-4 句行为陈述 + 一条 when-to-use，截断/限制规则写进 description 而不是另立协议。

## Caveats / Not Found

- pi 仓库 `badlogic/pi-mono` 已改组织为 `earendil-works/pi`，raw 链接经 redirect 仍可用；引用时建议同时记录新仓库名。
- Claude Code 是闭源 harness：read-before-edit 等契约来自官方文档明文 + 会话内工具描述，但其内部是否做原子写/备份**官方未记载**，本文未做断言。
- Claude Code Edit 工具在本研究会话中不可用（research agent 工具面无 Edit），其参数契约引自官方文档而非会话内 description 原文。
- pi 的 `grep/find/ls` 依赖按需下载的 ripgrep/fd 二进制（`utils/tools-manager.ts`），属于检索侧实现细节，与写侧收敛无关。

---

核实日期：2026-06-11（smart-search fetch 实抓源码与文档，命令与中间证据存于 /tmp/ss-evidence/）
