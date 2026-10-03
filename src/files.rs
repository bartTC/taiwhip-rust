//! File discovery and applying changes to files.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobSetBuilder};
use rayon::prelude::*;
use similar::{ChangeTag, TextDiff};
use walkdir::WalkDir;

use crate::config::verbosity;
use crate::console::{Console, style};
use crate::process::Processor;

/// Find all files from a list of paths.
///
/// A directory is expanded with the default globs (`dir/**/*.html`, ...);
/// anything else is a literal file path or a glob pattern. Patterns support
/// `*`, `?`, `[...]`, `{a,b}` alternatives and `**` for any number of
/// directories; hidden files are included. The result holds absolute paths,
/// deduplicated, in discovery order. Paths are made absolute lexically:
/// `.` and `..` are removed, symbolic links are not resolved.
pub fn find_files(paths: &[PathBuf], default_globs: &[String]) -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut found = Vec::new();

    for entry in paths {
        let matches = if entry.is_dir() {
            // One walk of the directory for all default globs
            let patterns: Vec<String> = default_globs
                .iter()
                .map(|glob| join_pattern(entry, glob))
                .collect();
            expand_globs(&patterns)
        } else {
            expand_globs(&[entry.to_string_lossy().into_owned()])
        };
        for path in matches {
            let path = absolute(&path, &cwd);
            if seen.insert(path.clone()) {
                found.push(path);
            }
        }
    }
    found
}

/// `path` made absolute against `cwd`, with `.` and `..` components removed.
/// Unlike `canonicalize`, this needs no system calls, which matters for
/// thousands of files.
fn absolute(path: &Path, cwd: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// `dir/pattern` with the directory normalized the way `pathlib` does:
/// `./templates/` becomes `templates`, and `.` disappears entirely.
fn join_pattern(dir: &Path, pattern: &str) -> String {
    let normalized: PathBuf = dir
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect();
    if normalized.as_os_str().is_empty() {
        pattern.to_string()
    } else {
        format!(
            "{}/{pattern}",
            normalized.to_string_lossy().trim_end_matches('/')
        )
    }
}

fn has_glob_meta(text: &str) -> bool {
    text.contains(['*', '?', '[', '{'])
}

/// The files matching the literal paths or glob patterns. Patterns that
/// share their literal base directory are matched in a single walk.
pub fn expand_globs(patterns: &[String]) -> Vec<PathBuf> {
    let mut literal = Vec::new();
    let mut walks: Vec<(String, Vec<String>, Vec<Glob>)> = Vec::new();

    for pattern in patterns {
        let mut pattern = pattern.as_str();
        while let Some(rest) = pattern.strip_prefix("./") {
            pattern = rest;
        }
        if !has_glob_meta(pattern) {
            let path = PathBuf::from(pattern);
            if path.is_file() {
                literal.push(path);
            }
            continue;
        }
        let Ok(glob) = GlobBuilder::new(pattern).literal_separator(true).build() else {
            continue;
        };
        let (base, remainder) = split_literal_base(pattern);
        match walks
            .iter_mut()
            .find(|(walk_base, _, _)| *walk_base == base)
        {
            Some((_, remainders, globs)) => {
                remainders.push(remainder);
                globs.push(glob);
            }
            None => walks.push((base, vec![remainder], vec![glob])),
        }
    }

    for (base, remainders, globs) in walks {
        let mut builder = GlobSetBuilder::new();
        for glob in globs {
            builder.add(glob);
        }
        let Ok(set) = builder.build() else {
            continue;
        };
        // Walk from the longest literal directory prefix, only as deep as
        // the patterns can reach
        let root = if base.is_empty() {
            Path::new(".")
        } else {
            Path::new(base.as_str())
        };
        let mut walker = WalkDir::new(root).follow_links(false);
        let depths: Option<Vec<usize>> = remainders.iter().map(|r| max_depth(r)).collect();
        if let Some(depth) = depths.and_then(|d| d.into_iter().max()) {
            walker = walker.max_depth(depth);
        }
        for entry in walker.into_iter().filter_map(Result::ok) {
            let file_type = entry.file_type();
            if file_type.is_dir() {
                continue;
            }
            let path = entry.into_path();
            // Symlinks are kept when they point at a file
            if !file_type.is_file() && !path.is_file() {
                continue;
            }
            let candidate = if base.is_empty() {
                path.strip_prefix(".")
                    .map(Path::to_path_buf)
                    .unwrap_or(path)
            } else {
                path
            };
            if set.is_match(&candidate) {
                literal.push(candidate);
            }
        }
    }
    literal
}

/// Split a pattern into its leading literal directories and the rest.
fn split_literal_base(pattern: &str) -> (String, String) {
    let absolute = pattern.starts_with('/');
    let parts: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let literal_count = parts[..parts.len() - 1]
        .iter()
        .take_while(|part| !has_glob_meta(part))
        .count();
    let mut base = parts[..literal_count].join("/");
    if absolute {
        base.insert(0, '/');
    }
    (base, parts[literal_count..].join("/"))
}

/// How deep below the base a pattern can match, or `None` for unlimited.
fn max_depth(remainder: &str) -> Option<usize> {
    if remainder.contains("**") {
        return None;
    }
    // An alternative containing "/" could span any number of directories
    let mut in_brace = false;
    for ch in remainder.chars() {
        match ch {
            '{' => in_brace = true,
            '}' => in_brace = false,
            '/' if in_brace => return None,
            _ => {}
        }
    }
    Some(remainder.split('/').count())
}

/// How processing a file went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileResult {
    pub skipped: bool,
    pub changed: bool,
    pub failed: bool,
}

/// What `apply_changes` needs besides the files.
pub struct Options<'a> {
    pub processor: &'a Processor,
    pub console: &'a Console,
    pub write_mode: bool,
    pub verbosity: u8,
}

/// Totals over all processed files.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub found_any: bool,
    pub skipped: usize,
    pub changed: usize,
    pub failed: usize,
}

/// Process the files in parallel: sort their classes and write the result
/// back in write mode, or report what would change.
pub fn apply_changes(files: &[PathBuf], options: &Options<'_>) -> Summary {
    let results: Vec<FileResult> = if files.len() > 1 {
        files
            .par_iter()
            .map(|file| process_file(file, options))
            .collect()
    } else {
        files
            .iter()
            .map(|file| process_file(file, options))
            .collect()
    };

    let mut summary = Summary {
        found_any: !files.is_empty(),
        ..Summary::default()
    };
    for result in results {
        if result.failed {
            summary.failed += 1;
        }
        if result.skipped {
            summary.skipped += 1;
        } else {
            summary.changed += 1;
        }
    }
    summary
}

fn process_file(path: &Path, options: &Options<'_>) -> FileResult {
    let console = options.console;
    let filename = |path: &Path| console.style(style::WHITE, &path.display().to_string());
    let skipped = FileResult {
        skipped: true,
        changed: false,
        failed: false,
    };

    let old_text = match fs::read(path).map(String::from_utf8) {
        Ok(Ok(text)) => text,
        _ => {
            console.error(&format!(
                "{} {}",
                console.style(style::RED, "Unable to read"),
                filename(path)
            ));
            return skipped;
        }
    };

    let new_text = options.processor.process_text(&old_text);
    if new_text == old_text {
        if options.verbosity >= verbosity::VERBOSE {
            console
                .print(&console.style(style::GREY, &format!("Already sorted {}", path.display())));
        }
        return skipped;
    }

    if options.write_mode
        && let Err(error) = fs::write(path, new_text.as_bytes())
    {
        console.error(&format!(
            "{} {}: {error}",
            console.style(style::RED, "Unable to write"),
            filename(path)
        ));
        return FileResult {
            skipped: true,
            changed: false,
            failed: true,
        };
    }

    if options.verbosity >= verbosity::NORMAL {
        let action = if options.write_mode {
            "Updated"
        } else {
            "Would update"
        };
        let mut message = format!("{} {}", console.style(style::DIM, action), filename(path));
        if options.verbosity >= verbosity::DIFF {
            message.push('\n');
            message.push_str(&render_diff(path, &old_text, &new_text, console));
        }
        console.print(&message);
    }

    FileResult {
        skipped: false,
        changed: true,
        failed: false,
    }
}

/// A unified diff with one line of context, in the format of Python's
/// `difflib`, indented and padded with blank lines like the Python tool
/// prints it. Every line is stripped of surrounding whitespace, which also
/// removes the leading space of context lines.
pub fn render_diff(path: &Path, old_text: &str, new_text: &str, console: &Console) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let diff = TextDiff::from_slices(&old_lines, &new_lines);

    let mut lines: Vec<String> = vec![
        console.style(style::BRIGHT_RED, &format!("--- {}", path.display())),
        console.style(style::GREEN, &format!("+++ {}", path.display())),
    ];

    for group in diff.grouped_ops(1) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let header = format!(
            "@@ -{} +{} @@",
            format_range(first.old_range().start, last.old_range().end),
            format_range(first.new_range().start, last.new_range().end),
        );
        lines.push(console.style(style::BOLD_MAGENTA, &header));

        for op in &group {
            for change in diff.iter_changes(op) {
                let (marker, color) = match change.tag() {
                    ChangeTag::Equal => (" ", None),
                    ChangeTag::Delete => ("-", Some(style::BRIGHT_RED)),
                    ChangeTag::Insert => ("+", Some(style::GREEN)),
                };
                let line = format!("{marker}{}", change.value());
                let line = line.trim();
                lines.push(match color {
                    Some(color) => console.style(color, line),
                    None => line.to_string(),
                });
            }
        }
    }

    let mut out = String::from("\n");
    for line in lines {
        out.push_str("    ");
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// A line range in the "ed" format difflib uses: `start` for one line,
/// `start,length` otherwise, with empty ranges at the line before.
fn format_range(start: usize, end: usize) -> String {
    let mut beginning = start + 1;
    let length = end - start;
    if length == 1 {
        return beginning.to_string();
    }
    if length == 0 {
        beginning -= 1;
    }
    format!("{beginning},{length}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_base_and_depth() {
        assert_eq!(
            split_literal_base("templates/**/*.html"),
            ("templates".into(), "**/*.html".into())
        );
        assert_eq!(split_literal_base("*.css"), ("".into(), "*.css".into()));
        assert_eq!(
            split_literal_base("a/b/*.css"),
            ("a/b".into(), "*.css".into())
        );
        assert_eq!(
            split_literal_base("/abs/dir/*.css"),
            ("/abs/dir".into(), "*.css".into())
        );
        assert_eq!(
            split_literal_base("a/*/b.css"),
            ("a".into(), "*/b.css".into())
        );
        assert_eq!(max_depth("**/*.html"), None);
        assert_eq!(max_depth("*.css"), Some(1));
        assert_eq!(max_depth("*/*.css"), Some(2));
        assert_eq!(max_depth("*.{css,scss}"), Some(1));
        assert_eq!(max_depth("{a/b,c}/*.css"), None);
    }

    #[test]
    fn absolute_paths_are_normalized_lexically() {
        let cwd = Path::new("/work/project");
        assert_eq!(
            absolute(Path::new("index.html"), cwd),
            PathBuf::from("/work/project/index.html")
        );
        assert_eq!(
            absolute(Path::new("./a/./b.html"), cwd),
            PathBuf::from("/work/project/a/b.html")
        );
        assert_eq!(
            absolute(Path::new("../x/../y.html"), cwd),
            PathBuf::from("/work/y.html")
        );
        assert_eq!(
            absolute(Path::new("/abs/../z.html"), cwd),
            PathBuf::from("/z.html")
        );
    }

    #[test]
    fn join_pattern_normalizes_the_directory() {
        assert_eq!(join_pattern(Path::new("."), "**/*.html"), "**/*.html");
        assert_eq!(
            join_pattern(Path::new("./templates/"), "**/*.html"),
            "templates/**/*.html"
        );
        assert_eq!(
            join_pattern(Path::new("/abs/dir"), "*.css"),
            "/abs/dir/*.css"
        );
    }

    #[test]
    fn diff_matches_difflib_format() {
        let console = Console::with_color(false, false);
        let diff = render_diff(Path::new("f"), "a\nb\nc\n", "a\nx\nc\n", &console);
        assert_eq!(
            diff,
            "\n    --- f\n    +++ f\n    @@ -1,3 +1,3 @@\n    a\n    -b\n    +x\n    c\n"
        );
        let diff = render_diff(Path::new("f"), "a\n", "b\n", &console);
        assert_eq!(
            diff,
            "\n    --- f\n    +++ f\n    @@ -1 +1 @@\n    -a\n    +b\n"
        );
        let diff = render_diff(Path::new("f"), "a\nb\n", "a\nb\nc\n", &console);
        assert_eq!(
            diff,
            "\n    --- f\n    +++ f\n    @@ -2 +2,2 @@\n    b\n    +c\n"
        );
    }
}
