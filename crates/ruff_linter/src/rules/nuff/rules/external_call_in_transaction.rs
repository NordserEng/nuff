use ruff_python_ast::name::QualifiedName;
use rustc_hash::FxHashSet;

use ruff_macros::{ViolationMetadata, derive_message_formats};
use ruff_python_ast::visitor::{self, Visitor};
use ruff_python_ast::{self as ast, Expr, Stmt, StmtFunctionDef};
use ruff_python_semantic::SemanticModel;
use ruff_text_size::Ranged;

use crate::Violation;
use crate::checkers::ast::Checker;
use crate::codes::Category;

/// ## What it does
/// Checks for a call to a configured outside service while the function's SQLAlchemy session has a
/// transaction open.
///
/// ## Why is this bad?
/// An open transaction holds a pooled connection, and any row lock it took, for as long as the
/// outside service takes to answer. A slow payment provider or registry then exhausts the pool, or
/// the database's idle-in-transaction timeout closes the connection under the request.
///
/// End the read with `commit()`, make the call, then re-read the row `with_for_update` and re-check
/// it before writing.
///
/// ## Scope
/// The rule follows one function, in source order, and an `if` branch that ends in `return` or
/// `raise` does not carry its state past the `if`. A session is a parameter annotated as a
/// SQLAlchemy `Session` or `AsyncSession`. Reading, adding or flushing through it opens the
/// transaction, and so does passing it to another function, which may read through it.
/// `commit()`, `rollback()` or `close()` ends it. A commit inside a function the session was passed
/// to is invisible, so commit in the function that makes the outside call. A session reached any
/// other way is outside the rule; tests that count the transactions idle at the moment of the call
/// cover those paths.
///
/// ## Example
/// ```python
/// async def cancel(session: AsyncSession, company_id: int) -> None:
///     company = await session.get(Company, company_id)
///     await vipps.stop_agreement(company.agreement_id)
/// ```
///
/// Use instead:
/// ```python
/// async def cancel(session: AsyncSession, company_id: int) -> None:
///     company = await session.get(Company, company_id)
///     agreement_id = company.agreement_id
///     await session.commit()
///     await vipps.stop_agreement(agreement_id)
///     company = await session.get(Company, company_id, with_for_update=True)
/// ```
///
/// ## Options
/// - `lint.nuff.external-functions`
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "0.2.0", category = Category::Restriction)]
pub(crate) struct ExternalCallInTransaction {
    name: String,
}

impl Violation for ExternalCallInTransaction {
    #[derive_message_formats]
    fn message(&self) -> String {
        let ExternalCallInTransaction { name } = self;
        format!("`{name}` is called while the session's transaction is open; commit first")
    }
}

const OPENS: &[&str] = &[
    "add",
    "add_all",
    "delete",
    "execute",
    "flush",
    "get",
    "get_one",
    "merge",
    "refresh",
    "scalar",
    "scalars",
    "stream",
    "stream_scalars",
];
const ENDS: &[&str] = &["close", "commit", "rollback"];

/// NUF005
pub(crate) fn external_call_in_transaction(checker: &Checker, function_def: &StmtFunctionDef) {
    let external = &checker.settings().nuff.external_functions;
    if external.is_empty() {
        return;
    }
    let semantic = checker.semantic();
    let sessions: FxHashSet<&str> = function_def
        .parameters
        .iter()
        .filter(|parameter| {
            parameter
                .annotation()
                .is_some_and(|annotation| is_session_annotation(annotation, semantic))
        })
        .map(|parameter| parameter.name().as_str())
        .collect();
    if sessions.is_empty() {
        return;
    }
    let mut walker = TransactionWalker {
        checker,
        external,
        sessions,
        open: false,
    };
    walker.visit_body(&function_def.body);
}

fn ends_flow(body: &[Stmt]) -> bool {
    matches!(
        body.last(),
        Some(Stmt::Return(_) | Stmt::Raise(_) | Stmt::Break(_) | Stmt::Continue(_))
    )
}

fn is_session_annotation(annotation: &Expr, semantic: &SemanticModel) -> bool {
    if let Expr::Subscript(ast::ExprSubscript { value, slice, .. }) = annotation {
        if semantic.match_typing_expr(value, "Annotated") {
            if let Expr::Tuple(tuple) = slice.as_ref() {
                return tuple
                    .iter()
                    .next()
                    .is_some_and(|first| is_session_annotation(first, semantic));
            }
        }
        return false;
    }
    semantic
        .resolve_qualified_name(annotation)
        .is_some_and(|name| is_session(&name))
}

fn is_session(name: &QualifiedName) -> bool {
    matches!(
        name.segments(),
        ["sqlalchemy", "orm", "Session"]
            | ["sqlalchemy", "orm", "session", "Session"]
            | ["sqlalchemy", "ext", "asyncio", "AsyncSession"]
            | ["sqlalchemy", "ext", "asyncio", "session", "AsyncSession"]
    )
}

struct TransactionWalker<'a, 'b> {
    checker: &'a Checker<'b>,
    external: &'a [String],
    sessions: FxHashSet<&'a str>,
    open: bool,
}

impl TransactionWalker<'_, '_> {
    fn session_method<'e>(&self, func: &'e Expr) -> Option<&'e str> {
        let Expr::Attribute(ast::ExprAttribute { value, attr, .. }) = func else {
            return None;
        };
        let name = value.as_name_expr()?;
        self.sessions
            .contains(name.id.as_str())
            .then_some(attr.as_str())
    }

    fn passes_session(&self, call: &ast::ExprCall) -> bool {
        call.arguments.iter_source_order().any(|argument| {
            argument
                .value()
                .as_name_expr()
                .is_some_and(|name| self.sessions.contains(name.id.as_str()))
        })
    }

    fn external_name(&self, expr: &Expr) -> Option<String> {
        let name = self
            .checker
            .semantic()
            .resolve_qualified_name(expr)?
            .to_string();
        self.external.contains(&name).then_some(name)
    }

    fn check_call(&self, call: &ast::ExprCall) {
        if !self.open {
            return;
        }
        if let Some(name) = self.external_name(&call.func) {
            self.checker
                .report_diagnostic(ExternalCallInTransaction { name }, call.func.range());
            return;
        }
        let is_to_thread = self
            .checker
            .semantic()
            .resolve_qualified_name(&call.func)
            .is_some_and(|name| matches!(name.segments(), ["asyncio", "to_thread"]));
        if is_to_thread {
            if let Some(first) = call.arguments.args.first() {
                if let Some(name) = self.external_name(first) {
                    self.checker
                        .report_diagnostic(ExternalCallInTransaction { name }, first.range());
                }
            }
        }
    }
}

impl<'a> Visitor<'a> for TransactionWalker<'_, '_> {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {}
            Stmt::If(ast::StmtIf {
                test,
                body,
                elif_else_clauses,
                ..
            }) => {
                self.visit_expr(test);
                let before = self.open;
                let mut after = None;
                let mut branch = |walker: &mut Self, body: &'a [Stmt]| {
                    walker.open = before;
                    walker.visit_body(body);
                    if !ends_flow(body) {
                        after = Some(after.unwrap_or(false) || walker.open);
                    }
                };
                branch(self, body);
                let mut has_else = false;
                for clause in elif_else_clauses {
                    self.open = before;
                    if let Some(test) = &clause.test {
                        self.visit_expr(test);
                    } else {
                        has_else = true;
                    }
                    branch(self, &clause.body);
                }
                if !has_else {
                    after = Some(after.unwrap_or(false) || before);
                }
                self.open = after.unwrap_or(before);
            }
            Stmt::With(ast::StmtWith { items, body, .. }) => {
                let begins = items.iter().any(|item| {
                    item.context_expr
                        .as_call_expr()
                        .and_then(|call| self.session_method(&call.func))
                        == Some("begin")
                });
                for item in items {
                    self.visit_with_item(item);
                }
                if begins {
                    self.open = true;
                }
                self.visit_body(body);
                if begins {
                    self.open = false;
                }
            }
            _ => visitor::walk_stmt(self, stmt),
        }
    }

    fn visit_expr(&mut self, expr: &'a Expr) {
        match expr {
            Expr::Lambda(_) => {}
            Expr::Call(call) => {
                visitor::walk_expr(self, expr);
                match self.session_method(&call.func) {
                    Some(method) if OPENS.contains(&method) => self.open = true,
                    Some(method) if ENDS.contains(&method) => self.open = false,
                    Some(_) => {}
                    None => {
                        self.check_call(call);
                        if self.passes_session(call) {
                            self.open = true;
                        }
                    }
                }
            }
            _ => visitor::walk_expr(self, expr),
        }
    }
}
