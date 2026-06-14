For AI agents: a documentation index is available at the root level at /llms.txt and /llms-full.txt. Append /llms.txt to any URL for a page-level index, or .md for the markdown version of any page.

[Docs](/docs/quickstart)[API Reference](/docs/api/reference/overview)[Client SDKs](/docs/client-sdks/overview)[Agent SDK](/docs/agent-sdk/overview)[Cookbook](/docs/cookbook/get-started/quickstart)[Changelog](/docs/changelog)



[Overview](/docs/quickstart)[Multimodal](/docs/guides/overview/multimodal/overview)

# Image Generation

How to generate images with OpenRouter models

OpenRouter supports image generation via the [Chat Completions](/docs/api/api-reference/chat/send-chat-completion-request) and [Responses](/docs/api/reference/responses/overview) endpoints. You can find the supported models, their capabilities, and pricing by filtering our [model list by image output](https://openrouter.ai/models?output_modalities=image).

## Model Discovery

You can find image generation models in several ways:

### Via the API

Use the `output_modalities` query parameter on the [Models API](/docs/api-reference/models/get-models) to programmatically discover image generation models:

```


|  |  |
| --- | --- |
| $ | # List only image generation models |
| $ | curl "https://openrouter.ai/api/v1/models?output_modalities=image" |
| $ |
| $ | # List models that support both text and image output |
| $ | curl "https://openrouter.ai/api/v1/models?output_modalities=text,image" |


```

See [Models - Query Parameters](/docs/guides/overview/models#query-parameters) for the full list of supported modality values.

### On the Models Page

Visit the [Models page](/docs/models) and filter by output modalities to find models capable of image generation. Look for models that list `"image"` in their output modalities.

### In the Chatroom

When using the [Chatroom](/docs/chat), click the **Image** button to automatically filter and select models with image generation capabilities. If no image-capable model is active, you’ll be prompted to add one.

## API Usage

To generate images, send a request to the `/api/v1/chat/completions` endpoint with the `modalities` parameter. The value depends on the model’s capabilities:

* **Models that output both text and images** (e.g., Gemini): Use `modalities: ["image", "text"]`
* **Models that only output images** (e.g., Sourceful, Flux): Use `modalities: ["image"]`

### Basic Image Generation

```


|  |  |
| --- | --- |
| 1 | import { OpenRouter } from '@openrouter/sdk'; |
| 2 |
| 3 | const openRouter = new OpenRouter({ |
| 4 | apiKey: '{{API_KEY_REF}}', |
| 5 | }); |
| 6 |
| 7 | const result = await openRouter.chat.send({ |
| 8 | model: '{{MODEL}}', |
| 9 | messages: [ |
| 10 | { |
| 11 | role: 'user', |
| 12 | content: 'Generate a beautiful sunset over mountains', |
| 13 | }, |
| 14 | ], |
| 15 | modalities: ['image', 'text'], |
| 16 | stream: false, |
| 17 | }); |
| 18 |
| 19 | // The generated image will be in the assistant message |
| 20 | if (result.choices) { |
| 21 | const message = result.choices[0].message; |
| 22 | if (message.images) { |
| 23 | message.images.forEach((image, index) => { |
| 24 | const imageUrl = image.imageUrl.url; // Base64 data URL |
| 25 | console.log(`Generated image ${index + 1}: ${imageUrl.substring(0, 50)}...`); |
| 26 | }); |
| 27 | } |
| 28 | } |


```

### Image Configuration Options

Some image generation models support additional configuration through the `image_config` parameter.

#### Aspect Ratio

Set `image_config.aspect_ratio` to request specific aspect ratios for generated images.

**Supported aspect ratios:**

* `1:1` → 1024×1024 (default)
* `2:3` → 832×1248
* `3:2` → 1248×832
* `3:4` → 864×1184
* `4:3` → 1184×864
* `4:5` → 896×1152
* `5:4` → 1152×896
* `9:16` → 768×1344
* `16:9` → 1344×768
* `21:9` → 1536×672

**Extended aspect ratios** (supported by [`google/gemini-3.1-flash-image-preview`](/docs/models/google/gemini-3.1-flash-image-preview) only):

* `1:4` → Tall, narrow format ideal for scrolling carousels and vertical UI elements
* `4:1` → Wide, short format for hero banners and horizontal layouts
* `1:8` → Extra-tall format for notification headers and narrow vertical spaces
* `8:1` → Extra-wide format for wide-format banners and panoramic layouts

#### Image Size

Set `image_config.image_size` to control the resolution of generated images.

**Supported sizes:**

* `1K` → Standard resolution (default)
* `2K` → Higher resolution
* `4K` → Highest resolution
* `0.5K` → Lower resolution, optimized for efficiency (supported by [`google/gemini-3.1-flash-image-preview`](/docs/models/google/gemini-3.1-flash-image-preview) only)

You can combine both `aspect_ratio` and `image_size` in the same request:

```


|  |  |
| --- | --- |
| 1 | import requests |
| 2 | import json |
| 3 |
| 4 | url = "https://openrouter.ai/api/v1/chat/completions" |
| 5 | headers = { |
| 6 | "Authorization": f"Bearer {API_KEY_REF}", |
| 7 | "Content-Type": "application/json" |
| 8 | } |
| 9 |
| 10 | payload = { |
| 11 | "model": "{{MODEL}}", |
| 12 | "messages": [ |
| 13 | { |
| 14 | "role": "user", |
| 15 | "content": "Create a picture of a nano banana dish in a fancy restaurant with a Gemini theme" |
| 16 | } |
| 17 | ], |
| 18 | "modalities": ["image", "text"], |
| 19 | "image_config": { |
| 20 | "aspect_ratio": "16:9", |
| 21 | "image_size": "4K" |
| 22 | } |
| 23 | } |
| 24 |
| 25 | response = requests.post(url, headers=headers, json=payload) |
| 26 | result = response.json() |
| 27 |
| 28 | if result.get("choices"): |
| 29 | message = result["choices"][0]["message"] |
| 30 | if message.get("images"): |
| 31 | for image in message["images"]: |
| 32 | image_url = image["image_url"]["url"] |
| 33 | print(f"Generated image: {image_url[:50]}...") |


```

#### Strength (Recraft only)

Set `image_config.strength` to control how much the output image differs from the input image during image-to-image generation. This parameter only applies when input images are provided in `messages`. It is only supported by Recraft models.

* **Range**: `0.0` to `1.0`
* **Default**: `0.2`
* Lower values produce outputs closer to the input image; higher values allow more creative deviation.

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "strength": 0.7 |
| 4 | } |
| 5 | } |


```

#### Text Layout (Recraft V3 only)

Use `image_config.text_layout` to place text at specific positions on the generated image. Each entry specifies the text to render and a bounding box defined by four corner points in normalized coordinates (0 to 1). This parameter is only supported by Recraft V3 (`recraft/recraft-v3`) for both text-to-image and image-to-image requests. Recraft V4 and V4 Pro do not support `text_layout`.

Each text layout entry is an object with:

* `text` (required): The text string to render
* `bbox` (required): Array of 4 `[x, y]` coordinate pairs defining the bounding box corners (top-left, top-right, bottom-right, bottom-left), with values from 0 to 1

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "text_layout": [ |
| 4 | { |
| 5 | "text": "Hello", |
| 6 | "bbox": [[0.3, 0.45], [0.6, 0.45], [0.6, 0.55], [0.3, 0.55]] |
| 7 | }, |
| 8 | { |
| 9 | "text": "World", |
| 10 | "bbox": [[0.35, 0.6], [0.65, 0.6], [0.65, 0.7], [0.35, 0.7]] |
| 11 | } |
| 12 | ] |
| 13 | } |
| 14 | } |


```

#### Style (Recraft V3 only)

Use `image_config.style` to apply a specific artistic style to the generated image. This parameter is only supported by Recraft V3 (`recraft/recraft-v3`). Recraft V4 and V4 Pro do not support styles.

See the [full list of available styles](https://www.recraft.ai/docs/api-reference/styles#list-of-styles) in Recraft’s documentation. Note that vector styles are not supported.

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "style": "Photorealism" |
| 4 | } |
| 5 | } |


```

#### RGB Colors (Recraft only)

Use `image_config.rgb_colors` to specify a color palette that influences the generated image. Each color is a `[r, g, b]` array of three integers (0 to 255). This parameter is supported by Recraft models for both text-to-image and image-to-image requests.

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "rgb_colors": [ |
| 4 | [255, 0, 0], |
| 5 | [0, 128, 0] |
| 6 | ] |
| 7 | } |
| 8 | } |


```

#### Background RGB Color (Recraft only)

Use `image_config.background_rgb_color` to set a specific background color for the generated image. The value is a `[r, g, b]` array of three integers (0 to 255). This parameter is supported by Recraft models for both text-to-image and image-to-image requests.

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "background_rgb_color": [0, 0, 255] |
| 4 | } |
| 5 | } |


```

You can combine `rgb_colors` and `background_rgb_color` in the same request:

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "rgb_colors": [[255, 0, 0]], |
| 4 | "background_rgb_color": [255, 255, 255] |
| 5 | } |
| 6 | } |


```

#### Font Inputs (Sourceful only)

Use `image_config.font_inputs` to render custom text with specific fonts in generated images. The text you want to render must also be included in your prompt for best results. This parameter is only supported by Sourceful models (`sourceful/riverflow-v2-fast` and `sourceful/riverflow-v2-pro`).

Each font input is an object with:

* `font_url` (required): URL to the font file
* `text` (required): Text to render with the font

**Limits:**

* Maximum 2 font inputs per request
* Additional cost: $0.03 per font input

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "font_inputs": [ |
| 4 | { |
| 5 | "font_url": "https://example.com/fonts/custom-font.ttf", |
| 6 | "text": "Hello World" |
| 7 | } |
| 8 | ] |
| 9 | } |
| 10 | } |


```

**Tips for best results:**

* Include the text in your prompt along with details about font name, color, size, and position
* The `text` parameter should match exactly what’s in your prompt - avoid extra wording or quotation marks
* Use line breaks or double spaces to separate headlines and sub-headers when using the same font
* Works best with short, clear headlines and sub-headlines

#### Super Resolution References (Sourceful only)

Use `image_config.super_resolution_references` to enhance low-quality elements in your input image using high-quality reference images. The output image will match the size of your input image, so use larger input images for better results. This parameter is only supported by Sourceful models (`sourceful/riverflow-v2-fast` and `sourceful/riverflow-v2-pro`) when using image-to-image generation (i.e., when input images are provided in `messages`).

**Limits:**

* Maximum 4 reference URLs per request
* Only works with image-to-image requests (ignored when there are no images in `messages`)
* Additional cost: $0.20 per reference

**Example:**

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "image_config": { |
| 3 | "super_resolution_references": [ |
| 4 | "https://example.com/reference1.jpg", |
| 5 | "https://example.com/reference2.jpg" |
| 6 | ] |
| 7 | } |
| 8 | } |


```

**Tips for best results:**

* Supply an input image where the elements to enhance are present but low quality
* Use larger input images for better output quality (output matches input size)
* Use high-quality reference images that show what you want the enhanced elements to look like

### Streaming Image Generation

Image generation also works with streaming responses:

```


|  |  |
| --- | --- |
| 1 | import requests |
| 2 | import json |
| 3 |
| 4 | url = "https://openrouter.ai/api/v1/chat/completions" |
| 5 | headers = { |
| 6 | "Authorization": f"Bearer {API_KEY_REF}", |
| 7 | "Content-Type": "application/json" |
| 8 | } |
| 9 |
| 10 | payload = { |
| 11 | "model": "{{MODEL}}", |
| 12 | "messages": [ |
| 13 | { |
| 14 | "role": "user", |
| 15 | "content": "Create an image of a futuristic city" |
| 16 | } |
| 17 | ], |
| 18 | "modalities": ["image", "text"], |
| 19 | "stream": True |
| 20 | } |
| 21 |
| 22 | response = requests.post(url, headers=headers, json=payload, stream=True) |
| 23 |
| 24 | for line in response.iter_lines(): |
| 25 | if line: |
| 26 | line = line.decode('utf-8') |
| 27 | if line.startswith('data: '): |
| 28 | data = line[6:] |
| 29 | if data != '[DONE]': |
| 30 | try: |
| 31 | chunk = json.loads(data) |
| 32 | if chunk.get("choices"): |
| 33 | delta = chunk["choices"][0].get("delta", {}) |
| 34 | if delta.get("images"): |
| 35 | for image in delta["images"]: |
| 36 | print(f"Generated image: {image['image_url']['url'][:50]}...") |
| 37 | except json.JSONDecodeError: |
| 38 | continue |


```

## Response Format

When generating images, the assistant message includes an `images` field containing the generated images:

```


|  |  |
| --- | --- |
| 1 | { |
| 2 | "choices": [ |
| 3 | { |
| 4 | "message": { |
| 5 | "role": "assistant", |
| 6 | "content": "I've generated a beautiful sunset image for you.", |
| 7 | "images": [ |
| 8 | { |
| 9 | "type": "image_url", |
| 10 | "image_url": { |
| 11 | "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAA..." |
| 12 | } |
| 13 | } |
| 14 | ] |
| 15 | } |
| 16 | } |
| 17 | ] |
| 18 | } |


```

### Image Format

* **Format**: Images are returned as base64-encoded data URLs
* **Types**: Typically PNG format (`data:image/png;base64,`)
* **Multiple Images**: Some models can generate multiple images in a single response
* **Size**: Image dimensions vary by model capabilities

## Model Compatibility

Not all models support image generation. To use this feature:

1. **Check Output Modalities**: Ensure the model has `"image"` in its `output_modalities`
2. **Set Modalities Parameter**: Use `["image", "text"]` for models that output both, or `["image"]` for image-only models
3. **Use Compatible Models**: Examples include:
   * `google/gemini-3.1-flash-image-preview` (supports extended aspect ratios and 0.5K resolution)
   * `google/gemini-2.5-flash-image`
   * `black-forest-labs/flux.2-pro`
   * `black-forest-labs/flux.2-flex`
   * `sourceful/riverflow-v2-standard-preview`
   * Other models with image generation capabilities

## Best Practices

* **Clear Prompts**: Provide detailed descriptions for better image quality
* **Model Selection**: Choose models specifically designed for image generation
* **Error Handling**: Check for the `images` field in responses before processing
* **Rate Limits**: Image generation may have different rate limits than text generation
* **Storage**: Consider how you’ll handle and store the base64 image data

## Troubleshooting

**No images in response?**

* Verify the model supports image generation (`output_modalities` includes `"image"`)
* Ensure you’ve set the `modalities` parameter correctly: `["image", "text"]` for models that output both, or `["image"]` for image-only models
* Check that your prompt is requesting image generation

**Model not found?**

* Use the [Models page](/docs/models) to find available image generation models
* Filter by output modalities to see compatible models

[![Logo](https://files.buildwithfern.com/openrouter.docs.buildwithfern.com/docs/5a7e2b0bd58241d151e9e352d7a4f898df12c062576c0ce0184da76c3635c5d3/content/assets/logo.svg)![Logo](https://files.buildwithfern.com/openrouter.docs.buildwithfern.com/docs/6f95fbca823560084c5593ea2aa4073f00710020e6a78f8a3f54e835d97a8a0b/content/assets/logo-white.svg)](https://openrouter.ai/)

[Models](https://openrouter.ai/models)[Chat](https://openrouter.ai/chat)[Rankings](https://openrouter.ai/rankings)[Docs](/docs/api-reference/overview)
