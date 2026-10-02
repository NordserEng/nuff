use nuff_python_semantic::ScopeKind;

use crate::checkers::ast::Checker;
use crate::codes::Rule;
use crate::rules::pyflakes;

/// Run lint rules over all deferred scopes in the [`SemanticModel`].
pub(crate) fn deferred_scopes(checker: &Checker) {
    if !checker.any_rule_enabled(&[
        Rule::ImportShadowedByLoopVar,
        Rule::RedefinedWhileUnused,
        Rule::UndefinedLocal,
        Rule::UnusedAnnotation,
        Rule::UnusedImport,
        Rule::UnusedVariable,
    ]) {
        return;
    }

    for scope_id in checker.analyze.scopes.iter().rev().copied() {
        let scope = &checker.semantic.scopes[scope_id];

        if checker.is_rule_enabled(Rule::UndefinedLocal) {
            pyflakes::rules::undefined_local(checker, scope_id, scope);
        }

        if checker.is_rule_enabled(Rule::ImportShadowedByLoopVar) {
            pyflakes::rules::import_shadowed_by_loop_var(checker, scope_id, scope);
        }

        if checker.is_rule_enabled(Rule::RedefinedWhileUnused) {
            pyflakes::rules::redefined_while_unused(checker, scope_id, scope);
        }

        if matches!(scope.kind, ScopeKind::Function(_) | ScopeKind::Lambda(_)) {
            if checker.is_rule_enabled(Rule::UnusedVariable)
                && !(scope.uses_locals() && scope.kind.is_function())
            {
                let unused_bindings = scope
                    .bindings()
                    .map(|(name, binding_id)| (name, checker.semantic().binding(binding_id)))
                    .filter_map(|(name, binding)| {
                        if (binding.kind.is_assignment()
                            || binding.kind.is_named_expr_assignment()
                            || binding.kind.is_with_item_var())
                            && binding.is_unused()
                            && !binding.is_nonlocal()
                            && !binding.is_global()
                            && !checker.settings().dummy_variable_rgx.is_match(name)
                            && !matches!(
                                name,
                                "__tracebackhide__"
                                    | "__traceback_info__"
                                    | "__traceback_supplement__"
                                    | "__debuggerskip__"
                            )
                        {
                            return Some((name, binding));
                        }

                        None
                    });

                for (unused_name, unused_binding) in unused_bindings {
                    pyflakes::rules::unused_variable(checker, unused_name, unused_binding);
                }
            }

            if checker.is_rule_enabled(Rule::UnusedAnnotation) {
                pyflakes::rules::unused_annotation(checker, scope);
            }
        }

        if matches!(scope.kind, ScopeKind::Function(_) | ScopeKind::Module) {
            if checker.is_rule_enabled(Rule::UnusedImport) {
                pyflakes::rules::unused_import(checker, scope);
            }
        }
    }
}
