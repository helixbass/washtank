use ropey::{Rope, RopeSlice};
use squalid::regex;

use crate::Editor;

impl Editor {
    pub fn set_current_file_indents(&mut self) {
        self.current_file_indents = Some(calculate_indents(
            self.current_file.rope(),
            self.current_file_shift_width,
        ));
    }
}

fn get_indent_level(line: RopeSlice, shift_width: usize) -> IndentLevel {
    let mut spaces_seen_so_far = 0;
    for chunk in line.chunks() {
        if let Some(match_) = regex!(r#"^[ \n]+"#).find(chunk) {
            spaces_seen_so_far += match_.len();
            if match_.len() == chunk.len() {
                continue;
            } else {
                return IndentLevel::Level(spaces_seen_so_far.div_ceil(shift_width));
            }
        } else {
            return IndentLevel::Level(spaces_seen_so_far.div_ceil(shift_width));
        }
    }
    IndentLevel::BlankLine
}

pub fn calculate_indents(rope: &Rope, shift_width: usize) -> Vec<IndentLevel> {
    rope.lines()
        .map(|line| get_indent_level(line, shift_width))
        .collect()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IndentLevel {
    Level(usize),
    BlankLine,
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use crate::strip_trailing_newline;

    use super::*;

    fn calculate_indent_test(text: &str, expected: Vec<IndentLevel>) {
        assert_eq!(
            calculate_indents(&Rope::from(strip_trailing_newline(text)), 4),
            expected
        );
    }

    #[test]
    fn test_calculate_indents() {
        calculate_indent_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                }
            "#
            ),
            vec![
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
            ],
        );

        calculate_indent_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                }

                trait Foo {
                    fn whee() -> Whee;
                }
            "#
            ),
            vec![
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
                IndentLevel::BlankLine,
                IndentLevel::Level(0),
                IndentLevel::Level(1),
                IndentLevel::Level(0),
            ],
        );
    }
}
