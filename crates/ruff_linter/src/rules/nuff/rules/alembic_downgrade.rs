use ruff_macros::{ViolationMetadata, derive_message_formats};
use ruff_python_ast::{Expr, Stmt, Suite};
use ruff_text_size::Ranged;

use crate::Violation;
use crate::checkers::ast::Checker;
use crate::codes::Category;

/// ## What it does
/// Checks for an Alembic revision that defines a `downgrade` function.
///
/// ## Why is this bad?
/// Migrations run forward only. Undoing a change is the next revision, so the schema history is
/// one chain that every database replays the same way, and a `downgrade` is code that never runs and
/// never gets tested.
///
/// A module is a revision when it assigns `down_revision` at the top level.
///
/// ## Example
/// ```python
/// revision = "0002"
/// down_revision = "0001"
///
///
/// def upgrade() -> None: ...
///
///
/// def downgrade() -> None: ...
/// ```
///
/// Use instead:
/// ```python
/// revision = "0002"
/// down_revision = "0001"
///
///
/// def upgrade() -> None: ...
/// ```
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "0.2.0", category = Category::Restriction)]
pub(crate) struct AlembicDowngrade;

impl Violation for AlembicDowngrade {
    #[derive_message_formats]
    fn message(&self) -> String {
        "Alembic revision defines `downgrade`; undo a change in the next revision".to_string()
    }
}

/// NUF003
pub(crate) fn alembic_downgrade(checker: &Checker, suite: &Suite) {
    let is_revision = suite.iter().any(|stmt| match stmt {
        Stmt::Assign(assign) => assign.targets.iter().any(is_down_revision),
        Stmt::AnnAssign(assign) => is_down_revision(&assign.target),
        _ => false,
    });
    if !is_revision {
        return;
    }
    for stmt in suite {
        if let Stmt::FunctionDef(function_def) = stmt {
            if function_def.name.as_str() == "downgrade" {
                checker.report_diagnostic(AlembicDowngrade, function_def.name.range());
            }
        }
    }
}

fn is_down_revision(target: &Expr) -> bool {
    target
        .as_name_expr()
        .is_some_and(|name| name.id.as_str() == "down_revision")
}
