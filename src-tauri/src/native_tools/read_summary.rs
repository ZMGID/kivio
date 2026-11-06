/*
Adapted from oh-my-pi crates/pi-ast/src/summary.rs, commit 5dec65e.
Copyright (c) 2025 Mario Zechner
Copyright (c) 2025-2026 Can Bölük
Copyright (c) 2026 Stencil Labs, Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/

use std::collections::VecDeque;
use std::fmt::Write;
use std::path::Path;

use serde_json::Value;
use tree_sitter::{Node, Parser};

use super::{
    resolve_tool_read_path, workspace_display_path, NativeToolWorkspace, MAX_READ_FILE_BYTES,
    TOOL_OUTPUT_MAX_BYTES, TOOL_OUTPUT_MAX_LINES,
};

#[derive(Clone, Copy)]
enum Language {
    Rust,
    JavaScript,
    TypeScript,
    Tsx,
    Python,
}

impl Language {
    fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "py" | "pyi" => Some(Self::Python),
            _ => None,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }

    fn comment(self, kind: &str) -> bool {
        match self {
            Self::Rust => kind == "block_comment",
            _ => kind == "comment",
        }
    }

    fn elidable(self, kind: &str) -> bool {
        match self {
            Self::Rust => matches!(
                kind,
                "block"
                    | "array_expression"
                    | "tuple_expression"
                    | "struct_expression"
                    | "match_block"
                    | "raw_string_literal"
                    | "declaration_list"
                    | "field_declaration_list"
                    | "ordered_field_declaration_list"
                    | "enum_variant_list"
                    | "where_clause"
                    | "use_list"
                    | "macro_definition"
                    | "token_tree"
            ),
            Self::JavaScript | Self::TypeScript | Self::Tsx => matches!(
                kind,
                "statement_block"
                    | "function_body"
                    | "object"
                    | "array"
                    | "template_string"
                    | "class_body"
                    | "interface_body"
                    | "enum_body"
                    | "object_type"
                    | "switch_body"
                    | "jsx_element"
                    | "jsx_self_closing_element"
            ),
            Self::Python => matches!(
                kind,
                "block"
                    | "dictionary"
                    | "list"
                    | "set"
                    | "string"
                    | "tuple"
                    | "argument_list"
                    | "parameters"
                    | "parenthesized_expression"
                    | "list_comprehension"
                    | "set_comprehension"
                    | "dictionary_comprehension"
                    | "generator_expression"
                    | "import_from_statement"
                    | "subscript"
            ),
        }
    }

    fn groupable(self, kind: &str) -> bool {
        match self {
            Self::Rust => matches!(kind, "use_declaration" | "extern_crate_declaration"),
            Self::JavaScript | Self::TypeScript | Self::Tsx => kind == "import_statement",
            Self::Python => matches!(
                kind,
                "import_statement" | "import_from_statement" | "future_import_statement"
            ),
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
}

impl Span {
    fn lines(self) -> usize {
        self.end - self.start + 1
    }
}

struct Fold {
    span: Span,
    children: Vec<usize>,
}

#[derive(Default)]
struct Folds {
    nodes: Vec<Fold>,
    roots: Vec<usize>,
}

impl Folds {
    fn push(&mut self, parent: Option<usize>, span: Span) -> usize {
        let index = self.nodes.len();
        self.nodes.push(Fold {
            span,
            children: Vec::new(),
        });
        if let Some(parent) = parent {
            self.nodes[parent].children.push(index);
        } else {
            self.roots.push(index);
        }
        index
    }
}

fn add_group(
    folds: &mut Folds,
    parent: Option<usize>,
    first: Option<Node<'_>>,
    last: Option<Node<'_>>,
    count: usize,
) {
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    if count < 2
        || last
            .end_position()
            .row
            .saturating_sub(first.start_position().row)
            + 1
            < 4
    {
        return;
    }
    let end = first.end_position();
    let first_last_row = if end.column == 0 && end.row > 0 {
        end.row - 1
    } else {
        end.row
    };
    let start = first_last_row.saturating_add(1);
    if let Some(end) = last.start_position().row.checked_sub(1) {
        if start <= end {
            folds.push(parent, Span { start, end });
        }
    }
}

fn collect_folds(root: Node<'_>, language: Language) -> Folds {
    let mut folds = Folds::default();
    let mut pending = vec![(root, None)];
    while let Some((node, mut parent)) = pending.pop() {
        let start = node.start_position().row;
        let end = node.end_position().row;
        let length = end.saturating_sub(start) + 1;
        if language.comment(node.kind()) {
            if length >= 6 && start + 2 < end {
                folds.push(
                    parent,
                    Span {
                        start: start + 2,
                        end: end - 1,
                    },
                );
            }
            continue;
        }
        if language.elidable(node.kind()) && length >= 4 && start + 1 < end {
            parent = Some(folds.push(
                parent,
                Span {
                    start: start + 1,
                    end: end - 1,
                },
            ));
        }
        let mut first = None;
        let mut last = None;
        let mut count = 0;
        for index in 0..node.child_count() {
            let Some(child) = node.child(index as u32) else {
                continue;
            };
            if language.groupable(child.kind()) {
                if first.is_none() {
                    first = Some(child);
                }
                last = Some(child);
                count += 1;
            } else {
                add_group(&mut folds, parent, first, last, count);
                first = None;
                last = None;
                count = 0;
            }
        }
        add_group(&mut folds, parent, first, last, count);
        for index in (0..node.child_count()).rev() {
            if let Some(child) = node.child(index as u32) {
                pending.push((child, parent));
            }
        }
    }
    folds
}

fn select_folds(folds: &Folds, total: usize) -> Vec<Span> {
    let mut selected = vec![false; folds.nodes.len()];
    let mut queue = VecDeque::new();
    let mut hidden = 0usize;
    for &index in &folds.roots {
        selected[index] = true;
        hidden = hidden.saturating_add(folds.nodes[index].span.lines());
        queue.push_back(index);
    }
    let mut visible = total.saturating_sub(hidden);
    while let Some(index) = queue.pop_front() {
        if visible >= 50 {
            break;
        }
        if !selected[index] {
            continue;
        }
        let fold = &folds.nodes[index];
        let children = fold
            .children
            .iter()
            .map(|&child| folds.nodes[child].span.lines())
            .sum::<usize>();
        let next = visible.saturating_add(fold.span.lines().saturating_sub(children));
        if next > 100 {
            continue;
        }
        selected[index] = false;
        for &child in &fold.children {
            selected[child] = true;
            queue.push_back(child);
        }
        visible = next;
    }
    let mut spans: Vec<Span> = folds
        .nodes
        .iter()
        .zip(selected)
        .filter_map(|(fold, selected)| selected.then_some(fold.span))
        .collect();
    spans.sort_by_key(|span| (span.start, span.end));
    let mut merged: Vec<Span> = Vec::new();
    for span in spans {
        if let Some(previous) = merged.last_mut() {
            if span.start <= previous.end.saturating_add(1) {
                previous.end = previous.end.max(span.end);
                continue;
            }
        }
        merged.push(span);
    }
    merged
}

fn summarize(path: &str, language: Language, source: &str) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if !(100..=20_000).contains(&lines.len()) {
        return None;
    }
    let mut parser = Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    let tree = parser.parse(source, None)?;
    if tree.root_node().has_error() {
        return None;
    }
    let spans = select_folds(&collect_folds(tree.root_node(), language), lines.len());
    if spans.is_empty() {
        return None;
    }
    let mut output = format!("{path} — structural summary of {} lines\n", lines.len());
    let mut cursor = 0;
    for span in spans {
        if span.end >= lines.len() {
            return None;
        }
        for (index, line) in lines.iter().enumerate().take(span.start).skip(cursor) {
            writeln!(output, "{:>6}\t{line}", index + 1).ok()?;
        }
        writeln!(
            output,
            "       \t[Lines {}-{} omitted; use offset={} and limit={} to read them.]",
            span.start + 1,
            span.end + 1,
            span.start + 1,
            span.lines()
        )
        .ok()?;
        cursor = span.end + 1;
    }
    for (index, line) in lines.iter().enumerate().skip(cursor) {
        writeln!(output, "{:>6}\t{line}", index + 1).ok()?;
    }
    if output.len() > TOOL_OUTPUT_MAX_BYTES || output.lines().count() > TOOL_OUTPUT_MAX_LINES {
        return None;
    }
    Some(output.trim_end().to_string())
}

pub(crate) fn read_file_summary(
    workspace: &NativeToolWorkspace,
    arguments: &Value,
) -> Option<String> {
    if ["offset", "limit"]
        .iter()
        .any(|key| arguments.get(key).is_some_and(|value| !value.is_null()))
    {
        return None;
    }
    let path = arguments.get("path")?.as_str()?;
    let full = resolve_tool_read_path(workspace, path).ok()?;
    let language = Language::from_path(&full)?;
    let metadata = std::fs::metadata(&full).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_READ_FILE_BYTES {
        return None;
    }
    let source = std::fs::read_to_string(&full).ok()?;
    summarize(&workspace_display_path(workspace, &full), language, &source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rust_source() -> String {
        (0..24).map(|index| format!("fn item_{index}() {{\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let _ = a + b + c + d;\n}}\n")).collect()
    }

    #[test]
    fn reads_structural_summary_with_source_lines_and_explicit_read_fallbacks() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("test.rs");
        let source = rust_source();
        std::fs::write(&file, &source).unwrap();
        let workspace = NativeToolWorkspace::standalone();
        let path = file.to_str().unwrap();
        let output = read_file_summary(&workspace, &json!({"path":path})).unwrap();
        assert!(output.contains("structural summary of 168 lines"));
        assert!(output.contains("fn item_0()"));
        assert!(output.contains("omitted; use offset="));
        assert!(output.contains("fn item_23()"));
        let source_lines: Vec<_> = source.lines().collect();
        for line in output.lines().skip(1) {
            let Some((number, content)) = line.split_once('\t') else {
                panic!("Missing line number in summary: {line}");
            };
            if let Ok(number) = number.trim().parse::<usize>() {
                assert_eq!(content, source_lines[number - 1]);
            }
        }
        assert!(read_file_summary(&workspace, &json!({"path":path,"offset":1})).is_none());
        assert!(read_file_summary(&workspace, &json!({"path":path,"limit":2})).is_none());
        std::fs::write(&file, format!("{source}\nfn broken( {{")).unwrap();
        assert!(read_file_summary(&workspace, &json!({"path":path})).is_none());
    }

    #[test]
    fn parses_supported_languages_and_leaves_short_or_unknown_files_raw() {
        for (extension, function) in [
            ("rs", "fn item_{}() {\n let a = 1;\n let b = 2;\n let c = 3;\n let _ = a + b + c;\n}\n"),
            ("js", "function item_{}() {\n const a = 1;\n const b = 2;\n const c = 3;\n return a + b + c;\n}\n"),
            ("ts", "function item_{}(): number {\n const a = 1;\n const b = 2;\n const c = 3;\n return a + b + c;\n}\n"),
            ("tsx", "function item_{}(): number {\n const a = 1;\n const b = 2;\n const c = 3;\n return a + b + c;\n}\n"),
            ("py", "def item_{}():\n    a = 1\n    b = 2\n    c = 3\n    return a + b + c\n"),
        ] {
            let source: String = (0..24).map(|index| function.replace("{}", &index.to_string())).collect();
            let language = Language::from_path(Path::new(&format!("test.{extension}"))).unwrap();
            assert!(summarize("test", language, &source).is_some(), "{extension}");
        }
        assert!(Language::from_path(Path::new("test.txt")).is_none());
        assert!(summarize("test.rs", Language::Rust, "fn f() {}\n").is_none());
    }
}
