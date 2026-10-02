use crate::fix::codemods::CodegenStylist;
use anyhow::{Result, bail};
use libcst_native::{
    Call, Dict, Expression, ImportAlias, ImportFrom, ImportNames, LazyImportFrom, NameOrAttribute,
    SmallStatement, Statement,
};
use ruff_python_codegen::Stylist;

pub(crate) fn match_statement(statement_text: &str) -> Result<Statement<'_>> {
    match libcst_native::parse_statement(statement_text) {
        Ok(statement) => Ok(statement),
        Err(_) => bail!("Failed to extract statement from source"),
    }
}

pub(crate) fn match_import_from<'a, 'b>(
    statement: &'a mut Statement<'b>,
) -> Result<(
    &'a mut Option<NameOrAttribute<'b>>,
    &'a mut ImportNames<'b>,
    bool,
)> {
    let Statement::Simple(statement) = statement else {
        bail!("Expected Statement::Simple")
    };
    match statement.body.first_mut() {
        Some(SmallStatement::ImportFrom(ImportFrom { module, names, .. })) => {
            Ok((module, names, false))
        }
        Some(SmallStatement::LazyImportFrom(LazyImportFrom { module, names, .. })) => {
            Ok((module, names, true))
        }
        _ => bail!("Expected SmallStatement::ImportFrom | SmallStatement::LazyImportFrom"),
    }
}

pub(crate) fn match_aliases<'a, 'b>(
    names: &'a mut ImportNames<'b>,
) -> Result<&'a mut Vec<ImportAlias<'b>>> {
    if let ImportNames::Aliases(aliases) = names {
        Ok(aliases)
    } else {
        bail!("Expected ImportNames::Aliases")
    }
}

pub(crate) fn match_call_mut<'a, 'b>(
    expression: &'a mut Expression<'b>,
) -> Result<&'a mut Call<'b>> {
    if let Expression::Call(call) = expression {
        Ok(call)
    } else {
        bail!("Expected Expression::Call")
    }
}

pub(crate) fn match_dict<'a, 'b>(expression: &'a mut Expression<'b>) -> Result<&'a mut Dict<'b>> {
    if let Expression::Dict(dict) = expression {
        Ok(dict)
    } else {
        bail!("Expected Expression::Dict")
    }
}

/// Given the source code for an expression, return the parsed [`Expression`].
///
/// If the expression is not guaranteed to be valid as a standalone expression (e.g., if it may
/// span multiple lines and/or require parentheses), use [`transform_expression`] instead.
pub(crate) fn match_expression(expression_text: &str) -> Result<Expression<'_>> {
    match libcst_native::parse_expression(expression_text) {
        Ok(expression) => Ok(expression),
        Err(_) => bail!("Failed to extract expression from source"),
    }
}

/// Run a transformation function over an expression.
///
/// Passing an expression to [`match_expression`] directly can lead to parse errors if the
/// expression is not a valid standalone expression (e.g., it was parenthesized in the original
/// source). This method instead wraps the expression in "fake" parentheses, runs the
/// transformation, then removes the "fake" parentheses.
pub(crate) fn transform_expression(
    source_code: &str,
    stylist: &Stylist,
    func: impl FnOnce(Expression) -> Result<Expression>,
) -> Result<String> {
    // Wrap the expression in parentheses.
    let source_code = format!("({source_code})");
    let expression = match_expression(&source_code)?;

    // Run the function on the expression.
    let expression = func(expression)?;

    // Codegen the expression.
    let mut source_code = expression.codegen_stylist(stylist);

    // Drop the outer parentheses.
    source_code.drain(0..1);
    source_code.drain(source_code.len() - 1..source_code.len());
    Ok(source_code)
}
