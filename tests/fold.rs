use indoc::indoc;

mod shared;

use shared::run_interactive_test;

#[test]
fn test_fold_text() {
    run_interactive_test(
        "foo.rs",
        "",
        indoc!(
            r#"
                fn foo() {
                +-- 11 lines: let foo = "foo";
                }

                fn bar() {
                +-- 11 lines: let bar = "bar";
                }
            "#
        ),
    );
}
