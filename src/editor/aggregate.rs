use std::pin::Pin;

use crossterm::event::{self, KeyCode};
use oelung_lantern::{
    is_any_simple_char_press, is_simple_char_press, is_simple_digit_press, is_simple_key_press,
    ReceiveEvent,
};
use smallvec::{smallvec, SmallVec};
use smol_str::SmolStr;
use squalid::_d;
use tracing::instrument;

use super::Event;
use crate::Config;

#[derive(Clone)]
pub struct EventAggregator {
    pub state: State,
    pub disallow_folding: bool,
    pub disallow_ex_command_mode: bool,
}

impl EventAggregator {
    pub fn new(config: &Config) -> Self {
        Self {
            state: _d(),
            disallow_folding: config.disallow_folding,
            disallow_ex_command_mode: config.disallow_ex_command_mode,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum State {
    #[default]
    Initial,
    SawZ,
    InExCommandMode,
    InInsertMode,
    SawDigits(Digits),
}

impl State {
    pub fn as_saw_digits_mut(&mut self) -> &mut Digits {
        match self {
            Self::SawDigits(digits) => digits,
            _ => panic!("Expected digits"),
        }
    }
}

pub type Digits = SmallVec<char, 4>;

impl ReceiveEvent<event::Event, Option<Event>> for EventAggregator {
    #[instrument(level = "trace", skip(self, event, _queue_effect))]
    fn receive<TQueueEffect: FnMut(Pin<Box<dyn Future<Output = ()> + Send + 'static>>)>(
        &mut self,
        event: &event::Event,
        _queue_effect: TQueueEffect,
    ) -> Result<Option<Event>, anyhow::Error> {
        match (&self.state, event) {
            (State::Initial, event) if is_simple_char_press(event, 'j') => {
                return Ok(Some(Event::MoveCursorDownNLines(1)));
            }
            (State::SawDigits(digits), event) if is_simple_char_press(event, 'j') => {
                let digits = digits_to_n(digits);
                self.state = State::Initial;
                return Ok(Some(Event::MoveCursorDownNLines(digits)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'k') => {
                return Ok(Some(Event::MoveCursorUpNLines(1)));
            }
            (State::SawDigits(digits), event) if is_simple_char_press(event, 'k') => {
                let digits = digits_to_n(digits);
                self.state = State::Initial;
                return Ok(Some(Event::MoveCursorUpNLines(digits)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'l') => {
                return Ok(Some(Event::MoveCursorRightNColumns(1)));
            }
            (State::SawDigits(digits), event) if is_simple_char_press(event, 'l') => {
                let digits = digits_to_n(digits);
                self.state = State::Initial;
                return Ok(Some(Event::MoveCursorRightNColumns(digits)));
            }
            (State::Initial, event) if is_simple_char_press(event, 'h') => {
                return Ok(Some(Event::MoveCursorLeftNColumns(1)));
            }
            (State::SawDigits(digits), event) if is_simple_char_press(event, 'h') => {
                let digits = digits_to_n(digits);
                self.state = State::Initial;
                return Ok(Some(Event::MoveCursorLeftNColumns(digits)));
            }
            (State::Initial, event) if is_simple_char_press(event, '0') => {
                return Ok(Some(Event::MoveCursorToBeginningOfLine));
            }
            (State::Initial, event) if is_simple_char_press(event, '$') => {
                return Ok(Some(Event::MoveCursorToEndOfLine));
            }
            (State::Initial, event) if is_simple_digit_press(event).is_some() => {
                self.state = State::SawDigits(smallvec![is_simple_digit_press(event).unwrap()]);
                return Ok(None);
            }
            (State::SawDigits(_), event) if is_simple_digit_press(event).is_some() => {
                self.state
                    .as_saw_digits_mut()
                    .push(is_simple_digit_press(event).unwrap());
                return Ok(None);
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
            (State::Initial, event)
                if is_simple_char_press(event, ':') && !self.disallow_ex_command_mode =>
            {
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
            (State::Initial, event) if is_simple_char_press(event, 'i') => {
                self.state = State::InInsertMode;
                return Ok(Some(Event::GoIntoInsertMode));
            }
            (State::InInsertMode, event) if is_any_simple_char_press(event).is_some() => {
                return Ok(Some(Event::InsertChar(
                    is_any_simple_char_press(event).unwrap(),
                )));
            }
            _ => panic!("unexpected event"),
        }
    }
}

fn digits_to_n(digits: &[char]) -> u16 {
    digits
        .into_iter()
        .copied()
        .collect::<SmolStr>()
        .parse::<u16>()
        .unwrap()
}
