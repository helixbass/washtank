use std::env;
use std::process::Command;

use indoc::indoc;
use vt100::Screen;

fn run_interactive_test(file_name: &str, input: &str, expected_screen_state: &str) {
    let path_to_editor_executable = env!("CARGO_BIN_EXE_washtank");

    let output = Command::new("faketty")
        .arg(path_to_editor_executable)
        .arg(file_name)
        .output()
        .unwrap();

    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&output.stdout);
    assert_expected_screen_contents(&parser.screen(), expected_screen_state);
}

fn assert_expected_screen_contents(screen: &Screen, expected_screen_state: &str) {
    let expected_screen_state = ExpectedScreenState::from(expected_screen_state);
    assert_eq!(vec![screen.contents()], expected_screen_state.text_contents);
}

struct ExpectedScreenState {
    pub text_contents: Vec<String>,
}

impl From<&str> for ExpectedScreenState {
    fn from(value: &str) -> Self {
        Self {
            text_contents: value.split("\n").map(ToOwned::to_owned).collect(),
        }
    }
}

#[test]
fn test_initial_screen() {
    run_interactive_test(
        "fixtures/foo.rs",
        "",
        indoc!(
            r#"
                fn foo() {
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                    let foo = "foo";
                }

                fn bar() {
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
                    let bar = "bar";
        "#
        ),
    );
}
