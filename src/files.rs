//! File discovery and applying changes to files.
//!
//! Directories are read in parallel on the thread pool, and each file is
//! processed there as soon as it is found, so reading the tree and sorting
//! classes overlap. What every file has to report is collected and printed
//! at the end, in path order, with one write per stream.
//!
//! Like ripgrep or fd, and unlike the Python tool, a walk skips hidden files
//! and directories and what `.gitignore` files exclude: the virtualenv, the
//! `node_modules` and the collected static files of a project root hold
//! more files than the project itself, and are not to be rewritten.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs::{self, DirEntry};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::gitignore::Gitignore;
use rustc_hash::FxHashSet;
use similar::{ChangeTag, TextDiff};

use crate::config::verbosity;
use crate::console::{Console, style};
use crate::process::Processor;

/// What a list of paths expands to: literal file paths, and directory walks
/// for the glob patterns.
#[derive(Debug, Default)]
struct Search {
    /// Absolute paths.
    files: Vec<PathBuf>,
    walks: Vec<Walk>,
    /// Whether walks skip hidden and ignored files.
    skip_ignored: bool,
}

/// One walk of a directory for all patterns that share it as their literal
/// base, such as `templates` for `templates/**/*.html`.
#[derive(Debug)]
struct Walk {
    /// The directory, as an absolute path.
    root: PathBuf,
    /// The rest of each pattern, such as `**/*.html`, matched against paths
    /// relative to `root`.
    globs: GlobSet,
    /// How deep below `root` the patterns can match, or `None` for
    /// unlimited.
    max_depth: Option<usize>,
}

impl Search {
    /// A directory stands for the default globs below it
    /// (`dir/**/*.html`, ...); anything else is a literal file path or a
    /// glob pattern.
    fn new(paths: &[PathBuf], default_globs: &[Cow<'static, str>], skip_ignored: bool) -> Search {
        let cwd = std::env::current_dir().unwrap_or_default();
        let mut search = Search {
            skip_ignored,
            ..Search::default()
        };
        let mut walks: Vec<(PathBuf, GlobSetBuilder, Option<usize>)> = Vec::new();

        for entry in paths {
            let patterns: Vec<String> = if entry.is_dir() {
                default_globs
                    .iter()
                    .map(|glob| join_pattern(entry, glob))
                    .collect()
            } else {
                vec![entry.to_string_lossy().into_owned()]
            };
            for pattern in &patterns {
                let mut pattern = pattern.as_str();
                while let Some(rest) = pattern.strip_prefix("./") {
                    pattern = rest;
                }
                if !has_glob_meta(pattern) {
                    search.files.push(absolute(Path::new(pattern), &cwd));
                    continue;
                }
                let (base, remainder) = split_literal_base(pattern);
                let Ok(glob) = GlobBuilder::new(&remainder).literal_separator(true).build() else {
                    continue;
                };
                let root = absolute(Path::new(&base), &cwd);
                let depth = max_depth(&remainder);
                match walks.iter_mut().find(|(walk_root, ..)| *walk_root == root) {
                    Some((_, globs, max)) => {
                        globs.add(glob);
                        *max = max.zip(depth).map(|(a, b)| a.max(b));
                    }
                    None => {
                        let mut globs = GlobSetBuilder::new();
                        globs.add(glob);
                        walks.push((root, globs, depth));
                    }
                }
            }
        }

        for (root, globs, max_depth) in walks {
            if let Ok(globs) = globs.build() {
                search.walks.push(Walk {
                    root,
                    globs,
                    max_depth,
                });
            }
        }
        search
    }

    /// No walks and at most one file: not worth starting the thread pool for.
    fn is_trivial(&self) -> bool {
        self.walks.is_empty() && self.files.len() <= 1
    }

    /// Calls `visit` once for every file found, with its absolute path, as
    /// soon as it is found. Directories are read in parallel on the thread
    /// pool, and `visit` runs there as well, unless the search
    /// [is trivial](Self::is_trivial).
    fn visit<F>(&self, visit: F)
    where
        F: Fn(PathBuf) + Sync,
    {
        let seen = Mutex::new(FxHashSet::default());
        let found = |path: PathBuf| {
            let new = seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(path.clone());
            if new {
                visit(path);
            }
        };

        if self.is_trivial() {
            for file in &self.files {
                if file.is_file() {
                    found(file.clone());
                }
            }
            return;
        }

        rayon::scope(|scope| self.spawn(scope, &found));
    }

    /// Spawn a task for every literal file and every walk. Matching files
    /// are visited in tasks of their own.
    fn spawn<'s, F>(&'s self, scope: &rayon::Scope<'s>, found: &'s F)
    where
        F: Fn(PathBuf) + Sync,
    {
        for file in &self.files {
            scope.spawn(move |_| {
                if file.is_file() {
                    found(file.clone());
                }
            });
        }
        for walk in &self.walks {
            scope.spawn(move |scope| {
                let ignores = if self.skip_ignored {
                    Ignores::above(&walk.root)
                } else {
                    None
                };
                self.read_dir(scope, walk, walk.root.clone(), 1, ignores, found);
            });
        }
    }

    /// Read one directory of a walk. Subdirectories are read, and matching
    /// files are handed to `found`, in tasks of their own. `ignores` are
    /// the `.gitignore` files that apply, if the directory is in a Git
    /// repository and ignored files are skipped.
    fn read_dir<'s, F>(
        &'s self,
        scope: &rayon::Scope<'s>,
        walk: &'s Walk,
        dir: PathBuf,
        depth: usize,
        mut ignores: Option<Ignores>,
        found: &'s F,
    ) where
        F: Fn(PathBuf) + Sync,
    {
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        let entries: Vec<DirEntry> = entries.flatten().collect();

        if self.skip_ignored {
            let has = |name: &str| entries.iter().any(|entry| entry.file_name() == name);
            // A repository starts afresh, without the files of any around it
            if has(".git") {
                ignores = Some(Ignores::default());
            }
            if let Some(outer) = &ignores
                && has(".gitignore")
            {
                ignores = Some(outer.with_file(&dir));
            }
        }
        let is_ignored = |path: &Path, is_dir: bool| {
            ignores
                .as_ref()
                .is_some_and(|ignores| ignores.is_ignored(path, is_dir))
        };

        for entry in entries {
            let name = entry.file_name();
            if self.skip_ignored && name.as_encoded_bytes().starts_with(b".") {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = dir.join(name);
            if file_type.is_dir() {
                if walk.max_depth.is_none_or(|max| depth < max) && !is_ignored(&path, true) {
                    let ignores = ignores.clone();
                    scope.spawn(move |scope| {
                        self.read_dir(scope, walk, path, depth + 1, ignores, found);
                    });
                }
            } else if walk.globs.is_match(path.strip_prefix(&walk.root).unwrap_or(&path))
                && !is_ignored(&path, false)
                // Symbolic links are followed only to files
                && (file_type.is_file() || path.is_file())
            {
                scope.spawn(move |_| found(path));
            }
        }
    }
}

/// The `.gitignore` files that apply to a directory, the innermost first.
#[derive(Clone, Default)]
struct Ignores(Option<Arc<IgnoreFile>>);

struct IgnoreFile {
    matcher: Gitignore,
    outer: Ignores,
}

impl Ignores {
    /// The `.gitignore` files of the directories above `dir`, up to the root
    /// of the Git repository it is in, or `None` outside of one: like Git,
    /// `.gitignore` files then mean nothing.
    fn above(dir: &Path) -> Option<Ignores> {
        let above: Vec<&Path> = dir.ancestors().skip(1).collect();
        let repository = above.iter().position(|dir| dir.join(".git").exists())?;
        let mut ignores = Ignores::default();
        for dir in above[..=repository].iter().rev() {
            if dir.join(".gitignore").is_file() {
                ignores = ignores.with_file(dir);
            }
        }
        Some(ignores)
    }

    /// These and the `.gitignore` file in `dir`. A file that cannot be read
    /// or parsed ignores what it can.
    fn with_file(&self, dir: &Path) -> Ignores {
        let (matcher, _) = Gitignore::new(dir.join(".gitignore"));
        Ignores(Some(Arc::new(IgnoreFile {
            matcher,
            outer: self.clone(),
        })))
    }

    /// Whether a path is ignored: the innermost file with a matching
    /// pattern decides.
    fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        let mut file = self.0.as_deref();
        while let Some(current) = file {
            let matched = current.matcher.matched(path, is_dir);
            if !matched.is_none() {
                return matched.is_ignore();
            }
            file = current.outer.0.as_deref();
        }
        false
    }
}

/// Calls `visit` once for every file the paths expand to, as soon as it is
/// found, with its absolute path. `visit` runs on several threads at once.
///
/// Patterns support `*`, `?`, `[...]`, `{a,b}` alternatives and `**` for any
/// number of directories. With `skip_ignored`, directories are walked the
/// way ripgrep does by default: hidden files and directories are skipped,
/// and so is what `.gitignore` files in a Git repository exclude. Files
/// passed by name are always visited. Symbolic links are followed only to
/// files. Paths are made absolute lexically: `.` and `..` are removed,
/// symbolic links are not resolved.
pub fn visit_files<F>(
    paths: &[PathBuf],
    default_globs: &[Cow<'static, str>],
    skip_ignored: bool,
    visit: F,
) where
    F: Fn(PathBuf) + Sync,
{
    Search::new(paths, default_globs, skip_ignored).visit(visit);
}

/// All files the paths expand to, as absolute paths in sorted order; see
/// [`visit_files`].
pub fn find_files(
    paths: &[PathBuf],
    default_globs: &[Cow<'static, str>],
    skip_ignored: bool,
) -> Vec<PathBuf> {
    let found = Mutex::new(Vec::new());
    visit_files(paths, default_globs, skip_ignored, |path| {
        found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(path);
    });
    let mut found = found.into_inner().unwrap_or_else(PoisonError::into_inner);
    found.sort_unstable();
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

/// What `apply_changes` needs besides the paths.
pub struct Options<'a> {
    pub processor: &'a Processor,
    pub console: &'a Console,
    pub default_globs: &'a [Cow<'static, str>],
    /// Whether directory walks skip hidden files and what `.gitignore`
    /// files exclude; see [`visit_files`].
    pub skip_ignored: bool,
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

/// How processing a file went, and what it has to print.
struct Report {
    path: PathBuf,
    result: FileResult,
    /// Lines for stdout.
    output: String,
    /// Lines for stderr.
    errors: String,
}

/// Process every file the paths expand to, as soon as it is found: sort its
/// classes and write the result back in write mode, or report what would
/// change. The reports are printed at the end, in path order.
///
/// Reading files is the larger part of the work for most projects, and on
/// macOS it slows down beyond a handful of threads; the binary limits the
/// thread pool accordingly.
pub fn apply_changes(paths: &[PathBuf], options: &Options<'_>) -> Summary {
    let reports = Mutex::new(Vec::new());
    visit_files(paths, options.default_globs, options.skip_ignored, |path| {
        let report = process_file(path, options);
        reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(report);
    });
    let mut reports = reports.into_inner().unwrap_or_else(PoisonError::into_inner);
    reports.sort_unstable_by(|a, b| a.path.cmp(&b.path));

    let mut summary = Summary {
        found_any: !reports.is_empty(),
        ..Summary::default()
    };
    let mut output = String::new();
    let mut errors = String::new();
    for report in &reports {
        if report.result.failed {
            summary.failed += 1;
        }
        if report.result.skipped {
            summary.skipped += 1;
        } else {
            summary.changed += 1;
        }
        output.push_str(&report.output);
        errors.push_str(&report.errors);
    }
    options.console.write_output(&output);
    options.console.write_errors(&errors);
    summary
}

fn process_file(path: PathBuf, options: &Options<'_>) -> Report {
    let console = options.console;
    let mut report = Report {
        path,
        result: FileResult {
            skipped: true,
            changed: false,
            failed: false,
        },
        output: String::new(),
        errors: String::new(),
    };
    let path = report.path.display();
    let filename = console.style(style::WHITE, &path);

    // Writing to a String cannot fail
    let Ok(Ok(old_text)) = fs::read(&report.path).map(String::from_utf8) else {
        let _ = writeln!(
            report.errors,
            "{} {filename}",
            console.style(style::RED, "Unable to read")
        );
        return report;
    };

    let new_text = options.processor.process_text(&old_text);
    if matches!(new_text, Cow::Borrowed(_)) || new_text == old_text {
        if options.verbosity >= verbosity::VERBOSE {
            let _ = writeln!(
                report.output,
                "{}",
                console.style(style::GREY, &format_args!("Already sorted {path}"))
            );
        }
        return report;
    }

    if options.write_mode
        && let Err(error) = fs::write(&report.path, new_text.as_bytes())
    {
        let _ = writeln!(
            report.errors,
            "{} {filename}: {error}",
            console.style(style::RED, "Unable to write")
        );
        report.result.failed = true;
        return report;
    }

    report.result = FileResult {
        skipped: false,
        changed: true,
        failed: false,
    };
    if options.verbosity >= verbosity::NORMAL {
        let action = if options.write_mode {
            "Updated"
        } else {
            "Would update"
        };
        let _ = writeln!(
            report.output,
            "{} {filename}",
            console.style(style::DIM, action)
        );
        if options.verbosity >= verbosity::DIFF {
            report
                .output
                .push_str(&render_diff(&report.path, &old_text, &new_text, console));
            report.output.push('\n');
        }
    }
    report
}

/// A unified diff with one line of context, in the format of Python's
/// `difflib`, indented and padded with blank lines like the Python tool
/// prints it. Every line is stripped of surrounding whitespace, which also
/// removes the leading space of context lines.
pub fn render_diff(path: &Path, old_text: &str, new_text: &str, console: &Console) -> String {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let diff = TextDiff::from_slices(&old_lines, &new_lines);

    // Writing to a String cannot fail
    let mut out = String::from("\n");
    let path = path.display();
    let _ = writeln!(
        out,
        "    {}",
        console.style(style::BRIGHT_RED, &format_args!("--- {path}"))
    );
    let _ = writeln!(
        out,
        "    {}",
        console.style(style::GREEN, &format_args!("+++ {path}"))
    );

    for group in diff.grouped_ops(1) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let header = format_args!(
            "@@ -{} +{} @@",
            format_range(first.old_range().start, last.old_range().end),
            format_range(first.new_range().start, last.new_range().end),
        );
        let _ = writeln!(out, "    {}", console.style(style::BOLD_MAGENTA, &header));

        for op in &group {
            for change in diff.iter_changes(op) {
                let (marker, color) = match change.tag() {
                    ChangeTag::Equal => (" ", None),
                    ChangeTag::Delete => ("-", Some(style::BRIGHT_RED)),
                    ChangeTag::Insert => ("+", Some(style::GREEN)),
                };
                let line = format!("{marker}{}", change.value());
                let line = line.trim();
                let _ = match color {
                    Some(color) => writeln!(out, "    {}", console.style(color, line)),
                    None => writeln!(out, "    {line}"),
                };
            }
        }
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
