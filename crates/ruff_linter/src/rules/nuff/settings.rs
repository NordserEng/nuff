//! Settings for the `nuff` house rules.

use std::fmt::{Display, Formatter};

use rustc_hash::FxHashMap;

use ruff_macros::CacheKey;

use crate::display_settings;

#[derive(Debug, Clone, Default, CacheKey)]
pub struct Settings {
    pub blocking_functions: Vec<String>,
    pub external_functions: Vec<String>,
    pub foreign_key_modules: FxHashMap<String, String>,
}

impl Display for Settings {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        display_settings! {
            formatter = f,
            namespace = "linter.nuff",
            fields = [
                self.blocking_functions | array,
                self.external_functions | array,
                self.foreign_key_modules | map,
            ]
        }
        Ok(())
    }
}
