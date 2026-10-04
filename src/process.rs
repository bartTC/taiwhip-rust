//! Finding class lists in text and rewriting them in sorted order.
//!
//! The built-in patterns (`class="..."`, `class='...'`, `@apply ...;`) and
//! then each configured one are applied to the whole text in turn. The class
//! list a pattern finds is replaced with the same classes sorted; everything
//! around it stays as written. A class list is left untouched when it
//! contains a template expression (`{{`, `{%`, ...) or no classes.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use regex_automata::Input;
use regex_automata::meta::{self, Regex};
use regex_automata::nfa::thompson::WhichCaptures;
use regex_syntax::hir::{Class, Hir, HirKind};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::config::{Config, PatternSpec};
use crate::scan;
use crate::sorting::{KeyCache, Sorter};

/// Why the configured class patterns could not be compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternError {
    pub name: String,
    pub message: String,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Invalid class pattern '{}': {}", self.name, self.message)
    }
}

impl std::error::Error for PatternError {}

/// How the span of the `classes` group is recovered from a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// The span follows from the match alone: the group ends `suffix_len`
    /// bytes before the end of the match and starts after the last
    /// `delimiter` byte before that. See [`delimited_group`].
    Delimited { delimiter: u8, suffix_len: usize },
    /// The regex engine reports the group with this index.
    Captured(usize),
}

/// How a pattern finds class lists.
#[derive(Debug, Clone)]
enum Finder {
    /// Built in: HTML attributes with this quote character.
    Attribute(u8),
    /// Built in: CSS `@apply` rules.
    Apply,
    /// A configured regex.
    Regex { regex: Regex, group: Group },
}

/// A pattern that finds class lists: one of the built-in ones, or a
/// configured regex.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub name: String,
    finder: Finder,
}

impl Pattern {
    /// The built-in patterns, which always apply, before any configured
    /// ones: `class="..."` and `class='...'` attributes, with any letter
    /// case and whitespace around the `=`, and `@apply ...;` rules. See
    /// [`scan`].
    pub fn builtin() -> Vec<Pattern> {
        let pattern = |name: &str, finder| Pattern {
            name: name.to_string(),
            finder,
        };
        vec![
            pattern("html_class", Finder::Attribute(b'"')),
            pattern("html_class_single_quoted", Finder::Attribute(b'\'')),
            pattern("css_apply", Finder::Apply),
        ]
    }

    /// Compile a configured pattern.
    pub fn compile(spec: &PatternSpec) -> Result<Pattern, PatternError> {
        let error = |message: String| PatternError {
            name: spec.name.clone(),
            message,
        };
        let hir = regex_syntax::Parser::new()
            .parse(&spec.regex)
            .map_err(|e| error(e.to_string()))?;
        let index = classes_group_index(&hir)
            .ok_or_else(|| error("the regex needs a (?P<classes>...) group".to_string()))?;

        // Neither engine is worth its build time here: the full DFA is only
        // built for tiny patterns, and the one-pass DFA only speeds up
        // capture groups, which a delimited group does not need
        let config = meta::Config::new().dfa(false);
        let (group, config) = match delimited_group(&hir) {
            Some((delimiter, suffix_len)) => (
                Group::Delimited {
                    delimiter,
                    suffix_len,
                },
                config
                    .which_captures(WhichCaptures::Implicit)
                    .onepass(false),
            ),
            None => (Group::Captured(index), config),
        };
        let regex = meta::Builder::new()
            .configure(config)
            .build_from_hir(&hir)
            .map_err(|e| error(e.to_string()))?;
        Ok(Pattern {
            name: spec.name.clone(),
            finder: Finder::Regex { regex, group },
        })
    }

    /// The byte span `(start, end)` of every class list found, in order.
    pub fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        self.spans_with(&mut None, text)
    }

    /// `spans` with a regex's scratch space kept by the caller. The regex
    /// would otherwise take it from a pool shared by all threads, on every
    /// match.
    fn spans_with(&self, cache: &mut Option<meta::Cache>, text: &str) -> Vec<(usize, usize)> {
        let (regex, group) = match &self.finder {
            Finder::Attribute(quote) => return scan::attribute_spans(text, *quote),
            Finder::Apply => return scan::apply_spans(text),
            Finder::Regex { regex, group } => (regex, *group),
        };
        let cache = cache.get_or_insert_with(|| regex.create_cache());
        match group {
            Group::Delimited {
                delimiter,
                suffix_len,
            } => {
                // Only the end of each match is searched for. Finding its
                // start would scan the match a second time, backwards, and
                // is not needed: the last delimiter before the group's end
                // opens the group, wherever the match started.
                let mut spans = Vec::new();
                let mut input = Input::new(text);
                while let Some(found) = regex.search_half_with(cache, &input) {
                    let end = found.offset() - suffix_len;
                    let opening = memchr::memrchr(delimiter, &text.as_bytes()[input.start()..end])
                        .expect("a delimited group follows its delimiter");
                    spans.push((input.start() + opening + 1, end));
                    // Matches are never empty, they contain the delimiter
                    input.set_start(found.offset());
                }
                spans
            }
            Group::Captured(index) => regex
                .captures_iter(text)
                .filter_map(|captures| captures.get_group(index))
                .map(|span| (span.start, span.end))
                .collect(),
        }
    }
}

/// The index of the group named `classes`, if the regex has one.
fn classes_group_index(hir: &Hir) -> Option<usize> {
    match hir.kind() {
        HirKind::Capture(capture) if capture.name.as_deref() == Some("classes") => {
            Some(capture.index as usize)
        }
        HirKind::Capture(capture) => classes_group_index(&capture.sub),
        HirKind::Repetition(repetition) => classes_group_index(&repetition.sub),
        HirKind::Concat(parts) | HirKind::Alternation(parts) => {
            parts.iter().find_map(classes_group_index)
        }
        HirKind::Empty | HirKind::Literal(_) | HirKind::Class(_) | HirKind::Look(_) => None,
    }
}

/// Recognizes a `classes` group enclosed in delimiters, as in
/// `class="(?P<classes>[^"]*)"`, and returns the opening delimiter and the
/// length of the text after the group.
///
/// The group must be part of the top-level sequence, directly after a
/// literal ending in an ASCII byte the group can never match, and followed
/// only by text of fixed length. Every match then has exactly one place for
/// the group: it ends that fixed length before the end of the match, and it
/// starts after the last delimiter before that, since the group itself
/// contains none. Neither a capture engine nor the start of the match is
/// needed, which makes scanning more than twice as fast.
fn delimited_group(hir: &Hir) -> Option<(u8, usize)> {
    let HirKind::Concat(parts) = hir.kind() else {
        return None;
    };
    let at = parts.iter().position(|part| {
        matches!(part.kind(), HirKind::Capture(capture) if capture.name.as_deref() == Some("classes"))
    })?;
    let HirKind::Capture(group) = parts[at].kind() else {
        return None;
    };
    let HirKind::Literal(before) = parts[..at].last()?.kind() else {
        return None;
    };
    let delimiter = *before.0.last()?;
    if !delimiter.is_ascii() || may_match_byte(&group.sub, delimiter) {
        return None;
    }
    let mut suffix_len = 0;
    for part in &parts[at + 1..] {
        let properties = part.properties();
        let len = properties.minimum_len()?;
        if properties.maximum_len() != Some(len) {
            return None;
        }
        suffix_len += len;
    }
    Some((delimiter, suffix_len))
}

/// Whether text matched by `hir` can contain the ASCII byte `byte`. Bytes of
/// multi-byte characters are never ASCII, so a character class only needs
/// checking for the character itself.
fn may_match_byte(hir: &Hir, byte: u8) -> bool {
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => false,
        HirKind::Literal(literal) => literal.0.contains(&byte),
        HirKind::Class(Class::Unicode(class)) => {
            let ch = char::from(byte);
            class
                .ranges()
                .iter()
                .any(|range| range.start() <= ch && ch <= range.end())
        }
        HirKind::Class(Class::Bytes(class)) => class
            .ranges()
            .iter()
            .any(|range| range.start() <= byte && byte <= range.end()),
        HirKind::Repetition(repetition) => may_match_byte(&repetition.sub, byte),
        HirKind::Capture(capture) => may_match_byte(&capture.sub, byte),
        HirKind::Concat(parts) | HirKind::Alternation(parts) => {
            parts.iter().any(|part| may_match_byte(part, byte))
        }
    }
}

/// What each thread keeps from one text to the next for the processor it
/// last ran, keyed by the processor's id so that processors with different
/// configurations never share anything.
#[derive(Debug, Default)]
struct Scratch {
    owner: u64,
    /// Sorted class lists, which repeat all over a codebase: the list as
    /// written -> the sorted list, or `None` when it is to be left alone.
    sorted: FxHashMap<Box<str>, Option<Box<str>>>,
    /// The sort keys of classes, which repeat even more.
    keys: KeyCache,
    /// The search space of each configured regex, which also holds the
    /// states its lazy DFA has built so far.
    searches: Vec<Option<meta::Cache>>,
}

/// Cached sorted lists per thread before the cache is emptied.
const CACHE_CAPACITY: usize = 8192;

/// Inputs with at least this many class lists are sorted on all cores.
/// Below it, the thread pool would cost more than it saves.
const PARALLEL_THRESHOLD: usize = 256;

/// Class lists per task when sorting on all cores.
const CHUNK_SIZE: usize = 64;

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

static NEXT_PROCESSOR_ID: AtomicU64 = AtomicU64::new(1);

/// Sorts the class lists found by the configured patterns.
#[derive(Debug)]
pub struct Processor {
    id: u64,
    sorter: Sorter,
    skip_expressions: Vec<Cow<'static, str>>,
    patterns: Vec<Pattern>,
}

impl Processor {
    pub fn new(config: &Config) -> Result<Processor, PatternError> {
        let mut patterns = Pattern::builtin();
        // Compiling the patterns on several threads was tried: each compile
        // then took two to four times as long, which ate up the gain
        for spec in &config.class_patterns {
            patterns.push(Pattern::compile(spec)?);
        }
        Ok(Processor {
            id: NEXT_PROCESSOR_ID.fetch_add(1, Ordering::Relaxed),
            sorter: Sorter::new(config),
            skip_expressions: config.skip_expressions.clone(),
            patterns,
        })
    }

    pub fn sorter(&self) -> &Sorter {
        &self.sorter
    }

    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    /// Sort a class list, or `None` to leave it as is: when it contains a
    /// template expression or no classes at all.
    pub fn sort_class_string(&self, classes: &str) -> Option<String> {
        if self.skips(classes) {
            return None;
        }
        let classes: Vec<&str> = classes.split_whitespace().collect();
        if classes.is_empty() {
            return None;
        }
        Some(self.sorter.sort_classes(&classes).join(" "))
    }

    /// [`sort_class_string`](Self::sort_class_string) with the sort keys of
    /// the classes taken from `keys`.
    fn sort_list(&self, classes: &str, keys: &mut KeyCache) -> Option<Box<str>> {
        if self.skips(classes) {
            return None;
        }
        let classes: SmallVec<[&str; 16]> = classes.split_whitespace().collect();
        if classes.is_empty() {
            return None;
        }
        Some(
            self.sorter
                .sort_classes_cached(&classes, keys)
                .join(" ")
                .into_boxed_str(),
        )
    }

    /// Whether a class list contains a template expression.
    fn skips(&self, classes: &str) -> bool {
        self.skip_expressions
            .iter()
            .any(|expression| classes.contains(&**expression))
    }

    /// Sort the classes in every match of every pattern. The text is
    /// returned unchanged, without copying, when nothing needs sorting.
    ///
    /// ```
    /// # use tailwhip::{config::Config, process::Processor};
    /// let processor = Processor::new(&Config::default()).unwrap();
    /// assert_eq!(
    ///     processor.process_text(r#"<div class="p-4 flex container"></div>"#),
    ///     r#"<div class="container flex p-4"></div>"#
    /// );
    /// assert_eq!(processor.process_text("@apply p-4 flex;"), "@apply flex p-4;");
    /// // Template expressions are left alone
    /// let html = r#"<div class="p-4 {{ extra }}"></div>"#;
    /// assert_eq!(processor.process_text(html), html);
    /// ```
    pub fn process_text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut current = Cow::Borrowed(text);
        for index in 0..self.patterns.len() {
            if let Some(replaced) = self.apply_pattern(index, &current) {
                current = Cow::Owned(replaced);
            }
        }
        current
    }

    /// This thread's scratch space for this processor. `f` must not use
    /// the thread pool: a task stolen meanwhile could need the space too.
    fn with_scratch<R>(&self, f: impl FnOnce(&mut Scratch) -> R) -> R {
        SCRATCH.with(|scratch| {
            let mut scratch = scratch.borrow_mut();
            if scratch.owner != self.id {
                scratch.owner = self.id;
                scratch.sorted.clear();
                scratch.keys = KeyCache::default();
                scratch.searches.clear();
                scratch.searches.resize_with(self.patterns.len(), || None);
            }
            f(&mut scratch)
        })
    }

    /// The text with the class lists of the pattern at `index` sorted, or
    /// `None` if that changes nothing.
    fn apply_pattern(&self, index: usize, text: &str) -> Option<String> {
        let pattern = &self.patterns[index];
        let spans =
            self.with_scratch(|scratch| pattern.spans_with(&mut scratch.searches[index], text));
        if spans.len() < PARALLEL_THRESHOLD {
            return self.splice(text, 0..text.len(), &spans);
        }

        // Large inputs: each task rewrites the stretch of text from its
        // first class list up to the next task's, then the stretches are
        // joined in order
        let pieces: Vec<Option<String>> = spans
            .par_chunks(CHUNK_SIZE)
            .enumerate()
            .map(|(index, chunk)| {
                let start = if index == 0 { 0 } else { chunk[0].0 };
                let end = spans
                    .get((index + 1) * CHUNK_SIZE)
                    .map_or(text.len(), |next| next.0);
                self.splice(text, start..end, chunk)
            })
            .collect();
        if pieces.iter().all(Option::is_none) {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        for (index, piece) in pieces.iter().enumerate() {
            match piece {
                Some(piece) => out.push_str(piece),
                None => {
                    let start = if index == 0 {
                        0
                    } else {
                        spans[index * CHUNK_SIZE].0
                    };
                    let end = spans
                        .get((index + 1) * CHUNK_SIZE)
                        .map_or(text.len(), |next| next.0);
                    out.push_str(&text[start..end]);
                }
            }
        }
        Some(out)
    }

    /// `text[range]` with the class lists at `spans`, which lie inside the
    /// range, replaced by their sorted versions. `None` if none changes.
    fn splice(&self, text: &str, range: Range<usize>, spans: &[(usize, usize)]) -> Option<String> {
        let mut out = String::new();
        let mut last = range.start;
        let mut changed = false;
        let mut emit = |(start, end): (usize, usize), sorted: &str| {
            if sorted == &text[start..end] {
                return;
            }
            if !changed {
                out.reserve(range.len());
                changed = true;
            }
            out.push_str(&text[last..start]);
            out.push_str(sorted);
            last = end;
        };

        self.with_scratch(|scratch| {
            let Scratch {
                sorted: cache,
                keys,
                ..
            } = scratch;
            for &span in spans {
                let classes = &text[span.0..span.1];
                if let Some(hit) = cache.get(classes) {
                    if let Some(sorted) = hit {
                        emit(span, sorted);
                    }
                    continue;
                }
                let sorted = self.sort_list(classes, keys);
                if let Some(sorted) = &sorted {
                    emit(span, sorted);
                }
                if cache.len() >= CACHE_CAPACITY {
                    cache.clear();
                }
                cache.insert(classes.into(), sorted);
            }
        });

        if !changed {
            return None;
        }
        out.push_str(&text[last..range.end]);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(regex: &str) -> Result<Pattern, PatternError> {
        Pattern::compile(&PatternSpec {
            name: "x".into(),
            regex: regex.into(),
        })
    }

    #[test]
    fn patterns_need_a_classes_group_and_a_valid_regex() {
        assert!(
            pattern(r"class=(?P<value>.*)")
                .unwrap_err()
                .message
                .contains("classes")
        );
        assert!(pattern("(").is_err());
        // Both named group spellings work
        assert!(pattern(r"class=(?<classes>.*)").is_ok());
        assert!(pattern(r"class=(?P<classes>.*)").is_ok());
    }

    #[test]
    fn delimited_groups_are_recognized() {
        let group = |regex: &str| match pattern(regex).unwrap().finder {
            Finder::Regex { group, .. } => group,
            _ => unreachable!("a configured pattern is a regex"),
        };
        let delimited = |delimiter: u8, suffix_len: usize| Group::Delimited {
            delimiter,
            suffix_len,
        };
        // The regexes of the built-in HTML patterns
        assert_eq!(
            group(r#"(?i)(?-u:\b)class\s*=\s*"(?P<classes>[^"]*)""#),
            delimited(b'"', 1)
        );
        assert_eq!(
            group(r#"(?i)(?-u:\b)class\s*=\s*'(?P<classes>[^']*)'"#),
            delimited(b'\'', 1)
        );
        assert_eq!(
            group(r#"className=\{"(?P<classes>[^"]*)"\}"#),
            delimited(b'"', 2)
        );
        assert_eq!(group(r#"x="(?P<classes>[^"]*)"\b"#), delimited(b'"', 1));
        assert_eq!(group(r#"x="(?P<classes>[a-z ]*)$"#), delimited(b'"', 0));
        // The group could contain the delimiter
        assert_eq!(group(r#"x="(?P<classes>[^']*)""#), Group::Captured(1));
        assert_eq!(group(r#"x="(?P<classes>.*)""#), Group::Captured(1));
        // Not preceded by a literal
        assert_eq!(group(r"@apply\s+(?P<classes>[^;]+);"), Group::Captured(1));
        // Followed by text of varying length
        assert_eq!(group(r#"x="(?P<classes>[^"]*)"+"#), Group::Captured(1));
        // Not at the top level
        assert_eq!(group(r#"(?:x="(?P<classes>[^"]*)")"#), delimited(b'"', 1));
        assert_eq!(group(r#"a|x="(?P<classes>[^"]*)""#), Group::Captured(1));
        // Other groups before it
        assert_eq!(group(r#"(x)="(?P<classes>[^"]*)""#), delimited(b'"', 1));
    }

    #[test]
    fn delimited_and_captured_groups_find_the_same_spans() {
        let text = "<a class=\"b a\"><b CLASS = \"\"><i class=\"x\ny\">\
                    <p data-class=\"q p\"><p class='s r'><a href=\"x\" class=\"d c\">\
                    <a class='a'class='b'class=\"c\"class=\"d\" title='\"'>";
        for regex in [
            r#"(?i)(?-u:\b)class\s*=\s*"(?P<classes>[^"]*)""#,
            r#"(?i)(?-u:\b)class\s*=\s*'(?P<classes>[^']*)'"#,
        ] {
            let delimited = pattern(regex).unwrap();
            assert!(matches!(
                delimited.finder,
                Finder::Regex {
                    group: Group::Delimited { .. },
                    ..
                }
            ));
            let captured = Pattern {
                finder: Finder::Regex {
                    regex: Regex::new(regex).unwrap(),
                    group: Group::Captured(1),
                },
                ..delimited.clone()
            };
            assert_eq!(delimited.spans(text), captured.spans(text), "{regex}");
            assert!(!delimited.spans(text).is_empty());
        }
    }

    #[test]
    fn only_the_classes_group_is_rewritten() {
        let processor = Processor::new(&Config::default()).unwrap();
        // Spacing, quotes and case around the classes stay as written
        assert_eq!(
            processor.process_text(r#"<div CLASS = 'p-4 m-2'>"#),
            r#"<div CLASS = 'm-2 p-4'>"#
        );
        // Quotes of the other style inside the value are fine
        assert_eq!(
            processor.process_text(r#"<i class="p-4 before:content-['★'] m-2">"#),
            r#"<i class="m-2 p-4 before:content-['★']">"#
        );
        // Empty attributes and template expressions stay exactly as they are
        for unchanged in [
            r#"<div class="">"#,
            r#"<div class="  ">"#,
            r#"<div class="{% if x %}a{% endif %}">"#,
        ] {
            assert!(matches!(
                processor.process_text(unchanged),
                Cow::Borrowed(_)
            ));
        }
        // Already sorted text is not copied
        assert!(matches!(
            processor.process_text(r#"<div class="m-2 p-4">"#),
            Cow::Borrowed(_)
        ));
        // Classes spanning lines are joined
        assert_eq!(
            processor.process_text("<div class=\"p-4\n   m-2\">"),
            "<div class=\"m-2 p-4\">"
        );
    }

    #[test]
    fn large_inputs_sort_in_parallel_with_the_same_result() {
        let processor = Processor::new(&Config::default()).unwrap();
        let line = |i: usize| {
            if i.is_multiple_of(3) {
                format!("<p class=\"m-2 flex\">{i}</p>\n")
            } else {
                format!("<p class=\"flex m-{}\">{i}</p>\n", i % 7)
            }
        };
        let count = PARALLEL_THRESHOLD + 3 * CHUNK_SIZE + 5;
        let text: String = (0..count).map(line).collect();
        let expected: String = (0..count)
            .map(|i| processor.process_text(&line(i)).into_owned())
            .collect();
        assert_eq!(processor.process_text(&text), expected);

        // Nothing to change: the input comes back without a copy
        let sorted: String = (0..count)
            .map(|i| format!("<p class=\"flex m-2\">{i}</p>\n"))
            .collect();
        assert!(matches!(processor.process_text(&sorted), Cow::Borrowed(_)));
    }

    #[test]
    fn the_cache_is_per_processor() {
        let default = Processor::new(&Config::default()).unwrap();
        assert_eq!(
            default.process_text(r#"<i class="text-red-500 text-brand">"#),
            r#"<i class="text-red-500 text-brand">"#
        );

        let config = Config {
            custom_colors: vec!["brand".into()],
            ..Config::default()
        };
        let custom = Processor::new(&config).unwrap();
        assert_eq!(
            custom.process_text(r#"<i class="text-red-500 text-brand">"#),
            r#"<i class="text-brand text-red-500">"#
        );

        // Back to the first processor: its results must not come from the other's cache
        assert_eq!(
            default.process_text(r#"<i class="text-red-500 text-brand">"#),
            r#"<i class="text-red-500 text-brand">"#
        );
    }
}
