use std::env;
use std::process::Command;

use vt100::Screen;

pub fn run_interactive_test(file_name: &str, input: &str, expected_screen_state: &str) {
    let path_to_editor_executable = env!("CARGO_BIN_EXE_washtank");

    // expects dtolnay/faketty to be available on the system I guess?
    // Eg per its docs `cargo intall faketty` or whatever?
    let output = Command::new("faketty")
        .arg(path_to_editor_executable)
        .arg(&format!("fixtures/{file_name}"))
        .output()
        .unwrap();

    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&output.stdout);
    assert_expected_screen_contents(&parser.screen(), expected_screen_state);
}

fn assert_expected_screen_contents(screen: &Screen, expected_screen_state: &str) {
    let expected_screen_state = ExpectedScreenState::from(expected_screen_state);
    assert_eq!(
        screen
            .contents()
            .split("\n")
            .map(|line| line[4..].to_owned())
            .collect::<Vec<_>>(),
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
