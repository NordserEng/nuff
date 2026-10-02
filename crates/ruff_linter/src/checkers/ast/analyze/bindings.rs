use ruff_text_size::Ranged;

use crate::Fix;
use crate::checkers::ast::Checker;
use crate::codes::Rule;
use crate::rules::pyflakes;

/// Run lint rules over the [`Binding`]s.
pub(crate) fn bindings(checker: &Checker) {
    if !checker.is_rule_enabled(Rule::UnusedVariable) {
        return;
    }

    for binding in checker.semantic.bindings.iter() {
        {
            if binding.kind.is_bound_exception()
                && binding.is_unused()
                && !checker
                    .settings()
                    .dummy_variable_rgx
                    .is_match(binding.name(checker.source()))
            {
                checker
                    .report_diagnostic(
                        pyflakes::rules::UnusedVariable {
                            name: binding.name(checker.source()).to_string(),
                        },
                        binding.range(),
                    )
                    .try_set_fix(|| {
                        pyflakes::fixes::remove_exception_handler_assignment(
                            binding,
                            checker.locator,
                        )
                        .map(Fix::safe_edit)
                    });
            }
        }
    }
}
