use zeron_markdown::{Block, IncrementalParser, mend};

/// Adapts Zeron's canonical, incremental parse to Kit's Markdown text input.
/// Synthetic closing markers belong only to the live display, never storage.
pub struct StreamingMarkdown {
    parser: IncrementalParser,
}

impl StreamingMarkdown {
    pub fn new() -> Self {
        Self {
            parser: IncrementalParser::new(),
        }
    }

    pub fn append(&mut self, delta: &str) {
        self.parser.append(delta);
    }

    pub fn source(&self) -> &str {
        self.parser.source()
    }

    pub fn display(&self) -> String {
        let source = self.source();
        // Keep the entire source, including nonlocal reference definitions.
        // Only the final inline-bearing block may receive display closers.
        let Some(last) = self.parser.tree().blocks.last() else {
            return source.to_owned();
        };
        if matches!(
            last.block,
            Block::CodeBlock { .. } | Block::Rule | Block::Table { .. } | Block::Math { .. }
        ) {
            return source.to_owned();
        }
        let start = last.range.start;
        match mend::close_hanging(&source[start..]) {
            Some(tail) => format!("{}{tail}", &source[..start]),
            None => source.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StreamingMarkdown;

    // Exercise the Kit adapter boundary: display stabilization must not alter
    // canonical bytes, completed earlier blocks, or fenced literal content.
    #[test]
    fn live_display_preserves_canonical_text_and_block_boundaries() {
        let mut text = StreamingMarkdown::new();
        text.append("[文档][ref]\n\n[ref]: https://example.com\n\n**正在");
        assert_eq!(text.display(), format!("{}**", text.source()));
        assert!(text.source().ends_with("**正在"));
        text.append("输出**\n\n```rust\nlet text = \"**literal");
        assert_eq!(text.display(), text.source());
        text.append("\";\n```\n\n中文 *尾部");
        assert_eq!(text.display(), format!("{}*", text.source()));
    }
}
