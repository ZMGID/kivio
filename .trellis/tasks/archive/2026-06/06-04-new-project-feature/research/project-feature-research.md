# Project Feature Research

## Sources

- ChatGPT Projects official help: https://help.openai.com/en/articles/10169521-projects-in-chatgpt
- Claude Projects official help: https://support.anthropic.com/en/articles/9517075-what-are-projects
- Perplexity Spaces official help: https://www.perplexity.ai/help-center/en/articles/10352961-what-are-spaces

Raw discovery/fetch outputs are saved in this task's `research/*.json` files.

## Comparable Product Patterns

### ChatGPT Projects

OpenAI describes projects as long-running workspaces that keep related chats, files, instructions, memory, context, and tools together. Creation happens from the sidebar, asks for a name, icon, and color, and then lets users add context through files, project instructions, app links, saved responses, and moved existing chats. Existing chats can be dragged or moved into a project, and moved chats inherit project instructions and file context. ChatGPT also distinguishes project memory from global memory and treats project-only memory as a boundary for long-running or sensitive work.

### Claude Projects

Anthropic frames projects as self-contained workspaces with their own chat histories and knowledge bases. Users can upload documents, text, code, and other files to project knowledge, define project instructions, and keep focused chats inside the project. Claude's paid tiers can automatically use RAG when project knowledge approaches context limits. Collaboration and sharing add permission levels, but the core single-user shape remains: project = chat history + knowledge + instructions.

### Perplexity Spaces

Perplexity Spaces are dedicated workspaces for research and tasks. They group threads by project/topic, apply custom AI instructions, support file search and sources, let users add existing threads to a Space, and include search/pin/manage affordances inside the Space. Spaces are private by default and can later support sharing/collaboration.

## Cross-Product Common Denominator

- Sidebar creation and navigation: a project appears as a persistent sidebar item, not just a transient filter.
- The first creation step is lightweight: name first; optional icon/color/details can follow.
- Project owns or groups chats/threads, including moving existing chats into it.
- New chats created from inside a project inherit the project association.
- The project view should have its own list and empty state, so an empty project is still useful.
- Project-level instructions are a major differentiator, but can be added after basic project existence.
- Project files/knowledge are valuable but require a larger retrieval/storage design.
- Sharing/collaboration is a later layer for multi-user products; Kivio is currently local desktop-first, so it should be out of MVP.

## Fit For Kivio

Kivio already has a partial project/folder concept:

- `Conversation.folder` and `ConversationListItem.folder` exist in Rust and TypeScript.
- `chat_get_conversations(offset, limit, folder?)` already supports folder filtering.
- `chat_create_conversation(provider, model, folder?)` already accepts a folder.
- The conversation context menu already contains "添加到项目", but derives available projects only from existing conversations with a non-empty folder.
- The sidebar has a disabled "新建项目" row.

That means the lowest-risk MVP is to turn the implicit `folder` label into a first-class project record while keeping conversation membership stored through the existing `folder` field for compatibility. This solves the biggest current gap: empty projects cannot exist because project names are currently inferred from conversations.

## Recommended MVP

Implement projects as lightweight local chat workspaces:

- Add a persisted project index, e.g. `{app_data_dir}/chat-projects/index.json`.
- Project fields: `id`, `name`, `created_at`, `updated_at`, optional `color`, optional `description`.
- Keep `Conversation.folder` as the membership key for now, probably storing the project name or a stable project id depending on migration choice.
- Enable sidebar "新建项目".
- Show a simple create-project dialog with a single required name and maybe optional color later.
- Add a Projects section in the sidebar above or below "聊天".
- Selecting a project filters the conversation list to that project and changes the main empty state to project-aware copy.
- "新建聊天" while inside a project creates the conversation with that project membership.
- Existing conversation menu can move a chat to any persisted project, not only projects inferred from existing chats.
- Empty projects remain visible and can be renamed/deleted.

## Out Of MVP

- Project file uploads / project knowledge RAG.
- Project-specific custom instructions injected into the system prompt.
- Cross-chat project memory / retrieval from other chats in the same project.
- Sharing/collaboration.
- Drag-and-drop chat-to-project.
- Icons, complex color picker, nested projects.
- Bulk move/import flows.

## Future Direction

After the MVP, project instructions are the most valuable next feature because they compose naturally with Kivio's existing `default_chat_system_prompt`, skill catalog, MCP/tool prompt injection, and per-conversation active skill. Project knowledge should come after a deliberate indexing/retrieval design, otherwise it risks becoming "uploaded files visible in UI but not actually useful to the model."
