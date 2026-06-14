# Optimize chat image attachment thumbnails

## Goal

When users drag or attach images into the chat input, preview them as compact thumbnails like the reference screenshot instead of rendering each image at its original or large display size. This keeps the input composer usable when one or more images are attached.

## What I already know

* User provided screenshots showing current oversized image previews and desired compact thumbnail previews.
* Desired behavior applies to dragged/attached images in the chat input composer.
* The app is React 18 + TypeScript + Vite + TailwindCSS v4 with Tauri backend.

## Assumptions

* This is a frontend layout/presentation change only.
* Upload/drag data handling should remain unchanged.
* Existing remove attachment behavior should be preserved.

## Requirements

* Image attachment previews render as small thumbnails above the text input area.
* Multiple images lay out horizontally and wrap only when needed.
* Thumbnails use stable dimensions and `object-fit: cover` so large source images do not expand the composer.
* Each thumbnail has a compact remove button overlaid in the corner.
* Non-image attachments, if supported by the existing component, should not regress.

## Acceptance Criteria

* [ ] Dragging one large image displays a compact thumbnail instead of a large image.
* [ ] Dragging multiple images displays compact thumbnails in a row similar to the provided target screenshot.
* [ ] Removing an image still works.
* [ ] Composer controls and typed message text remain visible and aligned.
* [ ] `npm run lint` and `npm run typecheck` pass when practical.

## Out of Scope

* Changing attachment storage, upload, file size limits, or backend APIs.
* Adding image compression or thumbnail generation.
* Redesigning the whole chat composer.

## Technical Notes

* Inspect frontend chat composer files and existing attachment preview styles before editing.
