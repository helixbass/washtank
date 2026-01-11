use std::pin::Pin;

use crossterm::style::Color;
use oelung::{soft, Component, ComponentInterface, Grid};
use oelung_lantern::ReceiveEvent;
use ropey::RopeSlice;
use smallvec::{smallvec, SmallVec};
use smol_str::format_smolstr;
use squalid::{_d, regex};
use tracing::instrument;

use super::{
    known_colors, num_columns_taken_up, Event, Happened, Mode, OpenFile, PrintedLineChunks,
    RowOrColumnNumber,
};
use crate::{Editor, Fold, LineNumber, LspIncomingMessage};

impl<'a> ComponentInterface for &'a Editor {
    fn render(&self, grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        self.last_rendered_grid.set(Some(grid));
        let current_percent = ((f64::from(self.cursor_position.row)
            / f64::from(u32::try_from(self.current_file.rope().len_lines()).unwrap()))
            * 100.0) as u32;
        Ok(soft! {
            %FlexColumn children => [
              %EditorGrid::new(self)
              %StatusLine::new(
                  current_percent,
                  match &self.current_file {
                      OpenFile::Anonymous(_) => None,
                      OpenFile::Named(named) => Some(named.path.file_name().unwrap().to_str().unwrap()),
                  },
                  self.current_file.rope().len_lines(),
                  self.cursor_position.column + 1,
              )
              %CommandLine::new(
                &self.mode
              )
            ]
        })
    }

    fn flex_grow(&self) -> Option<f64> {
        self.flex_grow
    }
}

impl ReceiveEvent<Event> for Editor {
    #[instrument(level = "trace", skip(self, event, queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &Event,
        queue_effect: TQueueEffect,
    ) -> Result<(), anyhow::Error> {
        match event {
            Event::MoveCursorDownNLines(n) => {
                self.maybe_move_cursor_down_n_lines(*n);
                Ok(())
            }
            Event::MoveCursorUpNLines(n) => {
                assert_eq!(*n, 1);
                self.maybe_move_cursor_up_one_line();
                // self.maybe_move_cursor_up_n_lines(n);
                Ok(())
            }
            Event::MoveCursorRightNColumns(n) => {
                assert_eq!(*n, 1);
                self.maybe_move_cursor_right_one_column();
                // self.maybe_move_cursor_right_n_columns(n);
                Ok(())
            }
            Event::MoveCursorLeftNColumns(n) => {
                assert_eq!(*n, 1);
                self.maybe_move_cursor_left_one_column();
                // self.maybe_move_cursor_left_n_columns(n);
                Ok(())
            }
            Event::MoveCursorToBeginningOfLine => {
                self.move_cursor_to_beginning_of_line();
                Ok(())
            }
            Event::MoveCursorToEndOfLine => {
                self.move_cursor_to_end_of_line();
                Ok(())
            }
            Event::FullyOpenFoldUnderCursor => {
                assert!(!self.disallow_folding);
                self.fully_open_fold_under_cursor();
                Ok(())
            }
            Event::OpenFoldUnderCursorOneLevel => {
                assert!(!self.disallow_folding);
                self.open_fold_under_cursor_one_level();
                Ok(())
            }
            Event::FullyCloseFoldUnderCursor => {
                assert!(!self.disallow_folding);
                self.fully_close_fold_under_cursor();
                Ok(())
            }
            Event::CloseFoldUnderCursorOneLevel => {
                assert!(!self.disallow_folding);
                self.close_fold_under_cursor_one_level();
                Ok(())
            }
            Event::GoIntoNormalMode => {
                if self.mode == Mode::Insert {
                    if self.cursor_position.column > 0 {
                        self.cursor_position.column -= 1;
                    }
                }
                self.mode = Mode::Normal;
                Ok(())
            }
            Event::GoIntoExCommandMode => {
                assert!(!self.disallow_ex_command_mode);
                self.mode = Mode::ExCommand(_d());
                Ok(())
            }
            Event::ExCommandChar(ch) => {
                assert!(!self.disallow_ex_command_mode);
                self.mode.as_ex_command_mut().push(*ch);
                Ok(())
            }
            Event::FinishExCommand => {
                assert!(!self.disallow_ex_command_mode);
                self.finish_ex_command(queue_effect);
                Ok(())
            }
            Event::GoIntoInsertMode => {
                self.mode = Mode::Insert;
                Ok(())
            }
            Event::InsertChar(ch) => {
                self.insert_char(*ch)?;
                Ok(())
            }
            Event::HighlightRange(range) => {
                self.highlight_range = Some(*range);
                self.recompute_on_highlights_or_content_changed()?;
                Ok(())
            }
            Event::UnhighlightRange => {
                self.highlight_range = None;
                self.recompute_on_highlights_or_content_changed()?;
                Ok(())
            }
        }
    }
}

impl ReceiveEvent<Happened> for Editor {
    #[instrument(level = "trace", skip(self, event, _queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &Happened,
        _queue_effect: TQueueEffect,
    ) -> Result<(), anyhow::Error> {
        Ok(match event {
            Happened::Quit => panic!("shouldn't get passed quit"),
            Happened::Lsp(lsp_incoming_message) => match lsp_incoming_message {
                LspIncomingMessage::InitializeResult(_) => {}
            },
        })
    }
}

struct EditorGrid<'a> {
    pub editor: &'a Editor,
}

impl<'a> EditorGrid<'a> {
    pub fn new(editor: &'a Editor) -> Self {
        Self { editor }
    }
}

impl<'a> ComponentInterface for EditorGrid<'a> {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        let num_relative_line_number_columns = self.editor.num_relative_line_number_columns();

        Ok(soft! {
            %FlexColumn
              children => self.editor.printed_line_chunks.iter().enumerate().map(|(printed_row_num, printed_line_chunks)| -> Result<_, anyhow::Error> {
                  let printed_row_num = u16::try_from(printed_row_num).unwrap();
                  let line_num = printed_line_chunks.start_line(&self.editor.folds);
                  let relative_line_number = soft! {
                      %RelativeLineNumber::new(
                          num_relative_line_number_columns,
                          match self.editor.cursor_position.row == printed_row_num {
                              true => RelativeOrCurrentLineNum::Current(line_num),
                              false => RelativeOrCurrentLineNum::Relative(
                                    u16::try_from(
                                        (i32::try_from(self.editor.cursor_position.row).unwrap()
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
                                let fold = &self.editor.folds[*fold_index];
                                FoldLine::new(
                                    fold,
                                    self.editor.current_file.rope().line(fold.range.start),
                                )
                            }
                          ]
                      },
                      PrintedLineChunks::Line(line_num, line_chunks) => {
                          let line_num = *line_num;
                          let line = self.editor.current_file.rope().line(line_num);
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
                                                maybe_color => line_chunk.style.as_ref().and_then(|style| {
                                                    style.foreground_color.clone()
                                                })
                                                maybe_background_color => line_chunk.style.as_ref().and_then(|style| {
                                                    style.background_color.clone()
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
                    self.editor.cursor_position.column + self.editor.num_relative_line_number_columns() + 1
                }
                y => self.editor.cursor_position.row
        })
    }

    fn flex_grow(&self) -> Option<f64> {
        Some(1.0)
    }
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
    Current(LineNumber),
}

struct StatusLine<'a> {
    pub percent: u32,
    pub file_name: Option<&'a str>,
    pub num_lines: LineNumber,
    pub column: RowOrColumnNumber,
}

impl<'a> StatusLine<'a> {
    pub fn new(
        percent: u32,
        file_name: Option<&'a str>,
        num_lines: LineNumber,
        column: RowOrColumnNumber,
    ) -> Self {
        Self {
            percent,
            file_name,
            num_lines,
            column,
        }
    }
}

impl<'a> ComponentInterface for StatusLine<'a> {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        Ok(soft! {
          %Text
            children => [
              %Text self.file_name.unwrap_or("[No Name]")
              %Text " ["
              %Text self.percent
              %Text "%] "
              %Text self.num_lines
              %Text " lines |"
              %Text self.column
            ]
            color => Color::AnsiValue(182)
        })
    }

    fn height(&self) -> Option<u16> {
        Some(1)
    }
}

struct CommandLine<'a> {
    pub mode: &'a Mode,
}

impl<'a> CommandLine<'a> {
    pub fn new(mode: &'a Mode) -> Self {
        Self { mode }
    }
}

impl<'a> ComponentInterface for CommandLine<'a> {
    fn render(&self, _grid: Grid) -> Result<Component<'_>, anyhow::Error> {
        Ok(match self.mode {
            Mode::ExCommand(ex_command) => match ex_command.is_empty() {
                true => soft! {
                    %Text ":"
                },
                false => soft! {
                    %Text children => [
                        %Text ":"
                        %Text ex_command
                    ]
                },
            },
            _ => soft! {
                %Text " "
            },
        })
    }

    fn height(&self) -> Option<u16> {
        Some(1)
    }
}
