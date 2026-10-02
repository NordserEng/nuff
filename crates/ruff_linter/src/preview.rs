//! Helpers to test if a specific preview style is enabled or not.
//!
//! The motivation for these functions isn't to avoid code duplication but to ease promoting preview behavior
//! to stable. The challenge with directly checking the `preview` attribute of [`LinterSettings`] is that it is unclear
//! which specific feature this preview check is for. Having named functions simplifies the promotion:
//! Simply delete the function and let Rust tell you which checks you have to remove.

use crate::settings::{LinterSettings, types::PreviewMode};

// Rule-specific behavior

// https://github.com/astral-sh/ruff/issues/23802
pub(crate) const fn is_annotated_assignment_redefinition_enabled(
    settings: &LinterSettings,
) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/11436
// https://github.com/astral-sh/ruff/pull/11168
pub(crate) const fn is_dunder_init_fix_unused_import_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/11370
pub(crate) const fn is_undefined_export_in_dunder_init_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/20200
pub(crate) const fn is_refined_submodule_import_match_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/22419
pub(crate) const fn is_py315_support_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/22560
pub(crate) const fn is_f811_shadowing_in_type_checking_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/25614
// TODO(brent) Remove ecosystem selector normalization when stabilizing human-readable rule names:
// https://github.com/astral-sh/ruff/pull/27158
pub const fn is_human_readable_names_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/26113
pub const fn is_warn_on_unknown_selectors_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}

// https://github.com/astral-sh/ruff/pull/27666
pub(crate) const fn is_rule_categories_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}
