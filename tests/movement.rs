use std::env;
use std::process::Command;

use indoc::indoc;
use vt100::Screen;

fn run_interactive_test(file_name: &str, input: &str, expected_screen_state: &str) {
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

struct ExpectedScreenState {
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

#[test]
fn test_initial_screen() {
    run_interactive_test(
        "no_indentation.txt",
        "",
        indoc!(
            r#"
                Hello world
                What a great day

                Hello world
                What a great day

                Goodbye

                Hello world
                What a great day

                Hello world
                What a great day

                Hello world
                What a great day

                Hello world
                What a great day

                Goodbye

                Hello world
                What a great day
        "#
        ),
    );
}

// TODO: share this with washtank? Eg change to a workspace
// with a shared `shared` crate?
fn strip_trailing_newline(file_contents: &str) -> &str {
    if file_contents.ends_with("\n") {
        &file_contents[..file_contents.len() - 1]
    } else {
        file_contents
    }
}
