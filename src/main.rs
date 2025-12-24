use std::cmp;
use std::fmt::{self, Display};
use std::io::{stdout, StdoutLock, Write};
use std::path::PathBuf;

use clap::Parser;
use crossterm::{
    cursor,
    event::{Event, EventStream, KeyCode},
    execute,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, size, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand, QueueableCommand,
};
use ropey::{Rope, RopeSlice};
use squalid::{EverythingExt, _d};
use tokio::fs;
use tokio_stream::StreamExt;

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
    /// position on-screen, not in terms of file line #
    pub cursor_position: Position,
    pub stdout: StdoutLock<'static>,
    pub size: Size,
    pub top_line: u16,
}

impl Editor {
    fn try_new() -> Result<Self, anyhow::Error> {
        Ok(Self {
            current_file: _d(),
            cursor_position: _d(),
            stdout: stdout().lock(),
            size: size()?.thrush(|(columns, rows)| Size {
                height: rows,
                width: columns,
            }),
            top_line: _d(),
        })
    }
}

impl Editor {
    async fn run(&mut self) -> Result<(), anyhow::Error> {
        let args = Args::parse();

        self.push_cursor_position()?;

        self.open_file(args.file_name).await?;

        let mut event_stream = EventStream::new();

        while let Some(Ok(event)) = event_stream.next().await {
            match event {
                Event::Key(key) => match key.code {
                    KeyCode::Char('j') => {
                        self.maybe_move_cursor_down_one_line()?;
                    }
                    KeyCode::Char('k') => {
                        self.maybe_move_cursor_up_one_line()?;
                    }
                    _ => unimplemented!(),
                },
                _ => unimplemented!(),
            }
        }

        Ok(())
    }

    async fn open_file(&mut self, file_name: PathBuf) -> Result<(), anyhow::Error> {
        let rope = Rope::from_str(&fs::read_to_string(&file_name).await?);
        self.current_file = OpenFile::Named(OpenFileNamed {
            rope,
            path: file_name,
        });

        self.rerender_screen()?;

        Ok(())
    }

    fn push_cursor_position(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.execute(cursor::MoveTo(
            self.cursor_position.column,
            self.cursor_position.row,
        ))?;

        Ok(())
    }

    fn cursor_file_line(&self) -> u16 {
        self.cursor_position.row + self.top_line
    }

    fn maybe_move_cursor_down_one_line(&mut self) -> Result<(), anyhow::Error> {
        if usize::from(self.cursor_file_line()) == self.current_file.rope().len_lines() - 1 {
            return Ok(());
        }

        if self.cursor_position.row == self.size.height - 1 {
            self.top_line += 1;
            self.rerender_screen()?;
        } else {
            self.cursor_position.row += 1;
            self.push_cursor_position()?;
        }

        Ok(())
    }

    fn maybe_move_cursor_up_one_line(&mut self) -> Result<(), anyhow::Error> {
        if self.cursor_file_line() == 0 {
            return Ok(());
        }

        if self.cursor_position.row == 0 {
            self.top_line -= 1;
            self.rerender_screen()?;
        } else {
            self.cursor_position.row -= 1;
            self.push_cursor_position()?;
        }

        Ok(())
    }

    fn rerender_screen(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.queue(Clear(ClearType::All))?;
        self.stdout.queue(cursor::SavePosition)?;
        self.stdout.queue(cursor::Hide)?;
        self.stdout.queue(cursor::MoveTo(0, 0))?;

        let rope = self.current_file.rope();
        let num_lines = rope.len_lines();
        let top_line = usize::from(self.top_line);
        assert!(top_line <= num_lines - 1);

        let last_line_num_to_render =
            cmp::min(num_lines - 1, top_line + usize::from(self.size.height) - 1);
        for line_num in top_line..=last_line_num_to_render {
            let line_without_trailing_newline = {
                let line = rope.line(line_num);
                struct Chunks<'a> {
                    chunks: ropey::iter::Chunks<'a>,
                }

                impl<'a> Iterator for Chunks<'a> {
                    type Item = <ropey::iter::Chunks<'a> as Iterator>::Item;

                    fn next(&mut self) -> Option<Self::Item> {
                        let next = self.chunks.next()?;
                        Some(if next.ends_with("\n") {
                            &next[..next.len() - 1]
                        } else {
                            next
                        })
                    }
                }

                struct Wrap<'a> {
                    rope_slice: RopeSlice<'a>,
                }

                impl<'a> Wrap<'a> {
                    pub fn chunks(&self) -> Chunks<'a> {
                        Chunks {
                            chunks: self.rope_slice.chunks(),
                        }
                    }
                }

                // copied this from RopeSlice's Display impl
                impl<'a> Display for Wrap<'a> {
                    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                        for chunk in self.chunks() {
                            write!(f, "{}", chunk)?
                        }
                        Ok(())
                    }
                }

                Wrap { rope_slice: line }
            };
            self.stdout.queue(Print(line_without_trailing_newline))?;
            if line_num != last_line_num_to_render {
                self.stdout.queue(Print("\r\n"))?;
            }
        }

        self.stdout.queue(cursor::RestorePosition)?;
        self.stdout.queue(cursor::Show)?;

        self.stdout.flush()?;

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

#[derive(Default)]
pub struct Position {
    pub row: u16,
    pub column: u16,
}

pub struct Size {
    pub height: u16,
    pub width: u16,
}
