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

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The built-in default configuration, the single source of truth for the
/// sort order. It is the same file the Python package ships.
pub const DEFAULT_CONFIGURATION: &str = include_str!("../configuration.toml");

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
    pub default_globs: Vec<String>,
    pub skip_expressions: Vec<String>,
    pub variant_separator: String,
    pub class_patterns: Vec<PatternSpec>,
    pub component_order: Vec<String>,
    pub variants: Vec<String>,
    pub prefixes: Vec<String>,
    pub directions: Vec<String>,
    pub sizes: Vec<String>,
    pub numerics: Vec<String>,
    pub colors: Vec<String>,
    pub custom_colors: Vec<String>,
    pub shades: Vec<String>,
    pub alphas: Vec<String>,
}

/// The keys a configuration layer may override. Unknown keys are ignored,
/// as Dynaconf ignores them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub verbosity: Option<u8>,
    pub write_mode: Option<bool>,
    pub default_globs: Option<Vec<String>>,
    pub skip_expressions: Option<Vec<String>>,
    pub variant_separator: Option<String>,
    pub class_patterns: Option<Vec<PatternSpec>>,
    pub component_order: Option<Vec<String>>,
    pub variants: Option<Vec<String>>,
    pub prefixes: Option<Vec<String>>,
    pub directions: Option<Vec<String>>,
    pub sizes: Option<Vec<String>>,
    pub numerics: Option<Vec<String>>,
    pub colors: Option<Vec<String>>,
    pub custom_colors: Option<Vec<String>>,
    pub shades: Option<Vec<String>>,
    pub alphas: Option<Vec<String>>,
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
    /// The built-in defaults.
    fn default() -> Self {
        toml::from_str(DEFAULT_CONFIGURATION).expect("the built-in configuration.toml is valid")
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
    pub fn apply_pyproject(&mut self, path: &Path) -> Result<bool, ConfigError> {
        let text = read(path)?;
        let table = parse_table(&text, path)?;
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
        assert_eq!(config.class_patterns.len(), 3);
        assert_eq!(config.class_patterns[0].name, "html_class");
        assert_eq!(config.class_patterns[1].name, "html_class_single_quoted");
        assert_eq!(config.class_patterns[2].name, "css_apply");
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
    fn wrong_types_are_errors() {
        let mut config = Config::default();
        let error = config
            .apply_toml("verbosity = \"loud\"", Path::new("x"))
            .unwrap_err();
        assert!(error.to_string().starts_with("Invalid configuration in x"));
    }
}
