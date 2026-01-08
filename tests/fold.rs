use indoc::indoc;

mod shared;

use shared::run_interactive_test;

#[tokio::test]
async fn test_fold_text() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "foo.rs",
        "",
        indoc!(
            r#"
                <cursor/>fn foo() {
                <color={Rgb(47, 47, 255)}>+-- 11 lines: let foo = "foo";</>
                }

                fn bar() {
                <color={Rgb(47, 47, 255)}>+-- 11 lines: let bar = "bar";</>
                }
            "#
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_cursor_on_fold() -> Result<(), anyhow::Error> {
    run_interactive_test(
        "foo.rs",
        "jjjjj",
        indoc!(
            r#"
                fn foo() {
                <color={Rgb(47, 47, 255)}>+-- 11 lines: let foo = "foo";</>
                }

                fn bar() {
                <cursor/><color={Rgb(47, 47, 255)}>+-- 11 lines: let bar = "bar";</>
                }
            "#
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_cursor_in_unfolded_moves_to_fold_line_when_folding_one_level(
) -> Result<(), anyhow::Error> {
    run_interactive_test(
        "foo.rs",
        "jzojjjjzc",
        indoc!(
            r#"
                fn foo() {
                <cursor/><color={Rgb(47, 47, 255)}>+-- 11 lines: let foo = "foo";</>
                }

                fn bar() {
                <color={Rgb(47, 47, 255)}>+-- 11 lines: let bar = "bar";</>
                }
            "#
        ),
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn test_cursor_in_unfolded_moves_to_fold_line_when_folding_fully() -> Result<(), anyhow::Error>
{
    run_interactive_test(
        "foo.rs",
        "jzojjjjzC",
        indoc!(
            r#"
                fn foo() {
                <cursor/><color={Rgb(47, 47, 255)}>+-- 11 lines: let foo = "foo";</>
                }

                fn bar() {
                <color={Rgb(47, 47, 255)}>+-- 11 lines: let bar = "bar";</>
                }
            "#
        ),
    )
    .await?;

    Ok(())
}
