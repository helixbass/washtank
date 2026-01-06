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
    )
    .await?;

    Ok(())
}
