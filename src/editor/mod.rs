use std::cell::Cell;
use std::cmp;
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
use smallvec::SmallVec;
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
        let printed_line_chunks = compute_printed_line_chunks(
            &printed_lines,
            current_file.rope(),
            &tree_sitter_highlights,
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
            tree_sitter_highlight_colors: vec![
                known_colors()["dark_blue"],
                known_colors()["dark_blue"],
                known_colors()["yellow"],
            ],
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

    fn maybe_move_cursor_down_one_line(&mut self) {
        if self.is_cursor_on_last_file_line() {
            return;
        }
        if self.cursor_position.row == self.file_editor_grid_size().height - 1 {
            let first_line_of_new_top_line = match self.top_line {
                PrintedLine::Line(line) => line + 1,
                PrintedLine::Fold(fold_index) => self.folds[fold_index].range.end,
            };
            self.top_line = match (!self.disallow_folding).then_and(|| {
                self.folds
                    .iter()
                    .position(|fold| fold.range.start == first_line_of_new_top_line)
            }) {
                Some(fold_index) => PrintedLine::Fold(fold_index),
                None => PrintedLine::Line(first_line_of_new_top_line),
            };
            self.recompute_printed_lines_and_printed_line_chunks();
            self.set_allowed_cursor_column();
        } else {
            self.cursor_position.row += 1;
            self.set_allowed_cursor_column();
        }
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
            self.size().height,
            self.disallow_folding,
        );
    }

    pub(crate) fn recompute_printed_lines_and_printed_line_chunks(&mut self) {
        self.recompute_printed_lines();
        self.printed_line_chunks = compute_printed_line_chunks(
            &self.printed_lines,
            self.current_file.rope(),
            &self.current_tree_sitter_highlights,
        );
    }

    fn finish_ex_command<
        TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>),
    >(
        &mut self,
        mut queue_effect: TQueueEffect,
    ) {
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

#[derive(Default)]
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
    // Lsp(LspIncomingMessage),
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
    pub highlight_type_index: Option<usize>,
}

fn compute_printed_line_chunks(
    printed_lines: &[PrintedLine],
    rope: &Rope,
    tree_sitter_highlights: &[TreeSitterHighlight],
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
                        let open_highlight = tree_sitter_highlights[index_in_highlights];
                        if open_highlight.end_byte < next_start_byte {
                            let num_bytes_to_print = open_highlight.end_byte - current_start_byte;
                            line_chunks.push(LineChunk {
                                chunk_index,
                                chunk_start_byte: 0,
                                chunk_end_byte: num_bytes_to_print,
                                highlight_type_index: Some(open_highlight.highlight_type_index),
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
                                highlight_type_index: Some(open_highlight.highlight_type_index),
                            });
                            current_start_byte = next_start_byte;
                            continue;
                        }
                    }
                    'more_highlights: while !matches!(
                        last_highlight,
                        OpenHighlightOrProgress::Next(last_highlight_next)
                            if last_highlight_next >= tree_sitter_highlights.len()
                                || tree_sitter_highlights[last_highlight_next].start_byte >= next_start_byte
                    ) && !matches!(
                        last_highlight,
                        OpenHighlightOrProgress::OpenHighlight(last_highlight_open)
                            if tree_sitter_highlights[last_highlight_open].end_byte >= next_start_byte
                    ) {
                        match last_highlight {
                            OpenHighlightOrProgress::OpenHighlight(last_highlight_open) => {
                                let open_highlight = tree_sitter_highlights[last_highlight_open];
                                let num_bytes_to_print = open_highlight.end_byte
                                    - (current_start_byte + bytes_printed);
                                line_chunks.push(LineChunk {
                                    chunk_index,
                                    chunk_start_byte: bytes_printed,
                                    chunk_end_byte: bytes_printed + num_bytes_to_print,
                                    highlight_type_index: Some(open_highlight.highlight_type_index),
                                });
                                bytes_printed += num_bytes_to_print;
                                last_highlight =
                                    OpenHighlightOrProgress::Next(last_highlight_open + 1);
                            }
                            OpenHighlightOrProgress::Next(last_highlight_next) => {
                                let next_highlight = tree_sitter_highlights[last_highlight_next];
                                while next_highlight.end_byte <= current_start_byte {
                                    last_highlight =
                                        OpenHighlightOrProgress::Next(last_highlight_next + 1);
                                    continue 'more_highlights;
                                }
                                let num_bytes_to_print = next_highlight.start_byte
                                    - (current_start_byte + bytes_printed);
                                line_chunks.push(LineChunk {
                                    chunk_index,
                                    chunk_start_byte: bytes_printed,
                                    chunk_end_byte: bytes_printed + num_bytes_to_print,
                                    highlight_type_index: None,
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
                                highlight_type_index: match last_highlight {
                                    OpenHighlightOrProgress::OpenHighlight(last_highlight_open) =>
                                        Some(tree_sitter_highlights[last_highlight_open].highlight_type_index),
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
                                highlight_type_index: match last_highlight {
                                    OpenHighlightOrProgress::OpenHighlight(last_highlight_open) =>
                                        Some(tree_sitter_highlights[last_highlight_open].highlight_type_index),
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

#[derive(Debug)]
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
