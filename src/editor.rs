use std::cmp;
use std::collections::HashMap;
use std::io::{stdout, StdoutLock, Write};
use std::path::PathBuf;
use std::process;
use std::sync::LazyLock;

use anyhow;
use crossterm::{
    cursor,
    event::{self, KeyCode},
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{size, Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use lsp_types::{ClientInfo, InitializeParams};
use oelung::{soft, Component, ComponentInterface, Grid};
use ropey::Rope;
use smallvec::{smallvec, SmallVec};
use smol_str::format_smolstr;
use squalid::{EverythingExt, _d, regex};
use tokio::{fs, sync::mpsc::channel};

use crate::{
    calculate_folds, calculate_indents, listen_to_crossterm_events, run_rust_analyzer,
    strip_trailing_newline,
    tree_sitter::{self as tree_sitter_mod, calculate_highlights},
    Args, Fold, FoldIndex, IndentLevel, LineNumber, LspIncomingMessage, LspOutgoingMessage,
    TreeSitterHighlight,
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
            path: file_name,
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
            &folds,
        );

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

    pub async fn run(&mut self, args: Args) -> Result<(), anyhow::Error> {
        self.push_cursor_position()?;

        self.open_file(args.file_name).await?;

        let (sender, mut receiver) = channel::<World>(100);

        listen_to_crossterm_events(sender.clone());

        let (rust_analyzer_sender, rust_analyzer_receiver) = channel::<LspOutgoingMessage>(100);

        run_rust_analyzer(sender.clone(), rust_analyzer_receiver);

        // rust_analyzer_sender
        //     .send(LspOutgoingMessage::Initialize(InitializeParams {
        //         // TODO: is std::process:id() blocking aka shouldn't use it
        //         // from tokio?
        //         process_id: Some(process::id()),
        //         client_info: Some(ClientInfo {
        //             name: "washtank".to_owned(),
        //             // TODO: make this real?
        //             version: Some("0.0.1-dev.0".to_owned()),
        //         }),
        //     }))
        //     .unwrap();

        let mut in_progress_command: Vec<char> = _d();

        while let Some(world) = receiver.recv().await {
            match world {
                World::Crossterm(Event::Key(key)) => match key.code {
                    KeyCode::Char('j') => {
                        assert!(in_progress_command.is_empty());
                        self.maybe_move_cursor_down_one_line()?;
                    }
                    KeyCode::Char('k') => {
                        assert!(in_progress_command.is_empty());
                        self.maybe_move_cursor_up_one_line()?;
                    }
                    KeyCode::Char('z') => {
                        assert!(in_progress_command.is_empty());
                        in_progress_command.push('z');
                    }
                    KeyCode::Char('O') => {
                        assert!(in_progress_command.len() == 1 && in_progress_command[0] == 'z');
                        self.fully_open_fold_under_cursor()?;
                        in_progress_command.clear();
                    }
                    KeyCode::Char('o') => {
                        assert!(in_progress_command.len() == 1 && in_progress_command[0] == 'z');
                        self.open_fold_under_cursor_one_level()?;
                        in_progress_command.clear();
                    }
                    KeyCode::Char('C') => {
                        assert!(in_progress_command.len() == 1 && in_progress_command[0] == 'z');
                        self.fully_close_fold_under_cursor()?;
                        in_progress_command.clear();
                    }
                    KeyCode::Char('c') => {
                        assert!(in_progress_command.len() == 1 && in_progress_command[0] == 'z');
                        self.close_fold_under_cursor_one_level()?;
                        in_progress_command.clear();
                    }
                    _ => unimplemented!(),
                },
                _ => unimplemented!(),
            }
        }

        Ok(())
    }

    pub fn push_cursor_position(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.execute(cursor::MoveTo(
            self.cursor_position.column + self.num_relative_line_number_columns() + 1,
            self.cursor_position.row,
        ))?;

        Ok(())
    }

    fn one_past_final_last_printed_row_line_number(&self) -> usize {
        let printed_lines = self.printed_lines.as_ref().unwrap();
        match printed_lines[printed_lines.len() - 1] {
            PrintedLine::Fold(fold_index) => self.folds.as_ref().unwrap()[fold_index].range.end,
            PrintedLine::Line(line) => line + 1,
        }
    }

    fn maybe_move_cursor_down_one_line(&mut self) -> Result<(), anyhow::Error> {
        if self.cursor_position.row == self.size.height - 1 {
            if self.one_past_final_last_printed_row_line_number()
                == self.current_file.rope().len_lines()
            {
                return Ok(());
            }
            let first_line_of_new_top_line = match self.top_line.unwrap() {
                PrintedLine::Line(line) => line + 1,
                PrintedLine::Fold(fold_index) => self.folds.as_ref().unwrap()[fold_index].range.end,
            };
            self.top_line = Some(
                match self
                    .folds
                    .as_ref()
                    .unwrap()
                    .into_iter()
                    .position(|fold| fold.range.start == first_line_of_new_top_line)
                {
                    Some(fold_index) => PrintedLine::Fold(fold_index),
                    None => PrintedLine::Line(first_line_of_new_top_line),
                },
            );
            self.compute_printed_lines();
        } else {
            self.cursor_position.row += 1;
            self.push_cursor_position()?;
        }
        self.rerender_screen()?;

        Ok(())
    }

    fn maybe_move_cursor_up_one_line(&mut self) -> Result<(), anyhow::Error> {
        if self.cursor_position.row == 0 {
            let top_line_start_line = self
                .top_line
                .unwrap()
                .start_line(self.folds.as_ref().unwrap());
            if top_line_start_line == 0 {
                return Ok(());
            }

            self.top_line = Some(
                match self
                    .folds
                    .as_ref()
                    .unwrap()
                    .into_iter()
                    .position(|fold| fold.range.end == top_line_start_line - 1)
                {
                    Some(fold_index) => PrintedLine::Fold(fold_index),
                    None => PrintedLine::Line(top_line_start_line - 1),
                },
            );
            self.compute_printed_lines();
        } else {
            self.cursor_position.row -= 1;
            self.push_cursor_position()?;
        }
        self.rerender_screen()?;

        Ok(())
    }

    fn num_relative_line_number_columns(&self) -> RowOrColumnNumber {
        cmp::max(
            3,
            num_columns_taken_up(self.current_file.rope().len_lines()),
        )
    }
}

impl<'a> ComponentInterface for &'a Editor {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        let num_relative_line_number_columns = self.num_relative_line_number_columns();

        Ok(soft! {
            %FlexColumn
              children => self.printed_line_chunks.iter().enumerate().map(|(printed_row_num, printed_line_chunks)| {
                  let printed_row_num = u16::try_from(printed_row_num).unwrap();
                  let line_num = printed_line_chunks.start_line(&self.folds);
                  let relative_line_number = soft! {
                      %RelativeLineNumber::new(
                          num_relative_line_number_columns,
                          line_num,
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
                  match printed_line_chunks {
                      PrintedLineChunks::Fold(fold_index) => soft! {
                          %Text children => [
                            relative_line_number
                            %FoldLine::new(&self.folds[fold_index])
                          ]
                      },
                      PrintedLineChunks::Line(line_num, line_chunks) => {
                          let line = self.current_file.rope().line(line_num);
                          let chunks = line.chunks.collect::<SmallVec<_, 10>>();
                          soft! {
                              %Text children => {
                                  [relative_line_number].into_iter().chain(
                                      line_chunks.map(|line_chunk| {
                                          soft! {
                                              %Text
                                                text => &chunks[line_chunk.chunk_index][line_chunk.chunk_start_byte..line_chunk.chunk_end_byte]
                                                color => self.tree_sitter_highlight_colors[line_chunk.highlight_type_index]
                                          }
                                      })
                                  ).collect()
                              }
                          }
                      }
                  }
              }).collect()
              overflow_y => hidden
        })
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
    Crossterm(event::Event),
    Lsp(LspIncomingMessage),
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

enum PrintedLineChunks {
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

struct LineChunk {
    pub chunk_index: usize,
    pub chunk_start_byte: usize,
    pub chunk_end_byte: usize,
    pub highlight_type_index: Option<usize>,
}

fn compute_printed_line_chunks(
    printed_lines: &[PrintedLine],
    rope: &Rope,
    tree_sitter_highlights: &[TreeSitterHighlight],
    folds: &[Fold],
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
    printed_lines.map(|printed_line| {
        match printed_line {
            PrintedLine::Fold(fold_index) => PrintedLineChunks::Fold(fold_index),
            PrintedLine::Line(line_num) => {
                let line = rope.line(line_num);

                let mut line_chunks = _d();
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
            }
        }
    }).collect()
}

struct RelativeLineNumber {
    pub num_columns: usize,
    pub relative_or_current_line_num: RelativeOrCurrentLineNum,
}

impl RelativeLineNumber {
    pub fn new(num_columns: usize, relative_or_current_line_num: RelativeOrCurrentLineNum) -> Self {
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
                              }
                          ].into_iter().chain(
                              if num_remaining_columns > 0 {
                                  smallvec![
                                      soft! {
                                          %Text {
                                              " ".repeat(num_remaining_columns)
                                          }
                                      }
                                  ]
                              } else {
                                  smallvec![]
                              }
                          ).collect()
                      }
                      color => color,
                }
            }
            RelativeOrCurrentLineNum::Relative(relative_line_num) => {
                let num_columns_taken_up = num_columns_taken_up(relative_line_num);
                let num_remaining_columns = self.num_columns - num_columns_taken_up;
                soft! {
                    %Text
                      children => {
                          if num_remaining_columns > 0 {
                              smallvec![
                                  soft! {
                                      %Text {
                                          " ".repeat(num_remaining_columns)
                                      }
                                  }
                              ]
                          } else {
                              smallvec![]
                          }.chain([
                              soft! {
                                  %Text relative_line_num
                              }
                          ]).collect()
                      }
                      color => color
                }
            }
        })
    }
}

enum RelativeOrCurrentLineNum {
    Relative(u16),
    Current(u16),
}
