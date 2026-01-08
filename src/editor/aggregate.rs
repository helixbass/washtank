use std::pin::Pin;

use crossterm::event::{self, KeyCode};
use oelung_lantern::{
    is_any_simple_char_press, is_simple_char_press, is_simple_key_press, ReceiveEvent,
};
use squalid::_d;
use tracing::instrument;

use super::Event;
use crate::Config;

#[derive(Copy, Clone)]
pub struct EventAggregator {
    pub state: State,
    pub disallow_folding: bool,
}

impl EventAggregator {
    pub fn new(config: &Config) -> Self {
        Self {
            state: _d(),
            disallow_folding: config.disallow_folding,
        }
    }
}

#[derive(Copy, Clone, Default)]
pub enum State {
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
        match (self.state, event) {
            (State::Initial, event) if is_simple_char_press(event, 'j') => {
                return Ok(Some(Event::MoveCursorDownNLines(1)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'k') => {
                return Ok(Some(Event::MoveCursorUpNLines(1)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'l') => {
                return Ok(Some(Event::MoveCursorRightNColumns(1)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'h') => {
                return Ok(Some(Event::MoveCursorLeftNColumns(1)));
            }
            (State::Initial, event) if is_simple_char_press(event, '0') => {
                return Ok(Some(Event::MoveCursorToBeginningOfLine));
            }
            (State::Initial, event)
                if is_simple_char_press(event, 'z') && !self.disallow_folding =>
            {
                self.state = State::SawZ;
                return Ok(None);
            }
            (State::SawZ, event) if is_simple_char_press(event, 'o') && !self.disallow_folding => {
                self.state = State::Initial;
                return Ok(Some(Event::OpenFoldUnderCursorOneLevel));
            }
            (State::SawZ, event) if is_simple_char_press(event, 'c') && !self.disallow_folding => {
                self.state = State::Initial;
                return Ok(Some(Event::CloseFoldUnderCursorOneLevel));
            }
            (State::SawZ, event) if is_simple_char_press(event, 'O') && !self.disallow_folding => {
                self.state = State::Initial;
                return Ok(Some(Event::FullyOpenFoldUnderCursor));
            }
            (State::SawZ, event) if is_simple_char_press(event, 'C') && !self.disallow_folding => {
                self.state = State::Initial;
                return Ok(Some(Event::FullyCloseFoldUnderCursor));
            }
            (_, event) if is_simple_key_press(event, KeyCode::Esc) => {
                self.state = State::Initial;
                return Ok(Some(Event::GoIntoNormalMode));
            }
            (State::Initial, event) if is_simple_char_press(event, ':') => {
                self.state = State::InExCommandMode;
                return Ok(Some(Event::GoIntoExCommandMode));
            }
            (State::InExCommandMode, event) if is_any_simple_char_press(event).is_some() => {
                return Ok(Some(Event::ExCommandChar(
                    is_any_simple_char_press(event).unwrap(),
                )));
            }
            (State::InExCommandMode, event) if is_simple_key_press(event, KeyCode::Enter) => {
                self.state = State::Initial;
                return Ok(Some(Event::FinishExCommand));
            }
            _ => panic!("unexpected event"),
        }
    }
}
