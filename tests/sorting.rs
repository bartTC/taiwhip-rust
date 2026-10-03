//! Sorting tests, ported from the Python test suite: parsing, the order of
//! each component, special cases, and the golden fixture of 87 hand-ordered
//! class groups that must survive any shuffle.

use std::path::Path;

use tailwhip::config::Config;
use tailwhip::process::Processor;
use tailwhip::sorting::Sorter;

fn sorter() -> Sorter {
    Sorter::new(&Config::default())
}

fn sorted(classes: &[&str]) -> Vec<String> {
    sorter()
        .sort_classes(classes)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn sorted_with(config: &Config, classes: &[&str]) -> Vec<String> {
    Sorter::new(config)
        .sort_classes(classes)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn index_of(list: &[String], item: &str) -> usize {
    list.iter()
        .position(|x| x == item)
        .unwrap_or_else(|| panic!("{item} missing"))
}

// =============================================================================
// Parsing
// =============================================================================

mod parse_class {
    use super::*;

    #[test]
    fn simple_prefix() {
        let s = sorter();
        let result = s.parse_class("flex");
        assert_eq!(result.prefix, "flex");
        assert!(result.variants.is_empty());
        assert!(!result.important);
        assert!(!result.negated);
    }

    #[test]
    fn prefix_with_value() {
        let result = sorter().parse_class("p-4");
        assert_eq!(result.prefix, "p");
        assert_eq!(result.value, Some("4"));
    }

    #[test]
    fn prefix_with_direction_and_value() {
        let s = sorter();
        let result = s.parse_class("border-t");
        assert_eq!(
            (result.prefix.as_ref(), result.direction),
            ("border", Some("t"))
        );
        let result = s.parse_class("border-t-2");
        assert_eq!(result.direction, Some("t"));
        assert_eq!(result.value, Some("2"));
    }

    #[test]
    fn color_shade_and_alpha() {
        let s = sorter();
        let result = s.parse_class("text-red-500");
        assert_eq!((result.color, result.shade), (Some("red"), Some("500")));
        let result = s.parse_class("border-t-red-500");
        assert_eq!(
            (result.direction, result.color, result.shade),
            (Some("t"), Some("red"), Some("500"))
        );
        let result = s.parse_class("bg-red-500/50");
        assert_eq!(
            (result.color, result.shade, result.alpha),
            (Some("red"), Some("500"), Some("50"))
        );
    }

    #[test]
    fn important_and_negated() {
        let s = sorter();
        let result = s.parse_class("!mt-4");
        assert!(result.important);
        assert_eq!((result.prefix.as_ref(), result.value), ("mt", Some("4")));
        let result = s.parse_class("-mt-4");
        assert!(result.negated);
        assert_eq!((result.prefix.as_ref(), result.value), ("mt", Some("4")));
        let result = s.parse_class("!-mt-4");
        assert!(result.important && result.negated);
        assert_eq!((result.prefix.as_ref(), result.value), ("mt", Some("4")));
    }

    #[test]
    fn variants() {
        let s = sorter();
        let result = s.parse_class("hover:bg-red-500");
        assert_eq!(result.variants, ["hover"]);
        assert_eq!(result.prefix, "bg");
        let result = s.parse_class("sm:hover:focus:bg-red-500");
        assert_eq!(result.variants, ["sm", "hover", "focus"]);
        assert_eq!(result.color, Some("red"));
    }

    #[test]
    fn compound_prefix() {
        assert_eq!(sorter().parse_class("inline-flex").prefix, "inline-flex");
    }

    #[test]
    fn arbitrary_value() {
        let result = sorter().parse_class("w-[100px]");
        assert_eq!(result.prefix, "w");
        assert_eq!(result.suffix, "[100px]");
    }

    #[test]
    fn sizes() {
        let s = sorter();
        let result = s.parse_class("text-xl");
        assert_eq!((result.prefix.as_ref(), result.size), ("text", Some("xl")));
        let result = s.parse_class("rounded-t-lg");
        assert_eq!((result.direction, result.size), (Some("t"), Some("lg")));
    }

    #[test]
    fn bracketed_separators_belong_to_the_value() {
        let s = sorter();
        let result = s.parse_class("supports-[display:grid]:grid");
        assert_eq!(result.variants, ["supports-[display:grid]"]);
        assert_eq!(result.prefix, "grid");

        let result = s.parse_class("has-[:checked]:p-2");
        assert_eq!(result.variants, ["has-[:checked]"]);
        assert_eq!((result.prefix.as_ref(), result.value), ("p", Some("2")));

        let result = s.parse_class("bg-(--brand-color)");
        assert_eq!(
            (result.prefix.as_ref(), result.suffix.as_ref(), result.color),
            ("bg", "(--brand-color)", None)
        );

        let result = s.parse_class("text-(length:--size)");
        assert!(result.variants.is_empty());
        assert_eq!(
            (result.prefix.as_ref(), result.suffix.as_ref()),
            ("text", "(length:--size)")
        );

        let result = s.parse_class("bg-(--brand)/50");
        assert_eq!(
            (result.suffix.as_ref(), result.alpha),
            ("(--brand)", Some("50"))
        );

        let result = s.parse_class("bg-[url(/img/bg.png)]");
        assert_eq!(
            (result.suffix.as_ref(), result.alpha),
            ("[url(/img/bg.png)]", None)
        );
    }

    #[test]
    fn variant_only_class_has_empty_prefix() {
        let result = sorter().parse_class("hover:");
        assert_eq!(result.variants, ["hover"]);
        assert_eq!(result.prefix, "");
    }
}

// =============================================================================
// Sorting by component
// =============================================================================

#[test]
fn variants_order() {
    assert_eq!(sorted(&["hover:p-4", "p-4"]), ["p-4", "hover:p-4"]);
    assert_eq!(
        sorted(&["xl:p-4", "sm:p-4", "lg:p-4", "md:p-4"]),
        ["sm:p-4", "md:p-4", "lg:p-4", "xl:p-4"]
    );
    assert_eq!(sorted(&["sm:p-4", "dark:p-4"]), ["dark:p-4", "sm:p-4"]);
    assert_eq!(
        sorted(&["active:p-4", "hover:p-4", "focus:p-4"]),
        ["hover:p-4", "focus:p-4", "active:p-4"]
    );
    assert_eq!(
        sorted(&["lg:hover:p-4", "sm:hover:p-4"]),
        ["sm:hover:p-4", "lg:hover:p-4"]
    );
    assert_eq!(
        sorted(&[
            "data-loading:opacity-50",
            "aria-busy:animate-spin",
            "hover:p-2"
        ]),
        [
            "hover:p-2",
            "aria-busy:animate-spin",
            "data-loading:opacity-50"
        ]
    );
    assert_eq!(
        sorted(&["**:text-sm", "*:p-2", "before:block"]),
        ["before:block", "*:p-2", "**:text-sm"]
    );
}

#[test]
fn prefix_order() {
    assert_eq!(sorted(&["p-4", "flex"]), ["flex", "p-4"]);
    assert_eq!(
        sorted(&["p-4", "flex", "container"]),
        ["container", "flex", "p-4"]
    );
    assert_eq!(sorted(&["p-4", "m-4"]), ["m-4", "p-4"]);
    // Unknown prefixes sort alphabetically, before everything else
    assert_eq!(
        sorted(&["zzz-custom", "aaa-custom"]),
        ["aaa-custom", "zzz-custom"]
    );
    assert_eq!(
        sorted(&["container", "select2-container"]),
        ["select2-container", "container"]
    );
}

#[test]
fn direction_order() {
    assert_eq!(sorted(&["border-t", "border"]), ["border", "border-t"]);
    assert_eq!(
        sorted(&["border-l", "border-t", "border-y", "border-x"]),
        ["border-x", "border-y", "border-t", "border-l"]
    );
    assert_eq!(
        sorted(&["border-b-2", "border-t-2"]),
        ["border-t-2", "border-b-2"]
    );
}

#[test]
fn size_order() {
    assert_eq!(
        sorted(&["text-xl", "text-sm", "text-lg", "text-xs"]),
        ["text-xs", "text-sm", "text-lg", "text-xl"]
    );
    assert_eq!(sorted(&["blur-sm", "blur"]), ["blur", "blur-sm"]);
    assert_eq!(
        sorted(&["rounded-t-xl", "rounded-t-sm"]),
        ["rounded-t-sm", "rounded-t-xl"]
    );
}

#[test]
fn value_order() {
    assert_eq!(sorted(&["p-8", "p-2", "p-4"]), ["p-2", "p-4", "p-8"]);
    assert_eq!(sorted(&["m-4", "m-0"]), ["m-0", "m-4"]);
}

#[test]
fn color_shade_and_alpha_order() {
    assert_eq!(
        sorted(&["text-red-500", "text-xl"]),
        ["text-xl", "text-red-500"]
    );
    assert_eq!(
        sorted(&["bg-blue-500", "bg-red-500", "bg-gray-500"]),
        ["bg-blue-500", "bg-gray-500", "bg-red-500"]
    );
    assert_eq!(
        sorted(&["text-red-500", "text-black", "text-white"]),
        ["text-black", "text-red-500", "text-white"]
    );
    assert_eq!(
        sorted(&["bg-red-700", "bg-red-300", "bg-red-500"]),
        ["bg-red-300", "bg-red-500", "bg-red-700"]
    );
    assert_eq!(
        sorted(&["bg-red-500/50", "bg-red-500"]),
        ["bg-red-500/50", "bg-red-500"]
    );
    assert_eq!(
        sorted(&["bg-red-500/75", "bg-red-500/25", "bg-red-500/50"]),
        ["bg-red-500/25", "bg-red-500/50", "bg-red-500/75"]
    );
}

// =============================================================================
// Special cases
// =============================================================================

#[test]
fn negated_and_important_classes_stay_with_their_group() {
    let result = sorted(&["mx-4", "-mx-2", "my-4"]);
    assert!(index_of(&result, "-mx-2") < index_of(&result, "my-4"));
    assert_eq!(
        sorted(&["-mb-4", "mt-4", "-ml-4"]),
        ["mt-4", "-mb-4", "-ml-4"]
    );

    let result = sorted(&["!mt-4", "mt-4", "p-4"]);
    assert!(index_of(&result, "!mt-4") < index_of(&result, "p-4"));
    assert!(index_of(&result, "mt-4") < index_of(&result, "p-4"));
}

#[test]
fn duplicates_are_removed_keeping_the_first() {
    assert_eq!(sorted(&["p-4", "p-4", "p-4"]), ["p-4"]);
    assert_eq!(sorted(&["p-4", "flex", "p-4"]), ["flex", "p-4"]);
    // The hash-set path for long lists behaves the same
    let many: Vec<&str> = std::iter::repeat_n(["p-4", "flex", "m-2"], 20)
        .flatten()
        .collect();
    assert_eq!(sorted(&many), ["flex", "m-2", "p-4"]);
}

#[test]
fn arbitrary_values_sort_after_known_values() {
    assert_eq!(sorted(&["w-[100px]", "w-4"]), ["w-4", "w-[100px]"]);
    assert_eq!(
        sorted(&["grid-cols-[200px_1fr]", "grid-cols-2"]),
        ["grid-cols-2", "grid-cols-[200px_1fr]"]
    );
}

#[test]
fn real_world_examples() {
    let result = sorted(&[
        "hover:bg-blue-600",
        "text-white",
        "font-bold",
        "py-2",
        "px-4",
        "rounded",
        "bg-blue-500",
    ]);
    assert!(index_of(&result, "py-2") < index_of(&result, "font-bold"));
    assert!(index_of(&result, "bg-blue-500") < index_of(&result, "rounded"));
    assert!(index_of(&result, "bg-blue-500") < index_of(&result, "hover:bg-blue-600"));

    let result = sorted(&[
        "shadow-lg",
        "p-6",
        "bg-white",
        "rounded-lg",
        "max-w-sm",
        "mx-auto",
    ]);
    assert!(index_of(&result, "max-w-sm") < index_of(&result, "mx-auto"));
    assert!(index_of(&result, "p-6") < index_of(&result, "bg-white"));

    assert_eq!(
        sorted(&["lg:text-xl", "text-base", "md:text-lg", "sm:text-sm"]),
        ["text-base", "sm:text-sm", "md:text-lg", "lg:text-xl"]
    );
}

// =============================================================================
// Configuration
// =============================================================================

#[test]
fn custom_colors_merge_alphabetically() {
    let config = Config {
        custom_colors: vec!["brand".into(), "accent".into()],
        ..Config::default()
    };
    assert_eq!(
        sorted_with(&config, &["text-brand", "text-red-500", "text-accent"]),
        ["text-accent", "text-brand", "text-red-500"]
    );

    let config = Config {
        custom_colors: vec!["primary".into()],
        ..Config::default()
    };
    assert_eq!(
        sorted_with(
            &config,
            &["text-purple-500", "text-primary-500", "text-pink-500"]
        ),
        ["text-pink-500", "text-primary-500", "text-purple-500"]
    );
}

#[test]
fn configuration_changes_take_effect() {
    let classes = ["text-blue-500", "text-aqua-500"];
    assert_eq!(sorted(&classes), ["text-blue-500", "text-aqua-500"]);
    let config = Config {
        custom_colors: vec!["aqua".into()],
        ..Config::default()
    };
    assert_eq!(
        sorted_with(&config, &classes),
        ["text-aqua-500", "text-blue-500"]
    );
}

#[test]
fn unknown_component_names_are_ignored() {
    let config = Config {
        component_order: ["variant", "prefix", "value", "bogus"]
            .map(String::from)
            .to_vec(),
        ..Config::default()
    };
    assert_eq!(
        sorted_with(&config, &["p-4", "m-2", "p-2"]),
        ["m-2", "p-2", "p-4"]
    );
}

#[test]
fn variant_separator_is_configurable() {
    let config = Config {
        variant_separator: "__".into(),
        ..Config::default()
    };
    assert_eq!(
        sorted_with(&config, &["hover__p-4", "p-4", "sm__p-4"]),
        ["p-4", "sm__p-4", "hover__p-4"]
    );
}

#[test]
fn process_text_integration() {
    let processor = Processor::new(&Config::default()).unwrap();
    assert_eq!(
        processor.process_text(r#"<div class="p-4 flex container"></div>"#),
        r#"<div class="container flex p-4"></div>"#
    );
    assert_eq!(
        processor.process_text("@apply p-4 flex container;"),
        "@apply container flex p-4;"
    );
    let html = r#"<div class="p-4 container {{ extra }}"></div>"#;
    assert_eq!(processor.process_text(html), html);
    let html = r#"<div class=""></div>"#;
    assert_eq!(processor.process_text(html), html);
}

#[test]
fn custom_patterns_from_a_config_file() {
    let mut config = Config::default();
    config
        .apply_toml(
            r#"
[[class_patterns]]
name = "html_class"
regex = '(?i)\bclass\s*=\s*"(?P<classes>[^"]*)"'

[[class_patterns]]
name = "css_apply"
regex = '@apply\s+(?P<classes>[^;]+);'

[[class_patterns]]
name = "jsx_classname"
regex = '\bclassName\s*=\s*"(?P<classes>[^"]*)"'
"#,
            Path::new("tailwhip.toml"),
        )
        .unwrap();
    let processor = Processor::new(&config).unwrap();
    let names: Vec<&str> = processor
        .patterns()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["html_class", "css_apply", "jsx_classname"]);

    assert_eq!(
        processor.process_text(r#"<Component className="p-4 m-2 flex" />"#),
        r#"<Component className="flex m-2 p-4" />"#
    );
    assert_eq!(
        processor.process_text(r#"<div class="p-4 m-2 flex"></div>"#),
        r#"<div class="flex m-2 p-4"></div>"#
    );
    assert_eq!(
        processor.process_text(".btn { @apply p-4 m-2 flex; }"),
        ".btn { @apply flex m-2 p-4; }"
    );
    // Single quotes are no longer covered by this custom set
    assert_eq!(
        processor.process_text("<div class='p-4 m-2'>"),
        "<div class='p-4 m-2'>"
    );
}

// =============================================================================
// Golden fixture: every group is in the correct order and must survive any
// permutation. The groups are the CLASS_GROUPS of the Python test suite.
// =============================================================================

/// A small deterministic xorshift generator, so failures are reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

fn class_groups() -> Vec<Vec<&'static str>> {
    include_str!("fixtures/class_groups.txt")
        .split("\n\n")
        .map(|group| {
            group
                .lines()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect()
}

#[test]
fn golden_groups_are_loaded() {
    let groups = class_groups();
    assert_eq!(groups.len(), 87);
    assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 601);
}

#[test]
fn golden_groups_are_already_sorted() {
    let sorter = sorter();
    for group in class_groups() {
        assert_eq!(
            sorter.sort_classes(&group),
            group,
            "group starting with {:?}",
            group[0]
        );
    }
}

#[test]
fn golden_groups_survive_reversal_and_rotation() {
    let sorter = sorter();
    for group in class_groups() {
        let mut reversed = group.clone();
        reversed.reverse();
        assert_eq!(
            sorter.sort_classes(&reversed),
            group,
            "reversed group starting with {:?}",
            group[0]
        );

        for shift in 1..group.len() {
            let mut rotated = group.clone();
            rotated.rotate_left(shift);
            assert_eq!(
                sorter.sort_classes(&rotated),
                group,
                "group starting with {:?} rotated by {shift}",
                group[0]
            );
        }
    }
}

#[test]
fn golden_groups_survive_random_shuffles() {
    let sorter = sorter();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for group in class_groups() {
        for round in 0..25 {
            let mut shuffled = group.clone();
            rng.shuffle(&mut shuffled);
            assert_eq!(
                sorter.sort_classes(&shuffled),
                group,
                "group starting with {:?}, shuffle round {round}: {shuffled:?}",
                group[0]
            );
        }
    }
}
