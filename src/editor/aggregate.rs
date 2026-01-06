use std::pin::Pin;

use crossterm::event::{self, KeyCode};
use oelung_lantern::{
    is_any_simple_char_press, is_simple_char_press, is_simple_key_press, ReceiveEvent,
};
use tracing::instrument;

use super::Event;

#[derive(Copy, Clone, Default)]
pub enum EventAggregator {
    #[default]
    Initial,
    SawZ,
    InExCommandMode,
}

impl ReceiveEvent<event::Event, Option<Event>> for EventAggregator {
    #[instrument(level = "trace", skip(self, event, _queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &event::Event,
        _queue_effect: TQueueEffect,
    ) -> Result<Option<Event>, anyhow::Error> {
        match (*self, event) {
            (Self::Initial, event) if is_simple_char_press(event, 'j') => {
                return Ok(Some(Event::MoveCursorDownNLines(1)));
            }
            (Self::Initial, event) if is_simple_char_press(event, 'k') => {
                return Ok(Some(Event::MoveCursorUpNLines(1)));
            }
            (Self::Initial, event) if is_simple_char_press(event, 'z') => {
                *self = Self::SawZ;
                return Ok(None);
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'o') => {
                *self = Self::Initial;
                return Ok(Some(Event::OpenFoldUnderCursorOneLevel));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'c') => {
                *self = Self::Initial;
                return Ok(Some(Event::CloseFoldUnderCursorOneLevel));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'O') => {
                *self = Self::Initial;
                return Ok(Some(Event::FullyOpenFoldUnderCursor));
            }
            (Self::SawZ, event) if is_simple_char_press(event, 'C') => {
                *self = Self::Initial;
                return Ok(Some(Event::FullyCloseFoldUnderCursor));
            }
            (_, event) if is_simple_key_press(event, KeyCode::Esc) => {
                *self = Self::Initial;
                return Ok(Some(Event::GoIntoNormalMode));
            }
            (Self::Initial, event) if is_simple_char_press(event, ':') => {
                *self = Self::InExCommandMode;
                return Ok(Some(Event::GoIntoExCommandMode));
            }
            (Self::InExCommandMode, event) if is_any_simple_char_press(event).is_some() => {
                return Ok(Some(Event::ExCommandChar(
                    is_any_simple_char_press(event).unwrap(),
                )));
            }
            (Self::InExCommandMode, event) if is_simple_key_press(event, KeyCode::Enter) => {
                *self = Self::Initial;
                return Ok(Some(Event::FinishExCommand));
            }
            _ => panic!("unexpected event"),
        }
    }
}
