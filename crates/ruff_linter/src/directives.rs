//! Extract `# noqa` directives from tokenized source.

use std::iter::Peekable;

use ruff_python_ast::token::{TokenKind, Tokens};
use ruff_python_index::Indexer;
use ruff_source_file::LineRanges;
use ruff_text_size::{Ranged, TextRange};

use crate::Locator;
use crate::comments::shebang::leading_shebang_range;
use crate::noqa::NoqaMapping;

pub struct Directives {
    pub noqa_line_for: NoqaMapping,
}

pub fn extract_directives(tokens: &Tokens, locator: &Locator, indexer: &Indexer) -> Directives {
    Directives {
        noqa_line_for: extract_noqa_line_for(tokens, locator, indexer),
    }
}

struct SortedMergeIter<L, R, Item>
where
    L: Iterator<Item = Item>,
    R: Iterator<Item = Item>,
{
    left: Peekable<L>,
    right: Peekable<R>,
}

impl<L, R, Item> Iterator for SortedMergeIter<L, R, Item>
where
    L: Iterator<Item = Item>,
    R: Iterator<Item = Item>,
    Item: Ranged,
{
    type Item = Item;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.left.peek(), self.right.peek()) {
            (Some(left), Some(right)) => {
                if left.start() <= right.start() {
                    Some(self.left.next().unwrap())
                } else {
                    Some(self.right.next().unwrap())
                }
            }
            (Some(_), None) => Some(self.left.next().unwrap()),
            (None, Some(_)) => Some(self.right.next().unwrap()),
            (None, None) => None,
        }
    }
}

/// Extract a mapping from logical line to noqa line.
fn extract_noqa_line_for(tokens: &Tokens, locator: &Locator, indexer: &Indexer) -> NoqaMapping {
    let mut string_mappings = Vec::new();

    for token in tokens {
        match token.kind() {
            // For multi-line strings, we expect `noqa` directives on the last line of the string.
            TokenKind::String if token.is_triple_quoted_string() => {
                if locator.contains_line_break(token.range()) {
                    string_mappings.push(TextRange::new(
                        locator.line_start(token.start()),
                        token.end(),
                    ));
                }
            }

            _ => {}
        }
    }

    let shebang_mapping = leading_shebang_range(locator.contents(), tokens);

    // The capacity allocated here might be more than we need if there are
    // nested interpolated strings.
    let mut interpolated_string_mappings =
        Vec::with_capacity(indexer.interpolated_string_ranges().len());

    // For nested interpolated strings, we expect `noqa` directives on the last line of the
    // outermost interpolated string. The last interpolated string range will be used to skip over
    // the inner interpolated strings.
    let mut last_interpolated_string_range: TextRange = TextRange::default();
    for interpolated_string_range in indexer.interpolated_string_ranges().values() {
        if !locator.contains_line_break(*interpolated_string_range) {
            continue;
        }
        if last_interpolated_string_range.contains_range(*interpolated_string_range) {
            continue;
        }
        let new_range = TextRange::new(
            locator.line_start(interpolated_string_range.start()),
            interpolated_string_range.end(),
        );
        interpolated_string_mappings.push(new_range);
        last_interpolated_string_range = new_range;
    }

    let mut continuation_mappings = Vec::new();

    // For continuations, we expect `noqa` directives on the last line of the
    // continuation.
    let mut last: Option<TextRange> = None;
    for continuation_line in indexer.continuation_line_starts() {
        let line_end = locator.full_line_end(*continuation_line);
        if let Some(last_range) = last.take() {
            if last_range.end() == *continuation_line {
                last = Some(TextRange::new(last_range.start(), line_end));
                continue;
            }
            // new continuation
            continuation_mappings.push(last_range);
        }

        last = Some(TextRange::new(*continuation_line, line_end));
    }

    if let Some(last_range) = last.take() {
        continuation_mappings.push(last_range);
    }

    // Merge the mappings in sorted order
    let mut mappings = NoqaMapping::with_capacity(
        continuation_mappings.len()
            + string_mappings.len()
            + interpolated_string_mappings.len()
            + usize::from(shebang_mapping.is_some()),
    );

    let all_mappings = SortedMergeIter {
        left: interpolated_string_mappings.into_iter().peekable(),
        right: string_mappings.into_iter().peekable(),
    };
    let all_mappings = SortedMergeIter {
        left: all_mappings.peekable(),
        right: continuation_mappings.into_iter().peekable(),
    };

    if let Some(mapping) = shebang_mapping {
        mappings.push_mapping(mapping);
    }
    for mapping in all_mappings {
        mappings.push_mapping(mapping);
    }

    mappings
}

#[cfg(test)]
mod tests {
    use ruff_python_index::Indexer;
    use ruff_python_parser::parse_module;

    use ruff_text_size::{TextRange, TextSize};

    use crate::Locator;
    use crate::directives::extract_noqa_line_for;
    use crate::noqa::NoqaMapping;

    fn noqa_mappings(contents: &str) -> NoqaMapping {
        let parsed = parse_module(contents).unwrap();
        let locator = Locator::new(contents);
        let indexer = Indexer::from_tokens(parsed.tokens(), locator.contents());

        extract_noqa_line_for(parsed.tokens(), &locator, &indexer)
    }

    #[test]
    fn noqa_extraction() {
        let contents = "x = 1
y = 2 \
    + 1
z = x + 1";

        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "
x = 1
y = 2
z = x + 1";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "x = 1
y = 2
z = x + 1
        ";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "x = 1

y = 2
z = x + 1
        ";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "x = '''abc
def
ghi
'''
y = 2
z = x + 1";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(22))])
        );

        let contents = "x = 1
y = '''abc
def
ghi
'''
z = 2";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(28))])
        );

        let contents = "x = 1
y = '''abc
def
ghi
'''";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(28))])
        );

        let contents = "x = f'abc {
a
    *
        b
}'
y = 2
";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(32))])
        );

        let contents = "x = f'''abc
def
ghi
'''
y = 2
z = x + 1";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(23))])
        );

        let contents = "x = 1
y = f'''abc
def
ghi
'''
z = 2";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(29))])
        );

        let contents = "x = 1
y = f'''abc
def
ghi
'''";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(29))])
        );

        let contents = "x = 1
y = f'''abc
def {f'''nested
fstring''' f'another nested'}
end'''
";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(70))])
        );

        let contents = "x = 1
y = t'''abc
def {f'''nested
interpolated string''' f'another nested'}
end'''
";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(6), TextSize::from(82))])
        );

        let contents = "x = 1
y = f'normal'
z = f'another but {f'nested but {f'still single line'} nested'}'
";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "x = 1
y = t'normal'
z = t'another but {t'nested but {t'still single line'} nested'}'
";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = "x = 1
y = f'normal'
z = f'another but {t'nested but {f'still single line'} nested'}'
";
        assert_eq!(noqa_mappings(contents), NoqaMapping::default());

        let contents = r"x = \
    1";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(6))])
        );

        let contents = r"from foo import \
    bar as baz, \
    qux as quux";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(36))])
        );

        let contents = r"
# Foo
from foo import \
    bar as baz, \
    qux as quux # Baz
x = \
    1
y = \
    2";
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([
                TextRange::new(TextSize::from(7), TextSize::from(43)),
                TextRange::new(TextSize::from(65), TextSize::from(71)),
                TextRange::new(TextSize::from(77), TextSize::from(83)),
            ])
        );

        // https://github.com/astral-sh/ruff/issues/7530
        let contents = r"
assert foo, \
    '''triple-quoted
    string'''
"
        .trim();
        assert_eq!(
            noqa_mappings(contents),
            NoqaMapping::from_iter([TextRange::new(TextSize::from(0), TextSize::from(48))])
        );
    }
}
