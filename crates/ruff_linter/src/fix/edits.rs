//! Interface for generating fix edits from higher-level actions (e.g., "remove an argument").

use anyhow::Result;

use ruff_python_ast::{self as ast, ExceptHandler, Expr, ExprList, Stmt};
use ruff_python_codegen::Stylist;
use ruff_python_index::Indexer;
use ruff_python_trivia::{PythonWhitespace, has_leading_content, is_python_whitespace};
use ruff_source_file::{LineRanges, NewlineWithTrailingNewline};
use ruff_text_size::{Ranged, TextLen, TextRange, TextSize};

use crate::Edit;
use crate::Locator;
use crate::fix::codemods;

/// Return the [`Edit`] to use when deleting a [`Stmt`].
///
/// In some cases, this is as simple as deleting the [`TextRange`] of the [`Stmt`]
/// itself. However, there are a few exceptions:
/// - If the [`Stmt`] is _not_ the terminal statement in a multi-statement line,
///   we need to delete up to the start of the next statement (and avoid
///   deleting any content that precedes the statement).
/// - If the [`Stmt`] is the terminal statement in a multi-statement line, we need
///   to avoid deleting any content that precedes the statement.
/// - If the [`Stmt`] has no trailing and leading content, then it's convenient to
///   remove the entire start and end lines.
/// - If the [`Stmt`] is the last statement in its parent body, replace it with a
///   `pass` instead.
pub(crate) fn delete_stmt(
    stmt: &Stmt,
    parent: Option<&Stmt>,
    locator: &Locator,
    indexer: &Indexer,
) -> Edit {
    if parent.is_some_and(|parent| is_lone_child(stmt, parent)) {
        // If removing this node would lead to an invalid syntax tree, replace
        // it with a `pass`.
        Edit::range_replacement("pass".to_string(), stmt.range())
    } else {
        if let Some(semicolon) = trailing_semicolon(stmt.end(), locator) {
            let next = next_stmt_break(semicolon, locator);
            Edit::deletion(stmt.start(), next)
        } else if has_leading_content(stmt.start(), locator.contents()) {
            Edit::range_deletion(stmt.range())
        } else if let Some(start) =
            indexer.preceded_by_continuations(stmt.start(), locator.contents())
        {
            Edit::deletion(start, stmt.end())
        } else {
            let range = locator.full_lines_range(stmt.range());
            Edit::range_deletion(range)
        }
    }
}

/// Generate a `Fix` to remove the specified imports from an `import` statement.
pub(crate) fn remove_unused_imports<'a>(
    member_names: impl Iterator<Item = &'a str>,
    stmt: &Stmt,
    parent: Option<&Stmt>,
    locator: &Locator,
    stylist: &Stylist,
    indexer: &Indexer,
) -> Result<Edit> {
    match codemods::remove_imports(member_names, stmt, locator, stylist)? {
        None => Ok(delete_stmt(stmt, parent, locator, indexer)),
        Some(content) => Ok(Edit::range_replacement(content, stmt.range())),
    }
}

/// Edits to make the specified imports explicit, e.g. change `import x` to `import x as x`.
pub(crate) fn make_redundant_alias<'a>(
    member_names: impl Iterator<Item = &'a str>,
    stmt: &Stmt,
) -> Vec<Edit> {
    let aliases = match stmt {
        Stmt::Import(ast::StmtImport { names, .. }) => names,
        Stmt::ImportFrom(ast::StmtImportFrom { names, .. }) => names,
        _ => {
            return Vec::new();
        }
    };
    member_names
        .filter_map(|name| {
            aliases
                .iter()
                .find(|alias| alias.asname.is_none() && *name == alias.name.id)
                .map(|alias| Edit::range_replacement(format!("{name} as {name}"), alias.range))
        })
        .collect()
}

/// Fix to add the specified imports to the `__all__` export list.
pub(crate) fn add_to_dunder_all<'a>(
    names: impl Iterator<Item = &'a str>,
    expr: &Expr,
    stylist: &Stylist,
) -> Vec<Edit> {
    let (insertion_point, export_prefix_length) = match expr {
        Expr::List(ExprList { elts, .. }) => (
            elts.last().map_or(expr.end() - "]".text_len(), Ranged::end),
            elts.len(),
        ),
        Expr::Tuple(tup) if tup.parenthesized => (
            tup.elts
                .last()
                .map_or(tup.end() - ")".text_len(), Ranged::end),
            tup.len(),
        ),
        Expr::Tuple(tup) if !tup.parenthesized => (
            tup.elts
                .last()
                .expect("unparenthesized empty tuple is not possible")
                .range()
                .end(),
            tup.len(),
        ),
        _ => {
            // we don't know how to insert into this expression
            return vec![];
        }
    };
    let quote = stylist.quote();
    let mut edits: Vec<_> = names
        .enumerate()
        .map(|(offset, name)| match export_prefix_length + offset {
            0 => Edit::insertion(format!("{quote}{name}{quote}"), insertion_point),
            _ => Edit::insertion(format!(", {quote}{name}{quote}"), insertion_point),
        })
        .collect();
    if let Expr::Tuple(tup) = expr {
        if tup.parenthesized && export_prefix_length + edits.len() == 1 {
            edits.push(Edit::insertion(",".to_string(), insertion_point));
        }
    }
    edits
}

/// Determine if a vector contains only one, specific element.
fn is_only<T: PartialEq>(vec: &[T], value: &T) -> bool {
    vec.len() == 1 && vec[0] == *value
}

/// Determine if a child is the only statement in its body.
fn is_lone_child(child: &Stmt, parent: &Stmt) -> bool {
    match parent {
        Stmt::FunctionDef(ast::StmtFunctionDef { body, .. })
        | Stmt::ClassDef(ast::StmtClassDef { body, .. })
        | Stmt::With(ast::StmtWith { body, .. })
            if is_only(body, child) =>
        {
            return true;
        }
        Stmt::For(ast::StmtFor { body, orelse, .. })
        | Stmt::While(ast::StmtWhile { body, orelse, .. })
            if (is_only(body, child) || is_only(orelse, child)) =>
        {
            return true;
        }
        Stmt::If(ast::StmtIf {
            body,
            elif_else_clauses,
            ..
        }) if (is_only(body, child)
            || elif_else_clauses
                .iter()
                .any(|ast::ElifElseClause { body, .. }| is_only(body, child))) =>
        {
            return true;
        }
        Stmt::Try(ast::StmtTry {
            body,
            handlers,
            orelse,
            finalbody,
            ..
        }) if (is_only(body, child)
            || is_only(orelse, child)
            || is_only(finalbody, child)
            || handlers.iter().any(|handler| match handler {
                ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler { body, .. }) => {
                    is_only(body, child)
                }
            })) =>
        {
            return true;
        }
        Stmt::Match(ast::StmtMatch { cases, .. })
            if cases.iter().any(|case| is_only(&case.body, child)) =>
        {
            return true;
        }
        _ => {}
    }
    false
}

/// Return the location of a trailing semicolon following a `Stmt`, if it's part
/// of a multi-statement line.
fn trailing_semicolon(offset: TextSize, locator: &Locator) -> Option<TextSize> {
    let contents = locator.after(offset);

    for line in NewlineWithTrailingNewline::from(contents) {
        let trimmed = line.trim_whitespace_start();

        if trimmed.starts_with(';') {
            let colon_offset = line.text_len() - trimmed.text_len();
            return Some(offset + line.start() + colon_offset);
        }

        if !trimmed.starts_with('\\') {
            break;
        }
    }
    None
}

/// Find the next valid break for a `Stmt` after a semicolon.
fn next_stmt_break(semicolon: TextSize, locator: &Locator) -> TextSize {
    let start_location = semicolon + TextSize::from(1);

    for line in
        NewlineWithTrailingNewline::with_offset(locator.after(start_location), start_location)
    {
        let trimmed = line.trim_whitespace();
        // Skip past any continuations.
        if trimmed.starts_with('\\') {
            continue;
        }

        return if trimmed.is_empty() {
            // If the line is empty, then despite the previous statement ending in a
            // semicolon, we know that it's not a multi-statement line.
            line.start()
        } else {
            // Otherwise, find the start of the next statement. (Or, anything that isn't
            // whitespace.)
            let relative_offset = line.find(|c: char| !is_python_whitespace(c)).unwrap();
            line.start() + TextSize::try_from(relative_offset).unwrap()
        };
    }

    locator.line_end(start_location)
}

/// Add leading whitespace to a snippet, if it's immediately preceded an identifier or keyword.
pub(crate) fn pad_start(mut content: String, start: TextSize, locator: &Locator) -> String {
    // Ex) When converting `except(ValueError,)` from a tuple to a single argument, we need to
    // insert a space before the fix, to achieve `except ValueError`.
    if locator
        .up_to(start)
        .chars()
        .last()
        .is_some_and(|char| char.is_ascii_alphabetic())
    {
        content.insert(0, ' ');
    }
    content
}

/// Add trailing whitespace to a snippet, if it's immediately followed by an identifier or keyword.
pub(crate) fn pad_end(mut content: String, end: TextSize, locator: &Locator) -> String {
    if locator
        .after(end)
        .chars()
        .next()
        .is_some_and(|char| char.is_ascii_alphabetic())
    {
        content.push(' ');
    }
    content
}

/// Add leading or trailing whitespace to a snippet, if it's immediately preceded or followed by
/// an identifier or keyword.
pub(crate) fn pad(content: String, range: TextRange, locator: &Locator) -> String {
    pad_start(
        pad_end(content, range.end(), locator),
        range.start(),
        locator,
    )
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, anyhow};
    use ruff_source_file::SourceFileBuilder;
    use test_case::test_case;

    use ruff_python_ast::Stmt;
    use ruff_python_codegen::Stylist;
    use ruff_python_parser::{parse_expression, parse_module};
    use ruff_text_size::{Ranged, TextRange, TextSize};

    use crate::fix::apply_fixes;
    use crate::fix::edits::{
        add_to_dunder_all, make_redundant_alias, next_stmt_break, trailing_semicolon,
    };
    use crate::{Edit, Fix, Locator, Violation};

    /// Parse the given source using [`Mode::Module`] and return the first statement.
    fn parse_first_stmt(source: &str) -> Result<Stmt> {
        let suite = parse_module(source)?.into_suite();
        Ok(suite.into_iter().next().unwrap())
    }

    #[test]
    fn find_semicolon() -> Result<()> {
        let contents = "x = 1";
        let stmt = parse_first_stmt(contents)?;
        let locator = Locator::new(contents);
        assert_eq!(trailing_semicolon(stmt.end(), &locator), None);

        let contents = "x = 1; y = 1";
        let stmt = parse_first_stmt(contents)?;
        let locator = Locator::new(contents);
        assert_eq!(
            trailing_semicolon(stmt.end(), &locator),
            Some(TextSize::from(5))
        );

        let contents = "x = 1 ; y = 1";
        let stmt = parse_first_stmt(contents)?;
        let locator = Locator::new(contents);
        assert_eq!(
            trailing_semicolon(stmt.end(), &locator),
            Some(TextSize::from(6))
        );

        let contents = r"
x = 1 \
  ; y = 1
"
        .trim();
        let stmt = parse_first_stmt(contents)?;
        let locator = Locator::new(contents);
        assert_eq!(
            trailing_semicolon(stmt.end(), &locator),
            Some(TextSize::from(10))
        );

        Ok(())
    }

    #[test]
    fn find_next_stmt_break() {
        let contents = "x = 1; y = 1";
        let locator = Locator::new(contents);
        assert_eq!(
            next_stmt_break(TextSize::from(4), &locator),
            TextSize::from(5)
        );

        let contents = "x = 1 ; y = 1";
        let locator = Locator::new(contents);
        assert_eq!(
            next_stmt_break(TextSize::from(5), &locator),
            TextSize::from(6)
        );

        let contents = r"
x = 1 \
  ; y = 1
"
        .trim();
        let locator = Locator::new(contents);
        assert_eq!(
            next_stmt_break(TextSize::from(10), &locator),
            TextSize::from(12)
        );
    }

    #[test]
    fn redundant_alias() -> Result<()> {
        let contents = "import x, y as y, z as bees";
        let stmt = parse_first_stmt(contents)?;
        assert_eq!(
            make_redundant_alias(["x"].into_iter(), &stmt),
            vec![Edit::range_replacement(
                String::from("x as x"),
                TextRange::new(TextSize::new(7), TextSize::new(8)),
            )],
            "make just one item redundant"
        );
        assert_eq!(
            make_redundant_alias(vec!["x", "y"].into_iter(), &stmt),
            vec![Edit::range_replacement(
                String::from("x as x"),
                TextRange::new(TextSize::new(7), TextSize::new(8)),
            )],
            "the second item is already a redundant alias"
        );
        assert_eq!(
            make_redundant_alias(vec!["x", "z"].into_iter(), &stmt),
            vec![Edit::range_replacement(
                String::from("x as x"),
                TextRange::new(TextSize::new(7), TextSize::new(8)),
            )],
            "the third item is already aliased to something else"
        );
        Ok(())
    }

    #[test_case("()",             &["x", "y"], r#"("x", "y")"#             ; "2 into empty tuple")]
    #[test_case("()",             &["x"],      r#"("x",)"#                 ; "1 into empty tuple adding a trailing comma")]
    #[test_case("[]",             &["x", "y"], r#"["x", "y"]"#             ; "2 into empty list")]
    #[test_case("[]",             &["x"],      r#"["x"]"#                  ; "1 into empty list")]
    #[test_case(r#""a", "b""#,    &["x", "y"], r#""a", "b", "x", "y""#     ; "2 into unparenthesized tuple")]
    #[test_case(r#""a", "b""#,    &["x"],      r#""a", "b", "x""#          ; "1 into unparenthesized tuple")]
    #[test_case(r#""a", "b","#,   &["x", "y"], r#""a", "b", "x", "y","#    ; "2 into unparenthesized tuple w/trailing comma")]
    #[test_case(r#""a", "b","#,   &["x"],      r#""a", "b", "x","#         ; "1 into unparenthesized tuple w/trailing comma")]
    #[test_case(r#"("a", "b")"#,  &["x", "y"], r#"("a", "b", "x", "y")"#   ; "2 into nonempty tuple")]
    #[test_case(r#"("a", "b")"#,  &["x"],      r#"("a", "b", "x")"#        ; "1 into nonempty tuple")]
    #[test_case(r#"("a", "b",)"#, &["x", "y"], r#"("a", "b", "x", "y",)"#  ; "2 into nonempty tuple w/trailing comma")]
    #[test_case(r#"("a", "b",)"#, &["x"],      r#"("a", "b", "x",)"#       ; "1 into nonempty tuple w/trailing comma")]
    #[test_case(r#"["a", "b",]"#, &["x", "y"], r#"["a", "b", "x", "y",]"#  ; "2 into nonempty list w/trailing comma")]
    #[test_case(r#"["a", "b",]"#, &["x"],      r#"["a", "b", "x",]"#       ; "1 into nonempty list w/trailing comma")]
    #[test_case(r#"["a", "b"]"#,  &["x", "y"], r#"["a", "b", "x", "y"]"#   ; "2 into nonempty list")]
    #[test_case(r#"["a", "b"]"#,  &["x"],      r#"["a", "b", "x"]"#        ; "1 into nonempty list")]
    fn add_to_dunder_all_test(raw: &str, names: &[&str], expect: &str) -> Result<()> {
        let locator = Locator::new(raw);
        let edits = {
            let parsed = parse_expression(raw)?;
            let stylist = Stylist::from_tokens(parsed.tokens(), locator.contents());
            add_to_dunder_all(names.iter().copied(), parsed.expr(), &stylist)
        };
        let diag = {
            use crate::rules::pyflakes::rules::BreakOutsideLoop;
            let mut iter = edits.into_iter();
            // The choice of rule here is arbitrary.
            let mut diagnostic = BreakOutsideLoop.into_diagnostic(
                TextRange::default(),
                &SourceFileBuilder::new("<filename>", "<code>").finish(),
            );
            diagnostic.set_fix(Fix::safe_edits(
                iter.next().ok_or(anyhow!("expected edits nonempty"))?,
                iter,
            ));
            diagnostic
        };
        assert_eq!(apply_fixes([diag].iter(), &locator).code, expect);
        Ok(())
    }
}
