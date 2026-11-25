//! Splits a document into top-level blocks that can be drawn independently.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Markdown(String),
    Mermaid(String),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub kind: Kind,
    /// Character range in the source, used to keep the two panes in step.
    pub chars: Range<usize>,
    pub lines: usize,
    /// Identifies the content, so measured heights survive re-parsing.
    pub hash: u64,
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
}

struct Raw {
    bytes: Range<usize>,
    mermaid: Option<String>,
    /// Byte ranges of footnote references inside this block, with their labels.
    footnotes: Vec<(Range<usize>, String)>,
}

pub fn split(text: &str) -> Vec<Block> {
    let parser = Parser::new_ext(text, options());
    // Link reference definitions produce no events; keep them to add back to
    // the blocks that use them.
    let mut definitions: Vec<(String, Range<usize>)> = parser
        .reference_definitions()
        .iter()
        .map(|(label, def)| (label.to_lowercase(), def.span.clone()))
        .collect();
    definitions.sort_by_key(|(_, span)| span.start);

    let mut raw: Vec<Raw> = Vec::new();
    let mut depth = 0usize;
    let mut diagram: Option<String> = None;
    for (event, range) in parser.into_offset_iter() {
        let top = depth == 0;
        if top && !matches!(event, Event::End(_)) {
            raw.push(Raw {
                bytes: range.clone(),
                mermaid: None,
                footnotes: Vec::new(),
            });
        }
        match event {
            Event::Start(tag) => {
                if top
                    && let Tag::CodeBlock(CodeBlockKind::Fenced(info)) = &tag
                    && info.split_whitespace().next() == Some("mermaid")
                {
                    diagram = Some(String::new());
                }
                depth += 1;
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && matches!(end, TagEnd::CodeBlock)
                    && let Some(block) = raw.last_mut()
                {
                    block.mermaid = diagram.take();
                }
            }
            Event::Text(t) => {
                if let Some(source) = &mut diagram {
                    source.push_str(&t);
                }
            }
            Event::FootnoteReference(label) => {
                if let Some(block) = raw.last_mut() {
                    block.footnotes.push((range, label.into_string()));
                }
            }
            _ => {}
        }
    }

    let (mut byte, mut chars) = (0, 0);
    let mut char_at = move |target: usize| {
        // Block offsets only move forward.
        chars += text[byte..target].chars().count();
        byte = target;
        chars
    };

    raw.into_iter()
        .map(|block| {
            let start = char_at(block.bytes.start);
            let end = char_at(block.bytes.end);
            let source = &text[block.bytes.clone()];
            let lines = source.lines().count().max(1);
            let kind = match block.mermaid {
                Some(diagram) => Kind::Mermaid(diagram),
                None => Kind::Markdown(standalone(text, &block, &definitions)),
            };
            let mut hasher = DefaultHasher::new();
            match &kind {
                Kind::Markdown(s) => (0u8, s).hash(&mut hasher),
                Kind::Mermaid(s) => (1u8, s).hash(&mut hasher),
            }
            Block {
                kind,
                chars: start..end,
                lines,
                hash: hasher.finish(),
            }
        })
        .collect()
}

/// The block's source, made renderable without the rest of the document.
fn standalone(text: &str, block: &Raw, definitions: &[(String, Range<usize>)]) -> String {
    // A footnote reference only parses when its definition is present, which it
    // usually is not here, so show it as a plain bracketed label.
    let mut out = String::new();
    let mut from = block.bytes.start;
    let is_definition = text[block.bytes.clone()].starts_with("[^");
    if !is_definition {
        for (range, label) in &block.footnotes {
            out.push_str(&text[from..range.start]);
            out.push_str("\\[");
            out.push_str(label);
            out.push_str("\\]");
            from = range.end;
        }
    }
    out.push_str(&text[from..block.bytes.end]);

    if !definitions.is_empty() && out.contains('[') {
        let used = bracketed_labels(&out);
        for (label, span) in definitions {
            if used.contains(label) && !block.bytes.contains(&span.start) {
                out.push_str("\n\n");
                out.push_str(&text[span.clone()]);
            }
        }
    }
    out
}

fn bracketed_labels(source: &str) -> HashSet<String> {
    source
        .split('[')
        .skip(1)
        .filter_map(|part| part.split_once(']'))
        .map(|(label, _)| label.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markdown(block: &Block) -> &str {
        match &block.kind {
            Kind::Markdown(s) => s,
            Kind::Mermaid(_) => panic!("expected markdown"),
        }
    }

    #[test]
    fn splits_top_level_blocks_only() {
        let blocks = split("# Title\n\npara\n\n- a\n  - b\n- c\n\n---\n\n| x |\n|---|\n| 1 |\n");
        assert_eq!(blocks.len(), 5);
        assert!(markdown(&blocks[2]).contains("- c"));
    }

    #[test]
    fn finds_mermaid_fences() {
        let blocks = split("```mermaid\nflowchart LR\n  A-->B\n```\n\n```rust\nfn a() {}\n```\n");
        assert_eq!(
            blocks[0].kind,
            Kind::Mermaid("flowchart LR\n  A-->B\n".into())
        );
        assert!(matches!(blocks[1].kind, Kind::Markdown(_)));
    }

    #[test]
    fn reference_links_work_across_blocks() {
        let blocks = split("See [Docs].\n\nOther.\n\n[docs]: https://example.org\n");
        assert!(markdown(&blocks[0]).contains("[docs]: https://example.org"));
        assert!(!markdown(&blocks[1]).contains("example.org"));
    }

    #[test]
    fn footnote_references_stay_readable() {
        let blocks = split("A claim.[^1]\n\n[^1]: Source.\n");
        assert_eq!(markdown(&blocks[0]).trim(), "A claim.\\[1\\]");
        assert!(markdown(&blocks[1]).starts_with("[^1]:"));
    }

    #[test]
    fn character_ranges_count_characters_not_bytes() {
        let blocks = split("é é\n\nnext\n");
        assert_eq!(blocks[1].chars.start, 5);
    }
}
