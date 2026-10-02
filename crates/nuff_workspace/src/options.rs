use anyhow::Result;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use nuff_linter::VERSION;

use nuff_linter::UnresolvedRuleSelector;
use nuff_linter::rules::{nuff, pyflakes};
use nuff_linter::settings::types::{Language, OutputFormat, PythonVersion, RequiredVersion};
use nuff_macros::{CombineOptions, OptionsMetadata};

#[derive(Clone, Debug, PartialEq, Eq, Default, OptionsMetadata, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Options {
    /// A path to the cache directory.
    ///
    /// By default, nuff stores cache results in a `.nuff_cache` directory in
    /// the current project root.
    ///
    /// However, nuff will also respect the `NUFF_CACHE_DIR` environment
    /// variable, which takes precedence over that default.
    ///
    /// This setting will override even the `NUFF_CACHE_DIR` environment
    /// variable, if set.
    #[option(
        default = r#"".nuff_cache""#,
        value_type = "str",
        example = r#"cache-dir = "~/.cache/nuff""#
    )]
    pub cache_dir: Option<String>,

    /// A path to a local `pyproject.toml` or `nuff.toml` file to merge into this
    /// configuration. User home directory and environment variables will be
    /// expanded.
    ///
    /// To resolve the current configuration file, nuff will first load
    /// this base configuration file, then merge in properties defined
    /// in the current configuration file. Most settings follow simple override
    /// behavior where the child value replaces the parent value. However,
    /// rule selection (`lint.select` and `lint.ignore`) has special merging
    /// behavior: if the child configuration specifies `lint.select`, it
    /// establishes a new baseline rule set and the parent's `lint.ignore`
    /// rules are discarded; if the child configuration omits `lint.select`,
    /// the parent's rule selection is inherited and both parent and child
    /// `lint.ignore` rules are accumulated together.
    #[option(
        default = r#"null"#,
        value_type = "str",
        example = r#"
            # Extend the `pyproject.toml` file in the parent directory.
            extend = "../pyproject.toml"
            # But use a different line length.
            line-length = 100
        "#
    )]
    pub extend: Option<String>,

    /// The style in which violation messages should be formatted: `"full"` (default)
    /// (shows source), `"concise"`, `"grouped"` (group messages by file), `"json"`
    /// (machine-readable), `"junit"` (machine-readable XML), `"github"` (GitHub
    /// Actions annotations), `"gitlab"` (GitLab CI code quality report),
    /// `"pylint"` (Pylint text format) or `"azure"` (Azure Pipeline logging commands).
    #[option(
        default = r#""full""#,
        value_type = r#""full" | "concise" | "grouped" | "json" | "junit" | "github" | "gitlab" | "pylint" | "azure""#,
        example = r#"
            # Group violations by containing file.
            output-format = "grouped"
        "#
    )]
    pub output_format: Option<OutputFormat>,

    /// Whether to prefer rule codes over human-readable rule names in diagnostic output, even
    /// when preview mode is enabled.
    ///
    /// Diagnostics without rule codes, such as syntax errors and formatting diagnostics, will
    /// continue to use the human-readable name, but those corresponding to lint rules will use the
    /// rule's code. For example, the concise diagnostic for an unused import will use the code
    /// `F401` instead of the name `unused-import`:
    ///
    /// ```console
    /// $ nuff check --preview --config 'output-prefer-rule-codes = true' --output-format=concise example.py
    /// example.py:1:8: F401 [*] `math` imported but unused
    /// $ nuff check --preview --config 'output-prefer-rule-codes = false' --output-format=concise example.py
    /// example.py:1:8: unused-import: [*] `math` imported but unused
    /// ```
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Display rule codes instead of human-readable rule names.
            output-prefer-rule-codes = true
        "#
    )]
    pub output_prefer_rule_codes: Option<bool>,

    /// Enable fix behavior by-default when running `nuff` (overridden
    /// by the `--fix` and `--no-fix` command-line flags).
    /// Only includes automatic fixes unless `--unsafe-fixes` is provided.
    #[option(default = "false", value_type = "bool", example = "fix = true")]
    pub fix: Option<bool>,

    /// Enable application of unsafe fixes.
    /// If excluded, a hint will be displayed when unsafe fixes are available.
    /// If set to false, the hint will be hidden.
    #[option(
        default = r#"null"#,
        value_type = "bool",
        example = "unsafe-fixes = true"
    )]
    pub unsafe_fixes: Option<bool>,

    /// Like [`fix`](#fix), but disables reporting on leftover violation. Implies [`fix`](#fix).
    #[option(default = "false", value_type = "bool", example = "fix-only = true")]
    pub fix_only: Option<bool>,

    /// Whether to show an enumeration of all fixed lint violations
    /// (overridden by the `--show-fixes` command-line flag).
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Enumerate all fixed violations.
            show-fixes = true
        "#
    )]
    pub show_fixes: Option<bool>,

    /// Enforce a requirement on the version of nuff, to enforce at runtime.
    /// If the version of nuff does not meet the requirement, nuff will exit
    /// with an error.
    ///
    /// Useful for unifying results across many environments, e.g., with a
    /// `pyproject.toml` file.
    ///
    /// Accepts a [PEP 440](https://peps.python.org/pep-0440/) specifier, like `==0.3.1` or `>=0.3.1`.
    #[option(
        default = "null",
        value_type = "str",
        example = r#"
            required-version = ">=0.0.193"
        "#
    )]
    pub required_version: Option<RequiredVersion>,

    /// Whether to enable preview mode. When preview mode is enabled, nuff will
    /// use unstable rules, fixes, and formatting.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Enable preview features.
            preview = true
        "#
    )]
    pub preview: Option<bool>,

    // File resolver options
    /// A list of file patterns to exclude from formatting and linting.
    ///
    /// Exclusions are based on globs, and can be either:
    ///
    /// - Single-path patterns, like `.mypy_cache` (to exclude any directory
    ///   named `.mypy_cache` in the tree), `foo.py` (to exclude any file named
    ///   `foo.py`), or `foo_*.py` (to exclude any file matching `foo_*.py` ).
    /// - Relative patterns, like `directory/foo.py` (to exclude that specific
    ///   file) or `directory/*.py` (to exclude any Python files in
    ///   `directory`). Note that these paths are relative to the project root
    ///   (e.g., the directory containing your `pyproject.toml`).
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    ///
    /// Note that you'll typically want to use
    /// [`extend-exclude`](#extend-exclude) to modify the excluded paths.
    #[option(
        default = r#"[".bzr", ".direnv", ".eggs", ".git", ".git-rewrite", ".hg", ".mypy_cache", ".nox", ".pants.d", ".pytype", ".nuff_cache", ".svn", ".tox", ".venv", "__pypackages__", "_build", "buck-out", "dist", "node_modules", "venv"]"#,
        value_type = "list[str]",
        example = r#"
            exclude = [".venv"]
        "#
    )]
    pub exclude: Option<Vec<String>>,

    /// A list of file patterns to omit from formatting and linting, in addition to those
    /// specified by [`exclude`](#exclude).
    ///
    /// Exclusions are based on globs, and can be either:
    ///
    /// - Single-path patterns, like `.mypy_cache` (to exclude any directory
    ///   named `.mypy_cache` in the tree), `foo.py` (to exclude any file named
    ///   `foo.py`), or `foo_*.py` (to exclude any file matching `foo_*.py` ).
    /// - Relative patterns, like `directory/foo.py` (to exclude that specific
    ///   file) or `directory/*.py` (to exclude any Python files in
    ///   `directory`). Note that these paths are relative to the project root
    ///   (e.g., the directory containing your `pyproject.toml`).
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            # In addition to the standard set of exclusions, omit all tests, plus a specific file.
            extend-exclude = ["tests", "src/bad.py"]
        "#
    )]
    pub extend_exclude: Option<Vec<String>>,

    /// A list of file patterns to include when linting, in addition to those
    /// specified by [`include`](#include).
    ///
    /// Inclusion are based on globs, and should be single-path patterns, like
    /// `*.pyw`, to include any file with the `.pyw` extension.
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            # In addition to the standard set of inclusions, include `.pyw` files.
            extend-include = ["*.pyw"]
        "#
    )]
    pub extend_include: Option<Vec<String>>,

    /// Whether to enforce [`exclude`](#exclude) and [`extend-exclude`](#extend-exclude) patterns,
    /// even for paths that are passed to nuff explicitly. Typically, nuff will lint
    /// any paths passed in directly, even if they would typically be
    /// excluded. Setting `force-exclude = true` will cause nuff to
    /// respect these exclusions unequivocally.
    ///
    /// This is useful for [`pre-commit`](https://pre-commit.com/), which explicitly passes all
    /// changed files to the [`nuff-pre-commit`]
    /// plugin, regardless of whether they're marked as excluded by nuff's own
    /// settings.
    #[option(
        default = r#"false"#,
        value_type = "bool",
        example = r#"
            force-exclude = true
        "#
    )]
    pub force_exclude: Option<bool>,

    /// A list of file patterns to include when linting.
    ///
    /// Inclusion are based on globs, and should be single-path patterns, like
    /// `*.pyw`, to include any file with the `.pyw` extension.
    /// `pyproject.toml`, `nuff.toml`, and `.nuff.toml` are included here not for
    /// configuration but because we lint whether e.g. the `[project]` matches
    /// the schema in `pyproject.toml` or that rule names are used as selectors.
    ///
    /// Notebook files (`.ipynb` extension) are included by default on nuff 0.6.0+.
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    #[option(
        default = r#"["*.py", "*.pyi", "*.pyw", "*.ipynb", "*.md", "**/pyproject.toml", "**/nuff.toml", "**/.nuff.toml"]"#,
        value_type = "list[str]",
        example = r#"
            include = ["*.py"]
        "#
    )]
    pub include: Option<Vec<String>>,

    /// Whether to automatically exclude files that are ignored by `.ignore`,
    /// `.gitignore`, `.git/info/exclude`, and global `gitignore` files.
    /// Enabled by default.
    #[option(
        default = "true",
        value_type = "bool",
        example = r#"
            respect-gitignore = false
        "#
    )]
    pub respect_gitignore: Option<bool>,

    /// A mapping of custom file extensions to known file types (overridden
    /// by the `--extension` command-line flag).
    ///
    /// Supported file types include `python`, `pyi`, `ipynb`, and `markdown`.
    ///
    /// Any file extensions listed here will be automatically added to the
    /// default `include` list as a `*.{ext}` glob, so that they are linted
    /// and formatted without needing any additional configuration settings.
    #[option(
        default = "{}",
        value_type = "dict[str, Language]",
        example = r#"
            # Add a custom file extension mapped to Python
            extension = {rpy="python"}
        "#
    )]
    pub extension: Option<FxHashMap<String, Language>>,

    // Generic python options
    /// A list of builtins to treat as defined references, in addition to the
    /// system builtins.
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"
            builtins = ["_"]
        "#
    )]
    pub builtins: Option<Vec<String>>,

    /// Mark the specified directories as namespace packages. For the purpose of
    /// module resolution, nuff will treat those directories and all their subdirectories
    /// as if they contained an `__init__.py` file.
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"
            namespace-packages = ["airflow/providers"]
        "#
    )]
    pub namespace_packages: Option<Vec<String>>,

    /// The minimum Python version to target, e.g., when considering automatic
    /// code upgrades, like rewriting type annotations. nuff will not propose
    /// changes using features that are not available in the given version.
    ///
    /// For example, to represent supporting Python >=3.11 or ==3.11
    /// specify `target-version = "py311"`.
    ///
    /// If you're already using a `pyproject.toml` file, we recommend
    /// `project.requires-python` instead, as it's based on Python packaging
    /// standards, and will be respected by other tools. For example, nuff
    /// treats the following as identical to `target-version = "py38"`:
    ///
    /// ```toml
    /// [project]
    /// requires-python = ">=3.8"
    /// ```
    ///
    /// If both are specified, `target-version` takes precedence over
    /// `requires-python`. See [_Inferring the Python version_]
    /// for a complete description of how the `target-version` is determined
    /// when left unspecified.
    ///
    /// Note that a stub file can [sometimes make use of a typing feature](https://typing.python.org/en/latest/spec/distributing.html#syntax)
    /// before it is available at runtime, as long as the stub does not make
    /// use of new *syntax*. For example, a type checker will understand
    /// `int | str` in a stub as being a `Union` type annotation, even if the
    /// type checker is run using Python 3.9, despite the fact that the `|`
    /// operator can only be used to create union types at runtime on Python
    /// 3.10+. As such, nuff will often recommend newer features in a stub
    /// file than it would for an equivalent runtime file with the same target
    /// version.
    #[option(
        default = r#""py310""#,
        value_type = r#""py37" | "py38" | "py39" | "py310" | "py311" | "py312" | "py313" | "py314""#,
        example = r#"
            # Always generate Python 3.7-compatible code.
            target-version = "py37"
        "#
    )]
    pub target_version: Option<PythonVersion>,

    /// A list of mappings from glob-style file pattern to Python version to use when checking the
    /// corresponding file(s).
    ///
    /// This may be useful for overriding the global Python version settings in `target-version` or
    /// `requires-python` for a subset of files. For example, if you have a project with a minimum
    /// supported Python version of 3.9 but a subdirectory of developer scripts that want to use a
    /// newer feature like the `match` statement from Python 3.10, you can use
    /// `per-file-target-version` to specify `"developer_scripts/*.py" = "py310"`.
    ///
    /// This setting is used by the linter to enforce any enabled version-specific lint rules, as
    /// well as by the formatter for any version-specific formatting options, such as parenthesizing
    /// context managers on Python 3.10+.
    #[option(
        default = "{}",
        value_type = "dict[str, PythonVersion]",
        scope = "per-file-target-version",
        example = r#"
            # Override the project-wide Python version for a developer scripts directory:
            "scripts/*.py" = "py312"
        "#
    )]
    pub per_file_target_version: Option<FxHashMap<String, PythonVersion>>,

    /// The directories to consider when resolving first- vs. third-party
    /// imports.
    ///
    /// When omitted, the `src` directory will typically default to including both:
    ///
    /// 1. The directory containing the nearest `pyproject.toml`, `nuff.toml`, or `.nuff.toml` file (the "project root").
    /// 2. The `"src"` subdirectory of the project root.
    ///
    /// These defaults ensure that nuff supports both flat layouts and `src` layouts out-of-the-box.
    /// (If a configuration file is explicitly provided (e.g., via the `--config` command-line
    /// flag), the current working directory will be considered the project root.)
    ///
    /// As an example, consider an alternative project structure, like:
    ///
    /// ```text
    /// my_project
    /// ├── pyproject.toml
    /// └── lib
    ///     └── my_package
    ///         ├── __init__.py
    ///         ├── foo.py
    ///         └── bar.py
    /// ```
    ///
    /// In this case, the `./lib` directory should be included in the `src` option
    /// (e.g., `src = ["lib"]`), such that when resolving imports, `my_package.foo`
    /// is considered first-party.
    ///
    /// This field supports globs. For example, if you have a series of Python
    /// packages in a `python_modules` directory, `src = ["python_modules/*"]`
    /// would expand to incorporate all packages in that directory. User home
    /// directory and environment variables will also be expanded.
    #[option(
        default = r#"[".", "src"]"#,
        value_type = "list[str]",
        example = r#"
            # Allow imports relative to the "src" and "test" directories.
            src = ["src", "test"]
        "#
    )]
    pub src: Option<Vec<String>>,

    #[option_group]
    pub lint: Option<LintOptions>,
}

impl Options {
    /// Deserialize inline configuration in one crate, avoiding repeated code generation.
    pub fn from_toml_table(table: toml::Table) -> Result<Self, toml::de::Error> {
        table.try_into()
    }
}

/// Configures how nuff checks your code.
///
/// Options specified in the `lint` section take precedence over the deprecated top-level settings.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Default, OptionsMetadata, Serialize, Deserialize)]
#[serde(
    from = "LintOptionsWire",
    deny_unknown_fields,
    rename_all = "kebab-case"
)]
#[cfg_attr(feature = "schemars", schemars(!from))]
pub struct LintOptions {
    #[serde(flatten)]
    pub common: LintCommonOptions,

    /// A list of file patterns to exclude from linting in addition to the files excluded globally (see [`exclude`](#exclude), and [`extend-exclude`](#extend-exclude)).
    ///
    /// Exclusions are based on globs, and can be either:
    ///
    /// - Single-path patterns, like `.mypy_cache` (to exclude any directory
    ///   named `.mypy_cache` in the tree), `foo.py` (to exclude any file named
    ///   `foo.py`), or `foo_*.py` (to exclude any file matching `foo_*.py` ).
    /// - Relative patterns, like `directory/foo.py` (to exclude that specific
    ///   file) or `directory/*.py` (to exclude any Python files in
    ///   `directory`). Note that these paths are relative to the project root
    ///   (e.g., the directory containing your `pyproject.toml`).
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"
            exclude = ["generated"]
        "#
    )]
    pub exclude: Option<Vec<String>>,

    /// Whether to enable preview mode. When preview mode is enabled, nuff will
    /// use unstable rules and fixes.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Enable preview features.
            preview = true
        "#
    )]
    pub preview: Option<bool>,

    /// Whether to allow imports from the third-party `typing_extensions` module for Python versions
    /// before a symbol was added to the first-party `typing` module.
    ///
    /// Many rules try to import symbols from the `typing` module but fall back to
    /// `typing_extensions` for earlier versions of Python. This option can be used to disable this
    /// fallback behavior in cases where `typing_extensions` is not installed.
    #[option(
        default = "true",
        value_type = "bool",
        example = r#"
            # Disable `typing_extensions` imports
            typing-extensions = false
        "#
    )]
    pub typing_extensions: Option<bool>,

    /// Whether to allow rules to add `from __future__ import annotations` in cases where this would
    /// simplify a fix or enable a new diagnostic.
    ///
    /// For example, `TC001`, `TC002`, and `TC003` can move more imports into `TYPE_CHECKING` blocks
    /// if `__future__` annotations are enabled.
    ///
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Enable `from __future__ import annotations` imports
            future-annotations = true
        "#
    )]
    pub future_annotations: Option<bool>,
}

pub(crate) fn validate_required_version(required_version: &RequiredVersion) -> anyhow::Result<()> {
    let version = pep440_rs::Version::from_str(VERSION)
        .expect("VERSION is not a valid PEP 440 version specifier");
    if !required_version.contains(&version) {
        return Err(anyhow::anyhow!(
            "Required version `{required_version}` does not match the running version \
            `{VERSION}`"
        ));
    }
    Ok(())
}

// Note: This struct should be inlined into [`LintOptions`] once support for the top-level lint settings
// is removed.
// Don't add any new options to this struct. Add them to [`LintOptions`] directly to avoid exposing them in the
// global settings.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(
    Clone, Debug, PartialEq, Eq, Default, OptionsMetadata, CombineOptions, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct LintCommonOptions {
    /// A regular expression used to identify "dummy" variables, or those which
    /// should be ignored when enforcing (e.g.) unused-variable rules. The
    /// default expression matches `_`, `__`, and `_var`, but not `_var_`.
    #[option(
        default = r#""^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$""#,
        value_type = "str",
        example = r#"
            # Only ignore variables named "_".
            dummy-variable-rgx = "^_$"
        "#
    )]
    pub dummy_variable_rgx: Option<String>,

    /// A list of rule codes or prefixes to ignore, in addition to those
    /// specified by `ignore`.
    ///
    /// This option is deprecated because it is now interchangeable with
    /// [`ignore`](#lint_ignore). In earlier versions of nuff, `ignore` would
    /// _replace_ the set of ignored rules when using configuration inheritance
    /// (via the top-level [`extend`]
    /// setting), while `extend-ignore` would _add_ to the inherited set. nuff
    /// now merges both `ignore` and `extend-ignore` into a single set, so the
    /// distinction no longer applies. Use [`ignore`](#lint_ignore) instead.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # Skip unused variable rules (`F841`).
            extend-ignore = ["F841"]
        "#
    )]
    #[deprecated(note = "The `extend-ignore` option is now interchangeable with \
        [`ignore`](#lint_ignore). Please update your configuration to use the \
        [`ignore`](#lint_ignore) option instead.")]
    pub extend_ignore: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes to enable, in addition to those
    /// specified by [`select`](#lint_select).
    ///
    /// Unlike [`select`](#lint_select), which _replaces_ the default rule set
    /// when specified, `extend-select` _adds_ to whatever rules are already
    /// active. This makes `extend-select` the preferred option when you want
    /// to enable additional rules on top of the defaults without having to
    /// enumerate them.
    ///
    /// For example, to enable the defaults plus flake8-bugbear:
    ///
    /// ```toml
    /// [tool.nuff.lint]
    /// # Adds flake8-bugbear on top of the default rules.
    /// extend-select = ["B"]
    /// ```
    ///
    /// Using `select = ["B"]` instead would _replace_ the defaults, enabling
    /// only flake8-bugbear.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # On top of the default `select`, enable flake8-bugbear (`B`) and flake8-quotes (`Q`).
            extend-select = ["B", "Q"]
        "#
    )]
    pub extend_select: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes to consider fixable, in addition to those
    /// specified by [`fixable`](#lint_fixable).
    #[option(
        default = r#"[]"#,
        value_type = "list[RuleSelector]",
        example = r#"
            # Enable fix for flake8-bugbear (`B`), on top of any rules specified by `fixable`.
            extend-fixable = ["B"]
        "#
    )]
    pub extend_fixable: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes to consider non-auto-fixable, in addition to those
    /// specified by [`unfixable`](#lint_unfixable).
    #[deprecated(note = "The `extend-unfixable` option is now interchangeable with \
        [`unfixable`](#lint_unfixable). Please update your configuration to \
        use the `unfixable` option instead.")]
    pub extend_unfixable: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes that are unsupported by nuff, but should be
    /// preserved when (e.g.) validating `# noqa` directives. Useful for
    /// retaining `# noqa` directives that cover plugins not yet implemented
    /// by nuff.
    #[option(
        default = "[]",
        value_type = "list[str]",
        example = r#"
            # Avoiding flagging (and removing) any codes starting with `V` from any
            # `# noqa` directives, despite nuff's lack of support for `vulture`.
            external = ["V"]
        "#
    )]
    pub external: Option<Vec<String>>,

    /// A list of rule codes or prefixes to consider fixable. By default,
    /// all rules are considered fixable.
    #[option(
        default = r#"["ALL"]"#,
        value_type = "list[RuleSelector]",
        example = r#"
            # Only allow fix behavior for `E` and `F` rules.
            fixable = ["E", "F"]
        "#
    )]
    pub fixable: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes to ignore. Prefixes can specify exact
    /// rules (like `F841`), entire groups (like `F`), or anything in
    /// between.
    ///
    /// When breaking ties between enabled and disabled rules (via `select` and
    /// `ignore`, respectively), more specific prefixes override less
    /// specific prefixes. `ignore` takes precedence over `select` if the same
    /// prefix appears in both.
    ///
    /// In preview, categories like `correctness` and `suspicious` can be used
    /// in addition to rule codes and linter group prefixes.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # Skip unused variable rules (`F841`).
            ignore = ["F841"]
        "#
    )]
    pub ignore: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes for which unsafe fixes should be considered
    /// safe.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # Allow applying all unsafe fixes in the `E` rules and `F401` without the `--unsafe-fixes` flag
            extend-safe-fixes = ["E", "F401"]
        "#
    )]
    pub extend_safe_fixes: Option<Vec<UnresolvedRuleSelector>>,

    /// A list of rule codes or prefixes for which safe fixes should be considered
    /// unsafe.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # Require the `--unsafe-fixes` flag when fixing the `E` rules and `F401`
            extend-unsafe-fixes = ["E", "F401"]
        "#
    )]
    pub extend_unsafe_fixes: Option<Vec<UnresolvedRuleSelector>>,

    /// Avoid automatically removing unused imports in `__init__.py` files. Such
    /// imports will still be flagged, but with a dedicated message suggesting
    /// that the import is either added to the module's `__all__` symbol, or
    /// re-exported with a redundant alias (e.g., `import os as os`).
    ///
    /// This option is enabled by default, but you can opt-in to removal of imports
    /// via an unsafe fix.
    #[option(
        default = "true",
        value_type = "bool",
        example = r#"
            ignore-init-module-imports = false
        "#
    )]
    #[deprecated(
        since = "0.4.4",
        note = "`ignore-init-module-imports` will be removed in a future version because F401 now \
            recommends appropriate fixes for unused imports in `__init__.py` (currently in \
            preview mode). See documentation for more information and please update your \
            configuration."
    )]
    pub ignore_init_module_imports: Option<bool>,

    /// A list of objects that should be treated equivalently to a
    /// `logging.Logger` object.
    ///
    /// This is useful for ensuring proper diagnostics (e.g., to identify
    /// `logging` deprecations and other best-practices) for projects that
    /// re-export a `logging.Logger` object from a common module.
    ///
    /// For example, if you have a module `logging_setup.py` with the following
    /// contents:
    /// ```python
    /// import logging
    ///
    /// logger = logging.getLogger(__name__)
    /// ```
    ///
    /// Adding `"logging_setup.logger"` to `logger-objects` will ensure that
    /// `logging_setup.logger` is treated as a `logging.Logger` object when
    /// imported from other modules (e.g., `from logging_setup import logger`).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"logger-objects = ["logging_setup.logger"]"#
    )]
    pub logger_objects: Option<Vec<String>>,

    /// A list of rule codes or prefixes to enable. Prefixes can specify exact
    /// rules (like `F841`), entire groups (like `F`), or anything in
    /// between.
    ///
    /// When breaking ties between enabled and disabled rules (via `select` and
    /// `ignore`, respectively), more specific prefixes override less
    /// specific prefixes. `ignore` takes precedence over `select` if the
    /// same prefix appears in both.
    ///
    /// In preview, categories like `correctness` and `suspicious` can be used
    /// in addition to rule codes and linter group prefixes.
    #[option(
        default = r#"["F"]"#,
        value_type = "list[RuleSelector]",
        example = r#"
            # On top of the defaults, enable flake8-bugbear (`B`) and flake8-quotes (`Q`).
            extend-select = ["B", "Q"]
        "#
    )]
    pub select: Option<Vec<UnresolvedRuleSelector>>,

    /// Whether to require exact codes to select preview rules. When enabled,
    /// preview rules will not be selected by prefixes — the full code of each
    /// preview rule will be required to enable the rule.
    #[option(
        default = "false",
        value_type = "bool",
        example = r#"
            # Require explicit selection of preview rules.
            explicit-preview-rules = true
        "#
    )]
    pub explicit_preview_rules: Option<bool>,

    /// A list of modules whose exports should be treated equivalently to
    /// members of the `typing` module.
    ///
    /// This is useful for ensuring proper type annotation inference for
    /// projects that re-export `typing` and `typing_extensions` members
    /// from a compatibility module. If omitted, any members imported from
    /// modules apart from `typing` and `typing_extensions` will be treated
    /// as ordinary Python objects.
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"typing-modules = ["airflow.typing_compat"]"#
    )]
    pub typing_modules: Option<Vec<String>>,

    /// A list of rule codes or prefixes to consider non-fixable.
    #[option(
        default = "[]",
        value_type = "list[RuleSelector]",
        example = r#"
            # Disable fix for unused imports (`F401`).
            unfixable = ["F401"]
        "#
    )]
    pub unfixable: Option<Vec<UnresolvedRuleSelector>>,

    /// Options for the `nuff` house rules.
    #[option_group]
    pub nuff: Option<NuffOptions>,

    /// Options for the `pyflakes` plugin.
    #[option_group]
    pub pyflakes: Option<PyflakesOptions>,

    // WARNING: Don't add new options to this type. Add them to `LintOptions` instead.

    // Tables are required to go last.
    /// A list of mappings from file pattern to rule codes or prefixes to
    /// exclude, when considering any matching files. An initial '!' negates
    /// the file pattern.
    ///
    /// For more information on the glob syntax, refer to the [`globset` documentation](https://docs.rs/globset/latest/globset/#syntax).
    #[option(
        default = "{}",
        value_type = "dict[str, list[RuleSelector]]",
        scope = "per-file-ignores",
        example = r#"
            # Ignore `E402` (import violations) in all `__init__.py` files, and in `path/to/file.py`.
            "__init__.py" = ["E402"]
            "path/to/file.py" = ["E402"]
            # Ignore `D` rules everywhere except for the `src/` directory.
            "!src/**.py" = ["D"]
            # Ignore check for packages that are missing an `__init__.py` file.
            "{benchmark,scripts,.github/action-name/}/*.py" = ["INP001"]
        "#
    )]
    pub per_file_ignores: Option<FxHashMap<String, Vec<UnresolvedRuleSelector>>>,

    /// A list of mappings from file pattern to rule codes or prefixes to
    /// exclude, in addition to any rules excluded by [`per-file-ignores`](#lint_per-file-ignores).
    #[option(
        default = "{}",
        value_type = "dict[str, list[RuleSelector]]",
        scope = "extend-per-file-ignores",
        example = r#"
            # Also ignore `E402` in all `__init__.py` files.
            "__init__.py" = ["E402"]
        "#
    )]
    pub extend_per_file_ignores: Option<FxHashMap<String, Vec<UnresolvedRuleSelector>>>,
    // WARNING: Don't add new options to this type. Add them to `LintOptions` instead.
}

/// Options for the `nuff` house rules.
#[derive(
    Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, OptionsMetadata, CombineOptions,
)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct NuffOptions {
    /// Fully qualified names of synchronous functions that block, which an `async` function may
    /// reach only through `asyncio.to_thread` (`NUF001`).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"blocking-functions = ["app.platform.email.send_email"]"#
    )]
    pub blocking_functions: Option<Vec<String>>,

    /// Fully qualified names of functions that call an outside service, which no function may
    /// call while its session's transaction is open (`NUF005`).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"external-functions = ["app.billing.services.vipps.create_charge"]"#
    )]
    pub external_functions: Option<Vec<String>>,

    /// The module that defines each table, by table name. A module whose `ForeignKey` names a
    /// table must import the module that defines it (`NUF004`).
    #[option(
        default = r#"{}"#,
        value_type = "dict[str, str]",
        example = r#"foreign-key-modules = { companies = "app.accounts.models" }"#
    )]
    pub foreign_key_modules: Option<FxHashMap<String, String>>,
}

impl NuffOptions {
    pub(crate) fn into_settings(self) -> nuff::settings::Settings {
        nuff::settings::Settings {
            blocking_functions: self.blocking_functions.unwrap_or_default(),
            external_functions: self.external_functions.unwrap_or_default(),
            foreign_key_modules: self.foreign_key_modules.unwrap_or_default(),
        }
    }
}

/// Options for the `pyflakes` plugin.
#[derive(
    Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, OptionsMetadata, CombineOptions,
)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PyflakesOptions {
    /// Additional functions or classes to consider generic, such that any
    /// subscripts should be treated as type annotation (e.g., `ForeignKey` in
    /// `django.db.models.ForeignKey["User"]`.
    ///
    /// Expects to receive a list of fully-qualified names (e.g., `django.db.models.ForeignKey`,
    /// rather than `ForeignKey`).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = "extend-generics = [\"django.db.models.ForeignKey\"]"
    )]
    extend_generics: Option<Vec<String>>,

    /// A list of modules to ignore when considering unused imports.
    ///
    /// Used to prevent violations for specific modules that are known to have side effects on
    /// import (e.g., `hvplot.pandas`).
    ///
    /// Modules in this list are expected to be fully-qualified names (e.g., `hvplot.pandas`). Any
    /// submodule of a given module will also be ignored (e.g., given `hvplot`, `hvplot.pandas`
    /// will also be ignored).
    #[option(
        default = r#"[]"#,
        value_type = "list[str]",
        example = r#"allowed-unused-imports = ["hvplot.pandas"]"#
    )]
    allowed_unused_imports: Option<Vec<String>>,
}

impl PyflakesOptions {
    pub(crate) fn into_settings(self) -> pyflakes::settings::Settings {
        pyflakes::settings::Settings {
            extend_generics: self.extend_generics.unwrap_or_default(),
            allowed_unused_imports: self.allowed_unused_imports.unwrap_or_default(),
        }
    }
}

/// Like [`LintCommonOptions`], but with any `#[serde(flatten)]` fields inlined. This leads to far,
/// far better error messages when deserializing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct LintOptionsWire {
    dummy_variable_rgx: Option<String>,
    extend_ignore: Option<Vec<UnresolvedRuleSelector>>,
    extend_select: Option<Vec<UnresolvedRuleSelector>>,
    extend_fixable: Option<Vec<UnresolvedRuleSelector>>,
    extend_unfixable: Option<Vec<UnresolvedRuleSelector>>,
    external: Option<Vec<String>>,
    fixable: Option<Vec<UnresolvedRuleSelector>>,
    ignore: Option<Vec<UnresolvedRuleSelector>>,
    extend_safe_fixes: Option<Vec<UnresolvedRuleSelector>>,
    extend_unsafe_fixes: Option<Vec<UnresolvedRuleSelector>>,
    ignore_init_module_imports: Option<bool>,
    logger_objects: Option<Vec<String>>,
    select: Option<Vec<UnresolvedRuleSelector>>,
    explicit_preview_rules: Option<bool>,
    typing_modules: Option<Vec<String>>,
    unfixable: Option<Vec<UnresolvedRuleSelector>>,
    nuff: Option<NuffOptions>,
    pyflakes: Option<PyflakesOptions>,
    per_file_ignores: Option<FxHashMap<String, Vec<UnresolvedRuleSelector>>>,
    extend_per_file_ignores: Option<FxHashMap<String, Vec<UnresolvedRuleSelector>>>,

    exclude: Option<Vec<String>>,
    preview: Option<bool>,
    typing_extensions: Option<bool>,
    future_annotations: Option<bool>,
}

impl From<LintOptionsWire> for LintOptions {
    fn from(value: LintOptionsWire) -> LintOptions {
        let LintOptionsWire {
            dummy_variable_rgx,
            extend_ignore,
            extend_select,
            extend_fixable,
            extend_unfixable,
            external,
            fixable,
            ignore,
            extend_safe_fixes,
            extend_unsafe_fixes,
            ignore_init_module_imports,
            logger_objects,
            select,
            explicit_preview_rules,
            typing_modules,
            unfixable,
            nuff,
            pyflakes,
            per_file_ignores,
            extend_per_file_ignores,
            exclude,
            preview,
            typing_extensions,
            future_annotations,
        } = value;

        LintOptions {
            #[expect(deprecated)]
            common: LintCommonOptions {
                dummy_variable_rgx,
                extend_ignore,
                extend_select,
                extend_fixable,
                extend_unfixable,
                external,
                fixable,
                ignore,
                extend_safe_fixes,
                extend_unsafe_fixes,
                ignore_init_module_imports,
                logger_objects,
                select,
                explicit_preview_rules,
                typing_modules,
                unfixable,
                nuff,
                pyflakes,
                per_file_ignores,
                extend_per_file_ignores,
            },
            exclude,
            preview,
            typing_extensions,
            future_annotations,
        }
    }
}
