# Protocol and Product Reference

## Tool Result Contract

OpenAI Chat Completions style tool calling and Anthropic Messages style tool use both follow the same high-level loop:

1. The model returns one or more tool calls.
2. The client/application executes those calls.
3. The client sends tool results back to the model.
4. The model continues or produces the final answer.

For Kivio this means `ask_user` should not be modeled as a normal user message. It should be modeled as a tool result associated with the tool call id that requested the clarification.

Relevant official docs:

- OpenAI function calling guide: https://developers.openai.com/api/docs/guides/function-calling
- Anthropic tool use guide: https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools

## MCP Elicitation

The Model Context Protocol draft has an "elicitation" concept where a server can request structured input from the user through the client. That is close to this feature semantically, though Kivio's first implementation should be internal to the Chat agent runtime rather than exposing remote MCP server elicitation.

Relevant official docs:

- MCP elicitation draft: https://modelcontextprotocol.io/specification/draft/client/elicitation

Useful alignment:

- Keep user-input collection client-owned.
- Use structured schemas/options instead of plain text whenever possible.
- Let the user decline/cancel.
- Treat the response as a controlled result back into the ongoing agent operation.

## Product Pattern

Cursor/Codex-style clarification cards are inline with the agent timeline. This matters because the clarification is part of the assistant's work trace, not a separate chat turn. It should appear where the model asked for it, then collapse into a completed answer state once submitted.

Claude Agent SDK also exposes explicit user-input mechanisms for agents, reinforcing that user questions are a runtime primitive, not only a prompt convention.

Relevant official docs:

- Claude Agent SDK user input: https://code.claude.com/docs/en/agent-sdk/user-input

## Design Implication for Kivio

Kivio should implement `ask_user` as:

- a model-facing tool,
- a backend async wait point,
- an inline frontend card,
- and a provider-compatible tool result.

This gives Kivio the desired "same run continues" behavior while preserving compatibility with current OpenAI-compatible and Anthropic-compatible replay paths.
