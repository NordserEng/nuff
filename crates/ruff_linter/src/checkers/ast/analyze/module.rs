use ruff_python_ast::Suite;

use crate::checkers::ast::Checker;
use crate::codes::Rule;
use crate::rules::nuff;

/// Run lint rules over a module.
pub(crate) fn module(suite: &Suite, checker: &Checker) {
    if checker.is_rule_enabled(Rule::AlembicDowngrade) {
        nuff::rules::alembic_downgrade(checker, suite);
    }
}
