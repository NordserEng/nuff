use nuff_python_ast::helpers::{map_callable, map_subscript};
use nuff_python_ast::{Expr, PythonVersion, StmtFunctionDef};
use nuff_python_semantic::{Modules, ScopeKind, SemanticModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnnotationContext {
    /// Python will evaluate the annotation at runtime, but it's not _required_ and, as such, could
    /// be quoted to convert it into a typing-only annotation.
    ///
    /// For example:
    /// ```python
    /// from pandas import DataFrame
    ///
    /// def foo() -> DataFrame:
    ///    ...
    /// ```
    ///
    /// Above, Python will evaluate `DataFrame` at runtime in order to add it to `__annotations__`.
    RuntimeEvaluated,
    /// The annotation is only evaluated at type-checking time.
    TypingOnly,
}

impl AnnotationContext {
    /// Determine the [`AnnotationContext`] for an annotation based on the current scope of the
    /// semantic model.
    pub(super) fn from_model(semantic: &SemanticModel, version: PythonVersion) -> Self {
        // If `__future__` annotations are enabled or it's a stub file,
        // then annotations are never evaluated at runtime,
        // so we can treat them as typing-only.
        if semantic.future_annotations_or_stub() || version.defers_annotations() {
            return Self::TypingOnly;
        }

        // Otherwise, if we're in a class or module scope, then the annotation needs to
        // be available at runtime.
        // See: https://docs.python.org/3/reference/simple_stmts.html#annotated-assignment-statements
        if matches!(
            semantic.current_scope().kind,
            ScopeKind::Class(_) | ScopeKind::Module
        ) {
            return Self::RuntimeEvaluated;
        }

        Self::TypingOnly
    }

    /// Determine the [`AnnotationContext`] to use for annotations in a function signature.
    pub(super) fn from_function(semantic: &SemanticModel, version: PythonVersion) -> Self {
        if semantic.future_annotations_or_stub() || version.defers_annotations() {
            Self::TypingOnly
        } else {
            Self::RuntimeEvaluated
        }
    }
}

/// Returns `true` if an annotation will be inspected at runtime by the `dataclasses` module.
///
/// Specifically, detects whether an annotation is to either `dataclasses.InitVar` or
/// `typing.ClassVar` within a `@dataclass` class definition.
///
/// See: <https://docs.python.org/3/library/dataclasses.html#init-only-variables>
pub(super) fn is_dataclass_meta_annotation(annotation: &Expr, semantic: &SemanticModel) -> bool {
    if !semantic.seen_module(Modules::DATACLASSES) {
        return false;
    }

    if let ScopeKind::Class(class_def) = semantic.current_scope().kind {
        if class_def.decorator_list.iter().any(|decorator| {
            semantic
                .resolve_qualified_name(map_callable(&decorator.expression))
                .is_some_and(|qualified_name| {
                    matches!(qualified_name.segments(), ["dataclasses", "dataclass"])
                })
        }) {
            return semantic
                .resolve_qualified_name(map_subscript(annotation))
                .is_some_and(|qualified_name| {
                    matches!(
                        qualified_name.segments(),
                        ["dataclasses", "InitVar" | "KW_ONLY"]
                    ) || semantic.match_typing_qualified_name(&qualified_name, "ClassVar")
                });
        }
    }

    false
}

/// Returns `true` if a function is registered as a `singledispatch` or `singledispatchmethod`
/// implementation, like `_` in:
/// ```python
/// @singledispatch
/// def fun(arg): ...
///
/// @fun.register
/// def _(arg: int): ...
/// ```
pub(super) fn is_singledispatch_implementation(
    function_def: &StmtFunctionDef,
    semantic: &SemanticModel,
) -> bool {
    function_def.decorator_list.iter().any(|decorator| {
        let Expr::Attribute(attribute) = &decorator.expression else {
            return false;
        };

        if attribute.attr.as_str() != "register" {
            return false;
        }

        let Some(id) = semantic.lookup_attribute(attribute.value.as_ref()) else {
            return false;
        };

        let binding = semantic.binding(id);
        let Some(function_def) = binding
            .kind
            .as_function_definition()
            .map(|id| &semantic.scopes[*id])
            .and_then(|scope| scope.kind.as_function())
        else {
            return false;
        };

        function_def.decorator_list.iter().any(|decorator| {
            semantic
                .resolve_qualified_name(&decorator.expression)
                .is_some_and(|qualified_name| {
                    matches!(
                        qualified_name.segments(),
                        ["functools", "singledispatch" | "singledispatchmethod"]
                    )
                })
        })
    })
}
