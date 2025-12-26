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
        if self.folds.as_ref().unwrap()[fold_index].num_closes == 1 {
            let fold = self.folds.as_mut().unwrap().remove(fold_index);
            let hoisted_nested = fold
                .nested
                .into_iter()
                .map(|nested| Fold {
                    range: nested.range,
                    num_closes: nested.additional_num_closes,
                    full_num_indents: fold.full_num_indents + nested.additional_full_num_indents,
                    nested: nested.nested,
                })
                .collect::<Vec<_>>();
            let _ = self
                .folds
                .as_mut()
                .unwrap()
                .splice(fold_index..fold_index, hoisted_nested);
        } else {
            self.folds
                .as_mut()
                .unwrap()
                .get_mut(fold_index)
                .unwrap()
                .num_closes -= 1;
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

        self.splice_in_new_fold(self.max_folds.as_ref().unwrap()[fold_index].clone());

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

    fn splice_in_new_fold(&mut self, new_fold: Fold) {
        let first_fold_index_to_replace =
            self.folds.as_ref().unwrap().into_iter().position(|fold| {
                fold.range.start >= new_fold.range.start && fold.range.end <= new_fold.range.end
            });
        let additional_count_to_replace =
            first_fold_index_to_replace.map(|first_fold_index_to_replace| {
                self.folds.as_ref().unwrap()[first_fold_index_to_replace..]
                    .into_iter()
                    .take_while(|fold| fold.range.end <= new_fold.range.end)
                    .count()
            });
        let range_to_splice = match first_fold_index_to_replace {
            None => {
                let first_after = self
                    .folds
                    .as_ref()
                    .unwrap()
                    .into_iter()
                    .position(|fold| fold.range.start >= new_fold.range.end);
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
            .splice(range_to_splice, [new_fold]);
    }

    pub fn close_fold_under_cursor_one_level(&mut self) -> Result<(), anyhow::Error> {
        match self.printed_lines.as_ref().unwrap()[usize::from(self.cursor_position.row)] {
            PrintedLine::Fold(fold_index) => {
                if self.folds.as_ref().unwrap()[fold_index].num_closes
                    == self.folds.as_ref().unwrap()[fold_index].full_num_indents
                {
                    return Ok(());
                }
                self.folds
                    .as_mut()
                    .unwrap()
                    .get_mut(fold_index)
                    .unwrap()
                    .num_closes += 1;
            }
            PrintedLine::Line(line_num) => {
                let Some(innermost_max_fold) = self.find_innermost_max_fold(line_num) else {
                    return Ok(());
                };
                self.splice_in_new_fold(
                    match innermost_max_fold {
                        FoldOrNestedFold::Fold(fold) => fold.clone(),
                        FoldOrNestedFold::NestedFold(nested) => Fold {
                            range: nested.range,
                            full_num_indents: 
                        }
                    }
                );
            }
        }

        self.compute_printed_lines();
        self.rerender_screen()?;
        return Ok(());
    }

    fn find_innermost_max_fold(&self, line_num: usize) -> Option<FoldOrNestedFold<'_>> {
        let fold = self
            .max_folds
            .as_ref()
            .unwrap()
            .into_iter()
            .find(|fold| fold.range.start <= line_num && fold.range.end > line_num)?;
        Some(
            find_innermost_max_fold_nested(line_num, fold)
                .map(|(nested, parent_num_indents_from_past_this_level)| FoldOrNestedFold::NestedFold(nested, fold.full_num_indents + parent_num_indents_from_past_this_level))
                .unwrap_or_else(|| FoldOrNestedFold::Fold(fold)),
        )
    }
}

type ParentNumIndentsFromPastThisLevel = usize;

fn find_innermost_max_fold_nested(line_num: usize, fold: &impl HasNested) -> Option<(&NestedFold, ParentNumIndentsFromPastThisLevel)> {
    let found_nested = fold
        .nested()
        .into_iter()
        .find(|nested| nested.range.start <= line_num && nested.range.end > line_num)?;
    Some(
        find_innermost_max_fold_nested(
            line_num,
            found_nested,
        ).map(|(nested, parent_num_indents_from_past_this_level)| {
            (
                nested,
                parent_num_indents_from_past_this_level + found_nested.additional_full_num_indents
            )
        })
        .unwrap_or((found_nested, 0))
    )
}

type ParentFullNumIndents = usize;

enum FoldOrNestedFold<'a> {
    Fold(&'a Fold),
    NestedFold(&'a NestedFold, ParentFullNumIndents),
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
                        num_indents: indent,
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
                    if indent == in_progress.as_ref().unwrap().num_indents =>
                {
                    if in_progress.as_ref().unwrap().open_nested.is_some() {
                        let open_nested = in_progress.as_mut().unwrap().open_nested.take().unwrap();
                        let parent_num_indents = in_progress.as_ref().unwrap().num_indents;
                        in_progress.as_mut().unwrap().nested.push(to_nested_fold(
                            *open_nested,
                            line_num,
                            parent_num_indents,
                        ));
                    }
                }
                IndentLevel::Level(indent)
                    if indent < in_progress.as_ref().unwrap().num_indents =>
                {
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
    pub full_num_indents: usize,
    pub nested: Vec<NestedFold>,
}

trait HasNested {
    fn nested(&self) -> &[NestedFold];
}

impl HasNested for Fold {
    fn nested(&self) -> &[NestedFold] {
        &self.nested
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NestedFold {
    pub range: Range,
    pub additional_num_closes: usize,
    pub additional_full_num_indents: usize,
    pub nested: Vec<NestedFold>,
}

impl HasNested for NestedFold {
    fn nested(&self) -> &[NestedFold] {
        &self.nested
    }
}

struct InProgressFold {
    pub start_line: usize,
    pub num_indents: usize,
    pub nested: Vec<NestedFold>,
    pub open_nested: Option<Box<InProgressFold>>,
}

fn to_fold(in_progress: InProgressFold, one_past_line_num: usize) -> Fold {
    let mut nested = in_progress.nested;
    if let Some(open_nested) = in_progress.open_nested {
        nested.push(to_nested_fold(
            *open_nested,
            one_past_line_num,
            in_progress.num_indents,
        ));
    }
    Fold {
        range: Range {
            start: in_progress.start_line,
            end: one_past_line_num,
        },
        num_closes: in_progress.num_indents,
        full_num_indents: in_progress.num_indents,
        nested,
    }
}

fn to_nested_fold(
    in_progress: InProgressFold,
    one_past_line_num: usize,
    parent_num_indents: usize,
) -> NestedFold {
    let mut nested = in_progress.nested;
    if let Some(open_nested) = in_progress.open_nested {
        nested.push(to_nested_fold(
            *open_nested,
            one_past_line_num,
            in_progress.num_indents,
        ));
    }
    let additional_num_indents = in_progress.num_indents - parent_num_indents;
    assert!(additional_num_indents > 0);
    NestedFold {
        range: Range {
            start: in_progress.start_line,
            end: one_past_line_num,
        },
        additional_num_closes: additional_num_indents,
        additional_full_num_indents: additional_num_indents,
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
        num_indents: new_lesser_indent,
        nested: vec![to_nested_fold(me, line_num, new_lesser_indent)],
        open_nested: None,
    }
}

fn apply_more_indented(indent: usize, line_num: usize, in_progress: &mut InProgressFold) {
    if in_progress.open_nested.is_none() {
        in_progress.open_nested = Some(Box::new(InProgressFold {
            start_line: line_num,
            num_indents: indent,
            nested: _d(),
            open_nested: _d(),
        }));
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents == indent {
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents < indent {
        apply_more_indented(indent, line_num, in_progress.open_nested.as_mut().unwrap());
        return;
    }
    if in_progress.open_nested.as_ref().unwrap().num_indents > indent {
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
