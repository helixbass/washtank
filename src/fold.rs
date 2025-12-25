use squalid::_d;

use crate::{Editor, IndentLevel, LineNumber, PrintedLine};

impl Editor {
    pub fn apply_initial_folds(&mut self) {
        self.folds = Some(calculate_folds(self.current_file_indents.as_ref().unwrap()));
        self.max_folds = self.folds.clone();
    }

    pub fn fully_open_fold_under_cursor(&mut self) -> Result<(), anyhow::Error> {
        let PrintedLine::Fold(fold_index) =
            self.printed_lines.as_ref().unwrap()[usize::from(self.cursor_position.row)]
        else {
            return Ok(());
        };
        let _ = self.folds.as_mut().unwrap().remove(fold_index);
        self.compute_printed_lines();
        self.rerender_screen()?;
        Ok(())
    }

    pub fn open_fold_under_cursor_one_level(&mut self) -> Result<(), anyhow::Error> {
        let PrintedLine::Fold(fold_index) =
            self.printed_lines.as_ref().unwrap()[usize::from(self.cursor_position.row)]
        else {
            return Ok(());
        };
        if self.folds.as_ref().unwrap()[fold_index].num_indents == 1 {
            let fold = self.folds.as_mut().unwrap().remove(fold_index);
            let mut nested = fold.nested;
            for nested in &mut nested {
                decrement_fold_num_indents(nested);
            }
            let _ = self
                .folds
                .as_mut()
                .unwrap()
                .splice(fold_index..fold_index, nested);
        } else {
            decrement_fold_num_indents(self.folds.as_mut().unwrap().get_mut(fold_index).unwrap());
        }

        self.compute_printed_lines();
        self.rerender_screen()?;
        Ok(())
    }

    pub fn fully_close_fold_under_cursor(&mut self) -> Result<(), anyhow::Error> {
        let start_line = self.printed_lines.as_ref().unwrap()
            [usize::from(self.cursor_position.row)]
        .start_line(self.folds.as_ref().unwrap());

        let Some(fold_index) = self
            .max_folds
            .as_ref()
            .unwrap()
            .into_iter()
            .position(|fold| fold.range.start <= start_line && fold.range.end > start_line)
        else {
            return Ok(());
        };

        let max_fold = &self.max_folds.as_ref().unwrap()[fold_index];

        let first_fold_index_to_replace =
            self.folds.as_ref().unwrap().into_iter().position(|fold| {
                fold.range.start >= max_fold.range.start && fold.range.end <= max_fold.range.end
            });
        let additional_count_to_replace =
            first_fold_index_to_replace.map(|first_fold_index_to_replace| {
                self.folds.as_ref().unwrap()[first_fold_index_to_replace..]
                    .into_iter()
                    .take_while(|fold| fold.range.end <= max_fold.range.end)
                    .count()
            });
        let range_to_splice = match first_fold_index_to_replace {
            None => {
                let first_after = self
                    .folds
                    .as_ref()
                    .unwrap()
                    .into_iter()
                    .position(|fold| fold.range.start >= max_fold.range.end);
                match first_after {
                    Some(first_after) => first_after..first_after,
                    None => {
                        let folds = self.folds.as_ref().unwrap();
                        folds.len()..folds.len()
                    }
                }
            }
            Some(first_fold_index_to_replace) => {
                first_fold_index_to_replace
                    ..first_fold_index_to_replace + additional_count_to_replace.unwrap() + 1
            }
        };
        self.folds
            .as_mut()
            .unwrap()
            .splice(range_to_splice, [max_fold.clone()]);

        self.compute_printed_lines();
        let new_cursor_position_row = self
            .printed_lines
            .as_ref()
            .unwrap()
            .into_iter()
            .position(|printed_line| {
                printed_line.start_line(self.folds.as_ref().unwrap())
                    == self.max_folds.as_ref().unwrap()[fold_index].range.start
            })
            .unwrap();
        self.cursor_position.row = u16::try_from(new_cursor_position_row).unwrap();
        self.push_cursor_position()?;

        self.rerender_screen()?;
        Ok(())
    }

    pub fn close_fold_under_cursor_one_level(&mut self) -> Result<(), anyhow::Error> {
        unimplemented!()
    }
}

pub type FoldIndex = usize;

pub fn calculate_folds(indents: &[IndentLevel]) -> Vec<Fold> {
    let mut folds: Vec<Fold> = _d();
    let mut in_progress: Option<InProgressFold> = _d();
    for (line_num, &indent) in indents.into_iter().enumerate() {
        if in_progress.is_none() {
            match indent {
                IndentLevel::BlankLine => continue,
                IndentLevel::Level(level) if level == 0 => continue,
                IndentLevel::Level(indent) => {
                    in_progress = Some(InProgressFold {
                        start_line: line_num,
                        num_closes: indent,
                        nested: _d(),
                        open_nested: _d(),
                    });
                }
            }
        } else {
            if indent == IndentLevel::Level(0) {
                folds.push(to_fold(in_progress.take().unwrap(), line_num));
                continue;
            }
            match indent {
                IndentLevel::BlankLine => {}
                IndentLevel::Level(indent)
                    if indent == in_progress.as_ref().unwrap().num_closes =>
                {
                    if in_progress.as_ref().unwrap().open_nested.is_some() {
                        let open_nested = in_progress.as_mut().unwrap().open_nested.take().unwrap();
                        in_progress
                            .as_mut()
                            .unwrap()
                            .nested
                            .push(to_fold(*open_nested, line_num));
                    }
                }
                IndentLevel::Level(indent) if indent < in_progress.as_ref().unwrap().num_closes => {
                    let prev_in_progress = in_progress.take().unwrap();
                    in_progress = Some(nest_myself_with_new_lesser_indent(
                        prev_in_progress,
                        indent,
                        line_num,
                    ));
                }
                IndentLevel::Level(indent) => {
                    apply_more_indented(indent, line_num, in_progress.as_mut().unwrap());
                }
            }
        }
    }
    if let Some(in_progress) = in_progress {
        folds.push(to_fold(in_progress, indents.len()));
    }
    folds
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub range: Range,
    pub num_closes: usize,
    pub num_indents_until_shown: usize,
    pub nested: Vec<Fold>,
}

fn decrement_fold_num_indents(fold: &mut Fold) {
    fold.num_indents -= 1;
    for nested in &mut fold.nested {
        decrement_fold_num_indents(nested);
    }
}

struct InProgressFold {
    pub start_line: usize,
    pub num_closes: usize,
    pub nested: Vec<Fold>,
    pub open_nested: Option<Box<InProgressFold>>,
}

fn to_fold(in_progress: InProgressFold, one_past_line_num: usize) -> Fold {
    let mut nested = in_progress.nested;
    if let Some(open_nested) = in_progress.open_nested {
        nested.push(to_fold(*open_nested, one_past_line_num));
    }
    Fold {
        range: Range {
            start: in_progress.start_line,
            end: one_past_line_num,
        },
        num_closes: in_progress.num_closes,
        num_indents_until_shown: 0,
        nested,
    }
}

fn nest_myself_with_new_lesser_indent(
    me: InProgressFold,
    new_lesser_indent: usize,
    line_num: usize,
) -> InProgressFold {
    InProgressFold {
        start_line: me.start_line,
        num_closes: new_lesser_indent,
        nested: vec![to_fold(me, line_num)],
        open_nested: None,
    }
}

fn apply_more_indented(indent: usize, line_num: usize, in_progress: &mut InProgressFold) {
    if in_progress.open_nested.is_none() {
        in_progress.open_nested = Some(Box::new(InProgressFold {
            start_line: line_num,
            num_closes: indent,
            nested: _d(),
            open_nested: _d(),
        }));
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_closes == indent {
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_closes < indent {
        apply_more_indented(indent, line_num, in_progress.open_nested.as_mut().unwrap());
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_closes > indent {
        let prev_open_nested = in_progress.open_nested.take().unwrap();
        in_progress.open_nested = Some(Box::new(nest_myself_with_new_lesser_indent(
            *prev_open_nested,
            indent,
            line_num,
        )));
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: LineNumber,
    pub end: LineNumber,
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use ropey::Rope;

    use crate::{calculate_indents, strip_trailing_newline};

    use super::*;

    fn calculate_folds_test(text: &str, expected: Vec<Fold>) {
        assert_eq!(
            calculate_folds(&calculate_indents(
                &Rope::from(strip_trailing_newline(text)),
                4
            )),
            expected
        );
    }

    #[test]
    fn test_calculate_folds() {
        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 3 },
                num_closes: 1,
                num_indents_until_shown: 0,
                nested: vec![],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";

                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 4 },
                num_closes: 1,
                num_indents_until_shown: 0,
                nested: vec![],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                        "foo";
                        "foo";
                    "foo";
                    "foo";
                        "foo";
                        "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 7 },
                num_closes: 1,
                num_indents_until_shown: 0,
                nested: vec![
                    Fold {
                        range: Range { start: 1, end: 3 },
                        num_closes: 2,
                        num_indents_until_shown: 0,
                        nested: vec![],
                    },
                    Fold {
                        range: Range { start: 5, end: 7 },
                        num_closes: 2,
                        num_indents_until_shown: 0,
                        nested: vec![],
                    },
                ],
            }],
        );

        calculate_folds_test(
            indoc!(
                r#"
                fn foo() {
                    "foo";
                    "foo";
                        "foo";
                        "foo";
                    "foo";
                    "foo";
                }
            "#
            ),
            vec![Fold {
                range: Range { start: 1, end: 7 },
                num_closes: 1,
                num_indents_until_shown: 0,
                nested: vec![Fold {
                    range: Range { start: 3, end: 5 },
                    num_closes: 2,
                    num_indents_until_shown: 0,
                    nested: vec![],
                }],
            }],
        );
    }
}
