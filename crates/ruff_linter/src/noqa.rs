use std::error::Error;
use std::fmt::Display;
use std::ops::Add;
use std::path::Path;

use anyhow::Result;
use log::warn;

use ruff_db::diagnostic::LintName;
use ruff_python_trivia::{CommentRanges, Cursor, indentation_at_offset};
use ruff_source_file::LineRanges;
use ruff_text_size::{Ranged, TextLen, TextRange, TextSize};

use crate::Locator;
use crate::fs::relativize_path;
use crate::registry::Rule;

/// A directive to ignore a set of rules either for a given line of Python source code or an entire file (e.g.,
/// `# noqa: F401, F841` or `#ruff: noqa: F401, F841`).
#[derive(Debug)]
pub(crate) enum Directive<'a> {
    /// The `noqa` directive ignores all rules (e.g., `# noqa`).
    All(All),
    /// The `noqa` directive ignores specific rules (e.g., `# noqa: F401, F841`).
    Codes(Codes<'a>),
}

impl Ranged for Directive<'_> {
    fn range(&self) -> TextRange {
        match self {
            Directive::All(all) => all.range(),
            Directive::Codes(codes) => codes.range(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct All {
    range: TextRange,
}

impl Ranged for All {
    /// The range of the `noqa` directive.
    fn range(&self) -> TextRange {
        self.range
    }
}

/// An individual rule code in a `noqa` directive (e.g., `F401`).
#[derive(Debug)]
pub(crate) struct Code<'a> {
    code: &'a str,
    range: TextRange,
}

impl<'a> Code<'a> {
    /// The code that is ignored by the `noqa` directive.
    pub(crate) fn as_str(&self) -> &'a str {
        self.code
    }
}

impl Display for Code<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.write_str(self.code)
    }
}

impl Ranged for Code<'_> {
    /// The range of the rule code.
    fn range(&self) -> TextRange {
        self.range
    }
}

#[derive(Debug)]
pub(crate) struct Codes<'a> {
    range: TextRange,
    codes: Vec<Code<'a>>,
}

impl Codes<'_> {
    /// Returns an iterator over the [`Code`]s in the `noqa` directive.
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, Code<'_>> {
        self.codes.iter()
    }

    /// Returns `true` if the string list of `codes` includes `code` (or an alias
    /// thereof).
    pub(crate) fn includes<T: for<'a> PartialEq<&'a str>>(&self, needle: &T) -> bool {
        self.iter().any(|code| *needle == code.as_str())
    }
}

impl Ranged for Codes<'_> {
    /// The range of the `noqa` directive.
    fn range(&self) -> TextRange {
        self.range
    }
}

/// Returns `true` if the given [`Rule`] is ignored at the specified `lineno`.
pub(crate) fn rule_is_ignored(
    code: Rule,
    offset: TextSize,
    noqa_line_for: &NoqaMapping,
    comment_ranges: &CommentRanges,
    locator: &Locator,
) -> bool {
    let offset = noqa_line_for.resolve(offset);
    let line_range = locator.line_range(offset);
    let &[comment_range] = comment_ranges.comments_in_range(line_range) else {
        return false;
    };
    match lex_inline_noqa(comment_range, locator.contents()) {
        Ok(Some(NoqaLexerOutput {
            directive: Directive::All(_),
            ..
        })) => true,
        Ok(Some(NoqaLexerOutput {
            directive: Directive::Codes(codes),
            ..
        })) => code.noqa_code().is_some_and(|code| codes.includes(&code)),
        _ => false,
    }
}

/// A summary of the file-level exemption as extracted from [`FileNoqaDirectives`].
#[derive(Debug)]
pub(crate) enum FileExemption {
    /// The file is exempt from all rules.
    All,
    /// The file is exempt from the given rules.
    Codes(Vec<Rule>),
}

impl FileExemption {
    /// Returns `true` if the file is exempt from the rule with the given name.
    pub(crate) fn includes_name(&self, needle: LintName) -> bool {
        match self {
            FileExemption::All => true,
            FileExemption::Codes(rules) => rules.iter().any(|rule| rule.name() == needle),
        }
    }
}

impl<'a> From<&'a FileNoqaDirectives<'a>> for FileExemption {
    fn from(directives: &'a FileNoqaDirectives) -> Self {
        if directives
            .lines()
            .iter()
            .any(|line| matches!(line.parsed_file_exemption, Directive::All(_)))
        {
            FileExemption::All
        } else {
            FileExemption::Codes(
                directives
                    .lines()
                    .iter()
                    .flat_map(|line| &line.matches)
                    .copied()
                    .collect(),
            )
        }
    }
}

/// The directive for a file-level exemption (e.g., `# ruff: noqa`) from an individual line.
#[derive(Debug)]
pub(crate) struct FileNoqaDirectiveLine<'a> {
    /// The range of the text line for which the noqa directive applies.
    pub(crate) range: TextRange,
    /// The blanket noqa directive.
    pub(crate) parsed_file_exemption: Directive<'a>,
    /// The codes that are ignored by the parsed exemptions.
    pub(crate) matches: Vec<Rule>,
}

impl Ranged for FileNoqaDirectiveLine<'_> {
    /// The range of the `noqa` directive.
    fn range(&self) -> TextRange {
        self.range
    }
}

/// All file-level exemptions (e.g., `# ruff: noqa`) from a given Python file.
#[derive(Debug)]
pub(crate) struct FileNoqaDirectives<'a>(Vec<FileNoqaDirectiveLine<'a>>);

impl<'a> FileNoqaDirectives<'a> {
    /// Extract the [`FileNoqaDirectives`] for a given Python source file, enumerating any rules
    /// that are globally ignored within the file.
    pub(crate) fn extract(
        locator: &Locator<'a>,
        comment_ranges: &CommentRanges,
        external: &[String],
        path: &Path,
    ) -> Self {
        let mut lines = vec![];

        for range in comment_ranges {
            let lexed = lex_file_exemption(range, locator.contents());
            match lexed {
                Ok(Some(NoqaLexerOutput {
                    warnings,
                    directive,
                })) => {
                    let no_indentation_at_offset =
                        indentation_at_offset(range.start(), locator.contents()).is_none();
                    if !warnings.is_empty() || no_indentation_at_offset {
                        #[expect(deprecated)]
                        let line = locator.compute_line_index(range.start());
                        let path_display = relativize_path(path);

                        for warning in warnings {
                            warn!(
                                "Missing or joined rule code(s) at {path_display}:{line}: \
                                {warning}"
                            );
                        }

                        if no_indentation_at_offset {
                            warn!(
                                "Unexpected `# ruff: noqa` directive at {path_display}:{line}. \
                                File-level suppression comments must appear on their own line. \
                                For line-level suppression, omit the `ruff:` prefix."
                            );
                            continue;
                        }
                    }

                    let matches = match &directive {
                        Directive::All(_) => {
                            vec![]
                        }
                        Directive::Codes(codes) => {
                            codes
                                .iter()
                                .filter_map(|code| {
                                    let code = code.as_str();
                                    // Ignore externally-defined rules.
                                    if external.iter().any(|external| code.starts_with(external)) {
                                        return None;
                                    }

                                    Rule::from_code(code).ok()
                                })
                                .collect()
                        }
                    };

                    lines.push(FileNoqaDirectiveLine {
                        range,
                        parsed_file_exemption: directive,
                        matches,
                    });
                }
                Err(err) => {
                    #[expect(deprecated)]
                    let line = locator.compute_line_index(range.start());
                    let path_display = relativize_path(path);
                    warn!("Invalid `# ruff: noqa` directive at {path_display}:{line}: {err}");
                }
                Ok(None) => {}
            }
        }
        Self(lines)
    }

    pub(crate) fn lines(&self) -> &[FileNoqaDirectiveLine<'_>] {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Output of lexing a `noqa` directive.
#[derive(Debug)]
pub(crate) struct NoqaLexerOutput<'a> {
    warnings: Vec<LexicalWarning>,
    directive: Directive<'a>,
}

/// Lexes in-line `noqa` comment, e.g. `# noqa: F401`
fn lex_inline_noqa(
    comment_range: TextRange,
    source: &str,
) -> Result<Option<NoqaLexerOutput<'_>>, LexicalError> {
    let lexer = NoqaLexer::in_range(comment_range, source);
    lexer.lex_inline_noqa()
}

/// Lexes file-level exemption comment, e.g. `# ruff: noqa: F401`
fn lex_file_exemption(
    comment_range: TextRange,
    source: &str,
) -> Result<Option<NoqaLexerOutput<'_>>, LexicalError> {
    let lexer = NoqaLexer::in_range(comment_range, source);
    lexer.lex_file_exemption()
}

/// Lexer for `noqa` comment lines.
///
/// Assumed to be initialized with range or offset starting
/// at the `#` at the beginning of a `noqa` comment.
#[derive(Debug)]
struct NoqaLexer<'a> {
    /// Length between offset and end of comment range
    line_length: TextSize,
    /// Contains convenience methods for lexing
    cursor: Cursor<'a>,
    /// Byte offset of the start of putative `noqa` comment
    ///
    /// Note: This is updated in the course of lexing in the case
    /// where there are multiple comments in a comment range.
    /// Ex) `# comment # noqa: F401`
    /// start^         ^-- changed to here during lexing
    offset: TextSize,
    /// Non-fatal warnings collected during lexing
    warnings: Vec<LexicalWarning>,
    /// Tracks whether we are lexing in a context with a missing delimiter
    /// e.g. at `C` in `F401C402`.
    missing_delimiter: bool,
    /// Rule codes collected during lexing
    codes: Vec<Code<'a>>,
}

impl<'a> NoqaLexer<'a> {
    /// Initialize [`NoqaLexer`] in the given range of source text.
    fn in_range(range: TextRange, source: &'a str) -> Self {
        Self {
            line_length: range.len(),
            offset: range.start(),
            cursor: Cursor::new(&source[range]),
            warnings: Vec::new(),
            missing_delimiter: false,
            codes: Vec::new(),
        }
    }

    fn lex_inline_noqa(mut self) -> Result<Option<NoqaLexerOutput<'a>>, LexicalError> {
        while !self.cursor.is_eof() {
            // Skip over any leading content.
            self.cursor.eat_while(|c| c != '#');

            // This updates the offset and line length in the case of
            // multiple comments in a range, e.g.
            // `# First # Second # noqa`
            self.offset += self.cursor.token_len();
            self.line_length -= self.cursor.token_len();
            self.cursor.start_token();

            if !self.cursor.eat_char('#') {
                continue;
            }

            self.eat_whitespace();

            if !is_noqa_uncased(self.cursor.as_str()) {
                continue;
            }

            self.cursor.skip_bytes("noqa".len());

            return Ok(Some(self.lex_directive()?));
        }

        Ok(None)
    }

    fn lex_file_exemption(mut self) -> Result<Option<NoqaLexerOutput<'a>>, LexicalError> {
        while !self.cursor.is_eof() {
            // Skip over any leading content
            self.cursor.eat_while(|c| c != '#');

            // This updates the offset and line length in the case of
            // multiple comments in a range, e.g.
            // # First # Second # noqa
            self.offset += self.cursor.token_len();
            self.line_length -= self.cursor.token_len();
            self.cursor.start_token();

            // The remaining logic implements a simple regex

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //  ^
            if !self.cursor.eat_char('#') {
                continue;
            }

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //   ^^^
            self.eat_whitespace();

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //       ^^^^^^^^^^^^^
            if self.cursor.as_str().starts_with("ruff") {
                self.cursor.skip_bytes("ruff".len());
            } else if self.cursor.as_str().starts_with("flake8") {
                self.cursor.skip_bytes("flake8".len());
            } else {
                continue;
            }

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //                     ^^^
            self.eat_whitespace();

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //                        ^
            if !self.cursor.eat_char(':') {
                continue;
            }

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //                         ^^^
            self.eat_whitespace();

            // `#\s*(?:ruff|flake8)\s*:\s*(?i)noqa`
            //                             ^^^^^^^
            if !is_noqa_uncased(self.cursor.as_str()) {
                continue;
            }
            self.cursor.skip_bytes("noqa".len());

            return Ok(Some(self.lex_directive()?));
        }
        Ok(None)
    }

    /// Collect codes in `noqa` comment.
    fn lex_directive(mut self) -> Result<NoqaLexerOutput<'a>, LexicalError> {
        let range = TextRange::at(self.offset, self.position());

        match self.cursor.bump() {
            // End of comment
            // Ex) # noqa# A comment
            None | Some('#') => {
                return Ok(NoqaLexerOutput {
                    warnings: self.warnings,
                    directive: Directive::All(All { range }),
                });
            }
            // Ex) # noqa: F401,F842
            Some(':') => self.lex_codes()?,
            // Ex) # noqa A comment
            // Ex) # noqa   : F401
            Some(c) if c.is_whitespace() => {
                self.eat_whitespace();
                match self.cursor.bump() {
                    Some(':') => self.lex_codes()?,
                    _ => {
                        return Ok(NoqaLexerOutput {
                            warnings: self.warnings,
                            directive: Directive::All(All { range }),
                        });
                    }
                }
            }
            // Ex) #noqaA comment
            // Ex) #noqaF401
            _ => {
                return Err(LexicalError::InvalidSuffix);
            }
        }

        let Some(last) = self.codes.last() else {
            return Err(LexicalError::MissingCodes);
        };

        Ok(NoqaLexerOutput {
            warnings: self.warnings,
            directive: Directive::Codes(Codes {
                range: TextRange::new(self.offset, last.range.end()),
                codes: self.codes,
            }),
        })
    }

    fn lex_codes(&mut self) -> Result<(), LexicalError> {
        // SAFETY: Every call to `lex_code` advances the cursor at least once.
        while !self.cursor.is_eof() {
            self.lex_code()?;
        }
        Ok(())
    }

    fn lex_code(&mut self) -> Result<(), LexicalError> {
        self.cursor.start_token();
        self.eat_whitespace();

        // Ex) # noqa: F401,    ,F841
        //                  ^^^^
        if self.cursor.first() == ',' {
            self.warnings.push(LexicalWarning::MissingItem(
                self.token_range().add(self.offset),
            ));
            self.cursor.eat_char(',');
            return Ok(());
        }

        let before_code = self.cursor.as_str();
        // Reset start of token so it does not include whitespace
        self.cursor.start_token();
        match self.cursor.bump() {
            // Ex) # noqa: F401
            //             ^
            Some(c) if c.is_ascii_uppercase() => {
                self.cursor.eat_while(|chr| chr.is_ascii_uppercase());
                if !self.cursor.eat_if(|c| c.is_ascii_digit()) {
                    // Fail hard if we're already attempting
                    // to lex squashed codes, e.g. `F401Fabc`
                    if self.missing_delimiter {
                        return Err(LexicalError::InvalidCodeSuffix);
                    }
                    // Otherwise we've reached the first invalid code,
                    // so it could be a comment and we just stop parsing.
                    // Ex) #noqa: F401 A comment
                    //       we're here^
                    self.cursor.skip_bytes(self.cursor.as_str().len());
                    return Ok(());
                }
                self.cursor.eat_while(|chr| chr.is_ascii_digit());

                let code = &before_code[..self.cursor.token_len().to_usize()];

                self.push_code(code);

                if self.cursor.is_eof() {
                    return Ok(());
                }

                self.missing_delimiter = match self.cursor.first() {
                    ',' => {
                        self.cursor.eat_char(',');
                        false
                    }

                    // Whitespace is an allowed delimiter or the end of the `noqa`.
                    c if c.is_whitespace() => false,

                    // e.g. #noqa:F401F842
                    //                ^
                    // Push a warning and update the `missing_delimiter`
                    // state but don't consume the character since it's
                    // part of the next code.
                    c if c.is_ascii_uppercase() => {
                        self.warnings
                            .push(LexicalWarning::MissingDelimiter(TextRange::empty(
                                self.offset + self.position(),
                            )));
                        true
                    }
                    // Start of a new comment
                    // e.g. #noqa: F401#A comment
                    //                 ^
                    '#' => false,
                    _ => return Err(LexicalError::InvalidCodeSuffix),
                };
            }
            Some(_) => {
                // The first time we hit an evidently invalid code,
                // it's probably a trailing comment. So stop lexing,
                // but don't push an error.
                // Ex)
                // # noqa: F401 we import this for a reason
                //              ^----- stop lexing but no error
                self.cursor.skip_bytes(self.cursor.as_str().len());
            }
            None => {}
        }
        Ok(())
    }

    /// Push current token to stack of [`Code`] objects
    fn push_code(&mut self, code: &'a str) {
        self.codes.push(Code {
            code,
            range: self.token_range().add(self.offset),
        });
    }

    /// Consume whitespace
    #[inline]
    fn eat_whitespace(&mut self) {
        self.cursor.eat_while(char::is_whitespace);
    }

    /// Current token range relative to offset
    #[inline]
    fn token_range(&self) -> TextRange {
        let end = self.position();
        let len = self.cursor.token_len();

        TextRange::at(end - len, len)
    }

    /// Retrieves the current position of the cursor within the line.
    #[inline]
    fn position(&self) -> TextSize {
        self.line_length - self.cursor.text_len()
    }
}

/// Helper to check if "noqa" (case-insensitive) appears at the given position
#[inline]
fn is_noqa_uncased(text: &str) -> bool {
    matches!(
        text.as_bytes(),
        [b'n' | b'N', b'o' | b'O', b'q' | b'Q', b'a' | b'A', ..]
    )
}

/// Indicates recoverable error encountered while lexing with [`NoqaLexer`]
#[derive(Debug, Clone, Copy)]
enum LexicalWarning {
    MissingItem(TextRange),
    MissingDelimiter(TextRange),
}

impl Display for LexicalWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingItem(_) => f.write_str("expected rule code between commas"),
            Self::MissingDelimiter(_) => {
                f.write_str("expected comma or space delimiter between codes")
            }
        }
    }
}

impl Ranged for LexicalWarning {
    fn range(&self) -> TextRange {
        match *self {
            LexicalWarning::MissingItem(text_range) => text_range,
            LexicalWarning::MissingDelimiter(text_range) => text_range,
        }
    }
}

/// Fatal error occurring while lexing a `noqa` comment as in [`NoqaLexer`]
#[derive(Debug)]
pub(crate) enum LexicalError {
    /// The `noqa` directive was missing valid codes (e.g., `# noqa: unused-import` instead of `# noqa: F401`).
    MissingCodes,
    /// The `noqa` directive used an invalid suffix (e.g., `# noqa; F401` instead of `# noqa: F401`).
    InvalidSuffix,
    /// The `noqa` code matched the regex "[A-Z]+[0-9]+" with text remaining
    /// (e.g. `# noqa: F401abc`)
    InvalidCodeSuffix,
}

impl Display for LexicalError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LexicalError::MissingCodes => fmt.write_str(
                "expected a comma-separated list of codes \
                (e.g., `# noqa: F401, F841`).",
            ),
            LexicalError::InvalidSuffix => fmt.write_str(
                "expected `:` followed by a comma-separated list of codes \
                (e.g., `# noqa: F401, F841`).",
            ),
            LexicalError::InvalidCodeSuffix => fmt.write_str(
                "expected code to consist of uppercase letters followed by digits only \
                (e.g. `F401`)",
            ),
        }
    }
}

impl Error for LexicalError {}

#[derive(Debug)]
pub(crate) struct NoqaDirectiveLine<'a> {
    /// The range of the text line for which the noqa directive applies.
    pub(crate) range: TextRange,
    /// The noqa directive.
    pub(crate) directive: Directive<'a>,
    /// Whether the directive applies to `range.end`.
    pub(crate) includes_end: bool,
}

impl Ranged for NoqaDirectiveLine<'_> {
    /// The range of the `noqa` directive.
    fn range(&self) -> TextRange {
        self.range
    }
}

#[derive(Debug, Default)]
pub(crate) struct NoqaDirectives<'a> {
    inner: Vec<NoqaDirectiveLine<'a>>,
}

impl<'a> NoqaDirectives<'a> {
    pub(crate) fn from_commented_ranges(
        comment_ranges: &CommentRanges,
        path: &Path,
        locator: &'a Locator<'a>,
    ) -> Self {
        let mut directives = Vec::new();

        for range in comment_ranges {
            let lexed = lex_inline_noqa(range, locator.contents());

            match lexed {
                Ok(Some(NoqaLexerOutput {
                    warnings,
                    directive,
                })) => {
                    if !warnings.is_empty() {
                        #[expect(deprecated)]
                        let line = locator.compute_line_index(range.start());
                        let path_display = relativize_path(path);
                        for warning in warnings {
                            warn!(
                                "Missing or joined rule code(s) \
                                at {path_display}:{line}: {warning}"
                            );
                        }
                    }
                    // noqa comments are guaranteed to be single line.
                    let range = locator.line_range(range.start());
                    directives.push(NoqaDirectiveLine {
                        range,
                        directive,
                        includes_end: range.end() == locator.contents().text_len(),
                    });
                }
                Err(err) => {
                    #[expect(deprecated)]
                    let line = locator.compute_line_index(range.start());
                    let path_display = relativize_path(path);
                    warn!("Invalid `# noqa` directive on {path_display}:{line}: {err}");
                }
                Ok(None) => {}
            }
        }

        Self { inner: directives }
    }

    pub(crate) fn find_line_with_directive(
        &self,
        offset: TextSize,
    ) -> Option<&NoqaDirectiveLine<'_>> {
        self.find_line_index(offset).map(|index| &self.inner[index])
    }

    fn find_line_index(&self, offset: TextSize) -> Option<usize> {
        self.inner
            .binary_search_by(|directive| {
                if directive.range.end() < offset {
                    std::cmp::Ordering::Less
                } else if directive.range.start() > offset {
                    std::cmp::Ordering::Greater
                }
                // At this point, end >= offset, start <= offset
                else if !directive.includes_end && directive.range.end() == offset {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .ok()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Remaps offsets falling into one of the ranges to instead check for a noqa comment on the
/// line specified by the offset.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct NoqaMapping {
    ranges: Vec<TextRange>,
}

impl NoqaMapping {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            ranges: Vec::with_capacity(capacity),
        }
    }

    /// Returns the re-mapped position or `position` if no mapping exists.
    pub(crate) fn resolve(&self, offset: TextSize) -> TextSize {
        let index = self.ranges.binary_search_by(|range| {
            if range.end() < offset {
                std::cmp::Ordering::Less
            } else if range.contains(offset) {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Greater
            }
        });

        if let Ok(index) = index {
            self.ranges[index].end()
        } else {
            offset
        }
    }

    pub(crate) fn push_mapping(&mut self, range: TextRange) {
        if let Some(last_range) = self.ranges.last_mut() {
            // Strictly sorted insertion
            if last_range.end() < range.start() {
                // OK
            } else if range.end() < last_range.start() {
                // Incoming range is strictly before the last range which violates
                // the function's contract.
                panic!("Ranges must be inserted in sorted order")
            } else {
                // Here, it's guaranteed that `last_range` and `range` overlap
                // in some way. We want to merge them into a single range.
                *last_range = last_range.cover(range);
                return;
            }
        }

        self.ranges.push(range);
    }
}

impl FromIterator<TextRange> for NoqaMapping {
    fn from_iter<T: IntoIterator<Item = TextRange>>(iter: T) -> Self {
        let mut mappings = NoqaMapping::default();

        for range in iter {
            mappings.push_mapping(range);
        }

        mappings
    }
}

#[cfg(test)]
mod tests {

    use anyhow::Result;
    use insta::assert_debug_snapshot;

    use ruff_text_size::{TextLen, TextRange};

    use crate::noqa::{
        Directive, LexicalError, NoqaLexerOutput, lex_file_exemption, lex_inline_noqa,
    };

    fn assert_lexed_ranges_match_slices(
        directive: Result<Option<NoqaLexerOutput>, LexicalError>,
        source: &str,
    ) {
        if let Ok(Some(NoqaLexerOutput {
            warnings: _,
            directive: Directive::Codes(codes),
        })) = directive
        {
            for code in codes.iter() {
                assert_eq!(&source[code.range], code.code);
            }
        }
    }

    #[test]
    fn noqa_all() {
        let source = "# noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..6,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_no_code() {
        let source = "# noqa:";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_no_code_invalid_suffix() {
        let source = "# noqa: foo";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_no_code_trailing_content() {
        let source = "# noqa:  # Foo";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn malformed_code_1() {
        let source = "# noqa: F";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn malformed_code_2() {
        let source = "# noqa: RUF001, F";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..14,
                            codes: [
                                Code {
                                    code: "RUF001",
                                    range: 8..14,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn malformed_code_3() {
        let source = "# noqa: RUF001F";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            InvalidCodeSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code() {
        let source = "# noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..12,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes() {
        let source = "# noqa: F401, F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_all_case_insensitive() {
        let source = "# NOQA";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..6,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_case_insensitive() {
        let source = "# NOQA: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..12,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes_case_insensitive() {
        let source = "# NOQA: F401, F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_leading_space() {
        let source = "#   # noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 4..16,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 12..16,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_trailing_space() {
        let source = "# noqa: F401   #";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..12,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_all_no_space() {
        let source = "#noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..5,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_no_space() {
        let source = "#noqa:F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..10,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 6..10,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes_no_space() {
        let source = "#noqa:F401,F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..15,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 6..10,
                                },
                                Code {
                                    code: "F841",
                                    range: 11..15,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_all_multi_space() {
        let source = "#  noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..7,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_multi_space() {
        let source = "#  noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..13,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 9..13,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes_multi_space() {
        let source = "#  noqa: F401,  F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..20,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 9..13,
                                },
                                Code {
                                    code: "F841",
                                    range: 16..20,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_leading_hashes() {
        let source = "###noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 2..13,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 9..13,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_leading_hashes_with_spaces() {
        let source = "#  #  # noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 6..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_all_leading_comment() {
        let source = "# Some comment describing the noqa # noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 35..41,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_leading_comment() {
        let source = "# Some comment describing the noqa # noqa: F401";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 35..47,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 43..47,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes_leading_comment() {
        let source = "# Some comment describing the noqa # noqa: F401, F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 35..53,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 43..47,
                                },
                                Code {
                                    code: "F841",
                                    range: 49..53,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_all_trailing_comment() {
        let source = "# noqa # Some comment describing the noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..6,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_trailing_comment() {
        let source = "# noqa: F401 # Some comment describing the noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..12,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_codes_trailing_comment() {
        let source = "# noqa: F401, F841 # Some comment describing the noqa";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_invalid_codes() {
        let source = "# noqa: unused-import, F401, some other code";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_squashed_codes() {
        let source = "# noqa: F401F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingDelimiter(
                            12..12,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..16,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 12..16,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_empty_comma() {
        let source = "# noqa: F401,,F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingItem(
                            13..13,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_empty_comma_space() {
        let source = "# noqa: F401, ,F841";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingItem(
                            13..14,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..19,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                                Code {
                                    code: "F841",
                                    range: 15..19,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_non_code() {
        let source = "# noqa: F401 We're ignoring an import";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..12,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 8..12,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_code_invalid_code_suffix() {
        let source = "# noqa: F401abc";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            InvalidCodeSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn noqa_invalid_suffix() {
        let source = "# noqa[F401]";
        let directive = lex_inline_noqa(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            InvalidSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn flake8_exemption_all() {
        let source = "# flake8: noqa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..14,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_noqa_no_code() {
        let source = "# flake8: noqa:";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_noqa_no_code_invalid_suffix() {
        let source = "# flake8: noqa: foo";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_noqa_no_code_trailing_content() {
        let source = "# flake8: noqa:  # Foo";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_malformed_code_1() {
        let source = "# flake8: noqa: F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn flake8_malformed_code_2() {
        let source = "# flake8: noqa: RUF001, F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..22,
                            codes: [
                                Code {
                                    code: "RUF001",
                                    range: 16..22,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn flake8_malformed_code_3() {
        let source = "# flake8: noqa: RUF001F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            InvalidCodeSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn ruff_exemption_all() {
        let source = "# ruff: noqa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_noqa_no_code() {
        let source = "# ruff: noqa:";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_noqa_no_code_invalid_suffix() {
        let source = "# ruff: noqa: foo";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_noqa_no_code_trailing_content() {
        let source = "# ruff: noqa:  # Foo";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_malformed_code_1() {
        let source = "# ruff: noqa: F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            MissingCodes,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn ruff_malformed_code_2() {
        let source = "# ruff: noqa: RUF001, F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..20,
                            codes: [
                                Code {
                                    code: "RUF001",
                                    range: 14..20,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn ruff_malformed_code_3() {
        let source = "# ruff: noqa: RUF001F";
        let directive = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(directive, @"
        Err(
            InvalidCodeSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(directive, source);
    }

    #[test]
    fn flake8_exemption_all_no_space() {
        let source = "#flake8:noqa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_no_space() {
        let source = "#ruff:noqa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..10,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_exemption_codes() {
        // Note: Flake8 doesn't support this; it's treated as a blanket exemption.
        let source = "# flake8: noqa: F401, F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..26,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 16..20,
                                },
                                Code {
                                    code: "F841",
                                    range: 22..26,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_codes() {
        let source = "# ruff: noqa: F401, F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..24,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                                Code {
                                    code: "F841",
                                    range: 20..24,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }
    #[test]
    fn ruff_exemption_codes_leading_hashes() {
        let source = "#### ruff: noqa: F401, F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 3..27,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 17..21,
                                },
                                Code {
                                    code: "F841",
                                    range: 23..27,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_squashed_codes() {
        let source = "# ruff: noqa: F401F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingDelimiter(
                            18..18,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..22,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                                Code {
                                    code: "F841",
                                    range: 18..22,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_empty_comma() {
        let source = "# ruff: noqa: F401,,F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingItem(
                            19..19,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..24,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                                Code {
                                    code: "F841",
                                    range: 20..24,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_empty_comma_space() {
        let source = "# ruff: noqa: F401, ,F841";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [
                        MissingItem(
                            19..20,
                        ),
                    ],
                    directive: Codes(
                        Codes {
                            range: 0..25,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                                Code {
                                    code: "F841",
                                    range: 21..25,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_invalid_code_suffix() {
        let source = "# ruff: noqa: F401abc";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Err(
            InvalidCodeSuffix,
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_code_leading_comment() {
        let source = "# Leading comment # ruff: noqa: F401";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 18..36,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 32..36,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_code_trailing_comment() {
        let source = "# ruff: noqa: F401 # Trailing comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_leading_comment() {
        let source = "# Leading comment # ruff: noqa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 18..30,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_trailing_comment() {
        let source = "# ruff: noqa # Trailing comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_code_trailing_comment_no_space() {
        let source = "# ruff: noqa: F401# And another comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_trailing_comment_no_space() {
        let source = "# ruff: noqa# Trailing comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_trailing_comment_no_hash() {
        let source = "# ruff: noqa Trailing comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_code_trailing_comment_no_hash() {
        let source = "# ruff: noqa: F401 Trailing comment";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @r#"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: Codes(
                        Codes {
                            range: 0..18,
                            codes: [
                                Code {
                                    code: "F401",
                                    range: 14..18,
                                },
                            ],
                        },
                    ),
                },
            ),
        )
        "#);
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn flake8_exemption_all_case_insensitive() {
        let source = "# flake8: NoQa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..14,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }

    #[test]
    fn ruff_exemption_all_case_insensitive() {
        let source = "# ruff: NoQa";
        let exemption = lex_file_exemption(TextRange::up_to(source.text_len()), source);
        assert_debug_snapshot!(exemption, @"
        Ok(
            Some(
                NoqaLexerOutput {
                    warnings: [],
                    directive: All(
                        All {
                            range: 0..12,
                        },
                    ),
                },
            ),
        )
        ");
        assert_lexed_ranges_match_slices(exemption, source);
    }
}
