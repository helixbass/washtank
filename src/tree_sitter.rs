use ouroboros::self_referencing;
use ropey::{Rope, RopeSlice};
use squalid::_d;
// use tree_sitter_highlight::{HighlightConfiguration, Highlighter};
use tree_sitter::{Node, Parser, Query, StreamingIterator};

#[derive(Copy, Clone, Debug)]
pub struct TreeSitterHighlight {
    pub start_byte: usize,
    pub end_byte: usize,
    pub highlight_type_index: usize,
}

pub struct RopeWrapper<'a>(&'a Rope);

// TODO: I pulled this from tree-sitter-grep, unify?
impl<'a> tree_sitter::TextProvider<&'a str> for RopeWrapper<'a> {
    type I = RopeTextProviderIterator<'a>;

    fn text(&mut self, node: tree_sitter::Node) -> Self::I {
        let rope_slice = self.0.byte_slice(node.byte_range());
        RopeTextProviderIterator::new(rope_slice, |rope_slice| rope_slice.chunks())
    }
}

#[self_referencing]
pub struct RopeTextProviderIterator<'a> {
    rope_slice: RopeSlice<'a>,

    #[borrows(rope_slice)]
    chunks_iterator: ropey::iter::Chunks<'a>,
}

impl<'a> Iterator for RopeTextProviderIterator<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        self.with_chunks_iterator_mut(|chunks_iterator| chunks_iterator.next())
    }
}

pub fn parse_from_scratch(rope: &Rope, parser: &mut Parser) -> tree_sitter::Tree {
    parser
        .parse_with_options(
            &mut |byte_offset, _| {
                let (chunk, chunk_start_byte_index, _, _) = rope.chunk_at_byte(byte_offset);
                &chunk[byte_offset - chunk_start_byte_index..]
            },
            None,
            None,
        )
        .unwrap()
}

pub fn calculate_highlights(
    highlight_query: &Query,
    node: Node,
    rope: &Rope,
) -> Result<Vec<TreeSitterHighlight>, anyhow::Error> {
    // self.current_tree_sitter_highlights = self
    //     .tree_sitter_highlighter
    //     .highlight(&self.tree_sitter_highlight_configuration)?
    //     .collect::<Result<_, _>>()?;
    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut captures = query_cursor.captures(highlight_query, node, RopeWrapper(rope));
    let mut ret: Vec<TreeSitterHighlight> = _d();
    while let Some(capture) = captures.next() {
        ret.push(TreeSitterHighlight {
            start_byte: capture.0.captures[0].node.start_byte(),
            end_byte: capture.0.captures[0].node.end_byte(),
            highlight_type_index: capture.0.pattern_index,
        });
    }

    Ok(ret)
}
