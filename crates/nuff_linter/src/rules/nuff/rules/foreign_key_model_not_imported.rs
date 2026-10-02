use nuff_macros::{ViolationMetadata, derive_message_formats};
use nuff_python_ast::{Expr, ExprCall, Stmt};
use nuff_text_size::Ranged;

use crate::Violation;
use crate::checkers::ast::Checker;
use crate::codes::Category;

/// ## What it does
/// Checks for a SQLAlchemy `ForeignKey` in a module that defines tables, whose target table is
/// defined in a module this one does not import.
///
/// ## Why is this bad?
/// SQLAlchemy resolves a foreign key's table by name from the tables already registered on the
/// metadata. A module that points at another module's table but does not import it only works when
/// something else happened to import that module first, so loading it alone fails at the first
/// flush.
///
/// Every table a foreign key names must be defined in this module or listed in
/// `lint.nuff.foreign-key-modules`, so a new table cannot slip past the rule. A module that defines
/// no table, such as an Alembic revision, names its tables in the database rather than the metadata
/// and is not checked.
///
/// ## Example
/// ```python
/// from sqlalchemy import ForeignKey
/// from sqlalchemy.orm import Mapped, mapped_column
///
///
/// class Charge(Base):
///     __tablename__ = "charges"
///     company_id: Mapped[int] = mapped_column(ForeignKey("companies.id"))
/// ```
///
/// Use instead:
/// ```python
/// from sqlalchemy import ForeignKey
/// from sqlalchemy.orm import Mapped, mapped_column
///
/// import app.accounts.models  # noqa: F401
///
///
/// class Charge(Base):
///     __tablename__ = "charges"
///     company_id: Mapped[int] = mapped_column(ForeignKey("companies.id"))
/// ```
///
/// ## Options
/// - `lint.nuff.foreign-key-modules`
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "0.2.0", category = Category::Restriction)]
pub(crate) struct ForeignKeyModelNotImported {
    table: String,
    module: Option<String>,
}

impl Violation for ForeignKeyModelNotImported {
    #[derive_message_formats]
    fn message(&self) -> String {
        let ForeignKeyModelNotImported { table, module } = self;
        match module {
            Some(module) => format!("Foreign key to `{table}` without importing `{module}`"),
            None => format!("Foreign key to `{table}`, which `foreign-key-modules` does not list"),
        }
    }
}

/// NUF004
pub(crate) fn foreign_key_model_not_imported(checker: &Checker, call: &ExprCall) {
    if !checker
        .semantic()
        .resolve_qualified_name(&call.func)
        .is_some_and(|name| {
            matches!(
                name.segments(),
                ["sqlalchemy", "ForeignKey"]
                    | ["sqlalchemy", "schema" | "sql", "ForeignKey"]
                    | ["sqlalchemy", "sql", "schema", "ForeignKey"]
            )
        })
    {
        return;
    }
    let Some(target) = call
        .arguments
        .find_argument_value("column", 0)
        .and_then(Expr::as_string_literal_expr)
    else {
        return;
    };
    let target = target.value.to_str();
    let parts: Vec<&str> = target.split('.').collect();
    let [.., table, _column] = parts.as_slice() else {
        return;
    };
    let suite = checker.module.python_ast;
    let defined = defined_tables(suite);
    if defined.is_empty() || defined.contains(table) {
        return;
    }
    let module = checker.settings().nuff.foreign_key_modules.get(*table);
    if let Some(module) = module {
        let current = checker.module.qualified_name().map(|name| name.join("."));
        if current.as_deref() == Some(module.as_str()) || imports_module(suite, module) {
            return;
        }
    }
    checker.report_diagnostic(
        ForeignKeyModelNotImported {
            table: (*table).to_string(),
            module: module.cloned(),
        },
        target_range(call),
    );
}

fn target_range(call: &ExprCall) -> nuff_text_size::TextRange {
    call.arguments
        .find_argument_value("column", 0)
        .map_or(call.range(), Ranged::range)
}

fn defined_tables(suite: &[Stmt]) -> Vec<&str> {
    suite
        .iter()
        .filter_map(Stmt::as_class_def_stmt)
        .flat_map(|class_def| &class_def.body)
        .filter_map(|stmt| {
            let (targets, value): (Vec<&Expr>, Option<&Expr>) = match stmt {
                Stmt::Assign(assign) => (assign.targets.iter().collect(), Some(&assign.value)),
                Stmt::AnnAssign(assign) => (vec![&assign.target], assign.value.as_deref()),
                _ => return None,
            };
            let is_tablename = targets.iter().any(|target| {
                target
                    .as_name_expr()
                    .is_some_and(|name| name.id.as_str() == "__tablename__")
            });
            if !is_tablename {
                return None;
            }
            value
                .and_then(Expr::as_string_literal_expr)
                .map(|value| value.value.to_str())
        })
        .collect()
}

fn imports_module(suite: &[Stmt], module: &str) -> bool {
    suite.iter().any(|stmt| match stmt {
        Stmt::Import(import) => import
            .names
            .iter()
            .any(|alias| alias.name.as_str() == module),
        Stmt::ImportFrom(import) if import.level == 0 => {
            let Some(from) = import.module.as_deref() else {
                return false;
            };
            from == module
                || import.names.iter().any(|alias| {
                    module
                        .strip_prefix(from)
                        .and_then(|rest| rest.strip_prefix('.'))
                        == Some(alias.name.as_str())
                })
        }
        _ => false,
    })
}
