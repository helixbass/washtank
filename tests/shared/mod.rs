use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use oelung::{soft, BackendInterface, BackendMemory, Renderer, RendererBuilder, Size};
use oelung_lantern::{
    assert_expected_screen_contents_rendered_grid, generate_sender, mpsc::Sender, ReceiveEvent,
};
use tokio::sync::mpsc::channel;

use washtank::{editor, Args, Editor, EventAggregator};

pub async fn run_interactive_test(
    file_name: &str,
    input: &str,
    expected_screen_state: &str,
) -> Result<(), anyhow::Error> {
    let memory_backend = Rc::new(RefCell::new(BackendMemory::new(Size {
        height: 26,
        width: 80,
    })));

    let mut renderer = RendererBuilder::default()
        .backend(memory_backend.clone())
        .build()?;

    let (sender, mut receiver) = channel::<World>(100);

    tokio::spawn({
        let sender = CrosstermSender::from(sender.clone());
        let input = input.to_owned();
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
            for ch in ":q".chars() {
                sender
                    .send(Event::Key(KeyEvent {
                        code: KeyCode::Char(ch),
                        modifiers: KeyModifiers::NONE,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }))
                    .await;
            }
            sender
                .send(Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }))
                .await;
        }
    });

    let mut editor = Editor::try_new(
        Args {
            file_name: format!("fixtures/{file_name}").into(),
        },
        Box::new(EditorSender::from(sender.clone())),
        renderer.backend.size()?,
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

    assert_expected_screen_contents(&memory_backend.borrow(), expected_screen_state);

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
    let grid = &memory_backend.grid;
    let total_grid_height = grid.len();
    let grid = grid
        .into_iter()
        .take(total_grid_height - 2)
        .map(|row| row[4..].to_owned())
        .collect::<Vec<_>>();
    assert_expected_screen_contents_rendered_grid(
        &grid,
        memory_backend.current_cursor_position(),
        expected_screen_state,
    );
}
