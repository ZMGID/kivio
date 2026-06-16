---
id: docx
name: docx
description: Read, summarize, revise, and analyze Word DOC/DOCX attachments saved by Kivio Chat.
recommended-tools:
  - read
  - run_python
---

# DOCX Skill

Use this skill when the user attaches or references a Word document (`.doc` or `.docx`) and asks to read, summarize, revise, extract, compare, translate, or answer questions about it.

## Inputs

Kivio stores each uploaded document as a safe local copy and includes its absolute path in the user message under `Kivio 安全副本路径`. Pass that safe copy path to `run_python` via `files`; Kivio mounts it inside the Pyodide filesystem for the run. Python code must use the mounted virtual paths in `KIVIO_INPUT_FILES`, not the host absolute path directly.

## Workflow

1. Identify the safe copy path from the attachment note.
2. For `.docx`, use `run_python` with `files=["Kivio 安全副本路径"]` to inspect the mounted zip package and extract text from `word/document.xml`.
3. Preserve paragraph order. If tables are needed, inspect `word/document.xml` table nodes or ask for a narrower extraction target.
4. For legacy `.doc`, explain that binary Word extraction may not be available in the sandbox and ask the user to convert to `.docx` if needed.
5. Do not invent content that was not extracted.

## Python Pattern

```python
from pathlib import Path
from zipfile import ZipFile
import xml.etree.ElementTree as ET

docx_path = Path(KIVIO_INPUT_FILES[0])
with ZipFile(docx_path) as zf:
    xml = zf.read("word/document.xml")

root = ET.fromstring(xml)
ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
paragraphs = []
for para in root.findall(".//w:p", ns):
    text = "".join(node.text or "" for node in para.findall(".//w:t", ns)).strip()
    if text:
        paragraphs.append(text)

print("\\n".join(paragraphs))
```

## Output

- For summaries: group by headings when possible.
- For edits: state what changed and provide replacement text or a concise revision plan.
- For extraction: keep document order and mark unclear formatting honestly.
