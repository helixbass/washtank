use clap::Parser;
use crossterm::event::{Event, EventStream};
use oelung::Renderer;
use oelung_lantern::{generate_sender, mpsc::Sender};
use tokio::sync::mpsc::channel;
use tokio_stream::StreamExt;

use washtank::{Args, Editor};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    let mut renderer = Renderer::try_new()?;

    let (sender, mut receiver) = channel::<World>(100);

    let editor = Editor::try_new(args).await?;

    render_screen(&mut renderer, &editor)?;

    Ok(())
}

enum World {
    Crossterm(Event),
}

generate_sender!(World, Crossterm, Event);

fn listen_to_crossterm_events(sender: CrosstermSender) {
    tokio::spawn(async move {
        let mut event_stream = EventStream::new();

        while let Some(Ok(event)) = event_stream.next().await {
            sender.send(event).await;
        }

        panic!("kill everything")
    });
}
