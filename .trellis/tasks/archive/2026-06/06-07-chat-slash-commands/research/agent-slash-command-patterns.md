# Agent Slash Command Patterns

## Sources

- Claude Code commands reference: https://code.claude.com/docs/en/commands
- Claude Code SDK slash commands: https://code.claude.com/docs/en/agent-sdk/slash-commands
- OpenCode TUI commands: https://dev.opencode.ai/docs/tui/
- OpenCode custom commands: https://opencode.ai/docs/commands/
- Hermes slash command reference: https://github.com/NousResearch/hermes-agent/blob/main/website/docs/reference/slash-commands.md
- Hermes command registry source: https://github.com/NousResearch/hermes-agent/blob/main/hermes_cli/commands.py

## Common Patterns

1. Slash commands are a command surface, not just prompt shortcuts.
   - Built-ins usually mutate session/UI state: new, clear, compact, resume, model, permissions, help.
   - Prompt commands convert a template plus arguments into a normal model turn.
   - Skill commands mount a workflow/skill and then hand control to the model/runtime.

2. Discovery is part of the feature.
   - Claude Code exposes available commands in the session init payload and shows both built-ins and custom commands in the slash menu.
   - OpenCode opens command suggestions after typing `/`.
   - Hermes drives help, autocomplete, messaging menus, and dispatch from one registry.

3. The successful implementations separate definition from dispatch.
   - Hermes has `CommandDef` fields like name, description, category, aliases, args hint, subcommands, and surface gating.
   - OpenCode custom commands can come from config or markdown files, but still normalize to the same runtime shape.
   - Claude Code now favors skills as the richer modern command unit, while still supporting legacy markdown commands.

4. Custom commands need arguments and optional runtime overrides.
   - Claude Code supports markdown commands with frontmatter, positional arguments, `$ARGUMENTS`, shell output, and file references.
   - OpenCode supports markdown/config commands with `template`, `description`, optional `agent`, `model`, and `subtask`.
   - Hermes reserves user quick commands mostly for shell/alias style and points longer prompt workflows to skills.

5. Safety and scope matter.
   - Hermes distinguishes CLI-only and messaging-only commands, has admin/user allowlists for messaging, and prompts for destructive commands.
   - Claude Code distinguishes `/clear` and `/compact` because one drops context while the other preserves a summary.
   - OpenCode uses permissions to control tools invoked by model turns.

## Implications For Kivio

Kivio already has the right primitives:

- `ChatAssistant.quick_commands` with `slash`, `description`, `prompt`, `placeholder`, and `starter_text`.
- Built-in assistants already define commands such as `/整理`, `/改清楚`, `/翻译`, `/润色`, and `/语气`.
- Chat messages already carry `active_skill_id`, and the composer already receives enabled skills.
- Settings/tool approval UI already exists in `InputBar`.

The missing layer is a normalized command registry and composer picker. Kivio should not invent a separate one-off slash string parser only inside `InputBar`; that would duplicate assistant, skill, model, and session state later.

## Recommended Shape

Start with a frontend registry for MVP, then move to a backend-backed registry when commands need persistent side effects.

MVP command classes:

- Assistant quick commands: commands from the current assistant snapshot first, then enabled built-in/user assistants if global discovery is desired.
- Skill commands: expose enabled skills as `/<skill-name>` entries that set `activeSkillId` for the next turn and optionally prepend a short invocation hint.
- Built-in session commands: a very small set only if they can be implemented safely now, such as `/help` and `/new`; defer destructive commands like `/clear`, `/delete`, and tool-permission changes until there is a confirmation flow.

Data shape:

```ts
interface ChatSlashCommand {
  id: string
  slash: string
  title: string
  description: string
  category: 'assistant' | 'skill' | 'session'
  aliases?: string[]
  placeholder?: string
  argsHint?: string
  sourceId?: string
  execute: 'insert-template' | 'send-template' | 'select-skill' | 'builtin'
  promptTemplate?: string
}
```

Composer behavior:

- Open the menu only when the current token starts with `/` at the beginning of the composer or after whitespace.
- Filter case-insensitively and support Chinese commands.
- Arrow keys move selection; Enter applies selected command; Escape closes; Shift+Enter keeps newline behavior.
- Selecting an assistant quick command should replace the slash token with either `starter_text` or leave the remaining user text after the command.
- Sending should expand the command into a real user message, not show raw implementation details.

Prompt expansion:

- For `/翻译 这段话`, send a user-visible message like `这段话`, plus hidden/prefixed instruction from the command prompt: `把用户内容翻译成目标语言...`.
- For commands without args, insert the command's `starter_text` or update the placeholder so the user knows what to type.
- Keep raw slash commands out of persistent transcript unless product intentionally wants command provenance.

Backend evolution:

- Add `chat_get_slash_commands(conversation_id?)` only when commands need authoritative backend state.
- Add `chat_execute_slash_command` for built-ins that mutate state, such as context compression, new chat, model switching, or tool toggles.
- Keep prompt commands frontend-expanded until persistence/audit/permissions require backend dispatch.

## Risks

- Name collisions: built-ins, skills, assistants, and user commands can all want `/翻译` or `/help`. Use priority: built-in exact > current assistant exact > enabled skill exact > other assistant/global. Show source labels in the picker.
- Hidden prompt surprise: command expansion must be inspectable enough that users understand what will happen.
- IME interaction: the composer recently has IME Enter handling. Slash menu key handling must respect `nativeEvent.isComposing`.
- Destructive commands: avoid `/clear`, `/undo`, `/delete`, `/reset` in MVP unless there is a confirmation modal.
- Skill invocation: setting `activeSkillId` for one turn vs permanently switching conversation skill must be explicit.

## MVP Recommendation

Implement Phase 1 as "command picker + assistant quick command execution":

1. Add slash command suggestions to `InputBar`.
2. Pass current assistant quick commands and enabled skills from `Chat.tsx` into `InputBar`.
3. Support selecting assistant quick commands and expanding them into a prompt for the next send.
4. Expose enabled skills as one-turn skill selectors if the current `chat_send_message` contract can carry `activeSkillId`.
5. Add `/help` as local-only menu discovery, not a model turn.

Defer custom markdown/config command files until after the UX is proven. Kivio's Assistant Center already gives users a structured place to create quick commands, so filesystem command files would add power but also more product surface.
