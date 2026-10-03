//! Finding class lists in text and rewriting them in sorted order.
//!
//! Each configured pattern is applied to the whole text in turn. The text
//! matched by its `classes` group is replaced with the same classes sorted;
//! everything around it stays as written. A class list is left untouched
//! when it contains a template expression (`{{`, `{%`, ...) or no classes.

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;
use regex::Regex;
use rustc_hash::FxHashMap;

use crate::config::{Config, PatternSpec};
use crate::sorting::Sorter;

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

/// A compiled class pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    pub name: String,
    regex: Regex,
    /// Index of the `classes` group.
    classes_group: usize,
}

impl Pattern {
    pub fn compile(spec: &PatternSpec) -> Result<Pattern, PatternError> {
        let error = |message: String| PatternError {
            name: spec.name.clone(),
            message,
        };
        let regex = Regex::new(&spec.regex).map_err(|e| error(e.to_string()))?;
        let classes_group = regex
            .capture_names()
            .position(|name| name == Some("classes"))
            .ok_or_else(|| error("the regex needs a (?P<classes>...) group".to_string()))?;
        Ok(Pattern {
            name: spec.name.clone(),
            regex,
            classes_group,
        })
    }
}

/// Sorted class lists repeat all over a codebase, so results are cached per
/// thread. Entries: class list as written -> sorted list, or `None` when it
/// is to be left alone.
type Cache = FxHashMap<String, Option<String>>;

/// Cached entries per thread before the cache is emptied.
const CACHE_CAPACITY: usize = 8192;

/// Inputs with at least this many class lists are sorted on all cores.
/// Below it, the thread pool would cost more than it saves.
const PARALLEL_THRESHOLD: usize = 256;

thread_local! {
    /// The cache of the processor last used on this thread, keyed by its id,
    /// so that several processors with different configurations never share
    /// results.
    static CACHE: RefCell<(u64, Cache)> = RefCell::new((0, Cache::default()));
}

static NEXT_PROCESSOR_ID: AtomicU64 = AtomicU64::new(1);

/// Sorts the class lists found by the configured patterns.
#[derive(Debug)]
pub struct Processor {
    id: u64,
    sorter: Sorter,
    skip_expressions: Vec<String>,
    patterns: Vec<Pattern>,
}

impl Processor {
    pub fn new(config: &Config) -> Result<Processor, PatternError> {
        Ok(Processor {
            id: NEXT_PROCESSOR_ID.fetch_add(1, Ordering::Relaxed),
            sorter: Sorter::new(config),
            skip_expressions: config.skip_expressions.clone(),
            patterns: config
                .class_patterns
                .iter()
                .map(Pattern::compile)
                .collect::<Result<_, _>>()?,
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
        if self
            .skip_expressions
            .iter()
            .any(|expression| classes.contains(expression.as_str()))
        {
            return None;
        }
        let classes: Vec<&str> = classes.split_whitespace().collect();
        if classes.is_empty() {
            return None;
        }
        Some(self.sorter.sort_classes(&classes).join(" "))
    }

    /// `sort_class_string` with the per-thread cache.
    fn sort_class_string_cached(&self, classes: &str) -> Option<String> {
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.0 != self.id {
                cache.0 = self.id;
                cache.1.clear();
            }
            if let Some(hit) = cache.1.get(classes) {
                return hit.clone();
            }
            let sorted = self.sort_class_string(classes);
            if cache.1.len() >= CACHE_CAPACITY {
                cache.1.clear();
            }
            cache.1.insert(classes.to_string(), sorted.clone());
            sorted
        })
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
        for pattern in &self.patterns {
            if let Some(replaced) = self.apply_pattern(pattern, &current) {
                current = Cow::Owned(replaced);
            }
        }
        current
    }

    /// The text with the pattern's class lists sorted, or `None` if that
    /// changes nothing.
    fn apply_pattern(&self, pattern: &Pattern, text: &str) -> Option<String> {
        // Find every class list first, then sort them, in parallel for
        // large inputs, then splice the results back in
        let spans: Vec<(usize, usize)> = pattern
            .regex
            .captures_iter(text)
            .filter_map(|captures| {
                captures
                    .get(pattern.classes_group)
                    .map(|m| (m.start(), m.end()))
            })
            .collect();
        if spans.is_empty() {
            return None;
        }

        let sort =
            |&(start, end): &(usize, usize)| self.sort_class_string_cached(&text[start..end]);
        let sorted: Vec<Option<String>> = if spans.len() >= PARALLEL_THRESHOLD {
            spans.par_iter().map(sort).collect()
        } else {
            spans.iter().map(sort).collect()
        };

        let mut out = String::new();
        let mut last = 0;
        let mut changed = false;
        for (&(start, end), sorted) in spans.iter().zip(&sorted) {
            let Some(sorted) = sorted else {
                continue;
            };
            if sorted == &text[start..end] {
                continue;
            }
            if !changed {
                out.reserve(text.len());
                changed = true;
            }
            out.push_str(&text[last..start]);
            out.push_str(sorted);
            last = end;
        }

        if !changed {
            return None;
        }
        out.push_str(&text[last..]);
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
