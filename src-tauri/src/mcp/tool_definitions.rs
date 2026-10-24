//! Shared model-facing tool definitions. Execution remains host-owned.
use crate::settings::ChatMcpServer;
use crate::mcp::types::{ChatToolDefinition, McpTool, PRESENT_ARTIFACTS_ARGUMENTS_MAX_CHARS};

pub fn tool_definition_from_mcp(server: &ChatMcpServer, tool: McpTool) -> ChatToolDefinition {
    let id = format!("mcp__{}__{}", server.id, tool.name);
    let sensitive = mcp_tool_requires_confirmation(&tool);
    ChatToolDefinition {
        id,
        name: tool.name.clone(),
        description: if tool.description.trim().is_empty() {
            format!("MCP tool {}", tool.name)
        } else {
            tool.description
        },
        source: "mcp".to_string(),
        server_id: Some(server.id.clone()),
        server_name: Some(server.name.clone()),
        input_schema: if tool.input_schema.is_null() {
            serde_json::json!({ "type": "object", "properties": {} })
        } else {
            tool.input_schema
        },
        sensitive,
        annotations: tool.annotations,
        output_schema: tool.output_schema,
    }
}

fn mcp_tool_requires_confirmation(_tool: &McpTool) -> bool {
    // MCP annotations are server-provided, untrusted hints. They may be displayed to the user,
    // but must never relax Kivio's approval boundary. A user-owned approval policy (for example
    // the explicit global `auto` policy) is the only way to bypass per-call MCP confirmation.
    true
}

pub fn native_web_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__web_search".to_string(),
        name: "web_search".to_string(),
        description: "Search the web for current facts and return source snippets.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_skill_activate_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "skill__activate".to_string(),
        name: "skill".to_string(),
        description: "Load the instructions for a listed skill according to the session's skill activation policy. Injects the skill's instructions and resources into the current conversation — the output may contain detailed workflow guidance plus references to scripts and files in the skill directory (use `read` for files and `bash` for scripts when those tools are declared in this request). The skill name must match one listed in available_skills. A previous activation is not proof that its instructions remain in context. Reload when needed after compaction or history changes, or when the skill files change. If the output is truncated, read the skill file in sections instead of repeatedly loading the same truncated output.".to_string(),
        source: "skill".to_string(),
        server_id: None,
        server_name: Some("Skill".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Skill name from available_skills"
                }
            },
            "required": ["name"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_skill_tools() -> Vec<ChatToolDefinition> {
    vec![native_skill_activate_tool()]
}

pub fn native_read_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__read_file".to_string(),
        name: "read".to_string(),
        description: "Read a local file or directory. For a file: text is line-numbered as `N<TAB>line` for easy reference; the numbers are display-only and are NOT part of the file — never include them in edit old_string. Output is capped at 2000 lines or 50KB, whichever is hit first, so a single read can never flood the context; when the cap or your own limit stops the read early, the result ENDS with a notice like `[Showing lines X-Y of Z. Use offset=N to continue.]` — if the part you need is not in this window, call read again with that offset before moving on. Optional offset/limit select a 1-based line window (the cap still applies on top); an offset past the last line is an error. For a directory path: returns its entries (folded in the former `ls` tool); offset/limit are ignored. Image files (png/jpg/webp/…) are also supported: the image is shown to you directly when your model has vision, otherwise it is described or OCR'd to text. To inspect several images at once, pass `paths` (up to 12). Images are read individually by default — always do that for analysis, QA, spelling, logos, or any per-image detail. A numbered contact sheet is used only for a first-pass overview of 6–12 similar images when the user asked to skim the set, or when you set overview=true for that skim; never for analysis. Re-read a single path for fine text. For PDF/Word/Excel, use the matching skill instead.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path or directory to read. Relative paths use the current working directory (project root or conversation workbench). Explicit absolute and ~/ paths can point outside it; normal OS permissions and tool approvals apply. Use the user's disk path directly with artifact_ids: [] and paths: []; no registration is required. When selecting artifact_ids or paths, omit path or use an empty string." },
                "artifact_ids": { "type": "array", "items": { "type": "string", "minLength": 1 }, "minItems": 0, "maxItems": 12, "description": "Exact art_ IDs already returned by tools in this conversation. For a disk path, omit artifact_ids or use []; never invent placeholder IDs. Nonempty artifact_ids take precedence over path/paths: those fields are ignored. For ID reads use path: empty string and paths: []. No filesystem search or registration is needed when the ID is known." },
                "paths": {
                    "type": "array",
                    "description": "Several image files to inspect in one call (png/jpg/webp/gif, max 12). Default is one image each. Omit or use [] when reading by path or artifact_ids; never add dummy or placeholder paths. Do not use this for text files.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 12
                },
                "overview": {
                    "type": "boolean",
                    "description": "If true, combine 6–12 images into one numbered contact sheet for a first-pass skim. Ignored when the user asked to analyze, verify, read text, or inspect each image — those always stay separate. Omit this (or false) unless you only need a coarse overview."
                },
                "offset": { "type": "integer", "description": "1-based start line (optional)" },
                "limit": { "type": "integer", "description": "Max lines to return (optional)" }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// Directory listing tool def. No longer part of the chat native tool set (chat's
/// `read` now lists directories directly), but still used by the Kivio Code surface,
/// which keeps a dedicated `ls` in its own tool list.
pub fn native_list_dir_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__list_dir".to_string(),
        name: "ls".to_string(),
        description: "List files and directories in a directory. Omit path (or pass \".\") to list the current working directory; relative paths resolve from it. Do not guess or invent an absolute path, and never translate/\"correct\" directory names — pass an absolute or ~/ path only when the user gave one or an earlier tool returned it.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory to list. Defaults to the current working directory; relative paths resolve from it. Do not fabricate an absolute path." },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "max_entries": { "type": "integer", "description": "Maximum entries to return, default 200, max 500" }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_search_files_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__search_files".to_string(),
        name: "grep".to_string(),
        description: "Search text in a file or under a directory. By default `query` is a literal substring; set regex=true to treat it as a regular expression. If you already know the exact file, pass that file path directly; for broader searches, pass a directory and use `glob` to narrow the scope. Relative paths use the current working directory (project root or conversation workbench); explicit absolute and ~/ paths can point outside it. Respects .gitignore and skips common dependency/build folders (node_modules, target, dist, …); no matches does not establish that a file is missing or inaccessible.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Text to search for (alias: pattern). Literal substring by default; a regular expression when regex=true." },
                "pattern": { "type": "string", "description": "Alias for query." },
                "path": { "type": "string", "description": "File or directory path; defaults to the current working directory. To search elsewhere, pass that absolute or ~/ path explicitly." },
                "regex": { "type": "boolean", "description": "Treat query as a regular expression, default false (literal substring)" },
                "case_sensitive": { "type": "boolean", "description": "Case-sensitive matching, default false" },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "glob": { "type": "string", "description": "Only search files whose relative path or name matches this glob. Supports brace expansion: \"*.{py,ts}\" matches both .py and .ts files. Examples: \"*.rs\", \"src/**/*.ts\", \"*.{py,ts,js}\"" },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "content: matching lines (default); files_with_matches: list of matching file paths; count: per-file match counts" },
                "context": { "type": "integer", "description": "Number of context lines to include before and after each match (content mode only), default 0" },
                "max_results": { "type": "integer", "description": "Maximum results to return, default 100, max 1000" }
            },
            "required": []
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_glob_files_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__glob_files".to_string(),
        name: "glob".to_string(),
        description: "Find files/directories under a directory by glob pattern such as \"src/**/*.tsx\". Relative paths use the current working directory (project root or conversation workbench); explicit absolute and ~/ paths can point outside it. Respects .gitignore; an empty result covers only the searched directory and filters. For a known file path, use read directly.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Relative glob pattern with *, ?, and ** support; put the search directory in path, not in an absolute pattern." },
                "path": { "type": "string", "description": "Directory path to search; defaults to the current working directory. To search elsewhere, pass that absolute or ~/ path explicitly." },
                "include_hidden": { "type": "boolean", "description": "Include dotfiles and hidden entries" },
                "max_results": { "type": "integer", "description": "Maximum paths to return, default 200, max 500" }
            },
            "required": ["pattern"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_write_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__write_file".to_string(),
        name: "write".to_string(),
        description: "Write a full text file: create it if missing, overwrite it if it exists. Use this for requested deliverables and necessary supporting files within the user's task scope; preserve unrelated files. For small changes to an existing file prefer edit. For a code block without saving, answer inline. Returns structured file mutation metadata including diff stats.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Target file path. Relative paths use the current working directory (project root or conversation workbench). Explicit absolute and ~/ paths can point outside it; normal OS permissions and tool approvals apply." },
                "content": { "type": "string", "description": "Full text content to save" }
            },
            "required": ["path", "content"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_edit_file_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__edit_file".to_string(),
        name: "edit".to_string(),
        description: "Edit a file with one or more exact text replacements in a single call. Each edit's old_string must match a unique, contiguous region of the current file (copy it from read output WITHOUT the leading line-number prefix); if a snippet appears more than once, extend it with surrounding context. Edits apply in order. Prefer this over write for changes to existing files. Returns structured file mutation metadata including diff stats.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Existing file path. Relative paths use the current working directory (project root or conversation workbench). Explicit absolute and ~/ paths can point outside it; normal OS permissions and tool approvals apply." },
                "edits": {
                    "type": "array",
                    "description": "One or more replacements, applied in order. Each old_string must occur exactly once in the current file.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string", "description": "Exact text to replace (unique in the file)" },
                            "new_string": { "type": "string", "description": "Replacement text" }
                        },
                        "required": ["old_string", "new_string"]
                    },
                    "minItems": 1
                }
            },
            "required": ["path", "edits"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_run_command_tool() -> ChatToolDefinition {
    let shell_hint = crate::native_tools::run_command_shell_hint();
    ChatToolDefinition {
        id: "native__run_command".to_string(),
        name: "bash".to_string(),
        description: format!("Run a host shell command (build, test, etc.).{shell_hint} Each call starts a fresh process; use cwd and command-local environment variables for the current call. With cwd omitted, start in the current working directory (project root or conversation workbench). An explicit absolute or ~/ cwd can point outside it and must be an existing directory; it is a startup directory, not a filesystem boundary. Normal OS permissions and tool approvals apply. Pass the startup directory through `cwd`, including paths with spaces, and put the operation itself in `command`. Foreground commands wait until they exit — omit timeout_ms unless you want the process killed at a deadline. Do not background finite jobs (builds, tests, image-generation batches); put parallel work inside one command. Long-running never-ending servers such as `npm run dev`, `npm run tauri dev`, and `vite` are started in the background automatically and return immediately with a job_id. Obey user constraints and obtain any required authorization for destructive, network, or environment-changing commands. A non-zero exit code is returned as a tool error with stdout/stderr. Host Python package installs require an explicit user request and allow_host_python_package_install=true."),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command" },
                "cwd": { "type": "string", "description": "Existing startup directory, defaults to the current working directory. Relative paths use that base; absolute and ~/ paths can point outside it. To run in another directory, pass it here, including when it contains spaces; do not use a leading `cd ... &&`." },
                "background": { "type": "boolean", "description": "Run in background and return a job_id immediately. Auto-enabled for never-ending dev servers. Do not use this for finite jobs that will exit." },
                "timeout_ms": { "type": "integer", "description": "Optional kill deadline in ms (max 600000). Omit to wait until the command exits. Timeout kills the process and returns partial output; it does not background the job." },
                "allow_host_python_package_install": { "type": "boolean", "description": "Only true when the user explicitly asked to modify the host Python environment; installs must use --user or a virtual environment." }
            },
            "required": ["command"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_bash_output_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__bash_output".to_string(),
        name: "bash_output".to_string(),
        description: "Inspect background commands started by bash (background:true / auto-detected never-ending servers). With a job_id: waits until that process exits or wait_ms elapses (default 30000; 0 = return immediately), then returns captured stdout/stderr since since_offset (default 0), status (running / exited with exit_code / killed / error), and next_offset; pass the returned next_offset as since_offset for the next read. New log lines do not end the wait. Finite jobs should use foreground bash (wait until exit) instead of this tool. With NO job_id: lists all background commands tracked in this app session (job_id, status, command, working directory, age) — background commands survive across turns until killed or the app exits. Always refresh once with bash_output before reporting a background command's result to the user.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "The job_id returned when the background command was started. Omit to list all tracked background jobs instead." },
                "since_offset": { "type": "integer", "description": "Byte offset to read from (use next_offset from the previous bash_output call for incremental reads; default 0)" },
                "wait_ms": { "type": "integer", "description": "Maximum time to wait for the process to exit (default 30000). Returns sooner if it exits. New output does not end the wait. 0 returns a snapshot immediately. Capped at the tool timeout." }
            }
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_kill_background_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__kill_background".to_string(),
        name: "kill_background".to_string(),
        description: "Stop a background command started by bash (background:true) by killing its process group. Pass the job_id. Use this to stop a dev server or other long-running background process when you are done with it; otherwise it keeps running until the app exits.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "The job_id of the background command to kill" }
            },
            "required": ["job_id"]
        }),
        sensitive: true,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_save_assistant_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__save_assistant".to_string(),
        name: "save_assistant".to_string(),
        description: "Create a new Kivio assistant (专家). ONLY available while building an assistant by chat, and only call it after you have restated the full config and the user confirmed. system_prompt is the assistant's own instructions (write it in the user's language). mcp_server_ids and skill_ids MUST be chosen from the available lists given in your builder instructions — use the exact ids, never invent them; leave a list empty if none apply. Returns the new assistant id.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Assistant display name (1-64 chars)" },
                "description": { "type": "string", "description": "Short one-line description (optional)" },
                "icon": { "type": "string", "description": "Optional short icon label/emoji" },
                "color": { "type": "string", "description": "Optional hex color like #6A8FBD" },
                "system_prompt": { "type": "string", "description": "The assistant's own system instructions" },
                "mcp_server_ids": { "type": "array", "items": { "type": "string" }, "description": "Allowed MCP server ids (exact ids from the available list; empty = none)" },
                "skill_ids": { "type": "array", "items": { "type": "string" }, "description": "Allowed skill ids (exact ids from the available list; empty = none)" }
            },
            "required": ["name", "system_prompt"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// `present_artifacts` only takes id/path strings. Models sometimes dump
/// base64 or file contents into the argument stream; abort past this.


pub fn native_present_artifacts_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__present_artifacts".to_string(),
        name: "present_artifacts".to_string(),
        description: "Prepare selected deliverables for final-answer references (default mode: prepare). For an existing local file, pass its path to obtain an art_ ID. In the final answer, place [label](artifact:art_ID) for a file or ![description](artifact:art_ID) for an image. Files that already have IDs can be referenced directly without calling this tool. Use mode preview only when the user explicitly requests an immediate preview or must inspect alternatives before you continue. Do not present internal QA screenshots, extracted frames, drafts, or failed attempts by default. Pass only a short JSON of identifiers: copy `art_…` ids from tool results into artifact_ids, or pass existing disk paths. Never both for the same file. Never invent a path for a generated file. Never put file contents, image bytes, base64, or data URLs in any field. Caption is optional plain text. Max 16 files. Unselected files stay hidden. Example: {\"artifact_ids\":[\"art_…\"]}".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "artifact_ids": {
                    "type": "array",
                    "description": "Copy `art_…` ids from tool results verbatim. For local files passed in paths, omit artifact_ids or use []; never supply dummy or placeholder IDs. Short strings only — not file names, paths, bytes, base64, or data URLs. Generated files have these ids and no usable path.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 16
                },
                "paths": {
                    "type": "array",
                    "description": "Existing disk paths for files you already read or wrote. Omit or use [] when selecting only artifact_ids. For a local file, use paths with artifact_ids: []. Do not use for generated artifacts (those use artifact_ids). Never file contents.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 16
                },
                "mode": {
                    "type": "string",
                    "enum": ["prepare", "preview"],
                    "description": "Defaults to prepare: register only selected final deliverables for inline references, without expanding them during work. preview immediately displays files for an explicit user preview/choice."
                },
                "caption": {
                    "type": "string",
                    "description": "Optional plain-text caption (max 300 chars). Not for file contents or base64.",
                    "maxLength": 300
                }
            },
            // 不用顶层 anyOf 表达“二选一必填”：grok/Vertex/Anthropic 都会拒或需剥离，
            // call_present_artifacts 在运行时校验，模型侧靠 description 提示即可。
            "additionalProperties": false
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_read_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_read".to_string(),
        name: "memory_read".to_string(),
        description: "Read Kivio Chat memory. L1 is online memory already injected when memory is enabled; use this mainly to inspect exact L1 text or read L2 long-term memory by exact query/heading.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Memory layer to read"
                },
                "query": {
                    "type": "string",
                    "description": "Optional exact text/heading query. Especially useful for L2."
                },
                "maxBytes": {
                    "type": "integer",
                    "description": "Maximum bytes returned to the model"
                }
            },
            "required": ["layer"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_modify_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_modify".to_string(),
        name: "memory_modify".to_string(),
        description: "Modify Kivio Chat memory. Use for adding, replacing, removing, or archiving durable user-approved memory. L1 is short online memory limited to 5000 bytes; L2 is long-term memory that is never auto-loaded.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Target memory layer"
                },
                "operation": {
                    "type": "string",
                    "enum": ["append", "replace", "remove", "archive"],
                    "description": "Modification operation"
                },
                "content": {
                    "type": "string",
                    "description": "New Markdown content for append/replace, or optional archived content"
                },
                "oldText": {
                    "type": "string",
                    "description": "Exact unique snippet for replace/remove/archive"
                },
                "heading": {
                    "type": "string",
                    "description": "Optional L2 heading to append/archive under"
                },
                "archiveMode": {
                    "type": "string",
                    "enum": ["move", "copy"],
                    "description": "archive only; default is move"
                }
            },
            "required": ["layer", "operation"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_memory_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__memory_search".to_string(),
        name: "memory_search".to_string(),
        description: "Search Kivio Chat long-term memory (L2) by keywords and get the most relevant entries back as heading + snippet. Prefer this over memory_read when you are not sure of the exact L2 heading: memory_read needs an exact heading/text match, while memory_search ranks sections by query-token overlap.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to search for in long-term memory"
                },
                "maxResults": {
                    "type": "integer",
                    "description": "Maximum number of matching entries to return (default 5, max 20)"
                },
                "layer": {
                    "type": "string",
                    "enum": ["l1", "l2"],
                    "description": "Memory layer to search; defaults to l2 (L1 is small and already injected)"
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn mixer_generate_image_tool() -> ChatToolDefinition {
    mixer_generate_image_tool_for(None)
}

/// Mixer 生图工具。`model` 写进 description，因为发给上游的只有 name/description/parameters，
/// `server_id` 只给 UI，模型看不见。
pub fn mixer_generate_image_tool_for(model: Option<&str>) -> ChatToolDefinition {
    ChatToolDefinition {
        id: "mixer__generate_image".to_string(),
        name: "mixer_generate_image".to_string(),
        description: mixer_generate_image_description(model),
        source: "mixer".to_string(),
        server_id: None,
        server_name: Some("Mixer".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "What the image should look like, or how to change the reference"
                },
                "size": {
                    "type": "string",
                    "description": "Only if the user asked. 512, 1K, 2K, 4K, or WIDTHxHEIGHT such as 1536x864."
                },
                "aspect_ratio": {
                    "type": "string",
                    "enum": [
                        "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "4:5", "5:4", "21:9",
                        "2:1", "1:2", "1:4", "4:1"
                    ],
                    "description": "Only if the user asked for a shape. Omit to let the model choose."
                },
                "quality": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "description": "Only if the user asked"
                },
                "n": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4,
                    "description": "How many images. Default 1"
                },
                "paths": {
                    "type": "array",
                    "description": "Local image files to edit. Omit or use [] when no disk file is needed; never invent placeholder paths.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 4
                },
                "artifact_ids": {
                    "type": "array",
                    "description": "Exact art_ IDs from earlier generate/edit results. Omit or use [] when no prior artifact is needed; never invent placeholder IDs.",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 0,
                    "maxItems": 4
                }
            },
            "required": ["prompt"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

fn mixer_generate_image_description(model: Option<&str>) -> String {
    let model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("the Mixer image model from Settings");
    format!(
        "Generate or edit an image with {model}. The image model is fixed; do not pass a model name.\n\
         New image: {{\"prompt\":\"a fox in snow\"}}\n\
         Widescreen 2K: {{\"prompt\":\"city skyline\",\"aspect_ratio\":\"16:9\",\"size\":\"2K\"}}\n\
         4K: {{\"prompt\":\"poster\",\"aspect_ratio\":\"16:9\",\"size\":\"4K\"}}\n\
         Custom pixels: {{\"prompt\":\"poster\",\"size\":\"1536x864\"}}\n\
         Edit a file: {{\"prompt\":\"make it night\",\"paths\":[\"C:/ref.png\"]}}\n\
         Edit a prior result: {{\"prompt\":\"add a title\",\"artifact_ids\":[\"art_...\"]}}\n\
         User attached images this turn: omit paths and artifact_ids.\n\
         Omit size and aspect_ratio unless the user asked. quality=low|medium|high only if asked. n=1-4."
    )
}

pub fn native_web_fetch_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__web_fetch".to_string(),
        name: "web_fetch".to_string(),
        description: "Fetch readable text from an HTTPS URL. Uses the configured fetch provider's extract API when available (independent of the search provider: Tavily, Exa, Ollama, TinyFish, Serper, Kimi); otherwise fetches the page directly (HTML stripped to plain text, with a hosted reader fallback).".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "HTTPS URL" },
                "reader_fallback": {
                    "type": "boolean",
                    "description": "Whether to try a hosted reader fallback when direct fetch fails or returns too little readable text. Defaults to true."
                }
            },
            "required": ["url"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

fn native_automation_tool(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
    sensitive: bool,
) -> ChatToolDefinition {
    ChatToolDefinition {
        id: format!("native__{name}"),
        name: name.to_string(),
        description: description.to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema,
        sensitive,
        annotations: None,
        output_schema: None,
    }
}

pub fn native_automation_list_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_list",
        "List Kivio automations (id, name, enabled, trigger type). Call this once to avoid duplicates, then create with automation_upsert. Do not discover node types by trial.",
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        false,
    )
}

pub fn native_automation_get_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_get",
        "Get the full graph JSON for one automation by id (nodes, edges, viewport).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Automation id from automation_list." }
            },
            "required": ["id"]
        }),
        false,
    )
}

pub fn native_automation_upsert_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_upsert",
        "Create or replace a Kivio automation graph in ONE call. Only when the user explicitly asks for an automation/workflow; for 定时任务 or reminders use schedule_create instead. Omit id to create. Omit node positions to auto-layout. Activate the `automation` skill first for node types and examples. Do not glob the repo and do not dry_run-probe types. Validation errors return allowedNodeTypes and schemaHint.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "automation": {
                    "type": "object",
                    "description": "Complete graph: {name, enabled, nodes:[{id, type, data}], edges:[{id, source, target, sourceHandle?, targetHandle?}]}. schemaVersion is set by Kivio. Allowed node types are in the `automation` skill."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Validate a COMPLETE candidate graph without saving. Never use this to discover node types."
                }
            },
            "required": ["automation"]
        }),
        true,
    )
}

pub fn native_automation_set_enabled_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_set_enabled",
        "Enable or disable an automation. Enabled graphs honor schedule and hotkey triggers.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "enabled": { "type": "boolean" }
            },
            "required": ["id", "enabled"]
        }),
        true,
    )
}

pub fn native_automation_run_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_run",
        "Run an automation and wait for it to finish. Optional input is exposed to later nodes as {{output}} / {{json.input.*}}. Returns per-node status plus the last step's output. If it is still running after timeout_seconds, returns runId so you can poll with automation_runs.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Automation id to run." },
                "input": {
                    "description": "Optional payload: a string, or { text, json }. Downstream nodes read it via {{output}} and {{json.input.*}}."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "How long to wait (default 600, max 1800). On timeout the run continues in the background."
                }
            },
            "required": ["id"]
        }),
        true,
    )
}

pub fn native_automation_runs_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_runs",
        "List recent runs for an automation, or fetch one run's per-node details when run_id is set.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "run_id": { "type": "string", "description": "Optional run id from automation_run." }
            },
            "required": ["id"]
        }),
        false,
    )
}

pub fn native_automation_delete_tool() -> ChatToolDefinition {
    native_automation_tool(
        "automation_delete",
        "Delete an automation and its run history. This cannot be undone.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" }
            },
            "required": ["id"]
        }),
        true,
    )
}

/// `knowledge_search` — retrieve passages from the user's knowledge bases
/// (RAG). Read-only. Returns passages each tagged with a `[n]` citation marker.
pub fn native_knowledge_search_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__knowledge_search".to_string(),
        name: "knowledge_search".to_string(),
        description: "Search the user's knowledge base(s) for passages relevant to a query and return them with citation markers. Use this whenever the question may be answered by the user's uploaded documents. Each returned passage is prefixed with a [n] marker and its source; when you use a passage, cite it inline as [n]. If no relevant passage is returned, say you don't have that information in the knowledge base instead of guessing. This searches only the libraries the user attached to the current conversation; if none are attached it returns nothing.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "What to look for. Use a focused natural-language query."
                },
                "kb_ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: restrict the search to these knowledge base ids. Defaults to the libraries attached to the current conversation; if none are attached, the search returns nothing."
                },
                "top_k": {
                    "type": "integer",
                    "description": "Optional: number of passages to return (default 5)."
                }
            },
            "required": ["query"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

/// `advisor` — consult a stronger model (executor-advisor pattern). The main
/// loop model calls this on demand when it is stuck, repeatedly failing, or
/// facing a significant design decision; the advisor model configured under
/// Mixer answers in a single shot (no tools, no recursion). Read-only.
pub fn native_advisor_tool() -> ChatToolDefinition {
    ChatToolDefinition {
        id: "native__advisor".to_string(),
        name: "advisor".to_string(),
        description: "Consult a stronger advisor model for guidance. Use this when you are stuck, have failed the same approach repeatedly, or face a significant design/architecture decision — NOT for routine steps you can handle yourself. Pass the specific question and enough context (relevant code, what you already tried, constraints). Returns the advisor's diagnosis and direction; you remain responsible for carrying it out.".to_string(),
        source: "native".to_string(),
        server_id: None,
        server_name: Some("Kivio".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The specific question or problem you want advice on."
                },
                "context": {
                    "type": "string",
                    "description": "Optional but recommended: relevant code, constraints, error messages, and what you have already tried. The advisor sees only what you pass here."
                }
            },
            "required": ["question"]
        }),
        sensitive: false,
        annotations: None,
        output_schema: None,
    }
}

pub fn looks_sensitive_tool(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "write", "delete", "remove", "exec", "shell", "command", "run", "update", "patch", "move",
        "rename", "create", "save", "upload", "publish", "replace", "modify", "edit", "insert",
        "drop", "truncate", "grant", "revoke", "deploy", "apply",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}
