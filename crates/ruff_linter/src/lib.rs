//! nuff's linter.

pub use locator::Locator;
#[cfg(feature = "clap")]
pub use rule_selector::clap_completion::UnresolvedRuleSelectorParser;
pub use rule_selector::{RuleSelector, UnresolvedRuleSelector};
pub use rules::pycodestyle::rules::IOError;

pub(crate) use ruff_diagnostics::{Applicability, Edit, Fix};
pub use violation::{AlwaysFixableViolation, FixAvailability, Violation, ViolationMetadata};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

mod checkers;
pub mod codes;
mod comments;
mod cst;
pub mod directives;
mod fix;
pub mod fs;
mod importer;
pub mod line_width;
pub mod linter;
mod locator;
pub mod logging;
pub mod message;
mod noqa;
pub mod package;
pub mod packaging;
pub mod preview;
pub mod registry;
mod renamer;
pub mod rule_selector;
pub mod rules;
pub mod settings;
pub mod source_kind;
mod text_helpers;
mod violation;

#[cfg(any(test, feature = "testing"))]
pub mod test;

pub const RUFF_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
