//! End-to-end tests of the binary: stdin mode, file mode, writing,
//! configuration precedence and file discovery. Ported from the Python
//! test suite.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn output(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Run the binary. Without `stdin`, stdin is empty but not a terminal, so
/// the tool behaves as if piped empty input.
fn tailwhip(args: &[&str], cwd: &Path, stdin: Option<&str>) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tailwhip"))
        .args(args)
        .current_dir(cwd)
        .env_remove("FORCE_COLOR")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary runs");
    {
        let mut pipe = child.stdin.take().unwrap();
        if let Some(input) = stdin {
            pipe.write_all(input.as_bytes()).unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

// =============================================================================
// Stdin mode
// =============================================================================

#[test]
fn stdin_sorts_html() {
    let tmp = TempDir::new().unwrap();
    let run = tailwhip(
        &[],
        tmp.path(),
        Some(r#"<div class="p-4 m-2 bg-white text-lg font-bold"></div>"#),
    );
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        r#"<div class="m-2 p-4 font-bold text-lg bg-white"></div>"#
    );
}

#[test]
fn stdin_sorts_css() {
    let tmp = TempDir::new().unwrap();
    let run = tailwhip(
        &[],
        tmp.path(),
        Some(".btn { @apply text-white p-4 m-2 rounded bg-blue-500; }"),
    );
    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        ".btn { @apply m-2 p-4 text-white bg-blue-500 rounded; }"
    );
}

#[test]
fn stdin_empty_input() {
    let tmp = TempDir::new().unwrap();
    let run = tailwhip(&[], tmp.path(), Some(""));
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "");
}

#[test]
fn stdin_preserves_multiline_formatting() {
    let tmp = TempDir::new().unwrap();
    let input = "<div class=\"p-4 m-2 bg-white\">\n    <span class=\"font-bold text-lg text-gray-900\"></span>\n</div>";
    let expected = "<div class=\"m-2 p-4 bg-white\">\n    <span class=\"font-bold text-lg text-gray-900\"></span>\n</div>";
    let run = tailwhip(&[], tmp.path(), Some(input));
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, expected);
}

#[test]
fn stdin_preserves_line_endings_and_bom() {
    let tmp = TempDir::new().unwrap();
    let input = "\u{feff}<div class=\"p-4 m-2\">\r\n</div>\r\n";
    let run = tailwhip(&[], tmp.path(), Some(input));
    assert_eq!(run.stdout, "\u{feff}<div class=\"m-2 p-4\">\r\n</div>\r\n");
}

#[test]
fn stdin_passes_invalid_utf8_through() {
    let tmp = TempDir::new().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tailwhip"))
        .current_dir(tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"\xff\xfe<div class=\"p-4 m-2\">")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"\xff\xfe<div class=\"p-4 m-2\">");
    assert!(String::from_utf8_lossy(&output.stderr).contains("not valid UTF-8"));
}

// =============================================================================
// Basic arguments
// =============================================================================

#[test]
fn help_and_version() {
    let tmp = TempDir::new().unwrap();
    for flag in ["-h", "--help"] {
        let run = tailwhip(&[flag], tmp.path(), None);
        assert_eq!(run.code, 0);
        assert!(
            run.stdout
                .contains("Sort Tailwind CSS classes in HTML and CSS files.")
        );
        assert!(run.stdout.contains("--configuration"));
    }
    for flag in ["-V", "--version"] {
        let run = tailwhip(&[flag], tmp.path(), None);
        assert_eq!(run.code, 0);
        assert_eq!(
            run.stdout,
            format!("tailwhip {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn invalid_arguments_are_usage_errors() {
    let tmp = TempDir::new().unwrap();
    for (args, message) in [
        (&["--bogus"][..], "'--bogus'"),
        (&["-x"], "'-x'"),
        (&["-c"], "'-c'"),
    ] {
        let run = tailwhip(args, tmp.path(), None);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.stderr.contains(message), "{}", run.stderr);
        assert!(run.stderr.contains("--help"), "{}", run.stderr);
    }
    // Short flags combine, and values attach in either form
    write(tmp.path(), "a.html", r#"<p class="p-4 m-2">"#);
    write(tmp.path(), "c.toml", "verbosity = 1\n");
    for args in [
        &["-wvq", "a.html"][..],
        &["a.html", "-cc.toml", "--configuration=c.toml"],
    ] {
        assert_eq!(tailwhip(args, tmp.path(), None).code, 0, "{args:?}");
    }
    assert_eq!(
        fs::read_to_string(tmp.path().join("a.html")).unwrap(),
        r#"<p class="m-2 p-4">"#
    );
}

#[test]
fn no_arguments_reads_empty_stdin() {
    let tmp = TempDir::new().unwrap();
    let run = tailwhip(&[], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(run.output(), "");
}

#[test]
fn no_files_found_is_an_error() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join("empty")).unwrap();
    let run = tailwhip(&["empty"], tmp.path(), None);
    assert_eq!(run.code, 1);
    assert!(run.output().contains("No files found"));

    // Quiet mode still reports the error
    let run = tailwhip(&["empty", "-q"], tmp.path(), None);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("No files found"));
    assert_eq!(run.stdout, "");
}

#[test]
fn nonexistent_configuration_file_is_an_error() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "test.html",
        r#"<div class="p-4 m-2">Test</div>"#,
    );
    let run = tailwhip(
        &["test.html", "--configuration", "nonexistent_config.toml"],
        tmp.path(),
        None,
    );
    assert_eq!(run.code, 1);
    assert!(run.output().to_lowercase().contains("not found"));
    assert!(run.output().contains("nonexistent_config.toml"));
    assert!(!run.output().contains("panicked"));
}

#[test]
fn invalid_configuration_is_reported_cleanly() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "test.html",
        r#"<div class="p-4 m-2">Test</div>"#,
    );
    write(tmp.path(), "bad.toml", "verbosity = [1, 2]\n");
    let run = tailwhip(&["test.html", "-c", "bad.toml"], tmp.path(), None);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("Invalid configuration in bad.toml"));

    write(
        tmp.path(),
        "badregex.toml",
        "[[class_patterns]]\nname = \"x\"\nregex = \"(\"\n",
    );
    let run = tailwhip(&["test.html", "-c", "badregex.toml"], tmp.path(), None);
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("Invalid class pattern 'x'"));

    // The Python tool's pattern format, with a template, is rejected clearly
    write(
        tmp.path(),
        "old.toml",
        "[[class_patterns]]\nname = \"x\"\nregex = \"(?P<classes>.*)\"\ntemplate = \"{classes}\"\n",
    );
    let run = tailwhip(&["test.html", "-c", "old.toml"], tmp.path(), None);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr.contains("Invalid configuration in old.toml"),
        "{}",
        run.stderr
    );
    assert!(run.stderr.contains("template"), "{}", run.stderr);
}

#[test]
fn runs_without_any_pyproject_in_reach() {
    let tmp = TempDir::new().unwrap();
    let deep = tmp.path().join("deep/nested/directory");
    fs::create_dir_all(&deep).unwrap();
    let run = tailwhip(&[], &deep, Some(r#"<p class="p-4 m-2">"#));
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, r#"<p class="m-2 p-4">"#);
}

// =============================================================================
// File mode and writing
// =============================================================================

const UNSORTED_HTML: &str = r#"<style>
    .btn { @apply px-4 py-2 rounded-lg font-semibold
                  bg-blue-500 text-white hover:bg-blue-600
                  focus:ring-2 active:scale-95; }
    .card { @apply shadow-lg p-6 rounded-xl bg-white border-2 border-gray-200 hover:shadow-xl; }
</style>
<div class="p-4 mx-auto container max-w-4xl
            lg:max-w-6xl dark:bg-gray-900">
    <h1 class="font-bold text-3xl mb-4 text-gray-900 dark:text-white sm:text-4xl md:text-5xl">Title</h1>
    <button class="transition-all px-6 py-3 rounded-md font-medium bg-indigo-500 text-white hover:bg-indigo-600 focus:ring-2 focus:ring-indigo-500 active:scale-95 disabled:opacity-50">Click</button>
</div>
"#;

const SORTED_HTML: &str = r#"<style>
    .btn { @apply px-4 py-2 font-semibold text-white bg-blue-500 rounded-lg hover:bg-blue-600 focus:ring-2 active:scale-95; }
    .card { @apply p-6 bg-white rounded-xl border-2 border-gray-200 shadow-lg hover:shadow-xl; }
</style>
<div class="container max-w-4xl mx-auto p-4 dark:bg-gray-900 lg:max-w-6xl">
    <h1 class="mb-4 font-bold text-3xl text-gray-900 dark:text-white sm:text-4xl md:text-5xl">Title</h1>
    <button class="px-6 py-3 font-medium text-white bg-indigo-500 rounded-md transition-all disabled:opacity-50 hover:bg-indigo-600 focus:ring-2 focus:ring-indigo-500 active:scale-95">Click</button>
</div>
"#;

#[test]
fn write_mode_sorts_and_saves() {
    let tmp = TempDir::new().unwrap();
    let file = write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(&["test.html", "-vv", "--write"], tmp.path(), None);
    assert_eq!(run.code, 0, "{}", run.output());
    assert_eq!(fs::read_to_string(&file).unwrap(), SORTED_HTML);
    assert!(run.stdout.contains("Updated "));
    assert!(
        run.stdout.contains("@@ "),
        "diff shown at -vv: {}",
        run.stdout
    );
    assert!(!run.stdout.contains("Dry Run"));
}

#[test]
fn dry_run_does_not_modify_the_file() {
    let tmp = TempDir::new().unwrap();
    let file = write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(&["test.html", "-vv"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(fs::read_to_string(&file).unwrap(), UNSORTED_HTML);
    assert!(run.stdout.contains("Would update "));
    assert!(run.stdout.contains("Dry Run"));
    assert!(run.stdout.contains("Completed in"));
    assert!(run.stdout.contains("for 1 files. (0 skipped)"));
}

#[test]
fn write_mode_processes_a_directory() {
    let tmp = TempDir::new().unwrap();
    let file1 = write(
        tmp.path(),
        "file1.html",
        r#"<div class="p-4 m-2 bg-white"></div>"#,
    );
    let file2 = write(
        tmp.path(),
        "file2.html",
        r#"<div class="text-lg font-bold text-gray-900"></div>"#,
    );
    let run = tailwhip(&["."], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout.matches("Would update").count(), 2);

    let run = tailwhip(&[".", "--write"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(
        fs::read_to_string(&file1).unwrap(),
        r#"<div class="m-2 p-4 bg-white"></div>"#
    );
    assert_eq!(
        fs::read_to_string(&file2).unwrap(),
        r#"<div class="font-bold text-lg text-gray-900"></div>"#
    );
}

#[test]
fn already_sorted_files_are_reported_when_verbose() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "sorted.html", r#"<div class="m-2 p-4"></div>"#);
    let run = tailwhip(&["sorted.html", "-v"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert!(run.stdout.contains("Already sorted"));
    assert!(run.stdout.contains("1 skipped"));

    // Not reported at the default verbosity
    let run = tailwhip(&["sorted.html"], tmp.path(), None);
    assert_eq!(run.output(), "");
}

#[test]
fn unreadable_files_are_skipped() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("binary.html"),
        b"\xff\xfe<div class=\"p-4 m-2\"></div>",
    )
    .unwrap();
    let run = tailwhip(&["binary.html", "-v"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert!(run.output().contains("Unable to read"));
    assert!(run.stdout.contains("1 skipped"));
}

#[test]
fn quiet_suppresses_regular_output() {
    let tmp = TempDir::new().unwrap();
    let file = write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(
        &["test.html", "-vvv", "--quiet", "--write"],
        tmp.path(),
        None,
    );
    assert_eq!(run.code, 0);
    assert_eq!(run.output(), "");
    assert_eq!(fs::read_to_string(&file).unwrap(), SORTED_HTML);
}

#[test]
fn files_keep_their_line_endings() {
    let tmp = TempDir::new().unwrap();
    let file = write(
        tmp.path(),
        "crlf.html",
        "<div class=\"p-4 m-2\">\r\n</div>\r\n",
    );
    let run = tailwhip(&["crlf.html", "--write"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "<div class=\"m-2 p-4\">\r\n</div>\r\n"
    );
}

// =============================================================================
// Configuration precedence: defaults < pyproject.toml < custom file < CLI
// =============================================================================

const PYPROJECT: &str = r#"
[tool.tailwhip]
custom_colors = ["brand", "accent"]
skip_expressions = ["{{", "{%", "[["]
"#;

const CUSTOM_CONFIG: &str = r#"
custom_colors = ["custom1", "custom2"]
skip_expressions = ["<%", "%>"]
"#;

/// "brand" sorts before "red" only when it is a known color
const COLOR_PROBE: &str = r#"<div class="text-red-500 text-brand">"#;

#[test]
fn defaults_apply_without_configuration() {
    let tmp = TempDir::new().unwrap();
    let run = tailwhip(&[], tmp.path(), Some(COLOR_PROBE));
    assert_eq!(run.stdout, r#"<div class="text-red-500 text-brand">"#);
    // Default skip expressions
    let run = tailwhip(
        &[],
        tmp.path(),
        Some(r#"<div class="p-4 m-2 [[x]]"> <p class="p-4 m-2 <% x %>">"#),
    );
    // "[[x]]" is an unknown class and sorts first; "<%" is skipped
    assert_eq!(
        run.stdout,
        r#"<div class="[[x]] m-2 p-4"> <p class="p-4 m-2 <% x %>">"#
    );
}

#[test]
fn pyproject_overrides_defaults() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pyproject.toml", PYPROJECT);
    let run = tailwhip(&[], tmp.path(), Some(COLOR_PROBE));
    assert_eq!(run.stdout, r#"<div class="text-brand text-red-500">"#);
    // "[[" is now skipped, "<%" no longer is
    let run = tailwhip(
        &[],
        tmp.path(),
        Some(r#"<div class="p-4 m-2 [[x]]"> <p class="p-4 m-2 <% x %>">"#),
    );
    assert_eq!(
        run.stdout,
        r#"<div class="p-4 m-2 [[x]]"> <p class="%> <% x m-2 p-4">"#
    );
}

#[test]
fn pyproject_is_found_in_parent_directories() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pyproject.toml", PYPROJECT);
    let nested = tmp.path().join("src/templates");
    fs::create_dir_all(&nested).unwrap();
    let run = tailwhip(&[], &nested, Some(COLOR_PROBE));
    assert_eq!(run.stdout, r#"<div class="text-brand text-red-500">"#);
}

#[test]
fn skip_expressions_from_pyproject_leave_lists_alone() {
    let project = TempDir::new().unwrap();
    write(
        project.path(),
        "pyproject.toml",
        "[tool.tailwhip]\nskip_expressions = [\"&\"]\n",
    );
    let alpine = r#"<div :class="open && 'p-4 m-2'" class="p-4 m-2">"#;
    let run = tailwhip(&[], project.path(), Some(alpine));
    assert_eq!(
        run.stdout,
        r#"<div :class="open && 'p-4 m-2'" class="m-2 p-4">"#
    );
    // With the default skip expressions, the Alpine expression is sorted
    let elsewhere = TempDir::new().unwrap();
    let run = tailwhip(&[], elsewhere.path(), Some(alpine));
    assert!(!run.stdout.contains("open && 'p-4"), "{}", run.stdout);
}

#[test]
fn pyproject_is_found_from_the_paths_not_the_working_directory() {
    let project = TempDir::new().unwrap();
    write(
        project.path(),
        "pyproject.toml",
        "[tool.tailwhip]\nskip_expressions = [\"&\"]\n",
    );
    let page = write(
        project.path(),
        "templates/page.html",
        r#"<div :class="open && 'p-4 m-2'">"#,
    );
    let elsewhere = TempDir::new().unwrap();
    let templates = project.path().join("templates");
    let glob = format!("{}/**/*.html", project.path().display());
    for arg in [
        templates.to_str().unwrap(),
        page.to_str().unwrap(),
        glob.as_str(),
    ] {
        let run = tailwhip(&[arg, "-v"], elsewhere.path(), None);
        assert!(
            run.stdout.contains("Already sorted"),
            "{arg}: {}",
            run.output()
        );
    }
}

#[test]
fn pyproject_without_tool_section_is_ignored() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pyproject.toml", "[project]\nname = \"x\"\n");
    let run = tailwhip(&[], tmp.path(), Some(COLOR_PROBE));
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, r#"<div class="text-red-500 text-brand">"#);
}

#[test]
fn custom_file_overrides_defaults_and_pyproject() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pyproject.toml", PYPROJECT);
    write(tmp.path(), "tailwhip.toml", CUSTOM_CONFIG);
    let probe = r#"<div class="text-red-500 text-brand text-custom1"> <p class="p-4 m-2 [[x]]"> <p class="p-4 m-2 <% x %>">"#;
    let run = tailwhip(
        &["--configuration", "tailwhip.toml"],
        tmp.path(),
        Some(probe),
    );
    assert_eq!(run.code, 0);
    // custom1 is a color, brand is not any more; "[[" is sorted, "<%" skipped
    assert_eq!(
        run.stdout,
        r#"<div class="text-custom1 text-red-500 text-brand"> <p class="[[x]] m-2 p-4"> <p class="p-4 m-2 <% x %>">"#
    );
}

#[test]
fn cli_arguments_override_configuration_files() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        "[tool.tailwhip]\nverbosity = 0\nwrite_mode = false\n",
    );
    write(
        tmp.path(),
        "tailwhip.toml",
        "verbosity = 0\nwrite_mode = false\n",
    );
    let file = write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(
        &["test.html", "-c", "tailwhip.toml", "--write", "-vvv"],
        tmp.path(),
        None,
    );
    assert_eq!(run.code, 0);
    assert_eq!(fs::read_to_string(&file).unwrap(), SORTED_HTML);
    assert!(run.stdout.contains("Updated "));
    assert!(run.stdout.contains("Completed in"));
}

#[test]
fn quiet_overrides_verbose() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(&["test.html", "-vvv", "--quiet"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(run.stdout, "");
}

#[test]
fn verbosity_and_write_mode_from_configuration_files_are_honored() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        "[tool.tailwhip]\nverbosity = 2\n",
    );
    write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(&["test.html"], tmp.path(), None);
    assert!(
        run.stdout.contains("Dry Run"),
        "verbosity 2 from pyproject shows the summary"
    );

    write(tmp.path(), "write.toml", "write_mode = true\n");
    let file = write(tmp.path(), "test.html", UNSORTED_HTML);
    let run = tailwhip(&["test.html", "-c", "write.toml"], tmp.path(), None);
    assert_eq!(run.code, 0);
    assert_eq!(fs::read_to_string(&file).unwrap(), SORTED_HTML);
}

#[test]
fn custom_patterns_from_pyproject() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        r#"
[[tool.tailwhip.class_patterns]]
name = "jsx_classname"
regex = '\bclassName\s*=\s*"(?P<classes>[^"]*)"'
"#,
    );
    let input = "<Component className=\"p-4 m-2 flex\" />\n<div class=\"p-4 m-2 flex\"></div>\n.btn { @apply p-4 m-2 flex; }";
    let run = tailwhip(&[], tmp.path(), Some(input));
    assert_eq!(
        run.stdout,
        "<Component className=\"flex m-2 p-4\" />\n<div class=\"flex m-2 p-4\"></div>\n.btn { @apply flex m-2 p-4; }"
    );
}

#[test]
fn text_around_the_classes_is_kept_as_written() {
    let tmp = TempDir::new().unwrap();
    let input = "<div CLASS = 'p-4 m-2'>\n<p class=\"p-4 before:content-['★'] m-2\">\n.a { @APPLY   p-4 m-2 ; }";
    let run = tailwhip(&[], tmp.path(), Some(input));
    assert_eq!(
        run.stdout,
        "<div CLASS = 'm-2 p-4'>\n<p class=\"m-2 p-4 before:content-['★']\">\n.a { @APPLY   m-2 p-4; }"
    );
}

// =============================================================================
// File discovery
// =============================================================================

/// The fixture tree of the Python file finder tests, with empty files, so
/// every discovered file is reported as "Already sorted" at -v.
fn fixture_tree() -> TempDir {
    let tmp = TempDir::new().unwrap();
    for name in [
        "index.html",
        "styles.css",
        "theme.pcss",
        "utilities.postcss",
        "app.less",
        "templates/page.html",
        "styles/components/button.scss",
        "styles/components/card.sass",
        ".hidden/secret.html",
    ] {
        write(tmp.path(), name, "");
    }
    fs::create_dir(tmp.path().join("nested")).unwrap();
    fs::create_dir(tmp.path().join("empty")).unwrap();
    tmp
}

/// The file names the tool found for the given path arguments.
fn found(args: &[&str], cwd: &Path) -> Vec<String> {
    let run = tailwhip(&[args, &["-v"]].concat(), cwd, None);
    let mut names: Vec<String> = run
        .stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Already sorted "))
        .map(|path| {
            Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn finds_files_in_directories() {
    let tmp = fixture_tree();
    // Hidden directories are skipped, unless asked for
    assert_eq!(
        found(&["."], tmp.path()),
        ["index.html", "page.html", "styles.css"]
    );
    assert_eq!(
        found(&[".", "--no-ignore"], tmp.path()),
        ["index.html", "page.html", "secret.html", "styles.css"]
    );
    assert_eq!(found(&[".hidden"], tmp.path()), ["secret.html"]);
    assert_eq!(found(&["templates/"], tmp.path()), ["page.html"]);
    assert_eq!(
        found(
            &[tmp.path().join("templates").to_str().unwrap()],
            tmp.path()
        ),
        ["page.html"]
    );
    // No HTML or CSS files below
    assert!(found(&["styles/"], tmp.path()).is_empty());
    assert!(found(&["empty/"], tmp.path()).is_empty());
    // A directory name that only exists deeper in the tree
    assert!(found(&["components"], tmp.path()).is_empty());
}

#[test]
fn finds_specific_files_of_any_extension() {
    let tmp = fixture_tree();
    assert_eq!(found(&["index.html"], tmp.path()), ["index.html"]);
    assert_eq!(found(&["styles.css"], tmp.path()), ["styles.css"]);
    assert_eq!(found(&["theme.pcss"], tmp.path()), ["theme.pcss"]);
    assert_eq!(found(&["./index.html"], tmp.path()), ["index.html"]);
}

#[test]
fn finds_files_by_glob() {
    let tmp = fixture_tree();
    assert_eq!(found(&["templates/*.html"], tmp.path()), ["page.html"]);
    assert_eq!(
        found(&["**/*.html"], tmp.path()),
        ["index.html", "page.html"]
    );
    assert_eq!(
        found(&["**/*.html", "--no-ignore"], tmp.path()),
        ["index.html", "page.html", "secret.html"]
    );
    assert_eq!(
        found(&["*.css", "*.pcss", "*.postcss"], tmp.path()),
        ["styles.css", "theme.pcss", "utilities.postcss"]
    );
    assert_eq!(
        found(&["styles/**/*.{scss,sass}"], tmp.path()),
        ["button.scss", "card.sass"]
    );
    assert_eq!(
        found(&["*/*/*.s[ac]ss"], tmp.path()),
        ["button.scss", "card.sass"]
    );
    assert!(found(&["nonexistent/*.html"], tmp.path()).is_empty());
    assert!(found(&["*.css"], &tmp.path().join("templates")).is_empty());
}

#[test]
fn gitignore_applies_inside_a_repository() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    for name in [
        "app/page.html",
        "app/build/out.html",
        "node_modules/pkg/index.html",
        "static/kept.css",
        "static/vendor.css",
        "nested/.gitignore",
        "nested/a.html",
        "nested/b.html",
    ] {
        write(&repo, name, "");
    }
    fs::create_dir(repo.join(".git")).unwrap();
    write(
        &repo,
        ".gitignore",
        "node_modules\nbuild/\n/static/*.css\n!/static/kept.css\n",
    );
    write(&repo, "nested/.gitignore", "a.html\n");

    assert_eq!(found(&["."], &repo), ["b.html", "kept.css", "page.html"]);
    // From a subdirectory, the .gitignore files above it still apply
    assert_eq!(found(&["."], &repo.join("app")), ["page.html"]);
    assert_eq!(found(&["app"], &repo), ["page.html"]);
    // Ignored directories passed by name are walked, and files passed by
    // name are always processed
    assert_eq!(found(&["node_modules"], &repo), ["index.html"]);
    assert_eq!(found(&["nested/a.html"], &repo), ["a.html"]);
    assert_eq!(
        found(&[".", "--no-ignore"], &repo),
        [
            "a.html",
            "b.html",
            "index.html",
            "kept.css",
            "out.html",
            "page.html",
            "vendor.css"
        ]
    );

    // Outside of a repository, .gitignore files mean nothing
    fs::remove_dir(repo.join(".git")).unwrap();
    assert_eq!(found(&["nested"], &repo), ["a.html", "b.html"]);
}

#[test]
fn deduplicates_and_combines_paths() {
    let tmp = fixture_tree();
    assert_eq!(
        found(&["index.html", "./index.html", "index.html"], tmp.path()),
        ["index.html"]
    );
    assert_eq!(
        found(&["index.html", "templates/", "*.css"], tmp.path()),
        ["index.html", "page.html", "styles.css"]
    );
}

#[test]
fn globs_matching_directories_yield_only_files() {
    let tmp = fixture_tree();
    let names = found(&["*"], tmp.path());
    assert!(names.contains(&"index.html".to_string()));
    assert!(!names.contains(&"templates".to_string()));
    assert!(!names.contains(&"empty".to_string()));
}

#[test]
fn reported_paths_are_absolute() {
    let tmp = fixture_tree();
    let run = tailwhip(
        &["./templates/../templates/page.html", "-v"],
        tmp.path(),
        None,
    );
    let line = run
        .stdout
        .lines()
        .find(|l| l.starts_with("Already sorted "))
        .expect("file reported");
    let path = line.trim_start_matches("Already sorted ");
    // Absolute, normalized, below the temporary directory (which the OS may
    // report through a symlink such as /private/var)
    assert!(path.starts_with('/'), "{path}");
    assert!(path.ends_with("/templates/page.html"), "{path}");
    assert!(!path.contains("/../") && !path.contains("/./"), "{path}");
    assert!(
        path.ends_with(
            tmp.path()
                .join("templates/page.html")
                .to_str()
                .unwrap()
                .trim_start_matches("/private")
        ),
        "{path}"
    );
}
