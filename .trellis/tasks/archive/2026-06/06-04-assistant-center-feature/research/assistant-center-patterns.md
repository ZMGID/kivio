# Assistant Center Patterns

## Sources

* OpenAI Help: Creating and editing GPTs — https://help.openai.com/en/articles/8554397-creating-and-editing-gpts
* Claude Help: What are projects? — https://support.anthropic.com/en/articles/9517075-what-are-projects
* Claude Help: How to create custom skills — https://support.anthropic.com/en/articles/12512198-creating-custom-skills
* Gemini: Gems overview — https://gemini.google/gp/overview/gems/?hl=en
* Gemini API: Building Managed Agents — https://ai.google.dev/gemini-api/docs/custom-agents
* Dify Docs: Agent — https://docs.dify.ai/en/use-dify/build/agent
* Poe Creator Platform: Prompt bots — https://creator.poe.com/docs/prompt-bots/how-to-create-a-prompt-bot

## Common Product Pattern

Comparable products treat a custom assistant as a reusable configuration object, not only as a chat folder:

* Identity: name, description, icon/avatar or visual marker.
* Behavior: system instructions / prompt / persona / workflow rules.
* Start affordance: conversation starters, greeting, or example prompts.
* Runtime choice: base model or recommended model, plus model parameters in some tools.
* Capabilities: built-in tools, external actions/connectors, MCP-like tools, code execution, web search, image generation, etc.
* Knowledge/context: uploaded files, project knowledge, workspace files, or skill resources.
* Lifecycle: create, edit, test/preview, duplicate, delete; some products add share/publish/version history.
* Invocation: open a new chat using the assistant, often by stable ID; some systems allow per-run overrides without changing the stored assistant.

## Source-Specific Notes

OpenAI GPTs:

* GPT configuration includes name, description, conversation starters, instructions, knowledge, recommended model, capabilities, apps, actions, testing, saving/updating, version history, duplicate/delete/share.
* Useful distinction: instructions define behavior, while knowledge files provide source material. This maps well to keeping Kivio assistant prompts separate from any future knowledge/file feature.

Claude Projects / Skills:

* Projects are self-contained workspaces with their own chat histories, knowledge bases, and project instructions.
* Skills are focused, repeatable workflows with name/description metadata, instructions, resources, scripts, and testing. Claude's guidance favors multiple focused skills over one large catch-all skill.
* This supports treating Kivio Skills as composable capabilities that an assistant can pin or recommend, rather than making the Assistant Center just a Skill directory.

Gemini Gems / Managed Agents:

* Gems are custom experts for repeatable tasks; the core promise is saving detailed prompt instructions and optionally providing files/resources.
* Managed Agents expose a developer-side shape close to Kivio's needs: agent ID, description, system instruction, tools, and environment/sources. They also support invoking by ID and overriding configuration per run.

Dify Agents:

* Agent setup emphasizes prompt, persona, output format, constraints, workflow steps, tool-use guidance, tools, knowledge base, model/tool-call capability, preview/debug, and publishing.
* Important for Kivio: tools should not only be enabled; prompts should describe when to use them.

Poe prompt bots:

* Prompt bots include visual identity, name, description, base bot/model, prompt, optional knowledge base, greeting, markdown, temperature, and final create/start flow.
* This validates adding greeting/conversation starters to Kivio Assistant Center even in an MVP.

## Mapping To Kivio

Kivio already has these building blocks:

* Chat conversations with `provider_id`, `model`, messages, `active_skill_id`, context state, attachments, and folders.
* Global Chat settings with stream/thinking/language/system prompt.
* Chat tools config with MCP servers, native tools, skill runtime, approval policy, max rounds, timeouts, disabled skills, and scan paths.
* Skill discovery and reading with metadata, recommended tools, source, files, and body.
* The Chat UI already has a disabled "中心" sidebar row and an embedded settings view.

Recommended first implementation:

* Add a local `ChatAssistant` profile domain under Chat storage, for example `conversations/assistants.json` or a sibling `assistants/` domain.
* Store assistant fields: id, name, description, avatar/icon marker, system prompt, provider/model override, skill id, tool policy/allowed tools, conversation starters, greeting, enabled/archived flag, timestamps.
* Store a snapshot on conversation creation: `assistant_id` plus `assistant_snapshot`. The snapshot prevents old chats from changing behavior when the user edits an assistant later.
* Compose system prompt as: global Chat base prompt (or default) + assistant instructions + runtime/tool/skill context. This keeps global app defaults and assistant-specific behavior separate.
* Keep Skill as a capability bound to an assistant, not the assistant itself.

## MVP Recommendation

MVP should be a practical local Assistant Center:

1. Sidebar row: rename "中心" to "助手中心" and open a new Chat subview.
2. Assistant list: searchable list of user assistants with name, description, model, skill/tool badges, last updated.
3. Built-in starter assistants: a few local presets such as General, Translator Helper, Screenshot Analyst, Code/Data Helper, Writing Polisher. Users can duplicate/edit them.
4. Editor: name, description, instructions, conversation starters, provider/model override, optional Skill, tool permission preset.
5. Actions: create, duplicate, edit, delete/archive, start chat with this assistant.
6. Chat binding: creating a chat from an assistant stores assistant id/snapshot and uses its prompt/model/skill/tool policy during `chat_send_message`.

## Defer

* Public marketplace, sharing, publishing, version history, organization permissions.
* Full RAG/knowledge base indexing.
* Multi-agent workflow builders.
* Complex assistant-specific MCP credentials.

## Open Decision

Whether MVP includes assistant-attached local files. Products consistently support knowledge/context, but Kivio does not yet have a RAG/indexing layer. A conservative MVP can defer this and add only conversation starters plus optional Skill resources. A slightly larger MVP can allow "reference files" but inject them only as normal attachments or plain prompt context, with clear size limits.

