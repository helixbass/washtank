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
                fn foo() {
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
