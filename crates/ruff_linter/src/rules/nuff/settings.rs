//! Settings for the `nuff` house rules.

use rustc_hash::FxHashMap;

use ruff_macros::CacheKey;

#[derive(Debug, Clone, Default, CacheKey)]
pub struct Settings {
    pub blocking_functions: Vec<String>,
    pub external_functions: Vec<String>,
    pub foreign_key_modules: FxHashMap<String, String>,
}
