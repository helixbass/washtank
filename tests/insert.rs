use indoc::indoc;

mod shared;

use shared::{char_event, esc_event, run_interactive_test};

#[tokio::test]
async fn test_insert() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "no_indentation.txt",
        vec![
            char_event('l'),
            char_event('i'),
            char_event('e'),
            esc_event(),
        ],
        indoc!(
            r#"
                H<cursor/>eello world
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
