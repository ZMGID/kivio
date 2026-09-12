# Video input

Kivio's built-in Agent and Chat runtimes accept local video attachments through
the existing file picker, drag-and-drop and file paste workflows. Video is a
separate model capability from image input.

- Video eligibility is determined by the selected model's **Video Input** capability,
  independent of provider identity, endpoint, API format, or API-key/OAuth authentication.
  The current OpenAI Chat and Gemini adapters serialize video data; Responses and
  Anthropic adapters report an unimplemented video transport explicitly.
- Enable **Video Input** in the model details. Known supported Gemini and Kimi
  models have defaults; fetching the Kimi model catalog also imports
  `supports_video_in`. Existing explicit capability overrides take precedence.
- **Settings > Mixer > Video analysis model** supplies video understanding when
  the main model lacks video input. A capable main model always receives videos
  directly, even with an explicit auxiliary selection. Auto picks an enabled,
  credentialed model with video input; an explicit selection takes priority over
  auto candidates and is validated before use. With no available model, the chat
  reports how to configure one instead of silently dropping the video.
- The auxiliary model analyzes all videos in the active context in relation to
  the user questions, then the main model answers from its observations. The
  chat shows a video analysis step; usage logs classify it as video analysis.
  Retry and follow-up requests re-analyze the active videos, so they incur another
  auxiliary call. Cleared or summarized videos are not replayed. Cancellation or
  analysis failure stops the reply. Original attachments remain unchanged.
- Send a video and a question. The video card opens the local file in its default
  application. External CLI agents continue receiving file paths rather than
  native video content.
- Supported extensions: mp4, mpeg, mpg, mov, avi, flv, webm, wmv, 3gp, 3gpp.
- Active context may contain at most 14 MiB of raw video; the final JSON request
  may not exceed 20 MB (20,000,000 bytes). Base64 expansion, images, prompts and tools all count
  toward the request limit. Trim videos or clear context when needed.
- Follow-up questions and regeneration reload videos from conversation storage.
  Cleared or summarized history is excluded. Missing files produce an error.
- Unsupported models and unimplemented video transports must fail explicitly, without sending video
  bytes as image or text content.
- Persisted transcripts externalize video bytes; estimates and summaries do not
  treat base64 as text tokens. Request debugging omits video bytes.

This version sends inline video data. Files API uploads, remote URLs, YouTube
links and user-controlled frame sampling are not part of this implementation.

Protocol references:

- Catalog defaults include Kimi K2.7 Code / Code HighSpeed. The official
  [K2.7 Code model card](https://huggingface.co/moonshotai/Kimi-K2.7-Code)
  documents video input. These defaults also apply when a relay lists IDs only.
- The legacy `gemini-3-pro-preview` ID retains video metadata for imported
  configurations, as documented in its
  [model card](https://ai.google.dev/gemini-api/docs/models/gemini-3-pro-preview).
  Google has retired that endpoint; metadata recognition does not restore it.
  Explicit user overrides and advertised provider capabilities still take priority.

- [Kimi Chat Completions](https://platform.kimi.ai/docs/api/chat):
  `content[].video_url.url = data:video/mp4;base64,...`.
- [Kimi model capabilities](https://platform.kimi.ai/docs/api/list-models):
  `supports_video_in`.
- [Gemini video understanding](https://ai.google.dev/gemini-api/docs/generate-content/video-understanding):
  `contents[].parts[].inlineData` with a video MIME type.

The local limits are deliberately conservative application limits, not claims
about the maximum size accepted by every provider or relay.

The Gemini adapter sends MOV as `video/quicktime`; stored attachments and Chat
data URLs keep their existing MIME. See the [official API audit](research/video-input-api-audit-2026-09-12.md)
for other suppliers, protocol differences and unsupported transport paths.
