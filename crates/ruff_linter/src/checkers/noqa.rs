//! `noqa` enforcement.

use std::path::Path;

use itertools::Itertools;

use ruff_python_trivia::CommentRanges;
use ruff_text_size::TextRange;

use crate::Locator;
use crate::noqa::{Directive, FileExemption, FileNoqaDirectives, NoqaDirectives, NoqaMapping};
use crate::settings::LinterSettings;

use super::ast::LintContext;

/// Returns the indices of the diagnostics a `noqa` directive suppresses, in ascending order.
pub(crate) fn check_noqa(
    context: &mut LintContext,
    path: &Path,
    locator: &Locator,
    comment_ranges: &CommentRanges,
    noqa_line_for: &NoqaMapping,
    settings: &LinterSettings,
) -> Vec<usize> {
    let file_noqa_directives =
        FileNoqaDirectives::extract(locator, comment_ranges, &settings.external, path);
    let noqa_directives = NoqaDirectives::from_commented_ranges(comment_ranges, path, locator);

    if file_noqa_directives.is_empty() && noqa_directives.is_empty() {
        return Vec::new();
    }

    let exemption = FileExemption::from(&file_noqa_directives);

    let mut ignored_diagnostics = vec![];

    for (index, diagnostic) in context.iter().enumerate() {
        // Syntax errors and other non-lint diagnostics cannot be suppressed.
        let Some(name) = diagnostic.id().as_lint() else {
            continue;
        };

        if exemption.includes_name(name) {
            ignored_diagnostics.push(index);
            continue;
        }

        let suppressed = diagnostic
            .parent()
            .into_iter()
            .chain(diagnostic.range().map(TextRange::start))
            .map(|position| noqa_line_for.resolve(position))
            .unique()
            .filter_map(|offset| noqa_directives.find_line_with_directive(offset))
            .any(|line| match &line.directive {
                Directive::All(_) => true,
                Directive::Codes(directive) => diagnostic
                    .secondary_code()
                    .is_some_and(|code| directive.includes(code)),
            });

        if suppressed {
            ignored_diagnostics.push(index);
        }
    }

    ignored_diagnostics
}
