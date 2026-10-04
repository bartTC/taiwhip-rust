//! Configuration management.
//!
//! Settings are loaded in this order, later sources overriding earlier ones:
//!
//! 1. the built-in defaults (`configuration.toml`, embedded in the binary)
//! 2. the `[tool.tailwhip]` section of the nearest `pyproject.toml`
//! 3. a custom configuration file passed via `--configuration`
//! 4. command line arguments
//!
//! Every key overrides its default as a whole: lists replace, they are not
//! merged. This matches the Python tool, which loads the files through
//! Dynaconf with merging disabled.

use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The built-in default configuration, the single source of truth for the
/// sort order. It is the same file the Python package ships.
pub const DEFAULT_CONFIGURATION: &str = include_str!("../configuration.toml");

/// A list of strings: borrowed from the binary for the built-in defaults,
/// so that loading them allocates nothing, and owned when read from a file.
pub type List = Vec<Cow<'static, str>>;

/// A pattern that finds class lists in text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternSpec {
    pub name: String,
    /// A regex (Rust syntax) with a `classes` group; the text that group
    /// matches is replaced with the sorted classes.
    pub regex: String,
}

/// Output verbosity levels, as used by the file mode.
pub mod verbosity {
    pub const QUIET: u8 = 0;
    /// Default: report files that change.
    pub const NORMAL: u8 = 1;
    /// Also report unchanged files and the summary.
    pub const VERBOSE: u8 = 2;
    /// Also show a diff for each changed file.
    pub const DIFF: u8 = 3;
}

/// The complete configuration. Every field has a value once the defaults are
/// loaded, so later layers only need to provide the keys they change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Config {
    pub verbosity: u8,
    pub write_mode: bool,
    pub default_globs: List,
    pub skip_expressions: List,
    pub variant_separator: String,
    pub class_patterns: Vec<PatternSpec>,
    pub component_order: List,
    pub variants: List,
    pub prefixes: List,
    pub directions: List,
    pub sizes: List,
    pub numerics: List,
    pub colors: List,
    pub custom_colors: List,
    pub shades: List,
    pub alphas: List,
}

/// The keys a configuration layer may override. Unknown keys are ignored,
/// as Dynaconf ignores them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub verbosity: Option<u8>,
    pub write_mode: Option<bool>,
    pub default_globs: Option<List>,
    pub skip_expressions: Option<List>,
    pub variant_separator: Option<String>,
    pub class_patterns: Option<Vec<PatternSpec>>,
    pub component_order: Option<List>,
    pub variants: Option<List>,
    pub prefixes: Option<List>,
    pub directions: Option<List>,
    pub sizes: Option<List>,
    pub numerics: Option<List>,
    pub colors: Option<List>,
    pub custom_colors: Option<List>,
    pub shades: Option<List>,
    pub alphas: Option<List>,
}

/// Why a configuration file could not be used.
#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, source } => {
                write!(f, "Unable to read {}: {source}", path.display())
            }
            ConfigError::Parse { path, message } => {
                write!(f, "Invalid configuration in {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Default for Config {
    /// The built-in defaults, converted from `configuration.toml` to Rust
    /// code at build time (see `build.rs`).
    fn default() -> Self {
        fn strings(items: &[&'static str]) -> List {
            items.iter().copied().map(Cow::Borrowed).collect()
        }
        include!(concat!(env!("OUT_DIR"), "/default_config.rs"))
    }
}

impl Config {
    /// Replace every setting for which the overrides carry a value.
    pub fn apply(&mut self, overrides: Overrides) {
        macro_rules! apply {
            ($($field:ident),* $(,)?) => {
                $( if let Some(value) = overrides.$field { self.$field = value; } )*
            };
        }
        apply!(
            verbosity,
            write_mode,
            default_globs,
            skip_expressions,
            variant_separator,
            class_patterns,
            component_order,
            variants,
            prefixes,
            directions,
            sizes,
            numerics,
            colors,
            custom_colors,
            shades,
            alphas,
        );
    }

    /// Apply the top-level keys of a TOML document, as found in a custom
    /// configuration file. `path` is only used in error messages.
    pub fn apply_toml(&mut self, text: &str, path: &Path) -> Result<(), ConfigError> {
        let table = parse_table(text, path)?;
        self.apply(overrides_from_table(table, path)?);
        Ok(())
    }

    /// Apply a custom configuration file.
    pub fn apply_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let text = read(path)?;
        self.apply_toml(&text, path)
    }

    /// Apply the `[tool.tailwhip]` section of a `pyproject.toml`. Returns
    /// whether the file had such a section.
    ///
    /// A `pyproject.toml` is mostly other tools' settings, and this runs on
    /// every start. Parsing only the section took 60 µs where the whole
    /// file of a Django project took 340, so the section is cut out when it
    /// has a header of its own. A file in which `tailwhip` is never a key is
    /// not parsed at all.
    pub fn apply_pyproject(&mut self, path: &Path) -> Result<bool, ConfigError> {
        let text = read(path)?;
        let table = match tailwhip_section(&text).and_then(|section| section.parse().ok()) {
            Some(table) => table,
            None if mentions_key(&text) => parse_table(&text, path)?,
            None => return Ok(false),
        };
        let Some(section) = tool_section(table) else {
            return Ok(false);
        };
        self.apply(overrides_from_table(section, path)?);
        Ok(true)
    }
}

/// Find the nearest `pyproject.toml`, looking in `start` first and then in
/// each parent of its resolved path.
pub fn find_pyproject(start: &Path) -> Option<PathBuf> {
    let candidate = start.join("pyproject.toml");
    if candidate.is_file() {
        return Some(candidate);
    }
    let resolved = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    resolved
        .ancestors()
        .skip(1)
        .map(|dir| dir.join("pyproject.toml"))
        .find(|candidate| candidate.is_file())
}

fn read(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_table(text: &str, path: &Path) -> Result<toml::Table, ConfigError> {
    text.parse::<toml::Table>()
        .map_err(|error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: error.message().to_string(),
        })
}

/// The `[tool.tailwhip]` section of a `pyproject.toml` with its header and
/// its sub-tables, up to the next table header, if it has a header of its
/// own.
///
/// Only a line starting with `[` ends the section. Inside the section, such
/// a line can only be part of a multi-line array or string, so cutting there
/// leaves an unclosed one: the cut-out text fails to parse, and the whole
/// file is parsed instead.
fn tailwhip_section(text: &str) -> Option<&str> {
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        match start {
            None if trimmed.starts_with("[tool.tailwhip]") => start = Some(offset),
            Some(start)
                if trimmed.starts_with('[')
                    && !trimmed.starts_with("[tool.tailwhip.")
                    && !trimmed.starts_with("[[tool.tailwhip.") =>
            {
                return Some(&text[start..offset]);
            }
            _ => {}
        }
        offset += line.len();
    }
    start.map(|start| &text[start..])
}

/// Whether `tailwhip` is a key anywhere in a TOML document, written in any
/// way, with optional quotes and spaces: after a `.` and followed by `.`,
/// `=` or `]` (`[tool."tailwhip"]`), or at the start of a line and followed
/// by `.` or `=` (`tailwhip.verbosity = 2` in `[tool]`). Dependencies such
/// as `"tailwhip>=0.14"` are not keys.
fn mentions_key(text: &str) -> bool {
    memchr::memmem::find_iter(text.as_bytes(), b"tailwhip").any(|at| {
        let before = text[..at]
            .trim_end_matches(['"', '\''])
            .trim_end_matches([' ', '\t']);
        let after = text[at + "tailwhip".len()..]
            .trim_start_matches(['"', '\''])
            .trim_start_matches([' ', '\t']);
        let line_start = before.is_empty() || before.ends_with('\n');
        (before.ends_with('.') && after.starts_with(['.', '=', ']']))
            || (line_start && after.starts_with(['.', '=']))
    })
}

/// The `[tool.tailwhip]` table of a parsed `pyproject.toml`, if any.
fn tool_section(mut table: toml::Table) -> Option<toml::Table> {
    let toml::Value::Table(mut tool) = table.remove("tool")? else {
        return None;
    };
    match tool.remove("tailwhip")? {
        toml::Value::Table(section) => Some(section),
        _ => None,
    }
}

/// Deserialize a table of settings. Keys are matched case-insensitively,
/// as Dynaconf does.
fn overrides_from_table(table: toml::Table, path: &Path) -> Result<Overrides, ConfigError> {
    let lowercased: toml::Table = table
        .into_iter()
        .map(|(key, value)| (key.to_lowercase(), value))
        .collect();
    lowercased
        .try_into()
        .map_err(|error: toml::de::Error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: error.message().to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_load() {
        let config = Config::default();
        assert_eq!(config.verbosity, 1);
        assert!(!config.write_mode);
        assert_eq!(config.default_globs, ["**/*.html", "**/*.css"]);
        assert_eq!(config.skip_expressions, ["{{", "{%", "<%", "?"]);
        assert_eq!(config.variant_separator, ":");
        // The HTML and @apply patterns are built in
        assert!(config.class_patterns.is_empty());
    }

    #[test]
    fn generated_defaults_match_the_toml() {
        let parsed: Config = toml::from_str(DEFAULT_CONFIGURATION).unwrap();
        assert_eq!(Config::default(), parsed);
    }

    #[test]
    fn overrides_replace_lists_entirely() {
        let mut config = Config::default();
        config
            .apply_toml(
                "custom_colors = [\"brand\"]\nskip_expressions = [\"<%\"]",
                Path::new("x"),
            )
            .unwrap();
        assert_eq!(config.custom_colors, ["brand"]);
        assert_eq!(config.skip_expressions, ["<%"]);
        // Untouched keys keep their defaults
        assert_eq!(config.variant_separator, ":");
    }

    #[test]
    fn keys_are_case_insensitive_and_unknown_keys_are_ignored() {
        let mut config = Config::default();
        config
            .apply_toml("Verbosity = 3\nbogus = 1", Path::new("x"))
            .unwrap();
        assert_eq!(config.verbosity, 3);
    }

    #[test]
    fn patterns_reject_unknown_keys() {
        let mut config = Config::default();
        let error = config
            .apply_toml("[[class_patterns]]\nname = \"x\"\nregex = \"(?P<classes>.*)\"\ntemplate = \"{classes}\"", Path::new("x"))
            .unwrap_err();
        assert!(error.to_string().contains("template"), "{error}");
    }

    #[test]
    fn pyproject_sections_are_cut_out() {
        let text = "[project]\nname = \"x\"\n\n[tool.tailwhip]\ncustom_colors = [\n  \"brand\",\n]\n\n\
                    [[tool.tailwhip.class_patterns]]\nname = \"x\"\n\n[tool.ruff]\nline-length = 88\n";
        assert_eq!(
            tailwhip_section(text),
            Some(
                "[tool.tailwhip]\ncustom_colors = [\n  \"brand\",\n]\n\n\
                 [[tool.tailwhip.class_patterns]]\nname = \"x\"\n\n"
            )
        );
        assert_eq!(
            tailwhip_section("[tool.tailwhip]\nverbosity = 2"),
            Some("[tool.tailwhip]\nverbosity = 2")
        );
        assert_eq!(tailwhip_section("[tool]\ntailwhip.verbosity = 2\n"), None);
        // A line starting with "[" inside an array cuts too early, and the
        // cut-out text does not parse
        let nested = "[tool.tailwhip]\nx = [\n[1],\n]\n";
        assert!(
            tailwhip_section(nested)
                .unwrap()
                .parse::<toml::Table>()
                .is_err()
        );
    }

    #[test]
    fn pyproject_keys_are_recognized_in_any_form() {
        for text in [
            "[tool.tailwhip]",
            "[tool]\ntailwhip.verbosity = 2",
            "[tool]\ntailwhip = { verbosity = 2 }",
            "tool.tailwhip.verbosity = 2",
            "[tool.\"tailwhip\"]",
        ] {
            assert!(mentions_key(text), "{text}");
        }
        for text in [
            "[project]\ndependencies = [\"tailwhip>=0.14\", \"tailwhip\"]",
            "dev = [\"tailwhip[extra]\"]",
            "dev = [\n  \"tailwhip\"]",
            "",
        ] {
            assert!(!mentions_key(text), "{text}");
        }
    }

    #[test]
    fn pyproject_section_is_applied_in_any_form() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pyproject.toml");
        for (text, applied) in [
            (
                "[project]\nname = \"x\"\n[tool.tailwhip]\nverbosity = 3\n[tool.x]\ny = 1\n",
                true,
            ),
            ("[tool]\ntailwhip.verbosity = 3\n", true),
            ("[tool.tailwhip]\nverbosity = 3\nx = [\n[1],\n]\n", true),
            ("[project]\ndependencies = [\"tailwhip\"]\n", false),
        ] {
            fs::write(&path, text).unwrap();
            let mut config = Config::default();
            assert_eq!(config.apply_pyproject(&path).unwrap(), applied, "{text}");
            assert_eq!(config.verbosity, if applied { 3 } else { 1 }, "{text}");
        }
        // A broken section is reported, with the whole file parsed
        fs::write(&path, "[tool.tailwhip]\nverbosity = \n").unwrap();
        assert!(Config::default().apply_pyproject(&path).is_err());
    }

    #[test]
    fn wrong_types_are_errors() {
        let mut config = Config::default();
        let error = config
            .apply_toml("verbosity = \"loud\"", Path::new("x"))
            .unwrap_err();
        assert!(error.to_string().starts_with("Invalid configuration in x"));
    }
}
