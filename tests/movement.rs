use indoc::indoc;

mod shared;

use shared::run_interactive_test;

#[tokio::test]
async fn test_initial_screen() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "",
        indoc!(
            r#"
                <cursor/>Hello world
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
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_move_cursor_down() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "j",
        indoc!(
            r#"
                Hello world
                <cursor/>What a great day

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
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_move_cursor_right() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "ll",
        indoc!(
            r#"
                He<cursor/>llo world
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
    )
    .await?;

    Ok(())
}
