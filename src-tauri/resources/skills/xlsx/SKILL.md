---
id: xlsx
name: xlsx
description: Read, summarize, calculate from, and analyze Excel/CSV/TSV spreadsheet attachments saved by Kivio Chat.
recommended-tools:
  - read
  - run_python
---

# XLSX Skill

Use this skill when the user attaches or references a spreadsheet (`.xls`, `.xlsx`, `.xlsm`, `.csv`, or `.tsv`) and asks to inspect, summarize, calculate, clean, compare, chart, or answer questions from it.

## Inputs

Kivio stores each uploaded spreadsheet as a safe local copy and includes its absolute path in the user message under `Kivio 安全副本路径`. Pass that safe copy path to `run_python` via `files`; Kivio mounts it inside the Pyodide filesystem for the run. Python code must use the mounted virtual paths in `KIVIO_INPUT_FILES`, not the host absolute path directly.

## Workflow

1. Identify the safe copy path from the attachment note.
2. For `.csv` / `.tsv`, use `read` for a small text preview or `run_python` with `files=["Kivio 安全副本路径"]` and `pandas` for analysis.
3. For `.xlsx` / `.xlsm`, use `run_python` with `files=["Kivio 安全副本路径"]` and `pandas.read_excel` when available.
4. For legacy `.xls`, try `pandas.read_excel`; if the engine is unavailable, explain the limitation and ask for `.xlsx` or `.csv`.
5. Inspect sheet names, columns, row counts, missing values, and representative rows before answering.
6. Do not invent numbers. Run calculations explicitly.

## Python Patterns

Preview workbook:

```python
from pathlib import Path
import pandas as pd

path = Path(KIVIO_INPUT_FILES[0])
book = pd.ExcelFile(path)
print(book.sheet_names)
for sheet in book.sheet_names:
    df = pd.read_excel(path, sheet_name=sheet)
    print("\\nSHEET:", sheet)
    print("shape:", df.shape)
    print(df.head().to_string())
```

Preview CSV/TSV:

```python
from pathlib import Path
import pandas as pd

path = Path(KIVIO_INPUT_FILES[0])
sep = "\\t" if path.suffix.lower() == ".tsv" else ","
df = pd.read_csv(path, sep=sep)
print(df.shape)
print(df.head().to_string())
```

## Output

- For analysis: include calculation assumptions and key columns used.
- For summaries: mention sheet names and row/column counts.
- For charts: save generated images to relative filenames such as `chart.png` so Kivio can capture the artifact.
