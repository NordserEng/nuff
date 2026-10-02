//! Remnant of the registry of all [`Rule`] implementations, now it's reexporting from codes.rs
//! with some helper symbols

use crate::diagnostic::LintName;
use strum_macros::EnumIter;

pub use codes::Rule;
use nuff_macros::RuleNamespace;
pub use rule_set::{RuleSet, RuleSetIterator};

use crate::codes::{self};

mod rule_set;

impl Rule {
    pub fn from_code(code: &str) -> Result<Self, FromCodeError> {
        let (linter, code) = Linter::parse_code(code).ok_or(FromCodeError::Unknown)?;
        linter
            .all_rules()
            .find(|rule| {
                rule.noqa_code()
                    .is_some_and(|rule_code| rule_code.suffix() == code)
            })
            .ok_or(FromCodeError::Unknown)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum FromCodeError {
    #[error("unknown rule code")]
    Unknown,
}

#[derive(EnumIter, Debug, PartialEq, Eq, Clone, Hash, RuleNamespace)]
pub enum Linter {
    /// [flake8-async](https://pypi.org/project/flake8-async/)
    #[prefix = "ASYNC"]
    Flake8Async,
    /// [nuff](https://github.com/NordserEng/nuff)
    #[prefix = "NUF"]
    Nuff,
    /// [pycodestyle](https://pypi.org/project/pycodestyle/)
    #[prefix = "E"]
    Pycodestyle,
    /// [Pyflakes](https://pypi.org/project/pyflakes/)
    #[prefix = "F"]
    Pyflakes,
    /// [Pylint](https://pypi.org/project/pylint/)
    #[prefix = "PL"]
    Pylint,
}

pub trait RuleNamespace: Sized {
    /// Returns the prefix that every single code that nuff uses to identify
    /// rules from this linter starts with.  In the case that multiple
    /// `#[prefix]`es are configured for the variant in the `Linter` enum
    /// definition this is the empty string.
    fn common_prefix(&self) -> &'static str;

    /// Attempts to parse the given rule code. If the prefix is recognized
    /// returns the respective variant along with the code with the common
    /// prefix stripped.
    fn parse_code(code: &str) -> Option<(Self, &str)>;

    fn name(&self) -> &'static str;

    fn url(&self) -> Option<&'static str>;
}

#[derive(is_macro::Is, Copy, Clone)]
pub enum LintSource {
    Ast,
    Io,
}

impl Rule {
    /// The source for the diagnostic: the file could not be read, or the AST.
    pub const fn lint_source(&self) -> LintSource {
        match self {
            Rule::IOError => LintSource::Io,
            _ => LintSource::Ast,
        }
    }

    pub fn name(&self) -> LintName {
        let name: &'static str = self.into();
        LintName::of(name)
    }

    /// Return the rule's name, followed by its code in parentheses when available.
    ///
    /// For example:
    ///
    /// ```text
    /// unused-import (F401)
    /// ```
    ///
    /// When formatted with the `#` flag, both the name and code will be surrounded by backticks:
    ///
    /// ```text
    /// `unused-import` (`F401`)
    /// ```
    pub fn name_and_code(&self) -> impl std::fmt::Display + use<> {
        let rule = *self;
        std::fmt::from_fn(move |f| {
            let quote = if f.alternate() { "`" } else { "" };
            write!(f, "{quote}{}{quote}", rule.name())?;
            if let Some(code) = rule.noqa_code() {
                write!(f, " ({quote}{code}{quote})")?;
            }
            Ok(())
        })
    }
}

#[cfg(feature = "clap")]
pub mod clap_completion {
    use clap::builder::{
        PossibleValue, PossibleValuesParser, TypedValueParser, ValueParserFactory,
    };
    use clap::error::ContextKind;
    use strum::IntoEnumIterator;

    use crate::registry::Rule;

    #[derive(Clone)]
    pub struct RuleParser;

    impl RuleParser {
        fn values() -> impl Iterator<Item = PossibleValue> {
            Rule::iter().flat_map(|rule| {
                let name = rule.name().as_str();
                let (code, name) = if let Some(code) = rule.noqa_code() {
                    let code = code.to_string();
                    (
                        Some(PossibleValue::new(&code).help(name)),
                        PossibleValue::new(name).help(code),
                    )
                } else {
                    (None, PossibleValue::new(name))
                };
                code.into_iter().chain(std::iter::once(name))
            })
        }
    }

    impl ValueParserFactory for Rule {
        type Parser = RuleParser;

        fn value_parser() -> Self::Parser {
            RuleParser
        }
    }

    impl TypedValueParser for RuleParser {
        type Value = Rule;

        fn parse_ref(
            &self,
            cmd: &clap::Command,
            arg: Option<&clap::Arg>,
            value: &std::ffi::OsStr,
        ) -> Result<Self::Value, clap::Error> {
            PossibleValuesParser::new(Self::values())
                .try_map(|value| Rule::from_code(&value).or_else(|_| Rule::from_name(&value)))
                .parse_ref(cmd, arg, value)
                .map_err(|mut error| {
                    error.remove(ContextKind::ValidValue);
                    error
                })
        }

        fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
            Some(Box::new(Self::values()))
        }
    }
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;
    use std::mem::size_of;

    use strum::IntoEnumIterator;

    use super::{Linter, Rule, RuleNamespace};

    #[test]
    fn documentation() {
        for rule in Rule::iter() {
            assert!(
                rule.explanation().is_some(),
                "Rule {} is missing documentation",
                rule.name()
            );
        }
    }

    #[test]
    fn rule_naming_convention() {
        // The disallowed rule names are defined in a separate file so that they can also be picked up by add_rule.py.
        let patterns: Vec<_> = include_str!("../resources/test/disallowed_rule_names.txt")
            .trim()
            .split('\n')
            .map(|line| {
                glob::Pattern::new(line).expect("malformed pattern in disallowed_rule_names.txt")
            })
            .collect();

        for rule in Rule::iter() {
            let rule_name = rule.name();
            for pattern in &patterns {
                assert!(
                    !pattern.matches(&rule_name),
                    "{rule_name} does not match naming convention, see CONTRIBUTING.md"
                );
            }
        }
    }

    #[test]
    fn check_code_serialization() {
        for rule in Rule::iter() {
            let Some(code) = rule.noqa_code() else {
                continue;
            };

            assert!(
                Rule::from_code(&code.to_string()).is_ok(),
                "{rule:?} could not be round-trip serialized."
            );
        }
    }

    #[test]
    fn linter_parse_code() {
        for rule in Rule::iter() {
            let Some(code) = rule.noqa_code() else {
                continue;
            };
            let code = code.to_string();
            let (linter, rest) =
                Linter::parse_code(&code).unwrap_or_else(|| panic!("couldn't parse {code:?}"));
            assert_eq!(code, format!("{}{rest}", linter.common_prefix()));
        }
    }

    #[test]
    fn rule_size() {
        assert_eq!(2, size_of::<Rule>());
    }

    #[test]
    fn linter_sorting() {
        let names: Vec<_> = Linter::iter()
            .map(|linter| linter.name().to_lowercase())
            .collect();

        let sorted: Vec<_> = names.iter().cloned().sorted().collect();

        assert_eq!(
            &names[..],
            &sorted[..],
            "Linters are not sorted alphabetically (case insensitive)"
        );
    }
}
