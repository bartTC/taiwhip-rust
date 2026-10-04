//! Parsing Tailwind CSS class names into components and sorting them.
//!
//! The order is entirely driven by the lists in the configuration: a class is
//! split into variants, a prefix and up to six components (direction, size,
//! value, color, shade, alpha), and each part is ranked by its position in
//! the corresponding list. See `configuration.toml` for the full description.

use std::borrow::Cow;
use std::collections::BTreeSet;

use rustc_hash::{FxHashMap, FxHashSet};
use smallvec::SmallVec;

use crate::config::Config;

/// Rank for values that appear in no list; they sort last.
pub const MAX_RANK: i64 = 999_999;

/// A parsed Tailwind CSS class with all its components.
///
/// Every part borrows from the class name where possible; `prefix` and
/// `suffix` only allocate when they are stitched together from
/// non-adjacent tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedClass<'a> {
    pub original: &'a str,
    pub important: bool,
    pub negated: bool,
    pub variants: Vec<&'a str>,
    pub prefix: Cow<'a, str>,
    pub direction: Option<&'a str>,
    pub size: Option<&'a str>,
    pub value: Option<&'a str>,
    pub color: Option<&'a str>,
    pub shade: Option<&'a str>,
    pub alpha: Option<&'a str>,
    /// Any remaining unparsed parts, joined with "-".
    pub suffix: Cow<'a, str>,
}

impl ParsedClass<'_> {
    /// A base utility has no modifiers after the prefix, e.g. `border`
    /// as opposed to `border-t` or `border-2`.
    pub fn is_base_utility(&self) -> bool {
        self.direction.is_none()
            && self.size.is_none()
            && self.value.is_none()
            && self.color.is_none()
            && self.shade.is_none()
            && self.alpha.is_none()
            && self.suffix.is_empty()
    }
}

/// The sort key of a class. Fields are compared top to bottom:
///
/// 1. variants, each as (rank, name)
/// 2. prefix as (rank, name); unknown prefixes rank -1 so non-Tailwind
///    classes sort first
/// 3. base utility flag, so `border` sorts before `border-t`
/// 4. the components in the configured `component_order`
/// 5. suffix, so arbitrary values sort last within a group
/// 6. the class name itself as the final tiebreaker
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey<'a> {
    pub variants: SmallVec<[(i64, &'a str); 4]>,
    pub prefix: (i64, Cow<'a, str>),
    pub base: u8,
    pub components: SmallVec<[(i64, &'a str); 6]>,
    pub suffix: (u8, Cow<'a, str>),
    pub original: &'a str,
}

impl SortKey<'_> {
    /// The key as bytes that compare like the key itself: for any two keys,
    /// `a.cmp(&b) == a.to_bytes().cmp(&b.to_bytes())`. Kept from one class
    /// list to the next, they let a list be sorted by plain byte comparisons
    /// without parsing its classes again.
    ///
    /// Every field is written in comparison order: ranks as big-endian
    /// numbers, offset by one so that -1 comes first, and strings with a
    /// terminator that sorts before any byte they contain. The class name
    /// comes last and needs no terminator.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + 2 * self.original.len());
        for (rank, name) in &self.variants {
            // Marks one more variant, so that a shorter list sorts first
            out.push(1);
            push_rank(&mut out, *rank);
            push_terminated(&mut out, name);
        }
        out.push(0);
        push_rank(&mut out, self.prefix.0);
        push_terminated(&mut out, &self.prefix.1);
        out.push(self.base);
        // Always one entry per configured component
        for (rank, value) in &self.components {
            push_rank(&mut out, *rank);
            push_terminated(&mut out, value);
        }
        out.push(self.suffix.0);
        push_terminated(&mut out, &self.suffix.1);
        out.extend_from_slice(self.original.as_bytes());
        out
    }
}

/// A rank from -1 up, as four big-endian bytes.
fn push_rank(out: &mut Vec<u8>, rank: i64) {
    let rank = u32::try_from(rank + 1).expect("ranks are list positions, -1 or MAX_RANK");
    out.extend_from_slice(&rank.to_be_bytes());
}

/// `text` followed by `00 00`, with every `00` byte in it written as
/// `00 ff`, so that a string sorts before every longer string it starts.
fn push_terminated(out: &mut Vec<u8>, text: &str) {
    for &byte in text.as_bytes() {
        out.push(byte);
        if byte == 0 {
            out.push(0xff);
        }
    }
    out.extend_from_slice(&[0, 0]);
}

/// The sort keys of classes seen before, as bytes (see
/// [`SortKey::to_bytes`]), so that each distinct class is parsed only once.
/// A cache must only be used with one sorter, since keys depend on its
/// configuration.
#[derive(Debug, Default)]
pub struct KeyCache {
    ids: FxHashMap<Box<str>, u32>,
    keys: Vec<Box<[u8]>>,
}

/// Classes in a key cache before it is emptied.
const KEY_CACHE_CAPACITY: usize = 32_768;

impl KeyCache {
    /// The id of a class's key, parsing the class if it is new.
    fn id(&mut self, sorter: &Sorter, class: &str) -> u32 {
        if let Some(&id) = self.ids.get(class) {
            return id;
        }
        let id = u32::try_from(self.keys.len()).expect("the cache is emptied long before");
        self.keys
            .push(sorter.sort_key(class).to_bytes().into_boxed_slice());
        self.ids.insert(class.into(), id);
        id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Direction,
    Size,
    Value,
    Color,
    Shade,
    Alpha,
}

/// A component to compare, with the rank used when a class has no such
/// component and the rank used when the value is not in its list.
///
/// - No direction sorts first (-1): `border-1` before `border-t-1`.
/// - No other component sorts last: `border-1` before `border-red`.
fn component_ranks(name: &str) -> Option<(Component, i64, i64)> {
    match name {
        "direction" => Some((Component::Direction, -1, MAX_RANK)),
        "size" => Some((Component::Size, MAX_RANK, MAX_RANK)),
        "value" => Some((Component::Value, MAX_RANK, MAX_RANK)),
        "color" => Some((Component::Color, MAX_RANK, MAX_RANK)),
        "shade" => Some((Component::Shade, MAX_RANK, MAX_RANK)),
        "alpha" => Some((Component::Alpha, MAX_RANK, MAX_RANK)),
        _ => None,
    }
}

/// The positions of one token in the component lists it appears in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TokenRanks {
    direction: Option<i64>,
    size: Option<i64>,
    value: Option<i64>,
    color: Option<i64>,
    shade: Option<i64>,
}

/// Map each value to its position. A value listed twice keeps its last
/// position, like a Python dict comprehension.
fn index_of(values: &[Cow<'static, str>]) -> FxHashMap<Cow<'static, str>, i64> {
    values
        .iter()
        .enumerate()
        .map(|(i, value)| (value.clone(), i as i64))
        .collect()
}

/// A parsed class with the rank of every component, as found while
/// parsing, so that building the sort key needs no further lookups.
struct Parsed<'a> {
    original: &'a str,
    important: bool,
    negated: bool,
    variants: SmallVec<[&'a str; 4]>,
    prefix: Cow<'a, str>,
    prefix_rank: Option<i64>,
    direction: Option<(&'a str, i64)>,
    size: Option<(&'a str, i64)>,
    value: Option<(&'a str, i64)>,
    color: Option<(&'a str, i64)>,
    shade: Option<(&'a str, i64)>,
    alpha: Option<(&'a str, Option<i64>)>,
    suffix: Cow<'a, str>,
}

/// Lookup tables derived from the configuration lists.
#[derive(Debug, Clone)]
pub struct Sorter {
    variant_separator: String,
    variant_index: FxHashMap<Cow<'static, str>, i64>,
    /// `(name, rank)` for variants that take an argument, such as
    /// `min-[320px]` or `aria-busy`, in configuration order.
    variant_prefixes: Vec<(Cow<'static, str>, i64)>,
    prefix_index: FxHashMap<Cow<'static, str>, i64>,
    alpha_index: FxHashMap<Cow<'static, str>, i64>,
    /// Which component lists a token appears in, and at which position.
    /// One table for all five lists, so parsing a token is a single lookup.
    tokens: FxHashMap<Cow<'static, str>, TokenRanks>,
    /// The components to compare, in configured order. Unknown names in
    /// `component_order` are ignored; "variant" and "prefix" always come
    /// first and are handled separately.
    component_ranks: Vec<(Component, i64, i64)>,
}

impl Sorter {
    pub fn new(config: &Config) -> Sorter {
        let variant_index = index_of(&config.variants);

        let mut seen = FxHashSet::default();
        let variant_prefixes = config
            .variants
            .iter()
            .filter(|name| seen.insert(&**name))
            .map(|name| (name.clone(), variant_index[&**name]))
            .collect();

        // Colors are merged with the custom colors and sorted alphabetically
        let colors: BTreeSet<&Cow<'static, str>> = config
            .colors
            .iter()
            .chain(config.custom_colors.iter())
            .collect();

        // Iterating in order, a token listed twice keeps its last position
        let lists = [
            &config.directions,
            &config.sizes,
            &config.numerics,
            &config.shades,
        ];
        let capacity = lists.iter().map(|list| list.len()).sum::<usize>() + colors.len();
        let mut tokens: FxHashMap<Cow<'static, str>, TokenRanks> =
            FxHashMap::with_capacity_and_hasher(capacity, Default::default());
        let mut set = |token: &Cow<'static, str>,
                       field: fn(&mut TokenRanks) -> &mut Option<i64>,
                       rank: usize| {
            *field(tokens.entry(token.clone()).or_default()) = Some(rank as i64);
        };
        for (rank, token) in config.directions.iter().enumerate() {
            set(token, |ranks| &mut ranks.direction, rank);
        }
        for (rank, token) in config.sizes.iter().enumerate() {
            set(token, |ranks| &mut ranks.size, rank);
        }
        for (rank, token) in config.numerics.iter().enumerate() {
            set(token, |ranks| &mut ranks.value, rank);
        }
        for (rank, color) in colors.into_iter().enumerate() {
            set(color, |ranks| &mut ranks.color, rank);
        }
        for (rank, token) in config.shades.iter().enumerate() {
            set(token, |ranks| &mut ranks.shade, rank);
        }

        Sorter {
            variant_separator: config.variant_separator.clone(),
            variant_index,
            variant_prefixes,
            prefix_index: index_of(&config.prefixes),
            alpha_index: index_of(&config.alphas),
            tokens,
            component_ranks: config
                .component_order
                .iter()
                .filter_map(|name| component_ranks(name))
                .collect(),
        }
    }

    /// Parse a class into its components.
    ///
    /// ```
    /// # use tailwhip::{config::Config, sorting::Sorter};
    /// let sorter = Sorter::new(&Config::default());
    /// let parsed = sorter.parse_class("!sm:hover:-border-t-2");
    /// assert!(parsed.important && parsed.negated);
    /// assert_eq!(parsed.variants, ["sm", "hover"]);
    /// assert_eq!(parsed.prefix, "border");
    /// assert_eq!(parsed.direction, Some("t"));
    /// assert_eq!(parsed.value, Some("2"));
    /// ```
    pub fn parse_class<'a>(&self, classname: &'a str) -> ParsedClass<'a> {
        let parsed = self.parse(classname);
        ParsedClass {
            original: parsed.original,
            important: parsed.important,
            negated: parsed.negated,
            variants: parsed.variants.to_vec(),
            prefix: parsed.prefix,
            direction: parsed.direction.map(|(value, _)| value),
            size: parsed.size.map(|(value, _)| value),
            value: parsed.value.map(|(value, _)| value),
            color: parsed.color.map(|(value, _)| value),
            shade: parsed.shade.map(|(value, _)| value),
            alpha: parsed.alpha.map(|(value, _)| value),
            suffix: parsed.suffix,
        }
    }

    fn parse<'a>(&self, classname: &'a str) -> Parsed<'a> {
        let mut remaining = classname;

        // 1. Important prefix (!)
        let important = remaining.starts_with('!');
        if important {
            remaining = &remaining[1..];
        }

        // 2. Variants and utility. Separators inside [...] or (...) belong to
        //    an arbitrary value, e.g. "supports-[display:grid]:grid".
        let mut variants: SmallVec<[&str; 4]> = SmallVec::new();
        let mut start = 0;
        for_each_top_level(remaining, &self.variant_separator, |at| {
            variants.push(&remaining[start..at]);
            start = at + self.variant_separator.len();
        });
        let mut utility = &remaining[start..];

        // 3. Negated prefix (-)
        let negated = utility.starts_with('-');
        if negated {
            utility = &utility[1..];
        }

        // 4. Alpha value, e.g. bg-red-500/50. A "/" inside brackets belongs to
        //    the arbitrary value, e.g. "bg-[url(/img/bg.png)]".
        let mut last_slash = None;
        for_each_top_level(utility, "/", |at| last_slash = Some(at));
        let alpha = last_slash.map(|at| {
            let alpha = &utility[at + 1..];
            utility = &utility[..at];
            (alpha, self.alpha_index.get(alpha).copied())
        });

        // 5. Tokens, skipping empty ones as in "border--x"
        let mut tokens: SmallVec<[&str; 8]> = SmallVec::new();
        let mut start = 0;
        for_each_top_level(utility, "-", |at| {
            if at > start {
                tokens.push(&utility[start..at]);
            }
            start = at + 1;
        });
        if start < utility.len() {
            tokens.push(&utility[start..]);
        }

        // 6. Prefix and components
        let (prefix, prefix_rank, consumed) = self.extract_prefix(utility, &tokens);

        // Each token fills the first component slot it is known for and that
        // is still free; the rest becomes the suffix
        let mut direction = None;
        let mut size = None;
        let mut value = None;
        let mut color = None;
        let mut shade = None;
        let mut suffix_parts: SmallVec<[&str; 4]> = SmallVec::new();
        for &token in &tokens[consumed..] {
            let ranks = self.tokens.get(token).copied().unwrap_or_default();
            if direction.is_none()
                && let Some(rank) = ranks.direction
            {
                direction = Some((token, rank));
            } else if size.is_none()
                && let Some(rank) = ranks.size
            {
                size = Some((token, rank));
            } else if value.is_none()
                && let Some(rank) = ranks.value
            {
                value = Some((token, rank));
            } else if color.is_none()
                && let Some(rank) = ranks.color
            {
                color = Some((token, rank));
            } else if shade.is_none()
                && let Some(rank) = ranks.shade
            {
                shade = Some((token, rank));
            } else {
                suffix_parts.push(token);
            }
        }
        let suffix = match suffix_parts.as_slice() {
            [] => Cow::Borrowed(""),
            [single] => Cow::Borrowed(*single),
            parts => Cow::Owned(parts.join("-")),
        };

        Parsed {
            original: classname,
            important,
            negated,
            variants,
            prefix,
            prefix_rank,
            direction,
            size,
            value,
            color,
            shade,
            alpha,
            suffix,
        }
    }

    /// The longest run of leading tokens that forms a known prefix, so that
    /// "inline-flex" is one prefix rather than "inline" plus a value "flex".
    /// Returns the prefix, its rank and the number of tokens it consumed.
    fn extract_prefix<'a>(
        &self,
        utility: &'a str,
        tokens: &[&'a str],
    ) -> (Cow<'a, str>, Option<i64>, usize) {
        let Some(&first) = tokens.first() else {
            return (Cow::Borrowed(""), self.prefix_index.get("").copied(), 0);
        };
        for count in (2..=tokens.len()).rev() {
            let candidate = join_tokens(utility, &tokens[..count]);
            if let Some(&rank) = self.prefix_index.get(candidate.as_ref()) {
                return (candidate, Some(rank), count);
            }
        }
        (
            Cow::Borrowed(first),
            self.prefix_index.get(first).copied(),
            1,
        )
    }

    /// The rank of a single variant. Variants taking an argument, such as
    /// `min-[320px]` or `data-loading`, rank with their base name.
    pub fn variant_rank(&self, variant: &str) -> i64 {
        if let Some(&rank) = self.variant_index.get(variant) {
            return rank;
        }
        for (name, rank) in &self.variant_prefixes {
            if variant
                .strip_prefix(&**name)
                .is_some_and(|argument| argument.starts_with(['-', '[']))
            {
                return *rank;
            }
        }
        MAX_RANK
    }

    /// The sort key of a class; see [`SortKey`] for the order.
    pub fn sort_key<'a>(&self, classname: &'a str) -> SortKey<'a> {
        let parsed = self.parse(classname);

        let variants = parsed
            .variants
            .iter()
            .map(|&variant| (self.variant_rank(variant), variant))
            .collect();

        let is_base = parsed.direction.is_none()
            && parsed.size.is_none()
            && parsed.value.is_none()
            && parsed.color.is_none()
            && parsed.shade.is_none()
            && parsed.alpha.is_none()
            && parsed.suffix.is_empty();

        let components = self
            .component_ranks
            .iter()
            .map(|&(component, none_rank, unknown_rank)| {
                let found = match component {
                    Component::Direction => parsed.direction,
                    Component::Size => parsed.size,
                    Component::Value => parsed.value,
                    Component::Color => parsed.color,
                    Component::Shade => parsed.shade,
                    Component::Alpha => parsed
                        .alpha
                        .map(|(value, rank)| (value, rank.unwrap_or(unknown_rank))),
                };
                match found {
                    None => (none_rank, ""),
                    Some((value, rank)) => (rank, value),
                }
            })
            .collect();

        SortKey {
            variants,
            prefix: (parsed.prefix_rank.unwrap_or(-1), parsed.prefix),
            base: u8::from(!is_base),
            components,
            suffix: (u8::from(!parsed.suffix.is_empty()), parsed.suffix),
            original: parsed.original,
        }
    }

    /// Sort classes into a consistent order. Duplicates are removed, keeping
    /// the first occurrence.
    ///
    /// ```
    /// # use tailwhip::{config::Config, sorting::Sorter};
    /// let sorter = Sorter::new(&Config::default());
    /// let sorted = sorter.sort_classes(&["p-4", "flex", "p-4", "container"]);
    /// assert_eq!(sorted, ["container", "flex", "p-4"]);
    /// ```
    pub fn sort_classes<'a>(&self, classes: &[&'a str]) -> Vec<&'a str> {
        let mut keyed: Vec<SortKey<'a>> =
            classes.iter().map(|class| self.sort_key(class)).collect();
        // Keys are unique up to the class name, their last field, so an
        // unstable sort gives one result, with duplicate classes adjacent
        keyed.sort_unstable();
        keyed.dedup_by(|a, b| a.original == b.original);
        keyed.into_iter().map(|key| key.original).collect()
    }

    /// [`sort_classes`](Self::sort_classes) with the keys taken from
    /// `cache`, which must only ever be used with this sorter. The classes
    /// are sorted as 4-byte ids into the cache, which makes for less copying
    /// than sorting the keys themselves.
    pub fn sort_classes_cached<'a>(
        &self,
        classes: &[&'a str],
        cache: &mut KeyCache,
    ) -> Vec<&'a str> {
        // Emptied between lists only, never while a list holds ids
        if cache.keys.len() + classes.len() > KEY_CACHE_CAPACITY {
            cache.ids.clear();
            cache.keys.clear();
        }
        let mut items: SmallVec<[(u32, &'a str); 16]> = classes
            .iter()
            .map(|&class| (cache.id(self, class), class))
            .collect();
        let keys = &cache.keys;
        items.sort_unstable_by(|a, b| keys[a.0 as usize].cmp(&keys[b.0 as usize]));
        // The same class has the same id; different classes never compare equal
        items.dedup_by_key(|item| item.0);
        items.into_iter().map(|(_, class)| class).collect()
    }
}

/// Byte offset of `part` within `text`; `part` must be a subslice of `text`.
fn offset_of(part: &str, text: &str) -> usize {
    part.as_ptr() as usize - text.as_ptr() as usize
}

/// `tokens` joined with "-". Tokens are subslices of `utility`, so when they
/// are adjacent in it (separated by exactly one "-") the result is a slice
/// of `utility`; otherwise, e.g. for "border--x", a new string is built.
fn join_tokens<'a>(utility: &'a str, tokens: &[&'a str]) -> Cow<'a, str> {
    let first = tokens[0];
    let last = tokens[tokens.len() - 1];
    let start = offset_of(first, utility);
    let end = offset_of(last, utility) + last.len();
    let joined_len: usize =
        tokens.iter().map(|token| token.len()).sum::<usize>() + tokens.len() - 1;
    if end - start == joined_len {
        Cow::Borrowed(&utility[start..end])
    } else {
        Cow::Owned(tokens.join("-"))
    }
}

/// Calls `found` with the byte position of every occurrence of `sep` that is
/// not nested inside `[...]` or `(...)`. An empty separator never matches.
///
/// Arbitrary values (`w-[calc(100%-2rem)]`), arbitrary variants
/// (`supports-[display:grid]:`) and CSS variable shorthands (`bg-(--brand)`)
/// contain characters that would otherwise be mistaken for separators.
///
/// Scanning bytes is safe: brackets are ASCII, and a separator can only
/// match at a character boundary, since UTF-8 continuation bytes never
/// equal the first byte of a valid string.
fn for_each_top_level(text: &str, sep: &str, mut found: impl FnMut(usize)) {
    let bytes = text.as_bytes();
    let sep = sep.as_bytes();
    let Some(&first) = sep.first() else {
        return;
    };
    let mut depth = 0usize;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            b'[' | b'(' => depth += 1,
            b']' | b')' if depth > 0 => depth -= 1,
            _ => {}
        }
        if depth == 0 && byte == first && bytes[index..].starts_with(sep) {
            found(index);
            index += sep.len();
            continue;
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str, sep: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut start = 0;
        for_each_top_level(text, sep, |at| {
            parts.push(text[start..at].to_string());
            start = at + sep.len();
        });
        parts.push(text[start..].to_string());
        parts
    }

    #[test]
    fn top_level_split_plain() {
        assert_eq!(split("a:b:c", ":"), ["a", "b", "c"]);
        assert_eq!(split("", ":"), [""]);
        assert_eq!(split("a::b", ":"), ["a", "", "b"]);
        assert_eq!(split("hover:", ":"), ["hover", ""]);
        assert_eq!(split("a__b__c", "__"), ["a", "b", "c"]);
        assert_eq!(split("a→b", "→"), ["a", "b"]);
    }

    #[test]
    fn top_level_split_nested() {
        assert_eq!(
            split("supports-[display:grid]:grid", ":"),
            ["supports-[display:grid]", "grid"]
        );
        assert_eq!(
            split("bg-[url(/img/bg.png)]/50", "/"),
            ["bg-[url(/img/bg.png)]", "50"]
        );
        assert_eq!(split("text-(length:--size)", ":"), ["text-(length:--size)"]);
        assert_eq!(
            split("w-[calc(100%-2rem)]", "-"),
            ["w", "[calc(100%-2rem)]"]
        );
        // Unbalanced closing brackets never make the depth negative
        assert_eq!(split("a]:b", ":"), ["a]", "b"]);
    }

    #[test]
    fn top_level_split_empty_separator_never_splits() {
        assert_eq!(split("abc", ""), ["abc"]);
    }

    #[test]
    fn join_tokens_slices_when_adjacent() {
        let utility = "inline-flex";
        let tokens: Vec<&str> = split(utility, "-")
            .iter()
            .map(|_| "")
            .collect::<Vec<_>>()
            .iter()
            .enumerate()
            .map(|(i, _)| if i == 0 { &utility[..6] } else { &utility[7..] })
            .collect();
        assert!(matches!(
            join_tokens(utility, &tokens),
            Cow::Borrowed("inline-flex")
        ));

        let utility = "border--x";
        let tokens = [&utility[..6], &utility[8..]];
        assert_eq!(
            join_tokens(utility, &tokens),
            Cow::<str>::Owned("border-x".to_string())
        );
    }

    #[test]
    fn key_bytes_compare_like_keys() {
        let sorter = Sorter::new(&Config::default());
        let classes = [
            "flex",
            "p-4",
            "p-2",
            "px-4",
            "-mt-4",
            "!mt-4",
            "mt-4",
            "hover:p-4",
            "sm:hover:p-4",
            "sm:p-4",
            "md:p-4",
            "min-[320px]:p-4",
            "min-[640px]:p-4",
            "data-[x]:p-4",
            "foo:p-4",
            "bar:p-4",
            "foo:bar:p-4",
            "foo:",
            "bg-red-500",
            "bg-red-500/50",
            "bg-red-500/[.3]",
            "bg-red-50",
            "bg-brand",
            "bg-[#fff]",
            "border",
            "border-t",
            "border-t-2",
            "border--x",
            "border-x",
            "w-[calc(100%-2rem)]",
            "text-(length:--size)",
            "custom",
            "custom-a",
            "a",
            "ab",
            "a\0b",
            "a\0",
            "inline-flex",
            "inline",
            "",
            "-",
            "!",
            "hover:",
            "z-[1]",
            "z-10",
            "z-auto",
        ];
        let keys: Vec<_> = classes.iter().map(|class| sorter.sort_key(class)).collect();
        for a in &keys {
            for b in &keys {
                assert_eq!(
                    a.to_bytes().cmp(&b.to_bytes()),
                    a.cmp(b),
                    "{} vs {}",
                    a.original,
                    b.original
                );
            }
        }
        // Nul bytes and terminators
        let mut out = Vec::new();
        push_terminated(&mut out, "a\0");
        assert_eq!(out, b"a\0\xff\0\0");
    }

    #[test]
    fn cached_sorting_matches_sorting() {
        let sorter = Sorter::new(&Config::default());
        let mut cache = KeyCache::default();
        for list in [
            "p-4 flex p-4 container",
            "hover:bg-red-500 bg-red-500 sm:p-2 p-2 flex",
            "b a c a b",
            "flex",
        ] {
            let classes: Vec<&str> = list.split_whitespace().collect();
            assert_eq!(
                sorter.sort_classes_cached(&classes, &mut cache),
                sorter.sort_classes(&classes)
            );
        }
    }

    #[test]
    fn empty_tokens_are_skipped() {
        let sorter = Sorter::new(&Config::default());
        let parsed = sorter.parse_class("border--x");
        assert_eq!(parsed.prefix, "border");
        assert_eq!(parsed.direction, Some("x"));
        let parsed = sorter.parse_class("-");
        assert_eq!(parsed.prefix, "");
        assert!(parsed.negated);
    }
}
