use std::pin::Pin;

use clap::Parser;
use crossterm::event::{Event, EventStream};
use oelung::{soft, BackendInterface, Renderer, RendererBuilder};
use oelung_lantern::{generate_sender, mpsc::Sender, ReceiveEvent};
use tokio::sync::mpsc::channel;
use tokio_stream::StreamExt;

use washtank::{editor, Args, Editor, EventAggregator};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    let mut renderer = RendererBuilder::default().build()?;

    let (sender, mut receiver) = channel::<World>(100);

    listen_to_crossterm_events(CrosstermSender::from(sender.clone()));

    let mut event_aggregator = EventAggregator::default();
    let mut editor = Editor::try_new(
        &args.into(),
        Box::new(EditorSender::from(sender.clone())),
        renderer.backend.size()?,
    )
    .await?;

    render_screen(&mut renderer, &editor)?;

    while let Some(world) = receiver.recv().await {
        let mut queued_effects: Vec<Pin<Box<dyn Future<Output = ()> + Send + 'static>>> = vec![];
        match world {
            World::Crossterm(event) => {
                let editor_event =
                    event_aggregator.receive(&event, |future| queued_effects.push(future))?;
                if let Some(editor_event) = editor_event {
                    editor.receive(&editor_event, |future| queued_effects.push(future))?;
                    render_screen(&mut renderer, &editor)?;
                }
            }
            World::Editor(editor::Happened::Quit) => break,
        }
        for effect in queued_effects {
            tokio::spawn(effect);
        }
    }

    Ok(())
}

fn render_screen(renderer: &mut Renderer, editor: &Editor) -> Result<(), anyhow::Error> {
    renderer.render(soft! {
      %editor
    })?;

    Ok(())
}

enum World {
    Crossterm(Event),
    Editor(editor::Happened),
}

generate_sender!(World, Crossterm, Event);
generate_sender!(World, Editor, editor::Happened);

fn listen_to_crossterm_events(sender: CrosstermSender) {
    tokio::spawn(async move {
        let mut event_stream = EventStream::new();

        while let Some(Ok(event)) = event_stream.next().await {
            sender.send(event).await;
        }

        panic!("kill everything")
    });
}
