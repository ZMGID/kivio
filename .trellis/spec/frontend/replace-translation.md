# Replace Translation Contract

## Scenario: Photo inpainting and region-based translation

### 1. Scope / Trigger

Use this contract when changing screenshot replace translation across Rust commands, OCR geometry, offline models, Tauri events, or Canvas rendering. Erasure geometry and text layout geometry are separate: OCR leaf polygons constrain glyph extraction; cell/paragraph regions constrain translated text.

### 2. Signatures

- `replace_translation_pack_status() -> ReplaceTranslationPackStatus`
- `replace_translation_pack_install() -> OfflineModelInstallResult`
- Event: `replace-translation-pack-progress -> OfflineModelProgress`
- Event: `lens-replace-stream -> LensReplaceStreamPayload`
- Event payload V2: `{ version: 2, imageId, phase, groups, slots, cleanedImage?, warning?, error? }`
- Translation input: `{ "regions": [{ "id": "r0000", "text": "..." }] }`
- Translation output: `{ "translations": [{ "id": "r0000", "text": "..." }] }`

### 3. Contracts

- `lens-replace-stream.version` is exactly `2`; the event boundary rejects the removed regions-only payload.
- `lens-replace-stream.phase` is `ocr | processing | done | error`.
- Only `done` may carry `cleanedImage`; the large data URL is emitted once.
- `groups[]` owns translation context only: `id`, `leafIds`, `sourceText`, and `translated`.
- `slots[]` owns render geometry only: `id`, `groupId`, `leafIds`, `bounds`, `anchor`, `flow`, `kind`, `align`, `verticalAlign`, `sourceFontPx`, and `sourceColor`.
- `OcrLeaf`, `TranslationGroup`, `RenderSlot`, and `EraseMask` are independent contracts. A translation group may reference several OCR leaves and several render slots; it never owns an erase rectangle.
- `RenderSlot.flow` is `exact_line | paragraph_flow | cell_flow | scene_patch`. Without a trusted semantic provider, every OCR visual line is its own translation group and `exact_line` slot. `paragraph_flow` is allowed only after semantic routing explicitly proves paragraph membership; table cells retain one hard-bounded cell slot.
- The frontend decodes V2 once at the Tauri event boundary. Rendering code consumes typed groups/slots and must not cast or reconstruct raw payload fields.
- Canvas backing size equals `cleanedImage.naturalWidth/naturalHeight`; CSS maps it to the captured frame.
- Complete translation is mandatory. Each conservative `exact_line` group wraps and scales only inside its own slot. A future semantically proven paragraph group may flow through ordered slots, but fallback OCR heuristics must never redistribute translated text across source lines.
- Kivio masks use `255 = hole`, `0 = preserve`. The exported MI-GAN pipeline uses the inverse input convention: `0 = hole`, `255 = known`. Convert only at the MI-GAN boundary and composite original pixels outside the Kivio mask exactly.
- Production masks are glyph-derived inside each OCR polygon. Estimate the dominant coarse background cluster, select high-contrast foreground pixels, then dilate those pixels. Do not fill the entire OCR polygon unless foreground separation is impossible.
- Estimate the dominant background from the original OCR polygon only. Candidate glyph sampling may extend beyond the polygon primarily along the text's horizontal direction to catch punctuation clipped by OCR; vertical expansion stays limited to antialiasing tolerance so nearby rules and row separators are not erased.
- Flat UI/table/code-badge backgrounds use boundary-to-interior local color propagation over the glyph mask. Photos or gradients without a dominant local background cluster use MI-GAN.
- Deterministic UI repair samples the nearest original unmasked background in multiple directions before propagation. This keeps an inline-code badge's fill from being replaced by surrounding page white when glyphs touch the badge edge.
- Fallback document layout treats numbered/bulleted list starts as hard region boundaries, keeps command/path lines separate from surrounding prose, and renders paragraph regions left-aligned. A continuation line may join the most recent compatible paragraph, but must not jump back to an older list item merely because its indent is closer.
- Vertically spaced menu/list rows are separate regions; only tightly spaced wrapped lines join. Same-baseline controls with a large horizontal gap (for example a section title and a `New` button) must not merge.
- `exact_line` text starts at `slot.anchor.x/slot.anchor.y`; it is never vertically centered from translated content height. Paragraph slots are also top-anchored. Only an explicitly semantic cell/label policy may opt into centering.
- Two detected vertical page edges do not prove a table. Cell aggregation requires at least one internal vertical divider; otherwise use document/list fallback layout.
- A compact single-character ASCII OCR candidate at the far right of a much longer same-band text/code span is treated as a likely copy-icon false positive and excluded from both erasure and translation.
- Compact single-character OCR candidates adjacent to longer same-band text are treated as avatar/icon false positives and excluded from erasure/translation. A short ASCII button label prefixed by one non-ASCII icon-like glyph (for example OCR `日New`) is preserved as one control when character geometry cannot separate icon and label.
- Dominant background cluster selection must have a deterministic tie-break; never rely on randomized `HashMap` iteration order when two coarse color clusters have equal counts.
- Runtime/model files live under `{app_data_dir}/rapidocr-models`. MI-GAN is `inpainting/migan_pipeline_v2.onnx`, 28,079,181 bytes, SHA-256 `6f1f3530a1a2324b19752018ce756088b07973cda8d7d890034ace5c8a48c40b`.
- ONNX Runtime 1.24.x has no macOS Intel release. Intel uses 1.23.2 with ORT API 23; arm64/Windows may use 1.24.4 because API 23 remains compatible.
- Model download is explicit in Settings. Replace execution must return `replace_translation_pack_missing`; it must never start a silent download.
- Backend error codes remain stable machine-readable values, but the Lens container must map them through i18n before rendering. `ReplaceTranslateOverlay` receives only the final localized `statusLabel`; it must not accept or prioritize a raw backend `error` prop.
- The replace status overlay and the pre-capture hint share the macOS safe-area top offset: `calc(env(safe-area-inset-top, 0px) + 36px)`. Phase changes must not move the hint into the notch area.

### 4. Validation & Error Matrix

- Missing/invalid runtime, OCR, or MI-GAN file -> pack `ready=false`, report exact `missingBytes`.
- Interrupted body/timeout/5xx/checksum mismatch -> retain `.part`, retry up to three attempts; resume only after valid `206 Content-Range`.
- Server ignores `Range` -> truncate `.part` and restart from byte zero.
- Size or SHA-256 mismatch -> never rename into the final path.
- Unknown, empty, missing, or duplicate translation ID -> ignore that item and fall back only that region to `sourceText`.
- Missing/duplicate group ID, duplicate slot ID, unknown `slot.groupId`, non-positive slot bounds, or invalid enum -> reject the V2 event before updating Lens state.
- `done` with no groups or no slots -> reject the event; `ocr`, `processing`, and `error` may carry empty arrays.
- MI-GAN failure -> deterministic leaf-mask fill and warning; other regions remain usable.
- Invalid mask dimensions -> structured `invalid_mask` error before inference.
- `replace_translation_pack_missing` -> localized Settings download guidance in the overlay; never show the raw code to the user.
- No dominant background cluster inside an OCR polygon -> classify the image as complex and use MI-GAN.
- Numbered/bulleted line -> start a new fallback region even when its vertical gap and indent resemble the previous paragraph.
- Command/path line -> isolate it from prose; the following prose starts a fresh region.
- Horizontally clipped punctuation -> include it through horizontal candidate sampling and dilation; a nearby horizontal rule outside the small vertical tolerance remains unmasked.
- Compact far-right copy-icon false positive -> preserve original pixels and omit the span from translation regions.
- Spaced or tightly wrapped menu rows without semantic evidence -> emit separate translation groups and exact-line slots at their original `y` anchors.
- Conservative OCR fallback -> group count equals visual-line count (except proven table cells), and every slot draws from its OCR anchor.
- Semantically proven multi-line paragraph -> preserve each source line's slot anchor and flow the complete translation through slots in anchor order.
- Equal-size background color clusters -> choose with a stable tie-break so repeated runs produce the same mask.
- macOS display with a notch -> status remains below the safe-area inset before, during, and after replacement.

### 5. Good / Base / Bad Cases

- Good: table leaves from the same cell aggregate into one region; different columns remain separate; erasure still uses each leaf polygon.
- Good: a table cell keeps its borders and alternating row fill because only glyph pixels enter the mask.
- Good: list items 1–4 become separate left-aligned regions, while wrapped lines stay with their own item and a shell command remains in its code block.
- Good: inline-code badge backgrounds and copy icons remain intact after text removal.
- Good: repository rows keep independent translation groups at their original vertical anchors, and avatars remain untouched.
- Good: the translation prompt still receives all exact-line groups in one batch, providing page context without cross-line render redistribution.
- Base: a uniform UI background uses boundary-to-interior deterministic propagation and never changes pixels outside the glyph mask.
- Bad: using the aggregated cell rectangle as the inpainting mask destroys separators and background content.
- Bad: rasterizing the full OCR line polygon creates visible rectangular patches or makes MI-GAN rebuild table rules.
- Bad: searching every previous paragraph by closest indent attaches `run:` to item 1 instead of the immediately preceding item 2.
- Bad: expanding foreground sampling equally in every direction erases a horizontal rule below a heading.
- Bad: merging an entire menu into one tall paragraph and vertically centering it shifts every translated row away from its icon.
- Bad: retaining multiple slots but flowing one heuristic paragraph translation across them; translated words then move into different source-line positions.
- Bad: attaching translated text and bounds to one `ReplaceLayoutRegion`, because semantic grouping then silently becomes render geometry.
- Bad: accepting avatar OCR such as `®` or `Q` pulls region bounds left into the icon column and erases the avatar.
- Bad: `max_by_key(count)` over a randomized map makes identical images intermittently choose different background clusters.
- Bad: passing Kivio's `255=hole` mask directly to MI-GAN preserves the text instead of erasing it.
- Bad: rendering `error || statusLabel` leaks internal codes and bypasses the localized guidance.
- Bad: switching from the safe-area capture hint to a fixed `top-3` replacement status makes the message jump upward under the macOS notch.

### 6. Tests Required

- Downloader: interrupted body, accepted Range, ignored Range, checksum retry, completed-file skip, shared archive byte deduplication.
- Mask: rotated polygon constraint, glyph mask does not fill the OCR polygon, boundary clipping, dominant-background classification, local-gradient repair, outside-mask pixels unchanged.
- Mask: light inline-code fills stay unmasked, glyphs touching a badge edge repair to the badge color, horizontal OCR under-capture is covered, and a nearby rule below the text remains untouched.
- Layout: same-cell multiline merge, hard cross-cell boundary, multicolumn fallback, stable visual-order IDs.
- Layout: numbered list boundaries, command-line isolation, most-recent continuation ownership, paragraph left alignment, one-column page-border rejection, and copy-icon false-positive filtering.
- Layout: spaced menu-row splitting, tightly wrapped row merging, distant same-baseline control separation, adjacent avatar/icon filtering, and icon-prefixed short-control preservation.
- Geometry contract: conservative fallback emits one exact-line group/slot per visual line; group/slot anchors equal the source-line anchors.
- Translation: reordered IDs, unknown/missing/empty/duplicate IDs, full-text prompt rule.
- Frontend: CJK/Latin wrapping, explicit line breaks, binary font fitting, full-text safe scaling.
- Frontend: multi-slot flow preserves the complete translation and uses a shared safe scale instead of dropping the last-slot tail.
- Frontend positioning: exact-line normal and safe-scale paths both draw their first glyph at `slot.anchor.x/slot.anchor.y`, independent of translated text height.
- Protocol: V2 groups/slots decode, removed regions-only payload rejection, duplicate IDs, and unknown group references.
- Frontend: paragraphs return zero vertical offset while line/cell labels retain centered offsets.
- Frontend status: a missing-pack event renders the localized guidance and does not render `replace_translation_pack_missing`.
- Frontend position: the replacement status container keeps the shared safe-area top class in every phase.
- Ignored real E2E: shared ORT + RapidOCR + MI-GAN, hot path below 500ms, at least one masked pixel changes, every unmasked pixel is byte-identical.

### 7. Wrong vs Correct

#### Wrong

```ts
canvas.width = frame.width
ctx.fillRect(ocr.x, ocr.y, ocr.width, ocr.height)

// Wrong: heuristic grouping lets translation move between source lines.
const layout = layoutReplaceTextFlow(paragraphTranslation, heuristicLineSlots, fontPx, measure)

// Wrong: the translated block height changes the source y position.
const y = bounds.y + (bounds.height - translatedHeight) / 2

// Translation grouping incorrectly owns one render rectangle.
type ReplaceLayoutRegion = { id: string; translated: string; bounds: Bounds }

// Raw backend code overrides the localized message.
<ReplaceTranslateOverlay error={replaceError} statusLabel={localizedStatus} />

// Fixed top ignores a MacBook notch and changes position after capture.
<div className="absolute top-3">...</div>
```

```rust
// Wrong for the exported MI-GAN pipeline: 255 is interpreted as known.
let model_mask = kivio_hole_mask;

// Wrong for UI text: destroys the full OCR line background.
rasterize_dilated_polygon(mask, points);
```

#### Correct

```ts
canvas.width = cleanedImage.naturalWidth
canvas.height = cleanedImage.naturalHeight
// Conservative fallback: one visual line = one group + one exact-line slot.

type TranslationGroup = { id: string; translated: string; leafIds: string[] }
type RenderSlot = { id: string; groupId: string; bounds: Bounds; anchor: TextAnchor }

const payload = parseLensReplaceStreamPayload(rawEvent)
const x = slot.flow === 'exact_line' ? slot.anchor.x : alignedX(slot.bounds)
const y = slot.flow === 'exact_line' ? slot.anchor.y : alignedY(slot.bounds)

// The parent owns error-code localization; the overlay renders one display label.
<ReplaceTranslateOverlay statusLabel={localizedStatus} />

<div className="absolute top-[calc(env(safe-area-inset-top,0px)+36px)]">...</div>
```

```rust
let model_mask = kivio_hole_mask
    .iter()
    .map(|value| if *value == 0 { 255 } else { 0 })
    .collect::<Vec<_>>();

let glyphs = foreground_pixels(image, points);
rasterize_dilated_pixels(mask, &glyphs);
```

```rust
// Wrong: an older paragraph with a closer indent can steal a continuation.
let target = compatible_paragraphs.min_by_key(indent_distance);

// Correct: prefer the most recent compatible block, using indent only to
// break ties between columns at the same vertical position.
let target = compatible_paragraphs.max_by(last_line_y_then_closest_indent);

// Wrong: symmetric expansion can capture a rule above/below the OCR box.
let samples = polygon_samples(image, points, margin, margin);

// Correct: extend mainly along the text direction and keep vertical tolerance
// to antialiasing size.
let samples = polygon_samples(image, points, horizontal_margin, vertical_margin);
```

```ts
// Wrong: a 500px menu paragraph with 320px of text moves down by 90px.
const y = bounds.y + (bounds.height - contentHeight) / 2

// Correct: paragraphs preserve the source's first-line anchor.
const y = bounds.y + (kind === 'paragraph' ? 0 : (bounds.height - contentHeight) / 2)
```
