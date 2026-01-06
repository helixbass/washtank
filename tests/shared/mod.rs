use std::pin::Pin;
use std::rc::Rc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use oelung::{soft, BackendMemory, Renderer, RendererBuilder};
use oelung_lantern::{generate_sender, mpsc::Sender, ReceiveEvent};
use tokio::sync::mpsc::channel;

use washtank::{editor, Args, Editor, EventAggregator};

pub async fn run_interactive_test(
    file_name: &str,
    input: &str,
    expected_screen_state: &str,
) -> Result<(), anyhow::Error> {
    let memory_backend = Rc::new(BackendMemory::new(24, 80));

    let mut renderer = RendererBuilder::default()
        .backend(memory_backend.clone())
        .build()?;

    let (sender, mut receiver) = channel::<World>(100);

    tokio::spawn({
        let sender = CrosstermSender::from(sender.clone());
        async move {
            for ch in input.chars() {
                sender
                    .send(Event::Key(KeyEvent {
                        code: KeyCode::Char(ch),
                        modifiers: KeyModifiers::NONE,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }))
                    .await;
            }
        }
    });

    let mut editor = Editor::try_new(
        Args {
            file_name: file_name.into(),
        },
        Box::new(EditorSender::from(sender.clone())),
    )
    .await?;

    let mut event_aggregator = EventAggregator::default();

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

    assert_expected_screen_contents(expected_screen_state, &memory_backend);

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

fn assert_expected_screen_contents(memory_backend: &BackendMemory, expected_screen_state: &str) {
    let expected_screen_state = ExpectedScreenState::from(expected_screen_state);
    assert_eq!(
        memory_backend
            .grid
            .iter()
            .map(|row| { row.into_iter().map(|cell| cell.content).collect::<String>() })
            .collect(),
        expected_screen_state.text_contents
    );
}

pub struct ExpectedScreenState {
    pub text_contents: Vec<String>,
}

impl From<&str> for ExpectedScreenState {
    fn from(value: &str) -> Self {
        Self {
            text_contents: strip_trailing_newline(value)
                .split("\n")
                .map(ToOwned::to_owned)
                .collect(),
        }
    }
}

// TODO: share this with washtank? Eg change to a workspace
// with a shared `shared` crate?
pub fn strip_trailing_newline(file_contents: &str) -> &str {
    if file_contents.ends_with("\n") {
        &file_contents[..file_contents.len() - 1]
    } else {
        file_contents
    }
}
