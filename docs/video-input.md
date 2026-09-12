# Video input

Kivio's built-in Agent and Chat runtimes accept local video attachments through
the existing file picker, drag-and-drop and file paste workflows. Video is a
separate model capability from image input.

- Use an API-key provider with OpenAI Chat (Kimi) or Gemini native format.
- Enable **Video Input** in the model details. Known supported Gemini and Kimi
  models have defaults; fetching the Kimi model catalog also imports
  `supports_video_in`. Existing explicit capability overrides take precedence.
- Send a video and a question. The video card opens the local file in its default
  application. External CLI agents continue receiving file paths rather than
  native video content.
- Supported extensions: mp4, mpeg, mpg, mov, avi, flv, webm, wmv, 3gp, 3gpp.
- Active context may contain at most 14 MiB of raw video; the final JSON request
  may not exceed 20 MiB. Base64 expansion, images, prompts and tools all count
  toward the request limit. Trim videos or clear context when needed.
- Follow-up questions and regeneration reload videos from conversation storage.
  Cleared or summarized history is excluded. Missing files produce an error.
- Unsupported models and protocols must fail explicitly, without sending video
  bytes as image or text content.
- Persisted transcripts externalize video bytes; estimates and summaries do not
  treat base64 as text tokens. Request debugging omits video bytes.

This version sends inline video data. Files API uploads, remote URLs, YouTube
links and user-controlled frame sampling are not part of this implementation.

Protocol references:

- [Kimi Chat Completions](https://platform.kimi.ai/docs/api/chat):
  `content[].video_url.url = data:video/mp4;base64,...`.
- [Kimi model capabilities](https://platform.kimi.ai/docs/api/list-models):
  `supports_video_in`.
- [Gemini video understanding](https://ai.google.dev/gemini-api/docs/generate-content/video-understanding):
  `contents[].parts[].inlineData` with a video MIME type.

The local limits are deliberately conservative application limits, not claims
about the maximum size accepted by every provider or relay.
