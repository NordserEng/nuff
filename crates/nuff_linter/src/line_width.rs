use std::error::Error;
use std::fmt;
use std::hash::Hasher;
use std::num::{NonZeroU8, NonZeroU16, ParseIntError};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use nuff_cache::{CacheKey, CacheKeyHasher};
use nuff_macros::CacheKey;

/// The length of a line of text that is considered too long.
///
/// The allowed range of values is 1..=65535
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LineLength(NonZeroU16);

impl LineLength {
    /// Maximum allowed value for a valid [`LineLength`]
    const MAX: u16 = u16::MAX;

    /// Return the numeric value for this [`LineLength`]
    pub fn value(&self) -> u16 {
        self.0.get()
    }
}

impl Default for LineLength {
    fn default() -> Self {
        Self(NonZeroU16::new(88).unwrap())
    }
}

impl fmt::Display for LineLength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl<'de> serde::Deserialize<'de> for LineLength {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = i64::deserialize(deserializer)?;

        u16::try_from(value)
            .ok()
            .and_then(|u16_value| Self::try_from(u16_value).ok())
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "line-length must be between 1 and {} (got {value})",
                    Self::MAX,
                ))
            })
    }
}

impl CacheKey for LineLength {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_u16(self.0.get());
    }
}

/// Error type returned when parsing a [`LineLength`] from a string fails
pub enum ParseLineWidthError {
    /// The string could not be parsed as a valid [u16]
    ParseError(ParseIntError),
    /// The [u16] value of the string is not a valid [`LineLength`]
    TryFromIntError(LineLengthFromIntError),
}

impl std::fmt::Debug for ParseLineWidthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for ParseLineWidthError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseLineWidthError::ParseError(err) => std::fmt::Display::fmt(err, fmt),
            ParseLineWidthError::TryFromIntError(err) => std::fmt::Display::fmt(err, fmt),
        }
    }
}

impl Error for ParseLineWidthError {}

impl FromStr for LineLength {
    type Err = ParseLineWidthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let value = u16::from_str(s).map_err(ParseLineWidthError::ParseError)?;
        let value = Self::try_from(value).map_err(ParseLineWidthError::TryFromIntError)?;
        Ok(value)
    }
}

/// Error type returned when converting a u16 to a [`LineLength`] fails
#[derive(Clone, Copy, Debug)]
pub struct LineLengthFromIntError(pub u16);

impl TryFrom<u16> for LineLength {
    type Error = LineLengthFromIntError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match NonZeroU16::try_from(value) {
            Ok(value) => Ok(LineLength(value)),
            Err(_) => Err(LineLengthFromIntError(value)),
        }
    }
}

impl std::fmt::Display for LineLengthFromIntError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "The line width must be a value between 1 and {}.",
            LineLength::MAX
        )
    }
}

impl From<LineLength> for u16 {
    fn from(value: LineLength) -> Self {
        value.0.get()
    }
}

impl From<LineLength> for NonZeroU16 {
    fn from(value: LineLength) -> Self {
        value.0
    }
}

/// The size of a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, CacheKey)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct IndentWidth(NonZeroU8);

impl IndentWidth {}

impl Default for IndentWidth {
    fn default() -> Self {
        Self(NonZeroU8::new(4).unwrap())
    }
}

impl fmt::Display for IndentWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<NonZeroU8> for IndentWidth {
    fn from(tab_size: NonZeroU8) -> Self {
        Self(tab_size)
    }
}

impl From<IndentWidth> for NonZeroU8 {
    fn from(value: IndentWidth) -> Self {
        value.0
    }
}
