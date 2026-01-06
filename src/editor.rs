use std::cmp;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::LazyLock;

use anyhow;
use crossterm::{
    event::{self, KeyCode},
    style::Color,
    terminal::size,
};
use oelung::{soft, Component, ComponentInterface, Grid};
use oelung_lantern::{
    is_any_simple_char_press, is_simple_char_press, is_simple_key_press, ReceiveEvent,
};
use ropey::{Rope, RopeSlice};
use smallvec::{smallvec, SmallVec};
use smol_str::format_smolstr;
use squalid::{EverythingExt, _d, regex};
use tokio::fs;
use tracing::instrument;

use crate::{
    calculate_folds, calculate_indents, strip_trailing_newline,
    tree_sitter::{self as tree_sitter_mod, calculate_highlights},
    Args, Fold, FoldIndex, IndentLevel, LineNumber, TreeSitterHighlight,
};

pub struct Editor {
    pub current_file: OpenFile,
    /// position on file-contents part of screen "grid",
    /// not in terms of file line # or actual terminal cursor
    /// position
    pub cursor_position: Position,
    pub size: Size,
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
}

impl Editor {
    pub async fn try_new(args: Args) -> Result<Self, anyhow::Error> {
        let rope = Rope::from_str(strip_trailing_newline(
            &fs::read_to_string(&args.file_name).await?,
        ));
        let current_file = OpenFile::Named(OpenFileNamed {
            rope,
            path: args.file_name,
        });

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

        let top_line = if matches!(
            folds.iter().next(),
            Some(fold) if fold.range.start == 0
        ) {
            PrintedLine::Fold(0)
        } else {
            PrintedLine::Line(0)
        };
        let size = size()?.thrush(|(columns, rows)| Size {
            height: rows,
            width: columns,
        });
        let printed_lines = compute_printed_lines(
            current_file.rope().len_lines(),
            top_line,
            &folds,
            size.height,
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
            size,
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
        })
    }

    fn one_past_final_last_printed_row_line_number(&self) -> usize {
        match self.printed_lines[self.printed_lines.len() - 1] {
            PrintedLine::Fold(fold_index) => self.folds[fold_index].range.end,
            PrintedLine::Line(line) => line + 1,
        }
    }

    fn maybe_move_cursor_down_one_line(&mut self) {
        if self.cursor_position.row == self.size.height - 1 {
            if self.one_past_final_last_printed_row_line_number()
                == self.current_file.rope().len_lines()
            {
                return;
            }
            let first_line_of_new_top_line = match self.top_line {
                PrintedLine::Line(line) => line + 1,
                PrintedLine::Fold(fold_index) => self.folds[fold_index].range.end,
            };
            self.top_line = match self
                .folds
                .iter()
                .position(|fold| fold.range.start == first_line_of_new_top_line)
            {
                Some(fold_index) => PrintedLine::Fold(fold_index),
                None => PrintedLine::Line(first_line_of_new_top_line),
            };
            self.recompute_printed_lines_and_printed_line_chunks();
        } else {
            self.cursor_position.row += 1;
        }
    }

    fn maybe_move_cursor_up_one_line(&mut self) {
        if self.cursor_position.row == 0 {
            let top_line_start_line = self.top_line.start_line(&self.folds);
            if top_line_start_line == 0 {
                return;
            }

            self.top_line = match self
                .folds
                .iter()
                .position(|fold| fold.range.end == top_line_start_line - 1)
            {
                Some(fold_index) => PrintedLine::Fold(fold_index),
                None => PrintedLine::Line(top_line_start_line - 1),
            };
            self.recompute_printed_lines_and_printed_line_chunks();
        } else {
            self.cursor_position.row -= 1;
        }
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
            self.size.height,
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
}

impl<'a> ComponentInterface for &'a Editor {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        let num_relative_line_number_columns = self.num_relative_line_number_columns();

        Ok(soft! {
            %FlexColumn
              children => self.printed_line_chunks.iter().enumerate().map(|(printed_row_num, printed_line_chunks)| -> Result<_, anyhow::Error> {
                  let printed_row_num = u16::try_from(printed_row_num).unwrap();
                  let line_num = printed_line_chunks.start_line(&self.folds);
                  let relative_line_number = soft! {
                      %RelativeLineNumber::new(
                          num_relative_line_number_columns,
                          match self.cursor_position.row == printed_row_num {
                              true => RelativeOrCurrentLineNum::Current(line_num),
                              false => RelativeOrCurrentLineNum::Relative(
                                    u16::try_from(
                                        (i32::try_from(self.cursor_position.row).unwrap()
                                            - i32::try_from(printed_row_num).unwrap())
                                        .abs(),
                                    )
                                    .unwrap()
                              ),
                          }
                      )
                  };
                  Ok(match printed_line_chunks {
                      PrintedLineChunks::Fold(fold_index) => soft! {
                          %Text children => [
                            relative_line_number
                            %Text " "
                            %{
                                let fold = &self.folds[*fold_index];
                                FoldLine::new(
                                    fold,
                                    self.current_file.rope().line(fold.range.start),
                                )
                            }
                          ]
                      },
                      PrintedLineChunks::Line(line_num, line_chunks) => {
                          let line_num = *line_num;
                          let line = self.current_file.rope().line(line_num);
                          let chunks = line.chunks().collect::<SmallVec<_, 10>>();
                          soft! {
                              %Text children => {
                                  [
                                      Ok(relative_line_number.into_text_child()),
                                      Ok(soft! {
                                          %Text " "
                                      }.into_text_child())
                                  ].into_iter().chain(
                                      line_chunks.into_iter().map(|line_chunk| -> Result<_, anyhow::Error> {
                                          Ok(soft! {
                                              %Text
                                                text => &chunks[line_chunk.chunk_index][line_chunk.chunk_start_byte..line_chunk.chunk_end_byte]
                                                maybe_color => line_chunk.highlight_type_index.map(|highlight_type_index| {
                                                    self.tree_sitter_highlight_colors[highlight_type_index]
                                                })
                                          }.into_text_child())
                                      })
                                  ).collect::<Result<_, _>>()?
                              }
                          }
                      }
                  })
              }).collect::<Result<_, _>>()?
              overflow_y => hidden
              cursor => %Cursor.Relative
                x => {
                    self.cursor_position.column + self.num_relative_line_number_columns() + 1
                }
                y => self.cursor_position.row
        })
    }
}

impl ReceiveEvent<Event> for Editor {
    #[instrument(level = "trace", skip(self, event, _queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &Event,
        _queue_effect: TQueueEffect,
    ) -> Result<(), anyhow::Error> {
        match event {
            Event::MoveCursorDownNLines(n) => {
                assert_eq!(*n, 1);
                self.maybe_move_cursor_down_one_line();
                Ok(())
            }
            Event::MoveCursorUpNLines(n) => {
                assert_eq!(*n, 1);
                self.maybe_move_cursor_up_one_line();
                Ok(())
            }
            Event::FullyOpenFoldUnderCursor => {
                self.fully_open_fold_under_cursor();
                Ok(())
            }
            Event::OpenFoldUnderCursorOneLevel => {
                self.open_fold_under_cursor_one_level();
                Ok(())
            }
            Event::FullyCloseFoldUnderCursor => {
                self.fully_close_fold_under_cursor();
                Ok(())
            }
            Event::CloseFoldUnderCursorOneLevel => {
                self.close_fold_under_cursor_one_level();
                Ok(())
            }
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

type RowOrColumnNumber = u16;

#[derive(Default)]
pub struct Position {
    pub row: RowOrColumnNumber,
    pub column: RowOrColumnNumber,
}

pub struct Size {
    pub height: RowOrColumnNumber,
    pub width: RowOrColumnNumber,
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

fn num_columns_taken_up(num: usize) -> RowOrColumnNumber {
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

fn known_colors() -> &'static HashMap<String, Color> {
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

pub enum Event {
    MoveCursorDownNLines(u16),
    MoveCursorUpNLines(u16),
    FullyOpenFoldUnderCursor,
    OpenFoldUnderCursorOneLevel,
    FullyCloseFoldUnderCursor,
    CloseFoldUnderCursorOneLevel,
    GoIntoExCommandMode,
    ExCommandChar(char),
    // Lsp(LspIncomingMessage),
}

fn compute_printed_lines(
    num_lines: usize,
    top_line: PrintedLine,
    folds: &[Fold],
    height: u16,
) -> Vec<PrintedLine> {
    let top_line = top_line.start_line(folds);
    assert!(top_line <= num_lines - 1);

    let mut current_line_num = top_line;
    let mut next_eligible_fold_index = folds
        .into_iter()
        .position(|fold| fold.range.start >= top_line);
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

struct RelativeLineNumber {
    pub num_columns: u16,
    pub relative_or_current_line_num: RelativeOrCurrentLineNum,
}

impl RelativeLineNumber {
    pub fn new(num_columns: u16, relative_or_current_line_num: RelativeOrCurrentLineNum) -> Self {
        Self {
            num_columns,
            relative_or_current_line_num,
        }
    }
}

impl ComponentInterface for RelativeLineNumber {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        let color = Color::Rgb {
            r: 122,
            g: 122,
            b: 122,
        };
        Ok(match self.relative_or_current_line_num {
            RelativeOrCurrentLineNum::Current(line_num) => {
                let line_num_to_show = line_num + 1;
                let num_columns_taken_up = num_columns_taken_up(line_num_to_show);
                let num_remaining_columns = self.num_columns - num_columns_taken_up;
                soft! {
                    %Text
                      children => {
                          [
                              soft! {
                                  %Text line_num_to_show
                              }.into_text_child()
                          ].into_iter().chain(
                              if num_remaining_columns > 0 {
                                  smallvec![
                                      soft! {
                                          %Text {
                                              " ".repeat(usize::from(num_remaining_columns))
                                          }
                                      }.into_text_child()
                                  ]
                              } else {
                                  SmallVec::<_, 1>::default()
                              }
                          ).collect()
                      }
                      color => color
                }
            }
            RelativeOrCurrentLineNum::Relative(relative_line_num) => {
                let num_columns_taken_up = num_columns_taken_up(usize::from(relative_line_num));
                let num_remaining_columns = self.num_columns - num_columns_taken_up;
                soft! {
                    %Text
                      children => {
                          if num_remaining_columns > 0 {
                              smallvec![
                                  soft! {
                                      %Text {
                                          " ".repeat(usize::from(num_remaining_columns))
                                      }
                                  }.into_text_child()
                              ]
                          } else {
                              SmallVec::<_, 1>::default()
                          }.into_iter().chain([
                              soft! {
                                  %Text relative_line_num
                              }.into_text_child()
                          ]).collect()
                      }
                      color => color
                }
            }
        })
    }
}

struct FoldLine<'a> {
    pub fold: &'a Fold,
    pub line: RopeSlice<'a>,
}

impl<'a> FoldLine<'a> {
    pub fn new(fold: &'a Fold, line: RopeSlice<'a>) -> Self {
        Self { fold, line }
    }
}

impl<'a> ComponentInterface for FoldLine<'a> {
    fn render(&self, grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        Ok(soft! {
            %Text
              children => {
                let mut children = smallvec![
                    soft! {
                        %Text "+--"
                    }.into_text_child()
                ];
                let mut num_bytes_printed_on_fold_line = 3;
                for _ in self.fold.num_closes..self.fold.full_num_indents {
                    children.push(soft! {
                        %Text "-"
                    }.into_text_child());
                    num_bytes_printed_on_fold_line += 1;
                }
                let printed_num_lines =
                    format_smolstr!("{}", self.fold.range.end - self.fold.range.start);
                if printed_num_lines.len() < 3 {
                    children.push(soft! {
                        %Text " "
                    }.into_text_child());
                    num_bytes_printed_on_fold_line += 1;
                }
                children.push(soft! {
                    %Text &printed_num_lines
                }.into_text_child());
                num_bytes_printed_on_fold_line += printed_num_lines.len();
                children.push(soft! {
                    %Text " lines: "
                }.into_text_child());
                num_bytes_printed_on_fold_line += 8;
                let mut has_passed_initial_blanks = false;
                for chunk in self.line.chunks() {
                    let to_print = if chunk.ends_with("\n") {
                        &chunk[..chunk.len() - 1]
                    } else {
                        chunk
                    };
                    let to_print = match has_passed_initial_blanks {
                        true => to_print,
                        false => match regex!(r#"^ +"#).find(to_print) {
                            None => {
                                has_passed_initial_blanks = true;
                                to_print
                            }
                            Some(initial_blanks) => {
                                if initial_blanks.len() == to_print.len() {
                                    continue;
                                }
                                has_passed_initial_blanks = true;
                                &to_print[initial_blanks.len()..]
                            }
                        },
                    };
                    let remaining_bytes_on_line =
                        usize::from(grid.width) - num_bytes_printed_on_fold_line;
                    if to_print.len() > remaining_bytes_on_line {
                        children.push(soft! {
                            %Text &to_print[..remaining_bytes_on_line]
                        }.into_text_child());
                        break;
                    }
                    children.push(soft! {
                        %Text to_print
                    }.into_text_child());
                    num_bytes_printed_on_fold_line += to_print.len();
                }
                children
              }
              color => known_colors()["dark_blue"]
        })
    }
}

enum RelativeOrCurrentLineNum {
    Relative(u16),
    Current(usize),
}

#[derive(Copy, Clone, Default)]
pub enum EventAggregator {
    #[default]
    Initial,
    SawZ,
    InExCommandMode,
}

impl ReceiveEvent<event::Event, Option<Event>> for EventAggregator {
    #[instrument(level = "trace", skip(self, event, _queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &event::Event,
        _queue_effect: TQueueEffect,
    ) -> Result<Option<Event>, anyhow::Error> {
        match (*self, event) {
            (Self::Initial, event) if is_simple_char_press(event, 'j') => {
                return Ok(Some(Event::MoveCursorDownNLines(1)));
            }
            (Self::Initial, event) if is_simple_char_press(event, 'k') => {
                return Ok(Some(Event::MoveCursorUpNLines(1)));
            }
            (Self::Initial, event) if is_simple_char_press(event, 'z') => {
                *self = Self::SawZ;
                return Ok(None);
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'o') => {
                *self = Self::Initial;
                return Ok(Some(Event::OpenFoldUnderCursorOneLevel));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'c') => {
                *self = Self::Initial;
                return Ok(Some(Event::CloseFoldUnderCursorOneLevel));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'O') => {
                *self = Self::Initial;
                return Ok(Some(Event::FullyOpenFoldUnderCursor));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'C') => {
                *self = Self::Initial;
                return Ok(Some(Event::FullyCloseFoldUnderCursor));
            }
            (_, event) if is_simple_key_press(event, KeyCode::Esc) => {
                *self = Self::Initial;
                return Ok(None);
            }
            (Self::Initial, event) if is_simple_char_press(event, ':') => {
                *self = Self::InExCommandMode;
                return Ok(Some(Event::GoIntoExCommandMode));
            }
            (Self::InExCommandMode, event) if is_any_simple_char_press(event).is_some() => {
                return Ok(Some(Event::ExCommandChar(
                    is_any_simple_char_press(event).unwrap(),
                )));
            }
            _ => panic!("unexpected event"),
        }
    }
}
