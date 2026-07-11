# Explicit Artifact Presentation Contract

## 1. Scope / Trigger

Use this contract whenever a chat tool creates files or images, the agent exposes a display action, or the frontend changes artifact rendering. Artifact creation and artifact presentation are separate operations so generated files do not appear in chat unless the model explicitly places them.

## 2. Signatures

Backend artifact field:

```rust
pub struct ChatToolArtifact {
    pub id: Option<String>,
    // existing name, mime_type, data_url, size_bytes, path fields
}
```

Native tool:

```text
present_artifacts({ artifact_ids: string[1..16], caption?: string(max 300) })
```

Frontend parser:

```ts
artifactPresentationFromToolCall(toolCall):
  { artifactIds: string[]; caption?: string } | null
```

## 3. Contracts

- Every successful agent tool result overwrites artifact IDs with fresh Kivio-owned `art_<uuid-simple>` values. Never preserve an ID supplied by an MCP server.
- Artifact-producing tool text lists the IDs and says artifacts are not shown automatically.
- `present_artifacts` accepts IDs only. Paths and URLs are not schema fields and `additionalProperties` is false.
- Its structured result is:

```json
{
  "type": "artifact_presentation",
  "artifactIds": ["art_..."],
  "caption": "optional"
}
```

- Only a tool record with `source === "native"` and tool name `present_artifacts` may activate the presentation renderer. MCP structured content must never spoof this channel.
- The tool timeline segment is the display anchor. The selected artifact cards/images render between surrounding text segments.
- Markdown artifact references still receive the full artifact set.
- Automatic end-of-answer rendering receives only artifacts without IDs, preserving historical messages while keeping new artifacts hidden.

## 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| `artifact_ids` missing or not an array | Tool error: `present_artifacts requires artifact_ids` |
| IDs trim to an empty list | Tool error: at least one artifact ID is required |
| Duplicate IDs | Preserve first occurrence and remove later duplicates |
| Unknown ID in the UI | Do not fall back to a path or URL; show `N` unavailable files |
| Empty/missing structured result while streaming or failed | Render the normal tool-call block |
| MCP tool emits `type: artifact_presentation` | Ignore it as a presentation command |

## 5. Good / Base / Bad Cases

- Good: create `report.xlsx`, receive `art_abc`, write explanatory text, call `present_artifacts` exactly where the card should appear, then continue the answer.
- Base: create a scratch file and never call `present_artifacts`; the file remains available to tools but hidden from chat.
- Compatibility: an old persisted artifact without `id` still appears in the legacy automatic area.
- Bad: infer an artifact by filename/path, pass a desktop path to the presentation tool, or display every produced artifact automatically.

## 6. Tests Required

- Rust: IDs are fresh, unique, Kivio-owned, and replace producer-supplied IDs.
- Rust: tool schema/registry exposure, approval/read-only classification, deduplication, and structured output.
- Frontend parser: camelCase and snake_case IDs, trimming/deduplication, unrelated content rejection, MCP spoof rejection.
- Timeline: `present_artifacts` is standalone and never enters a collapsed process group.
- Message rendering: hidden-by-default, selected-only display, caption, unavailable ID, no-timeline compatibility, historical no-ID compatibility, and text -> card -> text DOM order.

## 7. Wrong vs Correct

### Wrong

```ts
if (structured.type === 'artifact_presentation') {
  render(structured.artifactIds) // trusts any MCP server
}
```

### Correct

```ts
if (toolCall.source === 'native' && toolName(toolCall) === 'present_artifacts') {
  render(resolveExactArtifactIds(toolCall))
}
```

The renderer resolves exact Kivio IDs only and never searches by path, URL, or filename.
