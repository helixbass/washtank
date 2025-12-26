use indoc::indoc;

mod shared;

use shared::run_interactive_test;

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
