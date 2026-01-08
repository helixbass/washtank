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

#[tokio::test]
async fn test_move_cursor_left() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "lllh",
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

#[tokio::test]
async fn test_remembers_column_when_moving_to_a_new_line() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "llj",
        indoc!(
            r#"
                Hello world
                Wh<cursor/>at a great day

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
async fn test_disallows_cursoring_past_end_of_line() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "lllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllll",
        indoc!(
            r#"
                Hello worl<cursor/>d
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
async fn test_disallows_cursoring_past_beginning_of_line() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "lhhhhhhhh",
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
async fn test_0_brings_to_beginning_of_line() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "lllll0",
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
async fn test_dollar_brings_to_end_of_line() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        "ll$",
        indoc!(
            r#"
                Hello worl<cursor/>d
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
