/// In this module we generate [`Rule`], an enum of all rules, and [`RuleCodePrefix`], an enum of
/// all linter groups. A linter group is something like `pyflakes` or `flake8-todos`. Each linter
/// group contains all rules and their common prefixes, i.e. everything you can specify in
/// `--select`. For `pylint` this is e.g. `C0414` and `E0118` but also `C` and `E01`.
///
/// When [`crate::preview::is_rule_categories_enabled`] returns `true`, rules can also be selected by
/// their [`Category`].
use std::fmt::Formatter;
use std::sync::LazyLock;

use ruff_db::diagnostic::SecondaryCode;
use serde::Serialize;
use strum::{IntoEnumIterator, VariantArray as _};
use strum_macros::{Display, EnumIter, EnumMessage, EnumString, IntoStaticStr, VariantArray};

use crate::registry::Linter;
use crate::rules;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoqaCode(&'static str, &'static str);

impl NoqaCode {
    /// Return the prefix for the [`NoqaCode`], e.g., `SIM` for `SIM101`.
    pub fn prefix(&self) -> &str {
        self.0
    }

    /// Return the suffix for the [`NoqaCode`], e.g., `101` for `SIM101`.
    pub fn suffix(&self) -> &str {
        self.1
    }

    pub(crate) const fn into_parts(self) -> (&'static str, &'static str) {
        (self.0, self.1)
    }
}

impl std::fmt::Debug for NoqaCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for NoqaCode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}{}", self.0, self.1)
    }
}

impl PartialEq<&str> for NoqaCode {
    fn eq(&self, other: &&str) -> bool {
        match other.strip_prefix(self.0) {
            Some(suffix) => suffix == self.1,
            None => false,
        }
    }
}

impl PartialEq<NoqaCode> for &str {
    fn eq(&self, other: &NoqaCode) -> bool {
        other.eq(self)
    }
}

impl PartialEq<NoqaCode> for SecondaryCode {
    fn eq(&self, other: &NoqaCode) -> bool {
        &self.as_str() == other
    }
}

impl PartialEq<SecondaryCode> for NoqaCode {
    fn eq(&self, other: &SecondaryCode) -> bool {
        other.eq(self)
    }
}

impl serde::Serialize for NoqaCode {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

/// The category assigned to a lint rule.
///
/// These categories are similar to those found in [Clippy] and form much broader groupings than the
/// linter-based groups. Categories are intended to be our primary classification mechanism for
/// rules going forward, with the linter groups eventually being deprecated and removed, albeit in
/// the relatively distant future. The categorization of a rule determines two important properties:
/// - its default status, `style` and above are currently enabled by default
/// - its default severity, in a future where we have multiple diagnostic severities
///
/// Assuming we continue to follow Clippy, `correctness` lints will have a severity of `error` by
/// default, while the other on-by-default categories will have a severity of `warn` by default.
///
/// Secondary groups like the legacy linter groups are orthogonal selection mechanisms that have no
/// impact on severity or default status, and may, and usually do, include rules from multiple
/// categories. For example, many `F` rules are `correctness` lints, but `F` includes `suspicious`
/// and even `pedantic` rules too. At some point in the future, we may support additional secondary
/// groups that are not legacy linter groups as well.
///
/// The precedence between categories, linter groups, linter prefixes, and rules is determined by
/// the [`crate::rule_selector::Specificity`] returned by
/// [`crate::rule_selector::RuleSelector::specificity`], and currently follows this ordering:
///
/// ```text
/// ALL < category < linter group < linter prefix < rule
/// ```
///
/// Variants are ordered by descending severity, with error categories first, followed by warning,
/// and then by off-by-default categories. The rule documentation lists category filter options
/// in this order.
///
/// See our [rule categorization guidelines] for more information on assigning categories.
///
/// [Clippy]: https://doc.rust-lang.org/clippy/lints.html
/// [rule categorization guidelines]: https://docs.astral.sh/ruff/rule-proposals/#rule-categorization-guidelines
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    EnumIter,
    EnumString,
    IntoStaticStr,
    Display,
    Serialize,
    EnumMessage,
    VariantArray,
)]
#[strum(serialize_all = "kebab-case", const_into_str)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Rules that flag outright wrong code
    Correctness,

    /// Rules that flag likely outright wrong code but that could be intentional
    Suspicious,

    /// Rules that suggest rewriting code in a shorter and more readable way
    Complexity,

    /// Rules that suggest rewriting code in a more efficient way
    Performance,

    /// Rules that suggest rewriting code in a more idiomatic way
    Style,

    /// Rules that flag potential security vulnerabilities but may be prone to false positives
    Security,

    /// Rules that flag formatting issues that do not affect semantics
    Formatting,

    /// Rules that are highly opinionated or prone to false positives
    Pedantic,

    /// Rules that restrict the use of certain features
    Restriction,

    /// Internal testing rules that shouldn't be exposed to users.
    #[cfg(any(feature = "test-rules", test))]
    #[strum(disabled)]
    Testing,
}

impl Category {
    /// Return the description of the category, derived from its documentation.
    #[cfg(any(feature = "clap", test))]
    pub(crate) fn description(self) -> &'static str {
        let Some(docs) =
            strum::EnumMessage::get_documentation(&self).and_then(|docs| docs.lines().next())
        else {
            panic!("Category `{self}` missing required documentation");
        };

        docs
    }

    /// Return the rules in this category.
    pub(crate) fn rules(self) -> &'static [Rule] {
        static RULES_BY_CATEGORY: LazyLock<[Box<[Rule]>; Category::VARIANTS.len()]> =
            LazyLock::new(|| {
                let mut rules = [const { Vec::new() }; Category::VARIANTS.len()];

                for rule in Rule::iter() {
                    rules[rule.category() as usize].push(rule);
                }

                rules.map(Vec::into_boxed_slice)
            });

        &RULES_BY_CATEGORY[self as usize]
    }

    /// Return the categories that should be enabled by default.
    pub const fn default_categories() -> [Category; 5] {
        [
            Self::Correctness,
            Self::Suspicious,
            Self::Complexity,
            Self::Performance,
            Self::Style,
        ]
    }
}

#[derive(Debug, Copy, Clone, Serialize)]
pub enum RuleStatus {
    /// The rule is stable since the provided Ruff version.
    Stable { since: &'static str },
    /// The rule has been unstable since the provided Ruff version, and preview mode must be enabled
    /// for usage.
    Preview { since: &'static str },
    /// The rule has been deprecated since the provided Ruff version, warnings will be displayed
    /// during selection in stable and errors will be raised if used with preview mode enabled.
    Deprecated { since: &'static str },
    /// The rule was removed in the provided Ruff version, and errors will be displayed on use.
    Removed { since: &'static str },
}

impl std::fmt::Display for RuleStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RuleStatus::Stable { .. } => "stable",
            RuleStatus::Preview { .. } => "preview",
            RuleStatus::Deprecated { .. } => "deprecated",
            RuleStatus::Removed { .. } => "removed",
        })
    }
}

#[ruff_macros::map_codes]
pub fn code_to_rule(linter: Linter, code: &str) -> Option<(RuleStatus, Rule)> {
    #[expect(clippy::enum_glob_use)]
    use Linter::*;

    #[rustfmt::skip]
    Some(match (linter, code) {
        // pycodestyle
        (Pycodestyle, "902") => rules::pycodestyle::rules::IOError,

        // pyflakes
        (Pyflakes, "401") => rules::pyflakes::rules::UnusedImport,
        (Pyflakes, "402") => rules::pyflakes::rules::ImportShadowedByLoopVar,
        (Pyflakes, "403") => rules::pyflakes::rules::UndefinedLocalWithImportStar,
        (Pyflakes, "404") => rules::pyflakes::rules::LateFutureImport,
        (Pyflakes, "405") => rules::pyflakes::rules::UndefinedLocalWithImportStarUsage,
        (Pyflakes, "406") => rules::pyflakes::rules::UndefinedLocalWithNestedImportStarUsage,
        (Pyflakes, "407") => rules::pyflakes::rules::FutureFeatureNotDefined,
        (Pyflakes, "501") => rules::pyflakes::rules::PercentFormatInvalidFormat,
        (Pyflakes, "502") => rules::pyflakes::rules::PercentFormatExpectedMapping,
        (Pyflakes, "503") => rules::pyflakes::rules::PercentFormatExpectedSequence,
        (Pyflakes, "504") => rules::pyflakes::rules::PercentFormatExtraNamedArguments,
        (Pyflakes, "505") => rules::pyflakes::rules::PercentFormatMissingArgument,
        (Pyflakes, "506") => rules::pyflakes::rules::PercentFormatMixedPositionalAndNamed,
        (Pyflakes, "507") => rules::pyflakes::rules::PercentFormatPositionalCountMismatch,
        (Pyflakes, "508") => rules::pyflakes::rules::PercentFormatStarRequiresSequence,
        (Pyflakes, "509") => rules::pyflakes::rules::PercentFormatUnsupportedFormatCharacter,
        (Pyflakes, "521") => rules::pyflakes::rules::StringDotFormatInvalidFormat,
        (Pyflakes, "522") => rules::pyflakes::rules::StringDotFormatExtraNamedArguments,
        (Pyflakes, "523") => rules::pyflakes::rules::StringDotFormatExtraPositionalArguments,
        (Pyflakes, "524") => rules::pyflakes::rules::StringDotFormatMissingArguments,
        (Pyflakes, "525") => rules::pyflakes::rules::StringDotFormatMixingAutomatic,
        (Pyflakes, "541") => rules::pyflakes::rules::FStringMissingPlaceholders,
        (Pyflakes, "601") => rules::pyflakes::rules::MultiValueRepeatedKeyLiteral,
        (Pyflakes, "602") => rules::pyflakes::rules::MultiValueRepeatedKeyVariable,
        (Pyflakes, "621") => rules::pyflakes::rules::ExpressionsInStarAssignment,
        (Pyflakes, "622") => rules::pyflakes::rules::MultipleStarredExpressions,
        (Pyflakes, "631") => rules::pyflakes::rules::AssertTuple,
        (Pyflakes, "632") => rules::pyflakes::rules::IsLiteral,
        (Pyflakes, "633") => rules::pyflakes::rules::InvalidPrintSyntax,
        (Pyflakes, "634") => rules::pyflakes::rules::IfTuple,
        (Pyflakes, "701") => rules::pyflakes::rules::BreakOutsideLoop,
        (Pyflakes, "702") => rules::pyflakes::rules::ContinueOutsideLoop,
        (Pyflakes, "704") => rules::pyflakes::rules::YieldOutsideFunction,
        (Pyflakes, "706") => rules::pyflakes::rules::ReturnOutsideFunction,
        (Pyflakes, "707") => rules::pyflakes::rules::DefaultExceptNotLast,
        (Pyflakes, "722") => rules::pyflakes::rules::ForwardAnnotationSyntaxError,
        (Pyflakes, "811") => rules::pyflakes::rules::RedefinedWhileUnused,
        (Pyflakes, "821") => rules::pyflakes::rules::UndefinedName,
        (Pyflakes, "822") => rules::pyflakes::rules::UndefinedExport,
        (Pyflakes, "823") => rules::pyflakes::rules::UndefinedLocal,
        (Pyflakes, "841") => rules::pyflakes::rules::UnusedVariable,
        (Pyflakes, "842") => rules::pyflakes::rules::UnusedAnnotation,
        (Pyflakes, "901") => rules::pyflakes::rules::RaiseNotImplemented,

        // pylint
        (Pylint, "C0415") => rules::pylint::rules::ImportOutsideTopLevel,

        // flake8-async
        (Flake8Async, "100") => rules::flake8_async::rules::CancelScopeNoCheckpoint,
        (Flake8Async, "105") => rules::flake8_async::rules::TrioSyncCall,
        (Flake8Async, "109") => rules::flake8_async::rules::AsyncFunctionWithTimeout,
        (Flake8Async, "110") => rules::flake8_async::rules::AsyncBusyWait,
        (Flake8Async, "115") => rules::flake8_async::rules::AsyncZeroSleep,
        (Flake8Async, "116") => rules::flake8_async::rules::LongSleepNotForever,
        (Flake8Async, "119") => rules::flake8_async::rules::YieldInContextManagerInAsyncGenerator,
        (Flake8Async, "210") => rules::flake8_async::rules::BlockingHttpCallInAsyncFunction,
        (Flake8Async, "212") => rules::flake8_async::rules::BlockingHttpCallHttpxInAsyncFunction,
        (Flake8Async, "220") => rules::flake8_async::rules::CreateSubprocessInAsyncFunction,
        (Flake8Async, "221") => rules::flake8_async::rules::RunProcessInAsyncFunction,
        (Flake8Async, "222") => rules::flake8_async::rules::WaitForProcessInAsyncFunction,
        (Flake8Async, "230") => rules::flake8_async::rules::BlockingOpenCallInAsyncFunction,
        (Flake8Async, "240") => rules::flake8_async::rules::BlockingPathMethodInAsyncFunction,
        (Flake8Async, "250") => rules::flake8_async::rules::BlockingInputInAsyncFunction,
        (Flake8Async, "251") => rules::flake8_async::rules::BlockingSleepInAsyncFunction,

        // nuff
        (Nuff, "001") => rules::nuff::rules::BlockingCallOutsideThread,
        (Nuff, "002") => rules::nuff::rules::RouteReturnsDict,
        (Nuff, "003") => rules::nuff::rules::AlembicDowngrade,
        (Nuff, "004") => rules::nuff::rules::ForeignKeyModelNotImported,
        (Nuff, "005") => rules::nuff::rules::ExternalCallInTransaction,
    })
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(self.into())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FromNameError {
    #[error("unknown rule name")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use itertools::Itertools;
    use strum::IntoEnumIterator;

    use super::{Category, Rule};

    #[test]
    fn category_names_do_not_conflict_with_rule_names() {
        for category in Category::iter() {
            assert!(
                Rule::from_name(category.into_str()).is_err(),
                "category {category} conflicts with a rule name"
            );
        }
    }

    #[test]
    fn category_descriptions() {
        let snapshot = Category::iter().format_with("\n", |category, f| {
            f(&format_args!(
                "{name}: {description}",
                name = category.into_str(),
                description = category.description(),
            ))
        });

        assert_snapshot!(snapshot, @"
        correctness: Rules that flag outright wrong code
        suspicious: Rules that flag likely outright wrong code but that could be intentional
        complexity: Rules that suggest rewriting code in a shorter and more readable way
        performance: Rules that suggest rewriting code in a more efficient way
        style: Rules that suggest rewriting code in a more idiomatic way
        security: Rules that flag potential security vulnerabilities but may be prone to false positives
        formatting: Rules that flag formatting issues that do not affect semantics
        pedantic: Rules that are highly opinionated or prone to false positives
        restriction: Rules that restrict the use of certain features
        ");
    }
}
