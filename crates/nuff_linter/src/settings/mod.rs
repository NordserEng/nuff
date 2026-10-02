//! Effective program settings, taking into account pyproject.toml and
//! command-line options. Structure is optimized for internal usage, as opposed
//! to external visibility or parsing.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use types::CompiledPerFileTargetVersionList;

use nuff_macros::CacheKey;
use nuff_python_ast::PythonVersion;

use crate::registry::Linter;
use crate::registry::Rule;
use crate::rules::{nuff, pyflakes};
use crate::settings::types::{CompiledPerFileIgnoreList, ExtensionMapping, FilePatternSet};
use crate::{RuleSelector, fs};

use self::fix_safety_table::FixSafetyTable;
use self::rule_table::RuleTable;
use self::types::PreviewMode;
use crate::rule_selector::PreviewOptions;

pub mod fix_safety_table;
pub mod flags;
pub mod rule_table;
pub mod types;

#[derive(Debug, Clone, CacheKey)]
#[expect(clippy::struct_excessive_bools)]
pub struct LinterSettings {
    pub exclude: FilePatternSet,
    pub extension: ExtensionMapping,
    pub project_root: PathBuf,

    pub rules: RuleTable,
    pub per_file_ignores: CompiledPerFileIgnoreList,
    pub fix_safety: FixSafetyTable,

    /// The non-path-resolved Python version specified by the `target-version` input option.
    ///
    /// If you have a `Checker` available, see its `target_version` method instead.
    ///
    /// Otherwise, see [`LinterSettings::resolve_target_version`] for a way to obtain the Python
    /// version for a given file, while respecting the overrides in `per_file_target_version`.
    pub unresolved_target_version: TargetVersion,
    /// Path-specific overrides to `unresolved_target_version`.
    ///
    /// If you have a `Checker` available, see its `target_version` method instead.
    ///
    /// Otherwise, see [`LinterSettings::resolve_target_version`] for a way to check a given
    /// [`Path`] against these patterns, while falling back to `unresolved_target_version` if none
    /// of them match.
    pub per_file_target_version: CompiledPerFileTargetVersionList,
    pub preview: PreviewMode,
    pub explicit_preview_rules: bool,

    // Rule-specific settings
    pub builtins: Vec<String>,
    pub dummy_variable_rgx: Regex,
    pub external: Vec<String>,
    pub ignore_init_module_imports: bool,
    pub logger_objects: Vec<String>,
    pub namespace_packages: Vec<PathBuf>,
    pub src: Vec<PathBuf>,
    pub typing_modules: Vec<String>,
    pub typing_extensions: bool,
    pub future_annotations: bool,

    // Plugins
    pub nuff: nuff::settings::Settings,
    pub pyflakes: pyflakes::settings::Settings,
}

pub const DEFAULT_SELECTORS: &[RuleSelector] = &[RuleSelector::Linter(Linter::Pyflakes)];

pub static DUMMY_VARIABLE_RGX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$").unwrap());

impl LinterSettings {
    pub fn for_rule(rule_code: Rule) -> Self {
        Self {
            rules: RuleTable::from_iter([rule_code]),
            unresolved_target_version: PythonVersion::latest().into(),
            ..Self::default()
        }
    }

    pub fn for_rules(rules: impl IntoIterator<Item = Rule>) -> Self {
        Self {
            rules: RuleTable::from_iter(rules),
            unresolved_target_version: PythonVersion::latest().into(),
            ..Self::default()
        }
    }

    pub fn new(project_root: &Path) -> Self {
        Self {
            exclude: FilePatternSet::default(),
            unresolved_target_version: TargetVersion(None),
            per_file_target_version: CompiledPerFileTargetVersionList::default(),
            project_root: project_root.to_path_buf(),
            rules: DEFAULT_SELECTORS
                .iter()
                .flat_map(|selector| selector.rules(&PreviewOptions::default()))
                .collect(),

            // Needs duplicating
            builtins: vec![],
            dummy_variable_rgx: DUMMY_VARIABLE_RGX.clone(),

            external: vec![],
            ignore_init_module_imports: true,
            logger_objects: vec![],
            namespace_packages: vec![],

            per_file_ignores: CompiledPerFileIgnoreList::default(),
            fix_safety: FixSafetyTable::default(),

            src: vec![fs::get_cwd().to_path_buf(), fs::get_cwd().join("src")],
            typing_modules: vec![],
            nuff: nuff::settings::Settings::default(),
            pyflakes: pyflakes::settings::Settings::default(),
            preview: PreviewMode::default(),
            explicit_preview_rules: false,
            extension: ExtensionMapping::default(),
            typing_extensions: true,
            future_annotations: false,
        }
    }

    #[must_use]
    pub fn with_target_version(mut self, target_version: PythonVersion) -> Self {
        self.unresolved_target_version = target_version.into();
        self
    }

    #[must_use]
    pub fn with_preview_mode(mut self) -> Self {
        self.preview = PreviewMode::Enabled;
        self
    }

    #[must_use]
    pub fn with_external_rules(mut self, rules: &[&str]) -> Self {
        self.external
            .extend(rules.iter().map(std::string::ToString::to_string));
        self
    }

    /// Resolve the [`TargetVersion`] to use for linting.
    ///
    /// This method respects the per-file version overrides in
    /// [`LinterSettings::per_file_target_version`] and falls back on
    /// [`LinterSettings::unresolved_target_version`] if none of the override patterns match.
    pub fn resolve_target_version(&self, path: &Path) -> TargetVersion {
        self.per_file_target_version
            .is_match(path)
            .map_or(self.unresolved_target_version, TargetVersion::from)
    }
}

impl Default for LinterSettings {
    fn default() -> Self {
        Self::new(fs::get_cwd())
    }
}

/// A thin wrapper around `Option<PythonVersion>` to clarify the reason for different `unwrap`
/// calls in various places.
///
/// For example, we want to default to `PythonVersion::latest()` for parsing and detecting semantic
/// syntax errors because this will minimize version-related diagnostics when the Python version is
/// unset. In contrast, we want to default to `PythonVersion::default()` for lint rules. These
/// correspond to the [`TargetVersion::parser_version`] and [`TargetVersion::linter_version`]
/// methods, respectively.
#[derive(Debug, Clone, Copy, CacheKey, PartialEq, Eq)]
pub struct TargetVersion(pub Option<PythonVersion>);

impl TargetVersion {
    /// Return the [`PythonVersion`] to use for parsing.
    ///
    /// This will be either the Python version specified by the user or the latest supported
    /// version if unset.
    pub fn parser_version(&self) -> PythonVersion {
        self.0.unwrap_or_else(PythonVersion::latest)
    }

    /// Return the [`PythonVersion`] to use for version-dependent lint rules.
    ///
    /// This will either be the Python version specified by the user or the default Python version
    /// if unset.
    pub fn linter_version(&self) -> PythonVersion {
        self.0.unwrap_or_default()
    }
}

impl From<PythonVersion> for TargetVersion {
    fn from(value: PythonVersion) -> Self {
        Self(Some(value))
    }
}
