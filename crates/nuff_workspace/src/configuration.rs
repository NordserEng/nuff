//! User-provided program settings, taking into account pyproject.toml and
//! command-line options. Structure mirrors the user-facing representation of
//! the various parameters.

use std::borrow::Cow;
use std::env::VarError;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use glob::{GlobError, Paths, PatternError, glob};
use itertools::Itertools;
use log::debug;
use nuff_linter::codes::Category;
use nuff_linter::preview::is_warn_on_unknown_selectors_enabled;
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use shellexpand;
use shellexpand::LookupError;
use strum::IntoEnumIterator;

use nuff_cache::cache_dir;
use nuff_linter::registry::{Rule, RuleSet};
use nuff_linter::rule_selector::{PreviewOptions, RuleResolutionError, Specificity};
use nuff_linter::settings::fix_safety_table::FixSafetyTable;
use nuff_linter::settings::rule_table::RuleTable;
use nuff_linter::settings::types::{
    CompiledPerFileIgnoreList, CompiledPerFileTargetVersionList, ExtensionMapping, FilePattern,
    FilePatternSet, GlobPath, OutputFormat, PerFileIgnore, PerFileTargetVersion, PreviewMode,
    RequiredVersion, UnsafeFixes,
};
use nuff_linter::settings::{DEFAULT_SELECTORS, DUMMY_VARIABLE_RGX, LinterSettings, TargetVersion};
use nuff_linter::{
    RuleSelector, UnresolvedRuleSelector, fs, warn_user_once, warn_user_once_by_message,
};
use nuff_python_ast as ast;

use crate::options::{
    LintOptions, NuffOptions, Options, PyflakesOptions, validate_required_version,
};
use crate::pyproject;
use crate::resolver::ConfigurationOrigin;
use crate::settings::{EXCLUDE, FileResolverSettings, INCLUDE, INCLUDE_PREVIEW, Settings};

#[derive(Clone, Debug, Default)]
pub struct RuleSelection {
    pub select: Option<Vec<UnresolvedRuleSelector>>,
    pub ignore: Vec<UnresolvedRuleSelector>,
    pub extend_select: Vec<UnresolvedRuleSelector>,
    pub fixable: Option<Vec<UnresolvedRuleSelector>>,
    pub unfixable: Vec<UnresolvedRuleSelector>,
    pub extend_fixable: Vec<UnresolvedRuleSelector>,
}

struct ResolvedRuleSelection {
    select: Option<Vec<RuleSelector>>,
    ignore: Vec<RuleSelector>,
    extend_select: Vec<RuleSelector>,
    fixable: Option<Vec<RuleSelector>>,
    unfixable: Vec<RuleSelector>,
    extend_fixable: Vec<RuleSelector>,
}

#[derive(Debug, Eq, PartialEq, is_macro::Is)]
pub enum RuleSelectorKind {
    /// Enables the selected rules
    Enable,
    /// Disables the selected rules
    Disable,
    /// Modifies the behavior of selected rules
    Modify,
}

impl RuleSelection {
    fn resolve(&self, preview: PreviewMode) -> Result<ResolvedRuleSelection, RuleResolutionError> {
        fn resolve(
            setting: &'static str,
            selectors: &[UnresolvedRuleSelector],
            preview: PreviewMode,
        ) -> Result<Vec<RuleSelector>, RuleResolutionError> {
            selectors
                .iter()
                .filter_map(|selector| match selector.resolve(preview) {
                    Ok(selector) => Some(Ok(selector)),
                    Err(mut err) => {
                        err = err.with_setting(setting);
                        if is_warn_on_unknown_selectors_enabled(preview) {
                            err.log_warning();
                            None
                        } else {
                            Some(Err(err))
                        }
                    }
                })
                .collect()
        }

        Ok(ResolvedRuleSelection {
            select: self
                .select
                .as_deref()
                .map(|selectors| resolve("select", selectors, preview))
                .transpose()?,
            ignore: resolve("ignore", &self.ignore, preview)?,
            extend_select: resolve("extend-select", &self.extend_select, preview)?,
            fixable: self
                .fixable
                .as_deref()
                .map(|selectors| resolve("fixable", selectors, preview))
                .transpose()?,
            unfixable: resolve("unfixable", &self.unfixable, preview)?,
            extend_fixable: resolve("extend-fixable", &self.extend_fixable, preview)?,
        })
    }
}

impl ResolvedRuleSelection {
    fn selectors_by_kind(&self) -> impl Iterator<Item = (RuleSelectorKind, &RuleSelector)> {
        self.select
            .iter()
            .flatten()
            .map(|selector| (RuleSelectorKind::Enable, selector))
            .chain(
                self.fixable
                    .iter()
                    .flatten()
                    .map(|selector| (RuleSelectorKind::Modify, selector)),
            )
            .chain(
                self.ignore
                    .iter()
                    .map(|selector| (RuleSelectorKind::Disable, selector)),
            )
            .chain(
                self.extend_select
                    .iter()
                    .map(|selector| (RuleSelectorKind::Enable, selector)),
            )
            .chain(
                self.unfixable
                    .iter()
                    .map(|selector| (RuleSelectorKind::Modify, selector)),
            )
            .chain(
                self.extend_fixable
                    .iter()
                    .map(|selector| (RuleSelectorKind::Modify, selector)),
            )
    }
}

#[derive(Debug, Default, Clone)]
pub struct Configuration {
    // Global options
    pub cache_dir: Option<PathBuf>,
    pub extend: Option<PathBuf>,
    pub fix: Option<bool>,
    pub fix_only: Option<bool>,
    pub unsafe_fixes: Option<UnsafeFixes>,
    pub output_format: Option<OutputFormat>,
    pub output_prefer_rule_codes: Option<bool>,
    pub preview: Option<PreviewMode>,
    pub required_version: Option<RequiredVersion>,
    pub extension: Option<ExtensionMapping>,
    pub show_fixes: Option<bool>,

    // File resolver options
    pub exclude: Option<Vec<FilePattern>>,
    pub extend_exclude: Vec<FilePattern>,
    pub extend_include: Vec<FilePattern>,
    pub force_exclude: Option<bool>,
    pub include: Option<Vec<FilePattern>>,
    pub respect_gitignore: Option<bool>,

    // Generic python options settings
    pub builtins: Option<Vec<String>>,
    pub namespace_packages: Option<Vec<PathBuf>>,
    pub src: Option<Vec<PathBuf>>,
    pub target_version: Option<ast::PythonVersion>,
    pub per_file_target_version: Option<Vec<PerFileTargetVersion>>,

    pub lint: LintConfiguration,
}

impl Configuration {
    pub fn into_settings(self, project_root: &Path) -> Result<Settings> {
        if let Some(required_version) = &self.required_version {
            validate_required_version(required_version)?;
        }

        let linter_target_version = TargetVersion(self.target_version);
        let global_preview = self.preview.unwrap_or_default();

        let per_file_target_version = CompiledPerFileTargetVersionList::resolve(
            self.per_file_target_version.unwrap_or_default(),
        )
        .context("failed to resolve `per-file-target-version` table")?;

        let lint = self.lint;
        let lint_preview = lint.preview.unwrap_or(global_preview);

        let rules = lint.as_rule_table(lint_preview)?;

        let future_annotations = lint.future_annotations.unwrap_or_default();

        Ok(Settings {
            cache_dir: self
                .cache_dir
                .clone()
                .unwrap_or_else(|| cache_dir(project_root)),
            fix: self.fix.unwrap_or(false),
            fix_only: self.fix_only.unwrap_or(false),
            unsafe_fixes: self.unsafe_fixes.unwrap_or_default(),
            output_format: self.output_format.unwrap_or_default(),
            output_prefer_rule_codes: self.output_prefer_rule_codes.unwrap_or_default(),
            show_fixes: self.show_fixes.unwrap_or(false),

            file_resolver: FileResolverSettings {
                exclude: FilePatternSet::try_from_iter(
                    self.exclude.unwrap_or_else(|| EXCLUDE.to_vec()),
                )?,
                extend_exclude: FilePatternSet::try_from_iter(self.extend_exclude)?,
                extend_include: FilePatternSet::try_from_iter(self.extend_include)?,
                force_exclude: self.force_exclude.unwrap_or(false),
                include: FilePatternSet::try_from_iter(self.include.unwrap_or_else(|| {
                    let mut patterns = match global_preview {
                        PreviewMode::Disabled => INCLUDE.to_vec(),
                        PreviewMode::Enabled => INCLUDE_PREVIEW.to_vec(),
                    };
                    if let Some(extension_map) = &self.extension {
                        patterns.extend(
                            extension_map
                                .extensions()
                                .map(|ext| FilePattern::Config(format!("*.{ext}"))),
                        );
                    }
                    patterns
                }))?,
                respect_gitignore: self.respect_gitignore.unwrap_or(true),
                project_root: project_root.to_path_buf(),
            },

            linter: LinterSettings {
                rules,
                exclude: FilePatternSet::try_from_iter(lint.exclude.unwrap_or_default())?,
                extension: self.extension.unwrap_or_default(),
                preview: lint_preview,
                unresolved_target_version: linter_target_version,
                per_file_target_version,
                project_root: project_root.to_path_buf(),
                builtins: self.builtins.unwrap_or_default(),
                dummy_variable_rgx: lint
                    .dummy_variable_rgx
                    .unwrap_or_else(|| DUMMY_VARIABLE_RGX.clone()),
                external: lint.external.unwrap_or_default(),
                ignore_init_module_imports: lint.ignore_init_module_imports.unwrap_or(true),
                namespace_packages: self.namespace_packages.unwrap_or_default(),
                per_file_ignores: CompiledPerFileIgnoreList::resolve(
                    lint.per_file_ignores
                        .unwrap_or_default()
                        .into_iter()
                        .chain(lint.extend_per_file_ignores)
                        .collect(),
                    lint_preview,
                )?,
                fix_safety: FixSafetyTable::from_rule_selectors(
                    &lint.extend_safe_fixes,
                    &lint.extend_unsafe_fixes,
                    &PreviewOptions {
                        mode: lint_preview,
                        require_explicit: false,
                    },
                )?,
                src: self
                    .src
                    .unwrap_or_else(|| vec![project_root.to_path_buf(), project_root.join("src")]),
                explicit_preview_rules: lint.explicit_preview_rules.unwrap_or_default(),

                logger_objects: lint.logger_objects.unwrap_or_default(),
                typing_modules: lint.typing_modules.unwrap_or_default(),
                // Plugins
                nuff: lint
                    .nuff
                    .map(NuffOptions::into_settings)
                    .unwrap_or_default(),
                pyflakes: lint
                    .pyflakes
                    .map(PyflakesOptions::into_settings)
                    .unwrap_or_default(),
                typing_extensions: lint.typing_extensions.unwrap_or(true),
                future_annotations,
            },
        })
    }

    /// Convert the [`Options`] read from the given [`Path`] into a [`Configuration`].
    /// If `None` is supplied for `path`, it indicates that the `Options` instance
    /// was created via "inline TOML" from the `--config` flag
    pub fn from_options(
        options: Options,
        _path: Option<&Path>,
        project_root: &Path,
    ) -> Result<Self> {
        let lint = options.lint.unwrap_or_default();

        Ok(Self {
            builtins: options.builtins,
            cache_dir: options
                .cache_dir
                .map(|dir| {
                    let dir = shellexpand::full(&dir);
                    dir.map(|dir| fs::normalize_path_to(dir.as_ref(), project_root))
                })
                .transpose()
                .map_err(|e| anyhow!("Invalid `cache-dir` value: {e}"))?,

            exclude: options.exclude.map(|paths| {
                paths
                    .into_iter()
                    .map(|pattern| {
                        let absolute = GlobPath::normalize(&pattern, project_root);
                        FilePattern::User(pattern, absolute)
                    })
                    .collect()
            }),
            extend: options
                .extend
                .map(|extend| {
                    let extend = shellexpand::full(&extend);
                    extend.map(|extend| PathBuf::from(extend.as_ref()))
                })
                .transpose()
                .map_err(|e| anyhow!("Invalid `extend` value: {e}"))?,
            extend_exclude: options
                .extend_exclude
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(|pattern| {
                            let absolute = GlobPath::normalize(&pattern, project_root);
                            FilePattern::User(pattern, absolute)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            extend_include: options
                .extend_include
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(|pattern| {
                            let absolute = GlobPath::normalize(&pattern, project_root);
                            FilePattern::User(pattern, absolute)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            include: options.include.map(|paths| {
                paths
                    .into_iter()
                    .map(|pattern| {
                        let absolute = GlobPath::normalize(&pattern, project_root);
                        FilePattern::User(pattern, absolute)
                    })
                    .collect()
            }),
            fix: options.fix,
            fix_only: options.fix_only,
            unsafe_fixes: options.unsafe_fixes.map(UnsafeFixes::from),
            output_format: options.output_format,
            output_prefer_rule_codes: options.output_prefer_rule_codes,
            force_exclude: options.force_exclude,
            namespace_packages: options
                .namespace_packages
                .map(|namespace_package| resolve_src(&namespace_package, project_root))
                .transpose()?,
            preview: options.preview.map(PreviewMode::from),
            required_version: options.required_version,
            respect_gitignore: options.respect_gitignore,
            show_fixes: options.show_fixes,
            src: options
                .src
                .map(|src| resolve_src(&src, project_root))
                .transpose()?,
            target_version: options.target_version.map(ast::PythonVersion::from),
            per_file_target_version: options.per_file_target_version.map(|versions| {
                versions
                    .into_iter()
                    .map(|(pattern, version)| {
                        PerFileTargetVersion::new(
                            pattern,
                            ast::PythonVersion::from(version),
                            Some(project_root),
                        )
                    })
                    .collect()
            }),
            extension: options.extension.map(ExtensionMapping::from),
            lint: LintConfiguration::from_options(lint, project_root)?,
        })
    }

    #[must_use]
    pub fn combine(self, config: Self) -> Self {
        Self {
            builtins: self.builtins.or(config.builtins),
            cache_dir: self.cache_dir.or(config.cache_dir),
            exclude: self.exclude.or(config.exclude),
            extend: self.extend.or(config.extend),
            extend_exclude: config
                .extend_exclude
                .into_iter()
                .chain(self.extend_exclude)
                .collect(),
            extend_include: config
                .extend_include
                .into_iter()
                .chain(self.extend_include)
                .collect(),
            include: self.include.or(config.include),
            fix: self.fix.or(config.fix),
            fix_only: self.fix_only.or(config.fix_only),
            unsafe_fixes: self.unsafe_fixes.or(config.unsafe_fixes),
            output_format: self.output_format.or(config.output_format),
            output_prefer_rule_codes: self
                .output_prefer_rule_codes
                .or(config.output_prefer_rule_codes),
            force_exclude: self.force_exclude.or(config.force_exclude),
            namespace_packages: self.namespace_packages.or(config.namespace_packages),
            required_version: self.required_version.or(config.required_version),
            respect_gitignore: self.respect_gitignore.or(config.respect_gitignore),
            show_fixes: self.show_fixes.or(config.show_fixes),
            src: self.src.or(config.src),
            target_version: self.target_version.or(config.target_version),
            per_file_target_version: self
                .per_file_target_version
                .or(config.per_file_target_version),
            preview: self.preview.or(config.preview),
            extension: self.extension.or(config.extension),

            lint: self.lint.combine(config.lint),
        }
    }

    #[must_use]
    pub(crate) fn apply_fallbacks(
        mut self,
        origin: ConfigurationOrigin,
        initial_config_path: &Path,
    ) -> Self {
        if matches!(origin, ConfigurationOrigin::Ancestor) {
            self.target_version = self.target_version.or_else(|| {
                let dir = initial_config_path.parent()?;
                let fallback = pyproject::find_fallback_target_version(dir)?;
                debug!("Derived `target-version` from `requires-python`: {fallback:?}");
                Some(fallback.into())
            });
        }
        // For `UserSettings`, `nuff::resolve::resolve` searches the cwd for a fallback target
        // version.
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct LintConfiguration {
    pub exclude: Option<Vec<FilePattern>>,
    pub preview: Option<PreviewMode>,

    // Rule selection
    pub extend_per_file_ignores: Vec<PerFileIgnore>,
    pub per_file_ignores: Option<Vec<PerFileIgnore>>,
    pub rule_selections: Vec<RuleSelection>,
    pub explicit_preview_rules: Option<bool>,

    // Fix configuration
    pub extend_unsafe_fixes: Vec<UnresolvedRuleSelector>,
    pub extend_safe_fixes: Vec<UnresolvedRuleSelector>,

    pub dummy_variable_rgx: Option<Regex>,
    pub external: Option<Vec<String>>,
    pub ignore_init_module_imports: Option<bool>,
    pub logger_objects: Option<Vec<String>>,
    pub typing_modules: Option<Vec<String>>,
    pub typing_extensions: Option<bool>,
    pub future_annotations: Option<bool>,

    pub nuff: Option<NuffOptions>,
    pub pyflakes: Option<PyflakesOptions>,
}

impl LintConfiguration {
    fn from_options(options: LintOptions, project_root: &Path) -> Result<Self> {
        #[expect(deprecated)]
        let ignore = options
            .common
            .ignore
            .into_iter()
            .flatten()
            .chain(options.common.extend_ignore.into_iter().flatten())
            .collect();
        #[expect(deprecated)]
        let unfixable = options
            .common
            .unfixable
            .into_iter()
            .flatten()
            .chain(options.common.extend_unfixable.into_iter().flatten())
            .collect();

        #[expect(deprecated)]
        let ignore_init_module_imports = {
            if options.common.ignore_init_module_imports.is_some() {
                warn_user_once!(
                    "The `ignore-init-module-imports` option is deprecated \
                    and will be removed in a future release. \
                    Nuff's handling of imports in `__init__.py` files \
                    has been improved (in preview) and unused imports \
                    will always be flagged."
                );
            }
            options.common.ignore_init_module_imports
        };

        Ok(LintConfiguration {
            exclude: options.exclude.map(|paths| {
                paths
                    .into_iter()
                    .map(|pattern| {
                        let absolute = GlobPath::normalize(&pattern, project_root);
                        FilePattern::User(pattern, absolute)
                    })
                    .collect()
            }),
            preview: options.preview.map(PreviewMode::from),

            rule_selections: vec![RuleSelection {
                select: options.common.select,
                ignore,
                extend_select: options.common.extend_select.unwrap_or_default(),
                fixable: options.common.fixable,
                unfixable,
                extend_fixable: options.common.extend_fixable.unwrap_or_default(),
            }],
            extend_safe_fixes: options.common.extend_safe_fixes.unwrap_or_default(),
            extend_unsafe_fixes: options.common.extend_unsafe_fixes.unwrap_or_default(),
            dummy_variable_rgx: options
                .common
                .dummy_variable_rgx
                .map(|pattern| Regex::new(&pattern))
                .transpose()
                .map_err(|e| anyhow!("Invalid `dummy-variable-rgx` value: {e}"))?,
            extend_per_file_ignores: options
                .common
                .extend_per_file_ignores
                .map(|per_file_ignores| {
                    per_file_ignores
                        .into_iter()
                        .map(|(pattern, prefixes)| {
                            PerFileIgnore::new(pattern, prefixes, Some(project_root))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            external: options.common.external,
            ignore_init_module_imports,
            explicit_preview_rules: options.common.explicit_preview_rules,
            per_file_ignores: options.common.per_file_ignores.map(|per_file_ignores| {
                per_file_ignores
                    .into_iter()
                    .map(|(pattern, prefixes)| {
                        PerFileIgnore::new(pattern, prefixes, Some(project_root))
                    })
                    .collect()
            }),
            logger_objects: options.common.logger_objects,
            typing_modules: options.common.typing_modules,
            typing_extensions: options.typing_extensions,
            future_annotations: options.future_annotations,

            nuff: options.common.nuff,
            pyflakes: options.common.pyflakes,
        })
    }

    fn as_rule_table(&self, preview: PreviewMode) -> Result<RuleTable> {
        let preview = PreviewOptions {
            mode: preview,
            require_explicit: self.explicit_preview_rules.unwrap_or_default(),
        };

        let preview_selectors;
        let selectors = if preview.mode.is_enabled() {
            preview_selectors = Category::default_categories().map(RuleSelector::Category);
            &preview_selectors
        } else {
            DEFAULT_SELECTORS
        };

        // The select_set keeps track of which rules have been selected.
        let mut select_set: RuleSet = selectors
            .iter()
            .flat_map(|selector| selector.rules(&preview))
            .collect();

        // The fixable set keeps track of which rules are fixable.
        let mut fixable_set: RuleSet = RuleSelector::All.all_rules().collect();

        let rule_selections = self
            .rule_selections
            .iter()
            .map(|selection| selection.resolve(preview.mode))
            .collect::<std::result::Result<Vec<_>, RuleResolutionError>>()?;

        // Ignores normally only subtract from the current set of selected
        // rules.  By that logic the ignore in `select = [], ignore = ["E501"]`
        // would be effectless. Instead we carry over the ignores to the next
        // selection in that case, creating a way for ignores to be reused
        // across config files (which otherwise wouldn't be possible since nuff
        // only has `extended` but no `extended-by`).
        let mut carryover_ignores: Option<&[RuleSelector]> = None;
        let mut carryover_unfixables: Option<&[RuleSelector]> = None;

        // Store selectors for displaying warnings
        let mut deprecated_selectors = FxHashSet::default();
        let mut removed_selectors = FxHashSet::default();
        let mut removed_ignored_rules = FxHashSet::default();
        let mut ignored_preview_selectors = FxHashSet::default();

        for selection in &rule_selections {
            // If a selection only specifies extend-select we cannot directly
            // apply its rule selectors to the select_set because we firstly have
            // to resolve the effectively selected rules within the current rule selection
            // (taking specificity into account since more specific selectors take
            // precedence over less specific selectors within a rule selection).
            // We do this via the following HashMap where the bool indicates
            // whether to enable or disable the given rule.
            let mut select_map_updates: FxHashMap<Rule, bool> = FxHashMap::default();
            let mut fixable_map_updates: FxHashMap<Rule, bool> = FxHashMap::default();

            let carriedover_ignores = carryover_ignores.take();
            let carriedover_unfixables = carryover_unfixables.take();

            for spec in Specificity::iter() {
                // Iterate over rule selectors in order of specificity.
                for selector in selection
                    .select
                    .iter()
                    .flatten()
                    .chain(&selection.extend_select)
                    .filter(|s| s.specificity() == spec)
                {
                    for rule in selector.rules(&preview) {
                        select_map_updates.insert(rule, true);
                    }
                }
                for selector in selection
                    .ignore
                    .iter()
                    .chain(carriedover_ignores.into_iter().flatten())
                    .filter(|s| s.specificity() == spec)
                {
                    for rule in selector.rules(&preview) {
                        select_map_updates.insert(rule, false);
                    }
                }

                // Apply the same logic to `fixable` and `unfixable`.
                for selector in selection
                    .fixable
                    .iter()
                    .flatten()
                    .chain(&selection.extend_fixable)
                    .filter(|s| s.specificity() == spec)
                {
                    for rule in selector.all_rules() {
                        fixable_map_updates.insert(rule, true);
                    }
                }
                for selector in selection
                    .unfixable
                    .iter()
                    .chain(carriedover_unfixables.into_iter().flatten())
                    .filter(|s| s.specificity() == spec)
                {
                    for rule in selector.all_rules() {
                        fixable_map_updates.insert(rule, false);
                    }
                }
            }

            if let Some(select) = &selection.select {
                // If the `select` option is given we reassign the whole select_set
                // (overriding everything that has been defined previously).
                select_set = select_map_updates
                    .into_iter()
                    .filter_map(|(rule, enabled)| enabled.then_some(rule))
                    .collect();

                if select.is_empty()
                    && selection.extend_select.is_empty()
                    && !selection.ignore.is_empty()
                {
                    carryover_ignores = Some(&selection.ignore);
                }
            } else {
                // Otherwise we apply the updates on top of the existing select_set.
                #[expect(
                    clippy::iter_over_hash_type,
                    reason = "each rule has one independent final enabled state"
                )]
                for (rule, enabled) in select_map_updates {
                    select_set.set(rule, enabled);
                }
            }

            // Apply the same logic to `fixable` and `unfixable`.
            if let Some(fixable) = &selection.fixable {
                fixable_set = fixable_map_updates
                    .into_iter()
                    .filter_map(|(rule, enabled)| enabled.then_some(rule))
                    .collect();

                if fixable.is_empty()
                    && selection.extend_fixable.is_empty()
                    && !selection.unfixable.is_empty()
                {
                    carryover_unfixables = Some(&selection.unfixable);
                }
            } else {
                #[expect(
                    clippy::iter_over_hash_type,
                    reason = "each rule has one independent final fixable state"
                )]
                for (rule, enabled) in fixable_map_updates {
                    fixable_set.set(rule, enabled);
                }
            }

            // Check for selections that require a warning
            for (kind, selector) in selection.selectors_by_kind() {
                // Some of these checks are only for `Kind::Enable` which means only `--select` will warn
                // and use with, e.g., `--ignore` or `--fixable` is okay

                // Unstable rules
                if preview.mode.is_disabled() && kind.is_enable() {
                    // Check if the selector is empty because preview mode is disabled
                    if selector.rules(&preview).next().is_none()
                        && selector
                            .rules(&PreviewOptions {
                                mode: PreviewMode::Enabled,
                                require_explicit: preview.require_explicit,
                            })
                            .next()
                            .is_some()
                    {
                        ignored_preview_selectors.insert(selector);
                    }
                }

                // Deprecated rules
                if kind.is_enable() && selector.is_exact() {
                    if selector.all_rules().all(|rule| rule.is_deprecated()) {
                        deprecated_selectors.insert(selector);
                    }
                }

                // Removed rules
                if selector.is_exact() {
                    if selector.all_rules().all(|rule| rule.is_removed()) {
                        if kind.is_disable() {
                            removed_ignored_rules.insert(selector);
                        } else {
                            removed_selectors.insert(selector);
                        }
                    }
                }
            }
        }

        let removed_selectors = removed_selectors.iter().sorted().collect::<Vec<_>>();
        match removed_selectors.as_slice() {
            [] => (),
            [selection] => {
                let (prefix, code) = selection.prefix_and_code();
                return Err(anyhow!(
                    "Rule `{prefix}{code}` was removed and cannot be selected."
                ));
            }
            [..] => {
                let mut message =
                    "The following rules have been removed and cannot be selected:".to_string();
                for selection in removed_selectors {
                    let (prefix, code) = selection.prefix_and_code();
                    message.push_str("\n    - ");
                    message.push_str(prefix);
                    message.push_str(code);
                }
                message.push('\n');
                return Err(anyhow!(message));
            }
        }

        if !removed_ignored_rules.is_empty() {
            let mut rules = String::new();
            for selection in removed_ignored_rules.iter().sorted() {
                let (prefix, code) = selection.prefix_and_code();
                rules.push_str("\n    - ");
                rules.push_str(prefix);
                rules.push_str(code);
            }
            rules.push('\n');
            warn_user_once_by_message!(
                "The following rules have been removed and ignoring them has no effect:{rules}"
            );
        }

        if preview.mode.is_disabled() {
            for selection in deprecated_selectors.iter().sorted() {
                let (prefix, code) = selection.prefix_and_code();
                warn_user_once_by_message!(
                    "Rule `{prefix}{code}` is deprecated and will be removed in a future release."
                );
            }
        } else {
            let deprecated_selectors = deprecated_selectors.iter().sorted().collect::<Vec<_>>();
            match deprecated_selectors.as_slice() {
                [] => (),
                [selection] => {
                    let (prefix, code) = selection.prefix_and_code();
                    return Err(anyhow!(
                        "Selection of deprecated rule `{prefix}{code}` is not allowed when \
                         preview is enabled."
                    ));
                }
                [..] => {
                    let mut message = "\
                        Selection of deprecated rules is not allowed \
                            when preview is enabled. Remove selection of:"
                        .to_string();
                    for selection in deprecated_selectors {
                        let (prefix, code) = selection.prefix_and_code();
                        message.push_str("\n\t- ");
                        message.push_str(prefix);
                        message.push_str(code);
                    }
                    message.push('\n');
                    return Err(anyhow!(message));
                }
            }
        }

        for selection in ignored_preview_selectors.iter().sorted() {
            let (prefix, code) = selection.prefix_and_code();
            warn_user_once_by_message!(
                "Selection `{prefix}{code}` has no effect because preview is not enabled.",
            );
        }

        let mut rules = RuleTable::empty();

        for rule in select_set {
            let fix = fixable_set.contains(rule);
            rules.enable(rule, fix);
        }

        Ok(rules)
    }

    #[must_use]
    fn combine(self, config: Self) -> Self {
        let mut rule_selections = config.rule_selections;
        rule_selections.extend(self.rule_selections);

        let mut extend_safe_fixes = config.extend_safe_fixes;
        extend_safe_fixes.extend(self.extend_safe_fixes);

        let mut extend_unsafe_fixes = config.extend_unsafe_fixes;
        extend_unsafe_fixes.extend(self.extend_unsafe_fixes);

        let mut extend_per_file_ignores = config.extend_per_file_ignores;
        extend_per_file_ignores.extend(self.extend_per_file_ignores);

        Self {
            exclude: self.exclude.or(config.exclude),
            preview: self.preview.or(config.preview),
            rule_selections,
            extend_safe_fixes,
            extend_unsafe_fixes,
            dummy_variable_rgx: self.dummy_variable_rgx.or(config.dummy_variable_rgx),
            extend_per_file_ignores,
            external: self.external.or(config.external),
            ignore_init_module_imports: self
                .ignore_init_module_imports
                .or(config.ignore_init_module_imports),
            logger_objects: self.logger_objects.or(config.logger_objects),
            per_file_ignores: self.per_file_ignores.or(config.per_file_ignores),
            explicit_preview_rules: self
                .explicit_preview_rules
                .or(config.explicit_preview_rules),
            typing_modules: self.typing_modules.or(config.typing_modules),

            nuff: self.nuff.combine(config.nuff),
            pyflakes: self.pyflakes.combine(config.pyflakes),
            typing_extensions: self.typing_extensions.or(config.typing_extensions),
            future_annotations: self.future_annotations.or(config.future_annotations),
        }
    }
}

pub(crate) trait CombinePluginOptions {
    #[must_use]
    fn combine(self, other: Self) -> Self;
}

impl<T: CombinePluginOptions> CombinePluginOptions for Option<T> {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Some(base), Some(other)) => Some(base.combine(other)),
            (Some(base), None) => Some(base),
            (None, Some(other)) => Some(other),
            (None, None) => None,
        }
    }
}

/// Given a list of source paths, which could include glob patterns, resolve the
/// matching paths.
fn resolve_src(src: &[String], project_root: &Path) -> Result<Vec<PathBuf>> {
    let expansions = src
        .iter()
        .map(shellexpand::full)
        .collect::<Result<Vec<Cow<'_, str>>, LookupError<VarError>>>()?;
    let globs = expansions
        .iter()
        .map(|path| Path::new(path.as_ref()))
        .map(|path| fs::normalize_path_to(path, project_root))
        .map(|path| glob(&path.to_string_lossy()))
        .collect::<Result<Vec<Paths>, PatternError>>()?;
    let paths: Vec<PathBuf> = globs
        .into_iter()
        .flatten()
        .collect::<Result<Vec<PathBuf>, GlobError>>()?;
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use nuff_linter::UnresolvedRuleSelector;
    use nuff_linter::registry::{Rule, RuleSet};
    use nuff_linter::rule_selector::PreviewOptions;
    use nuff_linter::settings::types::PreviewMode;

    use crate::configuration::{LintConfiguration, RuleSelection};

    const ASYNC22: &[Rule] = &[
        Rule::CreateSubprocessInAsyncFunction,
        Rule::RunProcessInAsyncFunction,
        Rule::WaitForProcessInAsyncFunction,
    ];

    fn resolve_rules(
        selections: impl IntoIterator<Item = RuleSelection>,
        preview: Option<PreviewOptions>,
    ) -> Result<RuleSet> {
        Ok(LintConfiguration {
            rule_selections: selections.into_iter().collect(),
            explicit_preview_rules: preview.as_ref().map(|preview| preview.require_explicit),
            ..LintConfiguration::default()
        }
        .as_rule_table(preview.map(|preview| preview.mode).unwrap_or_default())?
        .iter_enabled()
        .collect())
    }

    fn select(selectors: &[&str]) -> RuleSelection {
        RuleSelection {
            select: Some(
                selectors
                    .iter()
                    .map(|s| UnresolvedRuleSelector::cli(*s))
                    .collect(),
            ),
            ..RuleSelection::default()
        }
    }

    fn preview(mode: PreviewMode) -> PreviewOptions {
        PreviewOptions {
            mode,
            ..PreviewOptions::default()
        }
    }

    #[test]
    fn select_linter() -> Result<()> {
        let actual = resolve_rules([select(&["E", "PL"])], None)?;
        let expected = RuleSet::from_rules(&[Rule::IOError, Rule::ImportOutsideTopLevel]);
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn select_prefix() -> Result<()> {
        let actual = resolve_rules([select(&["ASYNC22"])], None)?;
        assert_eq!(actual, RuleSet::from_rules(ASYNC22));
        Ok(())
    }

    #[test]
    fn select_prefix_ignore_code() -> Result<()> {
        let actual = resolve_rules(
            [RuleSelection {
                ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                ..select(&["ASYNC22"])
            }],
            None,
        )?;
        let expected = RuleSet::from_rules(&[
            Rule::CreateSubprocessInAsyncFunction,
            Rule::WaitForProcessInAsyncFunction,
        ]);
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn select_code_ignore_prefix() -> Result<()> {
        let actual = resolve_rules(
            [RuleSelection {
                ignore: vec![UnresolvedRuleSelector::cli("ASYNC22")],
                ..select(&["ASYNC221"])
            }],
            None,
        )?;
        assert_eq!(actual, RuleSet::from_rule(Rule::RunProcessInAsyncFunction));
        Ok(())
    }

    #[test]
    fn select_code_ignore_code() -> Result<()> {
        let actual = resolve_rules(
            [RuleSelection {
                ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                ..select(&["ASYNC221"])
            }],
            None,
        )?;
        assert_eq!(actual, RuleSet::empty());
        Ok(())
    }

    #[test]
    fn select_prefix_ignore_code_then_extend_select_code() -> Result<()> {
        let actual = resolve_rules(
            [
                RuleSelection {
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ..select(&["ASYNC22"])
                },
                RuleSelection {
                    extend_select: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ..RuleSelection::default()
                },
            ],
            None,
        )?;
        assert_eq!(actual, RuleSet::from_rules(ASYNC22));
        Ok(())
    }

    #[test]
    fn select_prefix_ignore_code_then_extend_select_code_ignore_prefix() -> Result<()> {
        let actual = resolve_rules(
            [
                RuleSelection {
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ..select(&["ASYNC22"])
                },
                RuleSelection {
                    extend_select: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC22")],
                    ..RuleSelection::default()
                },
            ],
            None,
        )?;
        assert_eq!(actual, RuleSet::from_rule(Rule::RunProcessInAsyncFunction));
        Ok(())
    }

    #[test]
    fn ignore_code_then_select_prefix() -> Result<()> {
        let actual = resolve_rules(
            [
                RuleSelection {
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ..select(&[])
                },
                select(&["ASYNC22"]),
            ],
            None,
        )?;
        let expected = RuleSet::from_rules(&[
            Rule::CreateSubprocessInAsyncFunction,
            Rule::WaitForProcessInAsyncFunction,
        ]);
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn ignore_code_then_select_prefix_ignore_code() -> Result<()> {
        let actual = resolve_rules(
            [
                RuleSelection {
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC221")],
                    ..select(&[])
                },
                RuleSelection {
                    ignore: vec![UnresolvedRuleSelector::cli("ASYNC222")],
                    ..select(&["ASYNC22"])
                },
            ],
            None,
        )?;
        assert_eq!(
            actual,
            RuleSet::from_rule(Rule::CreateSubprocessInAsyncFunction)
        );
        Ok(())
    }

    #[test]
    fn select_all_preview() -> Result<()> {
        let preview_rule = RuleSet::from_rule(Rule::YieldInContextManagerInAsyncGenerator);

        let actual = resolve_rules([select(&["ALL"])], Some(preview(PreviewMode::Disabled)))?;
        assert!(!actual.intersects(&preview_rule));

        let actual = resolve_rules([select(&["ALL"])], Some(preview(PreviewMode::Enabled)))?;
        assert!(actual.intersects(&preview_rule));
        Ok(())
    }

    #[test]
    fn select_linter_and_prefix_preview() -> Result<()> {
        let preview_rule = RuleSet::from_rule(Rule::YieldInContextManagerInAsyncGenerator);
        for selector in ["ASYNC", "ASYNC1"] {
            let actual =
                resolve_rules([select(&[selector])], Some(preview(PreviewMode::Disabled)))?;
            assert!(!actual.intersects(&preview_rule), "{selector}");

            let actual = resolve_rules([select(&[selector])], Some(preview(PreviewMode::Enabled)))?;
            assert!(actual.intersects(&preview_rule), "{selector}");
        }
        Ok(())
    }

    #[test]
    fn select_rule_preview() -> Result<()> {
        let preview_rule = RuleSet::from_rule(Rule::YieldInContextManagerInAsyncGenerator);

        let actual = resolve_rules(
            [select(&["ASYNC119"])],
            Some(preview(PreviewMode::Disabled)),
        )?;
        assert_eq!(actual, RuleSet::empty());

        let actual = resolve_rules([select(&["ASYNC119"])], Some(preview(PreviewMode::Enabled)))?;
        assert_eq!(actual, preview_rule);

        let actual = resolve_rules(
            [select(&["ASYNC119"])],
            Some(PreviewOptions {
                mode: PreviewMode::Enabled,
                require_explicit: true,
            }),
        )?;
        assert_eq!(actual, preview_rule);
        Ok(())
    }
}
