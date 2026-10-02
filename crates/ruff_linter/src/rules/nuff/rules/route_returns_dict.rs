use ruff_macros::{ViolationMetadata, derive_message_formats};
use ruff_python_ast::{self as ast, Expr, Operator, StmtFunctionDef};
use ruff_python_semantic::{Modules, SemanticModel};
use ruff_text_size::Ranged;

use crate::Violation;
use crate::checkers::ast::Checker;
use crate::codes::Category;
use crate::rules::fastapi::rules::is_fastapi_route;

/// ## What it does
/// Checks for a FastAPI route whose return annotation is a `dict`.
///
/// ## Why is this bad?
/// A route's return type is its response schema. A Pydantic model names every field, validates the
/// response and gives the generated API types something to describe; a `dict` gives them nothing.
///
/// ## Example
/// ```python
/// from fastapi import APIRouter
///
/// router = APIRouter()
///
///
/// @router.get("/health")
/// async def health() -> dict[str, str]:
///     return {"status": "ok"}
/// ```
///
/// Use instead:
/// ```python
/// from fastapi import APIRouter
/// from pydantic import BaseModel
///
/// router = APIRouter()
///
///
/// class Health(BaseModel):
///     status: str
///
///
/// @router.get("/health")
/// async def health() -> Health:
///     return Health(status="ok")
/// ```
#[derive(ViolationMetadata)]
#[violation_metadata(stable_since = "0.2.0", category = Category::Restriction)]
pub(crate) struct RouteReturnsDict;

impl Violation for RouteReturnsDict {
    #[derive_message_formats]
    fn message(&self) -> String {
        "Route returns a `dict`; return a Pydantic schema model".to_string()
    }
}

/// NUF002
pub(crate) fn route_returns_dict(checker: &Checker, function_def: &StmtFunctionDef) {
    if !checker.semantic().seen_module(Modules::FASTAPI) {
        return;
    }
    let Some(returns) = function_def.returns.as_deref() else {
        return;
    };
    if !is_fastapi_route(function_def, checker.semantic()) {
        return;
    }
    if is_dict_annotation(returns, checker.semantic()) {
        checker.report_diagnostic(RouteReturnsDict, returns.range());
    }
}

fn is_dict_annotation(annotation: &Expr, semantic: &SemanticModel) -> bool {
    match annotation {
        Expr::BinOp(ast::ExprBinOp {
            left,
            op: Operator::BitOr,
            right,
            ..
        }) => is_dict_annotation(left, semantic) || is_dict_annotation(right, semantic),
        Expr::Subscript(ast::ExprSubscript { value, slice, .. }) => {
            if semantic.match_typing_expr(value, "Optional") {
                return is_dict_annotation(slice, semantic);
            }
            if semantic.match_typing_expr(value, "Union") {
                return match slice.as_ref() {
                    Expr::Tuple(tuple) => {
                        tuple.iter().any(|item| is_dict_annotation(item, semantic))
                    }
                    other => is_dict_annotation(other, semantic),
                };
            }
            is_dict_annotation(value, semantic)
        }
        _ => {
            semantic.match_builtin_expr(annotation, "dict")
                || semantic.match_typing_expr(annotation, "Dict")
        }
    }
}
