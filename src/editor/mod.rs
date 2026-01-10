use std::cell::Cell;
use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::LazyLock;

use ::oelung::{Grid, RowOrColumnNumber, Size};
use anyhow;
use crossterm::style::Color;
use futures::future::FutureExt;
use oelung_lantern::mpsc::Sender;
use ropey::{Rope, RopeSlice};
use smallvec::{smallvec, SmallVec};
use squalid::{BoolExt, EverythingExt, _d};
use tokio::fs;

use crate::{
    calculate_folds, calculate_indents, strip_trailing_newline,
    tree_sitter::{self as tree_sitter_mod, calculate_highlights},
    Config, Fold, FoldIndex, IndentLevel, InitialFile, LineNumber, TreeSitterHighlight,
};

mod aggregate;
mod oelung;

pub use aggregate::EventAggregator;

pub struct Editor {
    pub current_file: OpenFile,
    /// position on file-contents part of screen "grid",
    /// not in terms of file line # or actual terminal cursor
    /// position
    pub cursor_position: Position,
    pub initial_terminal_size: Size,
    pub top_line: PrintedLine,
    pub printed_lines: Vec<PrintedLine>,
    pub printed_line_chunks: Vec<PrintedLineChunks>,
    pub tree_sitter_parser: tree_sitter::Parser,
    pub current_tree_sitter_tree: tree_sitter::Tree,
    pub current_tree_sitter_highlights: Vec<TreeSitterHighlight>,
    // pub tree_sitter_highlighter: Highlighter,
    // pub tree_sitter_highlight_configuration: HighlightConfiguration,
    // pub tree_sitter_highlight_names: Vec<&'static str>,
    pub tree_sitter_highlight_query: tree_sitter::Query,
    pub tree_sitter_highlight_colors: Vec<Color>,
    pub current_file_shift_width: usize,
    pub current_file_indents: Vec<IndentLevel>,
    pub folds: Vec<Fold>,
    pub max_folds: Vec<Fold>,
    pub mode: Mode,
    pub sender: Box<dyn Sender<Happened>>,
    pub sticky_cursor_position_column: Option<RowOrColumnNumber>,
    pub last_rendered_grid: Cell<Option<Grid>>,
    pub flex_grow: Option<f64>,
    pub disallow_folding: bool,
    pub disallow_ex_command_mode: bool,
    pub highlight_range: Option<Range>,
    pub current_highlight_ranges: Vec<HighlightRange>,
}

impl Editor {
    pub async fn try_new(
        config: &Config,
        sender: Box<dyn Sender<Happened>>,
        initial_terminal_size: Size,
    ) -> Result<Self, anyhow::Error> {
        let current_file = match &config.initial_file {
            InitialFile::Path(file_name) => {
                let rope = Rope::from_str(strip_trailing_newline(
                    &fs::read_to_string(file_name).await?,
                ));
                OpenFile::Named(OpenFileNamed {
                    rope,
                    path: file_name.clone(),
                })
            }
            InitialFile::Anonymous(contents) => {
                let rope = Rope::from_str(strip_trailing_newline(contents));
                OpenFile::Anonymous(OpenFileAnonymous { rope })
            }
        };

        let mut tree_sitter_parser = {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_rust::LANGUAGE.into())
                .unwrap();
            parser
        };
        let current_tree_sitter_tree =
            tree_sitter_mod::parse_from_scratch(current_file.rope(), &mut tree_sitter_parser);
        let tree_sitter_highlight_query = tree_sitter::Query::new(
            &tree_sitter_rust::LANGUAGE.into(),
            r#"
                (line_comment) @line_comment
                (block_comment) @block_comment
                (string_literal) @string_literal
            "#,
        )?;
        let tree_sitter_highlights = calculate_highlights(
            &tree_sitter_highlight_query,
            current_tree_sitter_tree.root_node(),
            current_file.rope(),
        )?;

        let current_file_shift_width = 4;
        let current_file_indents = calculate_indents(current_file.rope(), current_file_shift_width);

        let folds = calculate_folds(&current_file_indents);
        let max_folds = folds.clone();

        let top_line = if !config.disallow_folding
            && matches!(
                folds.iter().next(),
                Some(fold) if fold.range.start == 0
            ) {
            PrintedLine::Fold(0)
        } else {
            PrintedLine::Line(0)
        };
        let printed_lines = compute_printed_lines(
            current_file.rope().len_lines(),
            top_line,
            &folds,
            initial_terminal_size.height,
            config.disallow_folding,
        );
        let highlight_range = None;
        let tree_sitter_highlight_colors = vec![
            known_colors()["dark_blue"],
            known_colors()["dark_blue"],
            known_colors()["yellow"],
        ];
        let current_highlight_ranges = compute_highlight_ranges(
            &tree_sitter_highlights,
            highlight_range,
            &tree_sitter_highlight_colors,
        );
        let printed_line_chunks = compute_printed_line_chunks(
            &printed_lines,
            current_file.rope(),
            &current_highlight_ranges,
        );

        // let (rust_analyzer_sender, rust_analyzer_receiver) = channel::<LspOutgoingMessage>(100);

        // run_rust_analyzer(sender.clone(), rust_analyzer_receiver);

        // // rust_analyzer_sender
        // //     .send(LspOutgoingMessage::Initialize(InitializeParams {
        // //         // TODO: is std::process:id() blocking aka shouldn't use it
        // //         // from tokio?
        // //         process_id: Some(process::id()),
        // //         client_info: Some(ClientInfo {
        // //             name: "washtank".to_owned(),
        // //             // TODO: make this real?
        // //             version: Some("0.0.1-dev.0".to_owned()),
        // //         }),
        // //     }))
        // //     .unwrap();

        // let tree_sitter_highlight_names = vec!["comment", "string_literal"];
        Ok(Self {
            current_file,
            cursor_position: _d(),
            initial_terminal_size,
            top_line,
            printed_lines,
            printed_line_chunks,
            tree_sitter_parser,
            current_tree_sitter_tree,
            current_tree_sitter_highlights: tree_sitter_highlights,
            // tree_sitter_highlighter: _d(),
            // tree_sitter_highlight_configuration: {
            //     let mut highlight_configuration = HighlightConfiguration::new(
            //         tree_sitter_rust::LANGUAGE.into(),
            //         "rust",
            //         tree_sitter_rust::HIGHLIGHTS_QUERY,
            //         tree_sitter_rust::INJECTIONS_QUERY,
            //         "",
            //     )?;
            //     highlight_configuration.configure(&tree_sitter_highlight_names);
            //     highlight_configuration
            // },
            // tree_sitter_highlight_names,
            tree_sitter_highlight_query,
            tree_sitter_highlight_colors,
            current_file_shift_width,
            current_file_indents,
            folds,
            max_folds,
            mode: Mode::Normal,
            sender,
            sticky_cursor_position_column: _d(),
            last_rendered_grid: _d(),
            flex_grow: config.flex_grow,
            disallow_folding: config.disallow_folding,
            disallow_ex_command_mode: config.disallow_ex_command_mode,
            highlight_range,
            current_highlight_ranges,
        })
    }

    fn size(&self) -> Size {
        match self.last_rendered_grid.get() {
            Some(grid) => Size {
                height: grid.height,
                width: grid.width,
            },
            None => self.initial_terminal_size,
        }
    }

    fn one_past_printed_line_line_number(&self, printed_line: &PrintedLine) -> usize {
        match printed_line {
            PrintedLine::Fold(fold_index) => self.folds[*fold_index].range.end,
            PrintedLine::Line(line) => line + 1,
        }
    }

    fn one_past_final_last_printed_row_line_number(&self) -> usize {
        self.one_past_printed_line_line_number(&self.printed_lines[self.printed_lines.len() - 1])
    }

    fn cursor_printed_line(&self) -> &PrintedLine {
        &self.printed_lines[usize::from(self.cursor_position.row)]
    }

    fn is_cursor_on_last_file_line(&self) -> bool {
        self.one_past_printed_line_line_number(self.cursor_printed_line())
            == self.current_file.rope().len_lines()
    }

    fn set_allowed_cursor_column(&mut self) {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            self.cursor_position.column = 0;
        } else {
            if let Some(sticky_cursor_position_column) = self.sticky_cursor_position_column {
                self.cursor_position.column = sticky_cursor_position_column;
            }
            if self.cursor_position.column > self.max_allowed_column() {
                self.cursor_position.column = self.max_allowed_column();
            }
        }
    }

    fn file_editor_grid_size(&self) -> Size {
        self.size().thrush(|size| Size {
            // TODO: this presumably would panic if trying to
            // render in a terminal window less than 2 rows tall?
            height: size.height - 2,
            width: size.width,
        })
    }

    fn can_scroll_down_further(&self) -> bool {
        !(self.printed_lines.len() < usize::from(self.file_editor_grid_size().height)
            || self.one_past_final_last_printed_row_line_number()
                >= self.current_file.rope().len_lines())
    }

    fn maybe_move_cursor_down_n_lines(&mut self, num_lines: u16) {
        if self.is_cursor_on_last_file_line() {
            return;
        }
        if !self.can_scroll_down_further()
            || self.cursor_position.row + num_lines < self.file_editor_grid_size().height
        {
            self.cursor_position.row += cmp::min(
                num_lines,
                u16::try_from(self.printed_lines.len()).unwrap() - (self.cursor_position.row + 1),
            );
            self.set_allowed_cursor_column();
            return;
        }

        let old_cursor_position_row = self.cursor_position.row;
        self.cursor_position.row = self.file_editor_grid_size().height - 1;
        let max_num_lines_to_scroll_by =
            old_cursor_position_row + num_lines + 1 - self.file_editor_grid_size().height;
        if max_num_lines_to_scroll_by < self.file_editor_grid_size().height {
            let tentative_new_top_line_if_we_can_still_fill_up_the_entire_screen =
                self.printed_lines[usize::from(max_num_lines_to_scroll_by)];
            let tentative_new_printed_lines = compute_printed_lines(
                self.current_file.rope().len_lines(),
                tentative_new_top_line_if_we_can_still_fill_up_the_entire_screen,
                &self.folds,
                self.file_editor_grid_size().height,
                self.disallow_folding,
            );
            if tentative_new_printed_lines.len() == usize::from(self.file_editor_grid_size().height)
            {
                self.top_line = tentative_new_top_line_if_we_can_still_fill_up_the_entire_screen;
                self.printed_lines = tentative_new_printed_lines;
                self.recompute_printed_line_chunks_only();
                return;
            }
            self.top_line = self.printed_lines[usize::from(
                max_num_lines_to_scroll_by
                    - (self.file_editor_grid_size().height
                        - u16::try_from(tentative_new_printed_lines.len()).unwrap()),
            )];
            self.recompute_printed_lines_and_printed_line_chunks();
            assert_eq!(
                self.printed_lines.len(),
                usize::from(self.file_editor_grid_size().height)
            );
            assert!(!self.can_scroll_down_further());
            return;
        }
        unimplemented!()
    }

    fn maybe_move_cursor_up_one_line(&mut self) {
        if self.cursor_position.row == 0 {
            let top_line_start_line = self.top_line.start_line(&self.folds);
            if top_line_start_line == 0 {
                return;
            }

            self.top_line = match (!self.disallow_folding).then_and(|| {
                self.folds
                    .iter()
                    .position(|fold| fold.range.end == top_line_start_line - 1)
            }) {
                Some(fold_index) => PrintedLine::Fold(fold_index),
                None => PrintedLine::Line(top_line_start_line - 1),
            };
            self.recompute_printed_lines_and_printed_line_chunks();
            self.set_allowed_cursor_column();
        } else {
            self.cursor_position.row -= 1;
            self.set_allowed_cursor_column();
        }
    }

    fn maybe_move_cursor_right_one_column(&mut self) {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            panic!("don't currently support left/right movement on fold line");
        }
        if self.cursor_position.column < self.max_allowed_column() {
            self.cursor_position.column += 1;
            self.remember_sticky_cursor_position_column();
        }
    }

    fn maybe_move_cursor_left_one_column(&mut self) {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            panic!("don't currently support left/right movement on fold line");
        }
        if self.cursor_position.column > 0 {
            self.cursor_position.column -= 1;
            self.remember_sticky_cursor_position_column();
        }
    }

    fn move_cursor_to_beginning_of_line(&mut self) {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            panic!("don't currently support left/right movement on fold line");
        }
        self.cursor_position.column = 0;
        self.remember_sticky_cursor_position_column();
    }

    fn move_cursor_to_end_of_line(&mut self) {
        if matches!(self.cursor_printed_line(), PrintedLine::Fold(_)) {
            panic!("don't currently support left/right movement on fold line");
        }
        self.cursor_position.column = self.max_allowed_column();
        self.remember_sticky_cursor_position_column();
    }

    fn remember_sticky_cursor_position_column(&mut self) {
        self.sticky_cursor_position_column = Some(self.cursor_position.column);
    }

    fn num_relative_line_number_columns(&self) -> RowOrColumnNumber {
        cmp::max(
            3,
            num_columns_taken_up(self.current_file.rope().len_lines()),
        )
    }

    fn recompute_printed_lines(&mut self) {
        self.printed_lines = compute_printed_lines(
            self.current_file.rope().len_lines(),
            self.top_line,
            &self.folds,
            self.file_editor_grid_size().height,
            self.disallow_folding,
        );
    }

    pub(crate) fn recompute_printed_lines_and_printed_line_chunks(&mut self) {
        self.recompute_printed_lines();
        self.recompute_printed_line_chunks_only();
    }

    pub(crate) fn recompute_printed_line_chunks_only(&mut self) {
        self.printed_line_chunks = compute_printed_line_chunks(
            &self.printed_lines,
            self.current_file.rope(),
            &self.current_highlight_ranges,
        );
    }

    fn finish_ex_command<
        TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>),
    >(
        &mut self,
        mut queue_effect: TQueueEffect,
    ) {
        assert!(!self.disallow_ex_command_mode);
        if self.mode.as_ex_command() != "q" {
            panic!("only support `:q` currently");
        }
        self.mode = Mode::Normal;
        queue_effect({
            let sender = self.sender.box_clone();
            async move {
                sender.send(Happened::Quit).await;
            }
            .boxed()
        });
    }

    fn max_allowed_column(&self) -> u16 {
        let cursor_line_num = *match self.cursor_printed_line() {
            PrintedLine::Line(line_num) => line_num,
            PrintedLine::Fold(_) => panic!("expected not to be called with fold"),
        };
        match line_len(&self.current_file.rope().line(cursor_line_num)) {
            0 => 0,
            line_len => u16::try_from(line_len).unwrap() - 1,
        }
    }

    fn recompute_tree_sitter_tree(&mut self) {
        self.current_tree_sitter_tree = tree_sitter_mod::parse_from_scratch(
            self.current_file.rope(),
            &mut self.tree_sitter_parser,
        );
    }

    fn recompute_tree_sitter_highlights(&mut self) -> Result<(), anyhow::Error> {
        self.current_tree_sitter_highlights = calculate_highlights(
            &self.tree_sitter_highlight_query,
            self.current_tree_sitter_tree.root_node(),
            self.current_file.rope(),
        )?;

        Ok(())
    }

    fn recompute_highlight_ranges(&mut self) -> Result<(), anyhow::Error> {
        self.recompute_tree_sitter_highlights()?;
        self.current_highlight_ranges = compute_highlight_ranges(
            &self.current_tree_sitter_highlights,
            self.highlight_range,
            &self.tree_sitter_highlight_colors,
        );

        Ok(())
    }

    fn recompute_on_highlights_or_content_changed(&mut self) -> Result<(), anyhow::Error> {
        self.recompute_tree_sitter_tree();
        self.recompute_highlight_ranges()?;
        self.recompute_printed_lines_and_printed_line_chunks();

        Ok(())
    }

    fn insert_char(&mut self, ch: char) -> Result<(), anyhow::Error> {
        let offset = get_char_offset(self.current_file.rope(), self.cursor_position);
        self.current_file.rope_mut().insert_char(offset, ch);
        self.cursor_position.column += 1;
        self.recompute_on_highlights_or_content_changed()?;

        Ok(())
    }
}

pub enum OpenFile {
    Anonymous(OpenFileAnonymous),
    Named(OpenFileNamed),
}

impl OpenFile {
    pub fn rope(&self) -> &Rope {
        match self {
            Self::Anonymous(file) => &file.rope,
            Self::Named(file) => &file.rope,
        }
    }

    pub fn rope_mut(&mut self) -> &mut Rope {
        match self {
            Self::Anonymous(file) => &mut file.rope,
            Self::Named(file) => &mut file.rope,
        }
    }
}

impl Default for OpenFile {
    fn default() -> Self {
        Self::Anonymous(_d())
    }
}

#[derive(Default)]
pub struct OpenFileAnonymous {
    pub rope: Rope,
}

pub struct OpenFileNamed {
    pub rope: Rope,
    pub path: PathBuf,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Position {
    pub row: RowOrColumnNumber,
    pub column: RowOrColumnNumber,
}

#[derive(Copy, Clone)]
pub enum PrintedLine {
    Line(LineNumber),
    Fold(FoldIndex),
}

impl PrintedLine {
    pub fn start_line(&self, folds: &[Fold]) -> LineNumber {
        match self {
            Self::Line(line) => *line,
            Self::Fold(fold_index) => folds[*fold_index].range.start,
        }
    }
}

pub(crate) fn num_columns_taken_up(num: usize) -> RowOrColumnNumber {
    if num >= 10000 {
        5
    } else if num >= 1000 {
        4
    } else if num >= 100 {
        3
    } else if num >= 10 {
        2
    } else {
        1
    }
}

pub(crate) fn known_colors() -> &'static HashMap<String, Color> {
    static KNOWN_COLORS: LazyLock<HashMap<String, Color>> = LazyLock::new(|| {
        [
            (
                "dark_blue".to_owned(),
                Color::Rgb {
                    r: 47,
                    g: 47,
                    b: 255,
                },
            ),
            (
                "yellow".to_owned(),
                Color::Rgb {
                    r: 240,
                    g: 240,
                    b: 0,
                },
            ),
        ]
        .into_iter()
        .collect()
    });
    &*KNOWN_COLORS
}

#[derive(Debug)]
pub enum Event {
    MoveCursorDownNLines(u16),
    MoveCursorUpNLines(u16),
    MoveCursorRightNColumns(u16),
    MoveCursorLeftNColumns(u16),
    FullyOpenFoldUnderCursor,
    OpenFoldUnderCursorOneLevel,
    FullyCloseFoldUnderCursor,
    CloseFoldUnderCursorOneLevel,
    GoIntoNormalMode,
    GoIntoExCommandMode,
    ExCommandChar(char),
    FinishExCommand,
    MoveCursorToBeginningOfLine,
    MoveCursorToEndOfLine,
    GoIntoInsertMode,
    InsertChar(char),
    HighlightRange(Range),
    // Lsp(LspIncomingMessage),
}

impl Event {
    pub fn is_file_contents_mutating(&self) -> bool {
        match self {
            Self::MoveCursorDownNLines(_)
            | Self::MoveCursorUpNLines(_)
            | Self::MoveCursorRightNColumns(_)
            | Self::MoveCursorLeftNColumns(_)
            | Self::FullyOpenFoldUnderCursor
            | Self::OpenFoldUnderCursorOneLevel
            | Self::FullyCloseFoldUnderCursor
            | Self::CloseFoldUnderCursorOneLevel
            | Self::GoIntoNormalMode
            | Self::GoIntoExCommandMode
            | Self::ExCommandChar(_)
            | Self::MoveCursorToBeginningOfLine
            | Self::MoveCursorToEndOfLine
            | Self::GoIntoInsertMode
            | Self::HighlightRange(_) => false,
            Self::InsertChar(_) | Self::FinishExCommand => true,
        }
    }
}

fn compute_printed_lines(
    num_lines: usize,
    top_line: PrintedLine,
    folds: &[Fold],
    height: u16,
    disallow_folding: bool,
) -> Vec<PrintedLine> {
    let top_line = top_line.start_line(folds);
    assert!(top_line <= num_lines - 1);

    let mut current_line_num = top_line;
    let mut next_eligible_fold_index = (!disallow_folding).then_and(|| {
        folds
            .into_iter()
            .position(|fold| fold.range.start >= top_line)
    });
    let mut ret: Vec<PrintedLine> = _d();
    for _ in 0..height {
        if current_line_num >= num_lines {
            break;
        }

        if let Some(next_eligible_fold_index_yes) =
            next_eligible_fold_index.filter(|&next_eligible_fold_index| {
                folds[next_eligible_fold_index].range.start == current_line_num
            })
        {
            ret.push(PrintedLine::Fold(next_eligible_fold_index_yes));
            current_line_num = folds[next_eligible_fold_index_yes].range.end;
            if next_eligible_fold_index_yes < folds.len() - 1 {
                next_eligible_fold_index = Some(next_eligible_fold_index_yes + 1);
            }
        } else {
            ret.push(PrintedLine::Line(current_line_num));
            current_line_num += 1;
        }
    }
    ret
}

pub enum PrintedLineChunks {
    Line(LineNumber, LineChunks),
    Fold(FoldIndex),
}

impl PrintedLineChunks {
    pub fn start_line(&self, folds: &[Fold]) -> LineNumber {
        match self {
            Self::Line(line, _) => *line,
            Self::Fold(fold_index) => folds[*fold_index].range.start,
        }
    }
}

type LineChunks = SmallVec<LineChunk, 10>;

pub struct LineChunk {
    pub chunk_index: usize,
    pub chunk_start_byte: usize,
    pub chunk_end_byte: usize,
    pub style: Option<Style>,
}

fn compute_printed_line_chunks(
    printed_lines: &[PrintedLine],
    rope: &Rope,
    highlight_ranges: &[HighlightRange],
) -> Vec<PrintedLineChunks> {
    type IndexInHighlights = usize;
    #[derive(Copy, Clone)]
    enum OpenHighlightOrProgress {
        OpenHighlight(IndexInHighlights),
        Next(IndexInHighlights),
    }

    impl Default for OpenHighlightOrProgress {
        fn default() -> Self {
            Self::Next(0)
        }
    }

    let mut last_highlight: OpenHighlightOrProgress = _d();
    printed_lines.into_iter().map(|printed_line| {
        match printed_line {
            PrintedLine::Fold(fold_index) => PrintedLineChunks::Fold(*fold_index),
            PrintedLine::Line(line_num) => {
                let line_num = *line_num;
                let line = rope.line(line_num);

                let mut line_chunks: LineChunks = _d();
                let mut current_start_byte = rope.line_to_byte(line_num);
                for (chunk_index, chunk) in line.chunks().enumerate() {
                    let next_start_byte = current_start_byte + chunk.len();
                    let mut bytes_printed = 0;
                    if let OpenHighlightOrProgress::OpenHighlight(index_in_highlights) =
                        last_highlight
                    {
                        let open_highlight = &highlight_ranges[index_in_highlights];
                        if open_highlight.range.end < next_start_byte {
                            let num_bytes_to_print = open_highlight.range.end - current_start_byte;
                            line_chunks.push(LineChunk {
                                chunk_index,
                                chunk_start_byte: 0,
                                chunk_end_byte: num_bytes_to_print,
                                style: Some(open_highlight.style.clone()),
                            });
                            bytes_printed += num_bytes_to_print;
                            last_highlight =
                                OpenHighlightOrProgress::Next(index_in_highlights + 1);
                        } else {
                            line_chunks.push(LineChunk {
                                chunk_index,
                                chunk_start_byte: 0,
                                chunk_end_byte: if chunk.ends_with("\n") {
                                    chunk.len() - 1
                                } else {
                                    chunk.len()
                                },
                                style: Some(open_highlight.style.clone()),
                            });
                            current_start_byte = next_start_byte;
                            continue;
                        }
                    }
                    'more_highlights: while !matches!(
                        last_highlight,
                        OpenHighlightOrProgress::Next(last_highlight_next)
                            if last_highlight_next >= highlight_ranges.len()
                                || highlight_ranges[last_highlight_next].range.start >= next_start_byte
                    ) && !matches!(
                        last_highlight,
                        OpenHighlightOrProgress::OpenHighlight(last_highlight_open)
                            if highlight_ranges[last_highlight_open].range.end >= next_start_byte
                    ) {
                        match last_highlight {
                            OpenHighlightOrProgress::OpenHighlight(last_highlight_open) => {
                                let open_highlight = &highlight_ranges[last_highlight_open];
                                let num_bytes_to_print = open_highlight.range.end
                                    - (current_start_byte + bytes_printed);
                                line_chunks.push(LineChunk {
                                    chunk_index,
                                    chunk_start_byte: bytes_printed,
                                    chunk_end_byte: bytes_printed + num_bytes_to_print,
                                    style: Some(open_highlight.style.clone()),
                                });
                                bytes_printed += num_bytes_to_print;
                                last_highlight =
                                    OpenHighlightOrProgress::Next(last_highlight_open + 1);
                            }
                            OpenHighlightOrProgress::Next(last_highlight_next) => {
                                let next_highlight = &highlight_ranges[last_highlight_next];
                                while next_highlight.range.end <= current_start_byte {
                                    last_highlight =
                                        OpenHighlightOrProgress::Next(last_highlight_next + 1);
                                    continue 'more_highlights;
                                }
                                let num_bytes_to_print = next_highlight.range.start
                                    - (current_start_byte + bytes_printed);
                                line_chunks.push(LineChunk {
                                    chunk_index,
                                    chunk_start_byte: bytes_printed,
                                    chunk_end_byte: bytes_printed + num_bytes_to_print,
                                    style: None,
                                });
                                bytes_printed += num_bytes_to_print;
                                last_highlight =
                                    OpenHighlightOrProgress::OpenHighlight(last_highlight_next);
                            }
                        }
                    }
                    if chunk.ends_with("\n") {
                        if bytes_printed < chunk.len() - 1 {
                            line_chunks.push(LineChunk {
                                chunk_index,
                                chunk_start_byte: bytes_printed,
                                chunk_end_byte: chunk.len() - 1,
                                style: match last_highlight {
                                    OpenHighlightOrProgress::OpenHighlight(last_highlight_open) =>
                                        Some(highlight_ranges[last_highlight_open].style.clone()),
                                    _ => None
                                }
                            });
                        }
                    } else {
                        if bytes_printed < chunk.len() {
                            line_chunks.push(LineChunk {
                                chunk_index,
                                chunk_start_byte: bytes_printed,
                                chunk_end_byte: chunk.len(),
                                style: match last_highlight {
                                    OpenHighlightOrProgress::OpenHighlight(last_highlight_open) =>
                                        Some(highlight_ranges[last_highlight_open].style.clone()),
                                    _ => None
                                }
                            });
                        }
                    }
                    current_start_byte = next_start_byte;
                }
                PrintedLineChunks::Line(line_num, line_chunks)
            }
        }
    }).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    ExCommand(String),
    Insert,
}

impl Mode {
    pub fn as_ex_command(&self) -> &str {
        match self {
            Self::ExCommand(command) => command,
            _ => panic!("expected ex command"),
        }
    }

    pub fn as_ex_command_mut(&mut self) -> &mut String {
        match self {
            Self::ExCommand(command) => command,
            _ => panic!("expected ex command"),
        }
    }
}

#[derive(Debug)]
pub enum Happened {
    Quit,
}

pub fn line_len(line: &RopeSlice<'_>) -> usize {
    let line_len = line.len_bytes();
    match line.byte(line_len - 1) {
        // \n
        0x0A => line_len - 1,
        _ => line_len,
    }
}

fn get_char_offset(rope: &Rope, position: Position) -> usize {
    let beginning_of_line = rope.line_to_char(usize::from(position.row));
    beginning_of_line + usize::from(position.column)
}

pub type Offset = usize;

#[derive(Copy, Clone, Debug)]
pub struct Range {
    pub start: Offset,
    pub end: Offset,
}

#[derive(Clone, Debug)]
pub struct Style {
    pub foreground_color: Option<Color>,
    pub background_color: Option<Color>,
}

#[derive(Debug)]
pub struct HighlightRange {
    pub range: Range,
    pub style: Style,
}

fn tree_sitter_highlight_style(
    tree_sitter_highlight: &TreeSitterHighlight,
    tree_sitter_highlight_colors: &[Color],
) -> Style {
    Style {
        foreground_color: Some(
            tree_sitter_highlight_colors[tree_sitter_highlight.highlight_type_index],
        ),
        background_color: _d(),
    }
}

fn tree_sitter_highlight_to_highlight_range(
    tree_sitter_highlight: &TreeSitterHighlight,
    tree_sitter_highlight_colors: &[Color],
) -> HighlightRange {
    HighlightRange {
        range: tree_sitter_highlight.range,
        style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
    }
}

fn highlight_range_background_color() -> Color {
    Color::AnsiValue(94)
}

fn highlight_range_style() -> Style {
    Style {
        foreground_color: None,
        background_color: Some(highlight_range_background_color()),
    }
}

fn highlight_range_to_highlight_range(highlight_range: Range) -> HighlightRange {
    HighlightRange {
        range: highlight_range,
        style: highlight_range_style(),
    }
}

fn tree_sitter_and_highlight_range_style(
    tree_sitter_highlight: &TreeSitterHighlight,
    tree_sitter_highlight_colors: &[Color],
) -> Style {
    Style {
        foreground_color: Some(
            tree_sitter_highlight_colors[tree_sitter_highlight.highlight_type_index],
        ),
        background_color: Some(highlight_range_background_color()),
    }
}

fn compute_highlight_ranges(
    tree_sitter_highlights: &[TreeSitterHighlight],
    highlight_range: Option<Range>,
    tree_sitter_highlight_colors: &[Color],
) -> Vec<HighlightRange> {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    enum EngagementWithHighlightRange {
        HasntStarted,
        InProgress,
        Done,
    }
    let mut engagement_with_highlight_range = EngagementWithHighlightRange::HasntStarted;
    let mut ret: Vec<HighlightRange> = _d();
    for (tree_sitter_highlight_index, tree_sitter_highlight) in
        tree_sitter_highlights.into_iter().enumerate()
    {
        ret.extend(match highlight_range {
            None => smallvec![tree_sitter_highlight_to_highlight_range(tree_sitter_highlight, tree_sitter_highlight_colors)],
            Some(highlight_range) => {
                match engagement_with_highlight_range {
                    EngagementWithHighlightRange::Done => smallvec![
                        tree_sitter_highlight_to_highlight_range(tree_sitter_highlight, tree_sitter_highlight_colors)
                    ],
                    EngagementWithHighlightRange::InProgress => {
                        let mut ret: SmallVec<_, 4> = _d();
                        if tree_sitter_highlights[tree_sitter_highlight_index - 1].range.end < tree_sitter_highlight.range.start {
                            ret.push(HighlightRange {
                                range: Range {
                                    start: tree_sitter_highlights[tree_sitter_highlight_index - 1].range.end,
                                    end: cmp::min(tree_sitter_highlight.range.start, highlight_range.end),
                                },
                                style: highlight_range_style(),
                            });
                        }
                        match highlight_range.end.cmp(&tree_sitter_highlight.range.start) {
                            Ordering::Less | Ordering::Equal => {
                                engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                ret.push(tree_sitter_highlight_to_highlight_range(tree_sitter_highlight, tree_sitter_highlight_colors));
                            }
                            Ordering::Greater => match highlight_range.end.cmp(&tree_sitter_highlight.range.end) {
                                Ordering::Greater => {
                                    ret.push(HighlightRange {
                                        range: tree_sitter_highlight.range,
                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                    });
                                }
                                Ordering::Equal => {
                                    ret.push(HighlightRange {
                                        range: tree_sitter_highlight.range,
                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                    });
                                    engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                }
                                Ordering::Less => {
                                    ret.push(HighlightRange {
                                        range: Range {
                                            start: tree_sitter_highlight.range.start,
                                            end: highlight_range.end,
                                        },
                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                    });
                                    ret.push(HighlightRange {
                                        range: Range {
                                            start: highlight_range.end,
                                            end: tree_sitter_highlight.range.end,
                                        },
                                        style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                    });
                                    engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                }
                            }
                        }
                        ret
                    }
                    EngagementWithHighlightRange::HasntStarted => {
                        match highlight_range.start.cmp(&tree_sitter_highlight.range.end) {
                            Ordering::Greater | Ordering::Equal => smallvec![tree_sitter_highlight_to_highlight_range(tree_sitter_highlight, tree_sitter_highlight_colors)],
                            Ordering::Less => {
                                let mut ret: SmallVec<_, 4> = _d();
                                match highlight_range.end.cmp(&tree_sitter_highlight.range.start) {
                                    Ordering::Less | Ordering::Equal => {
                                        ret.push(highlight_range_to_highlight_range(highlight_range));
                                        engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                        ret.push(tree_sitter_highlight_to_highlight_range(tree_sitter_highlight, tree_sitter_highlight_colors));
                                    }
                                    Ordering::Greater => match highlight_range.end.cmp(&tree_sitter_highlight.range.end) {
                                        Ordering::Equal => {
                                            match highlight_range.start.cmp(&tree_sitter_highlight.range.start) {
                                                Ordering::Equal => {
                                                    ret.push(HighlightRange {
                                                        range: tree_sitter_highlight.range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Less => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: highlight_range.start,
                                                            end: tree_sitter_highlight.range.start,
                                                        },
                                                        style: highlight_range_style(),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: tree_sitter_highlight.range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Greater => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: tree_sitter_highlight.range.start,
                                                            end: highlight_range.start,
                                                        },
                                                        style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: highlight_range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                            }
                                            engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                        }
                                        Ordering::Greater => {
                                            match highlight_range.start.cmp(&tree_sitter_highlight.range.start) {
                                                Ordering::Equal => {
                                                    ret.push(HighlightRange {
                                                        range: tree_sitter_highlight.range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Less => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: highlight_range.start,
                                                            end: tree_sitter_highlight.range.start,
                                                        },
                                                        style: highlight_range_style(),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: tree_sitter_highlight.range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Greater => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: tree_sitter_highlight.range.start,
                                                            end: highlight_range.start,
                                                        },
                                                        style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: highlight_range.start,
                                                            end: tree_sitter_highlight.range.end,
                                                        },
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                            }
                                            engagement_with_highlight_range = EngagementWithHighlightRange::InProgress;
                                        }
                                        Ordering::Less => {
                                            match highlight_range.start.cmp(&tree_sitter_highlight.range.start) {
                                                Ordering::Equal => {
                                                    ret.push(HighlightRange {
                                                        range: highlight_range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Less => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: highlight_range.start,
                                                            end: tree_sitter_highlight.range.start,
                                                        },
                                                        style: highlight_range_style(),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: tree_sitter_highlight.range.start,
                                                            end: highlight_range.end,
                                                        },
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                                Ordering::Greater => {
                                                    ret.push(HighlightRange {
                                                        range: Range {
                                                            start: tree_sitter_highlight.range.start,
                                                            end: highlight_range.start,
                                                        },
                                                        style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                    ret.push(HighlightRange {
                                                        range: highlight_range,
                                                        style: tree_sitter_and_highlight_range_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                                    });
                                                }
                                            }
                                            ret.push(HighlightRange {
                                                range: Range {
                                                    start: highlight_range.end,
                                                    end: tree_sitter_highlight.range.end,
                                                },
                                                style: tree_sitter_highlight_style(tree_sitter_highlight, tree_sitter_highlight_colors),
                                            });
                                            engagement_with_highlight_range = EngagementWithHighlightRange::Done;
                                        }
                                    }
                                }
                                ret
                            }
                        }
                    }
                }
            }
        });
    }

    ret.extend(match highlight_range {
        None => SmallVec::<_, 2>::default(),
        Some(highlight_range) => match engagement_with_highlight_range {
            EngagementWithHighlightRange::HasntStarted => {
                assert!(
                    tree_sitter_highlights.is_empty()
                        || highlight_range.start
                            >= tree_sitter_highlights[tree_sitter_highlights.len() - 1]
                                .range
                                .end
                );
                smallvec![highlight_range_to_highlight_range(highlight_range)]
            }
            EngagementWithHighlightRange::Done => smallvec![],
            EngagementWithHighlightRange::InProgress => {
                smallvec![HighlightRange {
                    range: Range {
                        start: tree_sitter_highlights[tree_sitter_highlights.len() - 1]
                            .range
                            .end,
                        end: highlight_range.end,
                    },
                    style: highlight_range_style(),
                }]
            }
        },
    });
    ret
}
