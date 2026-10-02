//! Helpers to test if a specific preview style is enabled or not.
//!
//! The motivation for these functions isn't to avoid code duplication but to ease promoting preview behavior
//! to stable. The challenge with directly checking the `preview` attribute of [`LinterSettings`] is that it is unclear
//! which specific feature this preview check is for. Having named functions simplifies the promotion:
//! Simply delete the function and let Rust tell you which checks you have to remove.

use crate::settings::{LinterSettings, types::PreviewMode};

// Rule-specific behavior

pub(crate) const fn is_annotated_assignment_redefinition_enabled(
    settings: &LinterSettings,
) -> bool {
    settings.preview.is_enabled()
}

pub(crate) const fn is_dunder_init_fix_unused_import_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

pub(crate) const fn is_undefined_export_in_dunder_init_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

pub(crate) const fn is_refined_submodule_import_match_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

pub(crate) const fn is_py315_support_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

pub(crate) const fn is_f811_shadowing_in_type_checking_enabled(settings: &LinterSettings) -> bool {
    settings.preview.is_enabled()
}

// TODO Remove ecosystem selector normalization when stabilizing human-readable rule names:
pub const fn is_human_readable_names_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}

pub const fn is_warn_on_unknown_selectors_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}

pub(crate) const fn is_rule_categories_enabled(preview: PreviewMode) -> bool {
    preview.is_enabled()
}
