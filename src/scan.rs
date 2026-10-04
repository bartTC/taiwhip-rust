//! The built-in class list finders: `class="..."` attributes and CSS
//! `@apply` rules, found without a regex engine.
//!
//! Compiling the equivalent regexes took 0.3 ms on every start, half of what
//! the tool itself spends on the one-line call an editor makes on save, and
//! scanning with them was two to three times slower. Each finder matches
//! exactly what its regex does, including Unicode whitespace for `\s` and
//! the `ſ` (U+017F) that `(?i)` accepts for `s`. A test compares them on
//! random input.

/// The span of the classes in every `class="..."` attribute, or
/// `class='...'` when `quote` is `'`: what the regex
/// `(?i)(?-u:\b)class\s*=\s*"(?P<classes>[^"]*)"` finds.
pub fn attribute_spans(text: &str, quote: u8) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(offset) = memchr::memchr2(b'c', b'C', &bytes[at..]) {
        let start = at + offset;
        match attribute_at(text, start, quote) {
            Some(span) => {
                spans.push(span);
                // Past the closing quote
                at = span.1 + 1;
            }
            None => at = start + 1,
        }
    }
    spans
}

/// The span of the classes of an attribute starting at `start`, if one
/// does. At most one match can start at any position, so trying positions
/// from left to right finds the matches the regex finds.
fn attribute_at(text: &str, start: usize, quote: u8) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if start > 0 && is_ascii_word(bytes[start - 1]) {
        return None;
    }
    let equals = skip_whitespace(text, class_at(bytes, start)?);
    if bytes.get(equals) != Some(&b'=') {
        return None;
    }
    let open = skip_whitespace(text, equals + 1);
    if bytes.get(open) != Some(&quote) {
        return None;
    }
    let len = memchr::memchr(quote, &bytes[open + 1..])?;
    Some((open + 1, open + 1 + len))
}

/// The end of the word "class" at `start`, in any letter case, and with
/// `ſ` for either `s`.
fn class_at(bytes: &[u8], start: usize) -> Option<usize> {
    let mut at = start;
    for expected in *b"cla" {
        if !bytes.get(at)?.eq_ignore_ascii_case(&expected) {
            return None;
        }
        at += 1;
    }
    for _ in 0..2 {
        match bytes.get(at..)? {
            [b's' | b'S', ..] => at += 1,
            // ſ in UTF-8
            [0xC5, 0xBF, ..] => at += 2,
            _ => return None,
        }
    }
    Some(at)
}

/// The span of the classes in every `@apply ...;` rule: what the regex
/// `(?i)@apply\s+(?P<classes>[^;]+);` finds.
pub fn apply_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    // A rule needs a semicolon after it, which bounds the search
    let Some(last_semicolon) = memchr::memrchr(b';', bytes) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    let mut at = 0;
    while at < last_semicolon {
        let Some(offset) = memchr::memchr(b'@', &bytes[at..last_semicolon]) else {
            break;
        };
        let start = at + offset;
        match apply_at(text, start) {
            Some(span) => {
                spans.push(span);
                // Past the semicolon
                at = span.1 + 1;
            }
            None => at = start + 1,
        }
    }
    spans
}

/// The span of the classes of an `@apply` rule starting at `start`, if one
/// does.
fn apply_at(text: &str, start: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    if !bytes
        .get(start + 1..start + 6)?
        .eq_ignore_ascii_case(b"apply")
    {
        return None;
    }
    let space = start + 6;
    let classes = skip_whitespace(text, space);
    if classes == space {
        return None;
    }
    match bytes.get(classes) {
        // The classes run up to the next semicolon
        Some(&byte) if byte != b';' => {
            let len = memchr::memchr(b';', &bytes[classes..])?;
            Some((classes, classes + len))
        }
        // Only whitespace before the semicolon: `\s+` gives up its last
        // character to the classes, if it has more than one
        Some(_) => {
            let (last, _) = text[space..classes].char_indices().next_back()?;
            (last > 0).then_some((space + last, classes))
        }
        None => None,
    }
}

/// Whether a byte is a word character to an ASCII word boundary.
fn is_ascii_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// The position after any whitespace at `at`, as `\s` matches it: Unicode
/// whitespace, which is what `char::is_whitespace` tests. `at` must be at
/// a character boundary.
fn skip_whitespace(text: &str, mut at: usize) -> usize {
    let bytes = text.as_bytes();
    while let Some(&byte) = bytes.get(at) {
        if byte.is_ascii() {
            // Unlike `u8::is_ascii_whitespace`, this includes vertical tab
            if !char::from(byte).is_whitespace() {
                break;
            }
            at += 1;
        } else {
            match text[at..].chars().next() {
                Some(ch) if ch.is_whitespace() => at += ch.len_utf8(),
                _ => break,
            }
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex_automata::meta::Regex;
    use std::sync::LazyLock;

    /// The regexes the finders replace, as the reference: double quotes,
    /// single quotes, `@apply`.
    static REGEXES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
        [
            r#"(?i)(?-u:\b)class\s*=\s*"(?P<classes>[^"]*)""#,
            r#"(?i)(?-u:\b)class\s*=\s*'(?P<classes>[^']*)'"#,
            r"(?i)@apply\s+(?P<classes>[^;]+);",
        ]
        .map(|regex| Regex::new(regex).unwrap())
    });

    /// The spans of the `classes` group found by a regex.
    fn regex_spans(regex: &Regex, text: &str) -> Vec<(usize, usize)> {
        regex
            .captures_iter(text)
            .filter_map(|captures| captures.get_group_by_name("classes"))
            .map(|span| (span.start, span.end))
            .collect()
    }

    fn check(text: &str) {
        let [double, single, apply] = &*REGEXES;
        assert_eq!(
            attribute_spans(text, b'"'),
            regex_spans(double, text),
            "{text:?}"
        );
        assert_eq!(
            attribute_spans(text, b'\''),
            regex_spans(single, text),
            "{text:?}"
        );
        assert_eq!(apply_spans(text), regex_spans(apply, text), "{text:?}");
    }

    #[test]
    fn finders_match_their_regexes() {
        for text in [
            "",
            r#"<div class="p-4 m-2">"#,
            r#"<div CLASS = 'p-4'><p Class="">"#,
            "<p class=\"x\ny\" data-class=\"a\" xclass=\"b\" _class=\"c\" éclass=\"d\">",
            "<p claſs=\"a\" claſſ='b' clasſ = \"c\" clas=\"d\" class=\"unclosed",
            "<p class\u{a0}=\u{3000}\"a\" class\u{85}=\u{b}'b'>",
            "class=\"a\"class=\"b\"class='c'class='d'",
            ".a { @apply p-4 m-2; } .b { @APPLY\n  flex ; } @apply;",
            "@apply  ; @apply \u{a0}; @apply ; @applyx; @apply x",
            "@apply @apply p-4; @media x { @apply m-2 }",
        ] {
            check(text);
        }
    }

    #[test]
    fn finders_match_their_regexes_on_random_input() {
        const PIECES: &[&str] = &[
            "class", "CLASS", "Class", "claſs", "claſſ", "clas", "x", "_", "é", "-", "=", " ",
            "\t", "\n", "\u{b}", "\u{a0}", "\u{3000}", "\u{85}", "\"", "'", ";", "@apply",
            "@APPLY", "@", "apply", "p-4", "{{ y }}", "ſ",
        ];
        // xorshift, so that failures are reproducible
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = next() % 16;
            let text: String = (0..len)
                .map(|_| PIECES[(next() % PIECES.len() as u64) as usize])
                .collect();
            check(&text);
        }
    }
}
