//! Settings for the `Pyflakes` plugin.

use ruff_macros::CacheKey;

#[derive(Debug, Clone, Default, CacheKey)]
pub struct Settings {
    pub extend_generics: Vec<String>,
    pub allowed_unused_imports: Vec<String>,
}
