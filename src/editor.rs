use std::cmp;
use std::collections::HashMap;
use std::io::{stdout, StdoutLock, Write};
use std::path::PathBuf;
use std::sync::LazyLock;

use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode},
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{size, Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};
use ropey::Rope;
use smol_str::format_smolstr;
use squalid::{EverythingExt, _d, regex};
use tokio::fs;
use tokio_stream::StreamExt;

use crate::{
    strip_trailing_newline, Args, Fold, FoldIndex, IndentLevel, LineNumber, TreeSitterHighlight,
};

pub struct Editor {
    pub current_file: OpenFile,
    /// position on file-contents part of screen "grid",
    /// not in terms of file line # or actual terminal cursor
    /// position
    pub cursor_position: Position,
    pub stdout: StdoutLock<'static>,
    pub size: Size,
    pub top_line: Option<PrintedLine>,
    pub printed_lines: Option<Vec<PrintedLine>>,
    pub tree_sitter_parser: tree_sitter::Parser,
    pub current_tree_sitter_tree: Option<tree_sitter::Tree>,
    pub current_tree_sitter_highlights: Vec<TreeSitterHighlight>,
    // pub tree_sitter_highlighter: Highlighter,
    // pub tree_sitter_highlight_configuration: HighlightConfiguration,
    // pub tree_sitter_highlight_names: Vec<&'static str>,
    pub tree_sitter_highlight_query: tree_sitter::Query,
    pub tree_sitter_highlight_colors: Vec<Color>,
    pub current_file_shift_width: usize,
    pub current_file_indents: Option<Vec<IndentLevel>>,
    pub folds: Option<Vec<Fold>>,
    pub max_folds: Option<Vec<Fold>>,
}

impl Editor {
    pub fn try_new() -> Result<Self, anyhow::Error> {
        // let tree_sitter_highlight_names = vec!["comment", "string_literal"];
        Ok(Self {
            current_file: _d(),
            cursor_position: _d(),
            stdout: stdout().lock(),
            size: size()?.thrush(|(columns, rows)| Size {
                height: rows,
                width: columns,
            }),
            top_line: _d(),
            printed_lines: _d(),
            tree_sitter_parser: {
                let mut parser = tree_sitter::Parser::new();
                parser
                    .set_language(&tree_sitter_rust::LANGUAGE.into())
                    .unwrap();
                parser
            },
            current_tree_sitter_tree: _d(),
            current_tree_sitter_highlights: _d(),
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
            tree_sitter_highlight_query: tree_sitter::Query::new(
                &tree_sitter_rust::LANGUAGE.into(),
                r#"
                    (line_comment) @line_comment
                    (block_comment) @block_comment
                    (string_literal) @string_literal
                "#,
            )?,
            tree_sitter_highlight_colors: vec![
                known_colors()["dark_blue"],
                known_colors()["dark_blue"],
                known_colors()["yellow"],
            ],
            current_file_shift_width: 4,
            current_file_indents: _d(),
            folds: _d(),
            max_folds: _d(),
        })
    }

    pub async fn run(&mut self, args: Args) -> Result<(), anyhow::Error> {
        self.push_cursor_position()?;

        self.open_file(args.file_name).await?;

        let mut event_stream = EventStream::new();

        let mut in_progress_command: Vec<char> = _d();

        while let Some(Ok(event)) = event_stream.next().await {
            match event {
                Event::Key(key) => match key.code {
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

    async fn open_file(&mut self, file_name: PathBuf) -> Result<(), anyhow::Error> {
        let rope = Rope::from_str(strip_trailing_newline(
            &fs::read_to_string(&file_name).await?,
        ));
        self.current_file = OpenFile::Named(OpenFileNamed {
            rope,
            path: file_name,
        });

        self.current_tree_sitter_tree = Some(self.parse_tree_sitter_from_scratch());
        self.calculate_tree_sitter_highlights()?;

        self.set_current_file_indents();
        self.apply_initial_folds();
        self.top_line = Some(
            if matches!(
                self.folds.as_ref().unwrap().into_iter().next(),
                Some(fold) if fold.range.start == 0
            ) {
                PrintedLine::Fold(0)
            } else {
                PrintedLine::Line(0)
            },
        );
        self.compute_printed_lines();
        self.rerender_screen()?;

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

    pub fn compute_printed_lines(&mut self) {
        let num_lines = self.current_file.rope().len_lines();
        let top_line = self
            .top_line
            .unwrap()
            .start_line(self.folds.as_ref().unwrap());
        assert!(top_line <= num_lines - 1);

        let mut current_line_num = top_line;
        let mut next_eligible_fold_index = self
            .folds
            .as_ref()
            .unwrap()
            .into_iter()
            .position(|fold| fold.range.start >= top_line);
        let mut printed_lines: Vec<PrintedLine> = _d();
        for _ in 0..self.size.height {
            if current_line_num >= num_lines {
                break;
            }

            if let Some(next_eligible_fold_index_yes) =
                next_eligible_fold_index.filter(|&next_eligible_fold_index| {
                    self.folds.as_ref().unwrap()[next_eligible_fold_index]
                        .range
                        .start
                        == current_line_num
                })
            {
                printed_lines.push(PrintedLine::Fold(next_eligible_fold_index_yes));
                current_line_num = self.folds.as_ref().unwrap()[next_eligible_fold_index_yes]
                    .range
                    .end;
                if next_eligible_fold_index_yes < self.folds.as_ref().unwrap().len() - 1 {
                    next_eligible_fold_index = Some(next_eligible_fold_index_yes + 1);
                }
            } else {
                printed_lines.push(PrintedLine::Line(current_line_num));
                current_line_num += 1;
            }
        }
        self.printed_lines = Some(printed_lines);
    }

    pub fn rerender_screen(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.queue(Clear(ClearType::All))?;
        self.stdout.queue(cursor::SavePosition)?;
        self.stdout.queue(cursor::Hide)?;
        self.stdout.queue(cursor::MoveTo(0, 0))?;

        let num_lines = self.current_file.rope().len_lines();
        let top_line = self
            .top_line
            .unwrap()
            .start_line(self.folds.as_ref().unwrap());
        assert!(top_line <= num_lines - 1);

        let num_relative_line_number_columns = self.num_relative_line_number_columns();
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
        for printed_row_num in 0..self.printed_lines.as_ref().unwrap().len() {
            let printed_line = self.printed_lines.as_ref().unwrap()[printed_row_num];
            let printed_row_num = u16::try_from(printed_row_num).unwrap();
            let current_line_num = match printed_line {
                PrintedLine::Line(line) => line,
                PrintedLine::Fold(fold_index) => {
                    self.folds.as_ref().unwrap()[fold_index].range.start
                }
            };
            self.print_relative_line_number(
                printed_row_num,
                num_relative_line_number_columns,
                current_line_num,
            )?;
            match printed_line {
                PrintedLine::Fold(fold_index) => {
                    self.stdout
                        .queue(SetForegroundColor(known_colors()["dark_blue"]))?;
                    let fold = &self.folds.as_ref().unwrap()[fold_index];
                    self.stdout.queue(Print("+--"))?;
                    let mut num_bytes_printed_on_fold_line = 3;
                    for _ in fold.num_closes..fold.full_num_indents {
                        self.stdout.queue(Print("-"))?;
                        num_bytes_printed_on_fold_line += 1;
                    }
                    let printed_num_lines =
                        format_smolstr!("{}", fold.range.end - fold.range.start);
                    if printed_num_lines.len() < 3 {
                        self.stdout.queue(Print(" "))?;
                        num_bytes_printed_on_fold_line += 1;
                    }
                    self.stdout.queue(Print(&printed_num_lines))?;
                    num_bytes_printed_on_fold_line += printed_num_lines.len();
                    self.stdout.queue(Print(" lines: "))?;
                    num_bytes_printed_on_fold_line += 8;
                    let line = self.current_file.rope().line(fold.range.start);
                    let mut has_passed_initial_blanks = false;
                    for chunk in line.chunks() {
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
                            usize::from(self.size.width) - num_bytes_printed_on_fold_line;
                        if to_print.len() > remaining_bytes_on_line {
                            self.stdout
                                .queue(Print(&to_print[..remaining_bytes_on_line]))?;
                            break;
                        }
                        self.stdout.queue(Print(to_print))?;
                        num_bytes_printed_on_fold_line += to_print.len();
                    }
                }
                PrintedLine::Line(_) => {
                    let line = self.current_file.rope().line(current_line_num);

                    let mut current_start_byte =
                        self.current_file.rope().line_to_byte(current_line_num);
                    for chunk in line.chunks() {
                        let next_start_byte = current_start_byte + chunk.len();
                        let mut bytes_printed = 0;
                        if let OpenHighlightOrProgress::OpenHighlight(index_in_highlights) =
                            last_highlight
                        {
                            let open_highlight =
                                self.current_tree_sitter_highlights[index_in_highlights];
                            if open_highlight.end_byte < next_start_byte {
                                let num_bytes_to_print =
                                    open_highlight.end_byte - current_start_byte;
                                self.stdout.queue(Print(&chunk[..num_bytes_to_print]))?;
                                bytes_printed += num_bytes_to_print;
                                self.stdout.queue(ResetColor)?;
                                last_highlight =
                                    OpenHighlightOrProgress::Next(index_in_highlights + 1);
                            } else {
                                self.stdout.queue(Print(if chunk.ends_with("\n") {
                                    &chunk[..chunk.len() - 1]
                                } else {
                                    chunk
                                }))?;
                                current_start_byte = next_start_byte;
                                continue;
                            }
                        }
                        'more_highlights: while !matches!(
                            last_highlight,
                            OpenHighlightOrProgress::Next(last_highlight_next)
                                if last_highlight_next >= self.current_tree_sitter_highlights.len()
                                    || self.current_tree_sitter_highlights[last_highlight_next].start_byte >= next_start_byte
                        ) && !matches!(
                            last_highlight,
                            OpenHighlightOrProgress::OpenHighlight(last_highlight_open)
                                if self.current_tree_sitter_highlights[last_highlight_open].end_byte >= next_start_byte
                        ) {
                            match last_highlight {
                                OpenHighlightOrProgress::OpenHighlight(last_highlight_open) => {
                                    let open_highlight =
                                        self.current_tree_sitter_highlights[last_highlight_open];
                                    let num_bytes_to_print = open_highlight.end_byte
                                        - (current_start_byte + bytes_printed);
                                    self.stdout.queue(Print(
                                        &chunk[bytes_printed..bytes_printed + num_bytes_to_print],
                                    ))?;
                                    bytes_printed += num_bytes_to_print;
                                    self.stdout.queue(ResetColor)?;
                                    last_highlight =
                                        OpenHighlightOrProgress::Next(last_highlight_open + 1);
                                }
                                OpenHighlightOrProgress::Next(last_highlight_next) => {
                                    let next_highlight =
                                        self.current_tree_sitter_highlights[last_highlight_next];
                                    while next_highlight.end_byte <= current_start_byte {
                                        last_highlight =
                                            OpenHighlightOrProgress::Next(last_highlight_next + 1);
                                        continue 'more_highlights;
                                    }
                                    let num_bytes_to_print = next_highlight.start_byte
                                        - (current_start_byte + bytes_printed);
                                    self.stdout.queue(Print(
                                        &chunk[bytes_printed..bytes_printed + num_bytes_to_print],
                                    ))?;
                                    bytes_printed += num_bytes_to_print;
                                    self.stdout.queue(SetForegroundColor(
                                        self.tree_sitter_highlight_colors
                                            [next_highlight.highlight_type_index],
                                    ))?;
                                    last_highlight =
                                        OpenHighlightOrProgress::OpenHighlight(last_highlight_next);
                                }
                            }
                        }
                        if chunk.ends_with("\n") {
                            if bytes_printed < chunk.len() - 1 {
                                self.stdout
                                    .queue(Print(&chunk[bytes_printed..chunk.len() - 1]))?;
                            }
                        } else {
                            if bytes_printed < chunk.len() {
                                self.stdout.queue(Print(&chunk[bytes_printed..]))?;
                            }
                        }
                        current_start_byte = next_start_byte;
                    }
                }
            }

            if printed_row_num != self.size.height - 1 {
                self.stdout.queue(Print("\r\n"))?;
            }
        }

        self.stdout.queue(cursor::RestorePosition)?;
        self.stdout.queue(cursor::Show)?;

        self.stdout.flush()?;

        Ok(())
    }

    fn print_relative_line_number(
        &mut self,
        printed_row_num: RowOrColumnNumber,
        num_relative_line_number_columns: RowOrColumnNumber,
        line_number_to_show_if_cursor_line: LineNumber,
    ) -> Result<(), anyhow::Error> {
        self.stdout.queue(SetForegroundColor(Color::Rgb {
            r: 122,
            g: 122,
            b: 122,
        }))?;
        if self.cursor_position.row == printed_row_num {
            let num_columns_taken_up = num_columns_taken_up(line_number_to_show_if_cursor_line + 1);
            self.stdout
                .queue(Print(line_number_to_show_if_cursor_line + 1))?;
            for _blank_column in 0..num_relative_line_number_columns - num_columns_taken_up {
                self.stdout.queue(Print(" "))?;
            }
        } else {
            let relative_line_number = usize::try_from(
                (i32::try_from(self.cursor_position.row).unwrap()
                    - i32::try_from(printed_row_num).unwrap())
                .abs(),
            )
            .unwrap();
            let num_columns_taken_up = num_columns_taken_up(relative_line_number);
            for _blank_column in 0..num_relative_line_number_columns - num_columns_taken_up {
                self.stdout.queue(Print(" "))?;
            }
            self.stdout.queue(Print(relative_line_number))?;
        };
        self.stdout.queue(ResetColor)?;
        self.stdout.queue(Print(" "))?;

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
