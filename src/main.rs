use std::io::{stdout, StdoutLock};
use std::path::PathBuf;

use clap::Parser;
use crossterm::{
    cursor,
    event::EventStream,
    execute,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    QueueableCommand,
};
use ropey::Rope;
use squalid::_d;
use tokio::fs;
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;

    Editor::default().run().await?;

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
    pub cursor_position: Position,
    pub stdout: StdoutLock<'static>,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            current_file: _d(),
            cursor_position: _d(),
            stdout: stdout().lock(),
        }
    }
}

impl Editor {
    async fn run(&mut self) -> Result<(), anyhow::Error> {
        let args = Args::parse();

        self.open_file(args.file_name).await?;

        let mut event_stream = EventStream::new();

        while let Some(event) = event_stream.next().await {
            unimplemented!()
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

        // unimplemented!();
        Ok(())
    }

    fn rerender_screen(&mut self) -> Result<(), anyhow::Error> {
        self.stdout.queue(Clear(ClearType::All))?;
        self.stdout.queue(cursor::SavePosition)?;
        self.stdout.queue(cursor::Hide)?;
        self.stdout.queue(cursor::MoveTo(0, 0))?;
        self.stdout.queue(Print("hello world"))?;

        self.stdout.queue(cursor::RestorePosition)?;
        self.stdout.queue(cursor::Show)?;

        Ok(())
    }
}

pub enum OpenFile {
    Anonymous(OpenFileAnonymous),
    Named(OpenFileNamed),
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
    pub row: u32,
    pub column: u32,
}
