---
id: pdf
name: pdf
description: Read, extract, summarize, and analyze PDF attachments saved by Kivio Chat.
recommended-tools:
  - read
  - run_python
---

# PDF Skill

Use this skill when the user attaches or references a PDF and asks to read, summarize, extract, compare, translate, inspect, or answer questions about it.

## Inputs

Kivio stores each uploaded file as a safe local copy and includes its absolute path in the user message under `Kivio 安全副本路径`. Pass that safe copy path to `run_python` via `files`; Kivio mounts it inside the Pyodide filesystem for the run. Python code must use the mounted virtual paths in `KIVIO_INPUT_FILES`, not the host absolute path directly.

## Workflow

1. Identify the PDF safe copy path from the attachment note.
2. Use `run_python` with `files=["Kivio 安全副本路径"]` and `pypdf` to inspect or extract text from the PDF when possible.
3. If extraction fails or returns little text, explain that the PDF may be scanned/image-only and ask for OCR or use the screenshot/Lens flow when appropriate.
4. Ground answers in extracted content. Do not infer document contents from the filename alone.
5. For long PDFs, extract page-level text first, then summarize by section/page before answering.

## Python Pattern

Use Pyodide-friendly Python. Prefer `pypdf` for digitally generated PDFs. The `files` argument mounts attachment safe copies into Pyodide and exposes their virtual paths through `KIVIO_INPUT_FILES`. If text extraction returns little content, report that the PDF may be scanned/image-only and ask the user whether to use OCR or Lens.

```python
from pathlib import Path
from pypdf import PdfReader

pdf_path = Path(KIVIO_INPUT_FILES[0])
reader = PdfReader(str(pdf_path))
for index, page in enumerate(reader.pages, start=1):
    text = page.extract_text() or ""
    print(f"--- page {index} ---")
    print(text[:4000])
```

## Output

- For summaries: include the main points and mention any pages/sections you used when available.
- For extraction: preserve original order and tables/lists as much as practical.
- For analysis: quote only short snippets and explain conclusions separately.
