use ruff_python_ast::helpers;
use ruff_python_ast::{self as ast, Stmt};
use ruff_text_size::Ranged;

use crate::checkers::ast::Checker;
use crate::registry::Rule;
use crate::rules::{flake8_async, nuff, pyflakes, pylint};

/// Run lint rules over a [`Stmt`] syntax node.
pub(crate) fn statement(stmt: &Stmt, checker: &mut Checker) {
    match stmt {
        Stmt::FunctionDef(function_def) => {
            if checker.is_rule_enabled(Rule::RouteReturnsDict) {
                nuff::rules::route_returns_dict(checker, function_def);
            }
            if checker.is_rule_enabled(Rule::ExternalCallInTransaction) {
                nuff::rules::external_call_in_transaction(checker, function_def);
            }
            if checker.is_rule_enabled(Rule::AsyncFunctionWithTimeout) {
                flake8_async::rules::async_function_with_timeout(checker, function_def);
            }
        }
        Stmt::Import(_) => {
            if checker.is_rule_enabled(Rule::ImportOutsideTopLevel) {
                pylint::rules::import_outside_top_level(checker, stmt);
            }
        }
        Stmt::ImportFrom(ast::StmtImportFrom {
            names,
            module,
            level,
            ..
        }) => {
            let module = module.as_deref();
            if checker.is_rule_enabled(Rule::ImportOutsideTopLevel) {
                pylint::rules::import_outside_top_level(checker, stmt);
            }

            for alias in names {
                if module != Some("__future__") && &alias.name == "*" {
                    // F403
                    checker.report_diagnostic_if_enabled(
                        pyflakes::rules::UndefinedLocalWithImportStar {
                            name: helpers::format_import_from(*level, module).to_string(),
                        },
                        stmt.range(),
                    );
                }
            }
        }
        Stmt::Raise(ast::StmtRaise { exc, .. }) => {
            if checker.is_rule_enabled(Rule::RaiseNotImplemented) {
                if let Some(expr) = exc {
                    pyflakes::rules::raise_not_implemented(checker, expr);
                }
            }
        }
        Stmt::If(if_) => {
            if checker.is_rule_enabled(Rule::IfTuple) {
                pyflakes::rules::if_tuple(checker, if_);
            }
        }
        Stmt::Assert(ast::StmtAssert { test, .. }) => {
            if checker.is_rule_enabled(Rule::AssertTuple) {
                pyflakes::rules::assert_tuple(checker, stmt, test);
            }
        }
        Stmt::With(with_stmt @ ast::StmtWith { items, .. }) => {
            if checker.is_rule_enabled(Rule::CancelScopeNoCheckpoint) {
                flake8_async::rules::cancel_scope_no_checkpoint(checker, with_stmt, items);
            }
        }
        Stmt::While(while_stmt) => {
            if checker.is_rule_enabled(Rule::AsyncBusyWait) {
                flake8_async::rules::async_busy_wait(checker, while_stmt);
            }
        }
        Stmt::Try(ast::StmtTry { handlers, .. }) => {
            if checker.is_rule_enabled(Rule::DefaultExceptNotLast) {
                pyflakes::rules::default_except_not_last(checker, handlers, checker.locator);
            }
        }
        _ => {}
    }
}
