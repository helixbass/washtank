use std::cmp;
use std::fs::OpenOptions;
use std::io::{stdout, StdoutLock, Write};
use std::path::PathBuf;

use clap::Parser;
use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{
        disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand, QueueableCommand,
};
use ouroboros::self_referencing;
use ropey::{Rope, RopeSlice};
use squalid::{EverythingExt, _d, regex};
use tokio::fs;
use tokio_stream::StreamExt;
// use tree_sitter_highlight::{HighlightConfiguration, Highlighter};
use tree_sitter::StreamingIterator;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;

    Editor::try_new()?.run().await?;

    execute!(stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

#[derive(Parser)]
struct Args {
    pub file_name: PathBuf,
}

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
    fn try_new() -> Result<Self, anyhow::Error> {
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
                Color::Rgb {
                    r: 47,
                    g: 47,
                    b: 255,
                },
                Color::Rgb {
                    r: 47,
                    g: 47,
                    b: 255,
                },
                Color::Rgb {
                    r: 240,
                    g: 240,
                    b: 0,
                },
            ],
            current_file_shift_width: 4,
            current_file_indents: _d(),
            folds: _d(),
            max_folds: _d(),
        })
    }

    async fn run(&mut self) -> Result<(), anyhow::Error> {
        let args = Args::parse();

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

    fn set_current_file_indents(&mut self) {
        self.current_file_indents = Some(calculate_indents(
            self.current_file.rope(),
            self.current_file_shift_width,
        ));
    }

    fn apply_initial_folds(&mut self) {
        self.folds = Some(calculate_folds(self.current_file_indents.as_ref().unwrap()));
        self.max_folds = self.folds.clone();
    }

    fn parse_tree_sitter_from_scratch(&mut self) -> tree_sitter::Tree {
        self.tree_sitter_parser
            .parse_with_options(
                &mut |byte_offset, _| {
                    let (chunk, chunk_start_byte_index, _, _) =
                        self.current_file.rope().chunk_at_byte(byte_offset);
                    &chunk[byte_offset - chunk_start_byte_index..]
                },
                None,
                None,
            )
            .unwrap()
    }

    fn push_cursor_position(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.execute(cursor::MoveTo(
            self.cursor_position.column + self.num_relative_line_number_columns() + 1,
            self.cursor_position.row,
        ))?;

        Ok(())
    }

    fn calculate_tree_sitter_highlights(&mut self) -> Result<(), anyhow::Error> {
        // self.current_tree_sitter_highlights = self
        //     .tree_sitter_highlighter
        //     .highlight(&self.tree_sitter_highlight_configuration)?
        //     .collect::<Result<_, _>>()?;
        let mut query_cursor = tree_sitter::QueryCursor::new();
        let mut captures = query_cursor.captures(
            &self.tree_sitter_highlight_query,
            self.current_tree_sitter_tree.as_ref().unwrap().root_node(),
            RopeWrapper(self.current_file.rope()),
        );
        let mut ret: Vec<TreeSitterHighlight> = _d();
        while let Some(capture) = captures.next() {
            ret.push(TreeSitterHighlight {
                start_byte: capture.0.captures[0].node.start_byte(),
                end_byte: capture.0.captures[0].node.end_byte(),
                highlight_type_index: capture.0.pattern_index,
            });
        }
        self.current_tree_sitter_highlights = ret;

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

    fn compute_printed_lines(&mut self) {
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

    fn rerender_screen(&mut self) -> Result<(), anyhow::Error> {
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
                PrintedLine::Fold(_) => {
                    self.stdout.queue(Print("FOLD"))?;
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

    fn fully_open_fold_under_cursor(&mut self) -> Result<(), anyhow::Error> {
        let PrintedLine::Fold(fold_index) =
            self.printed_lines.as_ref().unwrap()[usize::from(self.cursor_position.row)]
        else {
            return Ok(());
        };
        let _ = self.folds.as_mut().unwrap().remove(fold_index);
        self.compute_printed_lines();
        self.rerender_screen()?;
        Ok(())
    }

    fn open_fold_under_cursor_one_level(&mut self) -> Result<(), anyhow::Error> {
        let PrintedLine::Fold(fold_index) =
            self.printed_lines.as_ref().unwrap()[usize::from(self.cursor_position.row)]
        else {
            return Ok(());
        };
        if self.folds.as_ref().unwrap()[fold_index].num_indents == 1 {
            let fold = self.folds.as_mut().unwrap().remove(fold_index);
            let mut nested = fold.nested;
            for nested in &mut nested {
                decrement_fold_num_indents(nested);
            }
            let _ = self
                .folds
                .as_mut()
                .unwrap()
                .splice(fold_index..fold_index, nested);
        } else {
            decrement_fold_num_indents(self.folds.as_mut().unwrap().get_mut(fold_index).unwrap());
        }

        self.compute_printed_lines();
        self.rerender_screen()?;
        Ok(())
    }

    fn fully_close_fold_under_cursor(&mut self) -> Result<(), anyhow::Error> {
        let start_line = self.printed_lines.as_ref().unwrap()
            [usize::from(self.cursor_position.row)]
        .start_line(self.folds.as_ref().unwrap());

        let Some(fold_index) = self
            .max_folds
            .as_ref()
            .unwrap()
            .into_iter()
            .position(|fold| fold.range.start <= start_line && fold.range.end > start_line)
        else {
            return Ok(());
        };

        let max_fold = &self.max_folds.as_ref().unwrap()[fold_index];

        let first_fold_index_to_replace =
            self.folds.as_ref().unwrap().into_iter().position(|fold| {
                fold.range.start >= max_fold.range.start && fold.range.end <= max_fold.range.end
            });
        let additional_count_to_replace =
            first_fold_index_to_replace.map(|first_fold_index_to_replace| {
                self.folds.as_ref().unwrap()[first_fold_index_to_replace..]
                    .into_iter()
                    .take_while(|fold| fold.range.end <= max_fold.range.end)
                    .count()
            });
        let range_to_splice = match first_fold_index_to_replace {
            None => {
                let first_after = self
                    .folds
                    .as_ref()
                    .unwrap()
                    .into_iter()
                    .position(|fold| fold.range.start >= max_fold.range.end);
                match first_after {
                    Some(first_after) => first_after..first_after,
                    None => {
                        let folds = self.folds.as_ref().unwrap();
                        folds.len()..folds.len()
                    }
                }
            }
            Some(first_fold_index_to_replace) => {
                first_fold_index_to_replace
                    ..first_fold_index_to_replace + additional_count_to_replace.unwrap() + 1
            }
        };
        self.folds
            .as_mut()
            .unwrap()
            .splice(range_to_splice, [max_fold.clone()]);

        self.compute_printed_lines();
        let new_cursor_position_row = self
            .printed_lines
            .as_ref()
            .unwrap()
            .into_iter()
            .position(|printed_line| {
                printed_line.start_line(self.folds.as_ref().unwrap())
                    == self.max_folds.as_ref().unwrap()[fold_index].range.start
            })
            .unwrap();
        self.cursor_position.row = u16::try_from(new_cursor_position_row).unwrap();
        self.push_cursor_position()?;

        self.rerender_screen()?;
        Ok(())
    }

    fn close_fold_under_cursor_one_level(&mut self) -> Result<(), anyhow::Error> {
        unimplemented!()
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

#[allow(dead_code)]
fn log(str: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open("dev.log")
        .unwrap();

    writeln!(file, "{str}").unwrap();
}

fn get_indent_level(line: RopeSlice, shift_width: usize) -> IndentLevel {
    let mut spaces_seen_so_far = 0;
    for chunk in line.chunks() {
        if let Some(match_) = regex!(r#"^[ \n]+"#).find(chunk) {
            spaces_seen_so_far += match_.len();
            if match_.len() == chunk.len() {
                continue;
            } else {
                return IndentLevel::Level(spaces_seen_so_far.div_ceil(shift_width));
            }
        } else {
            return IndentLevel::Level(spaces_seen_so_far.div_ceil(shift_width));
        }
    }
    IndentLevel::BlankLine
}

struct InProgressFold {
    pub start_line: usize,
    pub num_indents: usize,
    pub nested: Vec<Fold>,
    pub open_nested: Option<Box<InProgressFold>>,
}

fn to_fold(in_progress: InProgressFold, one_past_line_num: usize) -> Fold {
    let mut nested = in_progress.nested;
    if let Some(open_nested) = in_progress.open_nested {
        nested.push(to_fold(*open_nested, one_past_line_num));
    }
    Fold {
        range: Range {
            start: in_progress.start_line,
            end: one_past_line_num,
        },
        num_indents: in_progress.num_indents,
        nested,
    }
}

fn nest_myself_with_new_lesser_indent(
    me: InProgressFold,
    new_lesser_indent: usize,
    line_num: usize,
) -> InProgressFold {
    InProgressFold {
        start_line: me.start_line,
        num_indents: new_lesser_indent,
        nested: vec![to_fold(me, line_num)],
        open_nested: None,
    }
}

fn apply_more_indented(indent: usize, line_num: usize, in_progress: &mut InProgressFold) {
    if in_progress.open_nested.is_none() {
        in_progress.open_nested = Some(Box::new(InProgressFold {
            start_line: line_num,
            num_indents: indent,
            nested: _d(),
            open_nested: _d(),
        }));
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents == indent {
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents < indent {
        apply_more_indented(indent, line_num, in_progress.open_nested.as_mut().unwrap());
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents > indent {
        let prev_open_nested = in_progress.open_nested.take().unwrap();
        in_progress.open_nested = Some(Box::new(nest_myself_with_new_lesser_indent(
            *prev_open_nested,
            indent,
            line_num,
        )));
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IndentLevel {
    Level(usize),
    BlankLine,
}

type FoldIndex = usize;

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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: LineNumber,
    pub end: LineNumber,
}

type LineNumber = usize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub range: Range,
    pub num_indents: usize,
    pub nested: Vec<Fold>,
}

fn decrement_fold_num_indents(fold: &mut Fold) {
    fold.num_indents -= 1;
    for nested in &mut fold.nested {
        decrement_fold_num_indents(nested);
    }
}

fn calculate_indents(rope: &Rope, shift_width: usize) -> Vec<IndentLevel> {
    rope.lines()
        .map(|line| get_indent_level(line, shift_width))
        .collect()
}

fn strip_trailing_newline(file_contents: &str) -> &str {
    if file_contents.ends_with("\n") {
        &file_contents[..file_contents.len() - 1]
    } else {
        file_contents
    }
}

fn calculate_folds(indents: &[IndentLevel]) -> Vec<Fold> {
    let mut folds: Vec<Fold> = _d();
    let mut in_progress: Option<InProgressFold> = _d();
    for (line_num, &indent) in indents.into_iter().enumerate() {
        if in_progress.is_none() {
            match indent {
                IndentLevel::BlankLine => continue,
                IndentLevel::Level(level) if level == 0 => continue,
                IndentLevel::Level(indent) => {
                    in_progress = Some(InProgressFold {
                        start_line: line_num,
                        num_indents: indent,
                        nested: _d(),
                        open_nested: _d(),
                    });
                }
            }
        } else {
            if indent == IndentLevel::Level(0) {
                folds.push(to_fold(in_progress.take().unwrap(), line_num));
                continue;
            }
            match indent {
                IndentLevel::BlankLine => {}
                IndentLevel::Level(indent)
                    if indent == in_progress.as_ref().unwrap().num_indents =>
                {
                    if in_progress.as_ref().unwrap().open_nested.is_some() {
                        let open_nested = in_progress.as_mut().unwrap().open_nested.take().unwrap();
                        in_progress
                            .as_mut()
                            .unwrap()
                            .nested
                            .push(to_fold(*open_nested, line_num));
                    }
                }
                IndentLevel::Level(indent)
                    if indent < in_progress.as_ref().unwrap().num_indents =>
                {
                    let prev_in_progress = in_progress.take().unwrap();
                    in_progress = Some(nest_myself_with_new_lesser_indent(
                        prev_in_progress,
                        indent,
                        line_num,
                    ));
                }
                IndentLevel::Level(indent) => {
                    apply_more_indented(indent, line_num, in_progress.as_mut().unwrap());
                }
            }
        }
    }
    if let Some(in_progress) = in_progress {
        folds.push(to_fold(in_progress, indents.len()));
    }
    folds
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    fn calculate_indent_test(text: &str, expected: Vec<IndentLevel>) {
        assert_eq!(
            calculate_indents(&Rope::from(strip_trailing_newline(text)), 4),
            expected
        );
    }

    #[test]
    fn test_calculate_indents() {
        calculate_indent_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                }
            "#
            ),
            vec![
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
            ],
        );

        calculate_indent_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                }

                trait Foo {
                    fn whee() -> Whee;
                }
            "#
            ),
            vec![
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
                IndentLevel::BlankLine,
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
            ],
        );
    }

    fn calculate_folds_test(text: &str, expected: Vec<Fold>) {
        assert_eq!(
            calculate_folds(&calculate_indents(
                &Rope::from(strip_trailing_newline(text)),
                4
            )),
            expected
        );
    }

    #[test]
    fn test_calculate_folds() {
        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 3 },
                num_indents: 1,
                nested: vec![],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";

                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 4 },
                num_indents: 1,
                nested: vec![],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                        "foo";
                        "foo";
                    "foo";
                    "foo";
                        "foo";
                        "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 7 },
                num_indents: 1,
                nested: vec![
                    Fold {
                        range: Range { start: 1, end: 3 },
                        num_indents: 2,
                        nested: vec![],
                    },
                    Fold {
                        range: Range { start: 5, end: 7 },
                        num_indents: 2,
                        nested: vec![],
                    },
                ],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                    "foo";
                        "foo";
                        "foo";
                    "foo";
                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 7 },
                num_indents: 1,
                nested: vec![Fold {
                    range: Range { start: 3, end: 5 },
                    num_indents: 2,
                    nested: vec![],
                }],
            }],
        );
    }
}
