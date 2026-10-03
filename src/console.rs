//! Styled terminal output.
//!
//! Colors are used when stdout is a terminal, unless `NO_COLOR` is set or
//! `TERM` is `dumb`; `FORCE_COLOR` turns them on regardless. The 256-color
//! codes are the ones Rich picks for the Python tool's theme.

use std::env;
use std::io::{self, IsTerminal, Write};

/// ANSI SGR parameters for the styles the CLI uses.
pub mod style {
    pub const RED: &str = "31";
    pub const GREEN: &str = "92";
    pub const BRIGHT_RED: &str = "91";
    pub const BOLD_MAGENTA: &str = "1;95";
    pub const DIM: &str = "2";
    pub const WHITE: &str = "37";
    /// grey30
    pub const GREY: &str = "38;5;239";
    /// sky_blue1
    pub const HIGHLIGHT: &str = "38;5;117";
    /// white on deep_pink4
    pub const IMPORTANT: &str = "37;48;5;125";
}

#[derive(Debug, Clone)]
pub struct Console {
    quiet: bool,
    color: bool,
}

impl Console {
    /// A console for the current process. With `quiet`, regular output is
    /// suppressed; errors and warnings still go to stderr.
    pub fn new(quiet: bool) -> Console {
        Console {
            quiet,
            color: color_wanted(),
        }
    }

    pub fn with_color(quiet: bool, color: bool) -> Console {
        Console { quiet, color }
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    /// `text` wrapped in the style, or unchanged without colors.
    pub fn style(&self, style: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{style}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    /// Print a line of regular output, unless quiet.
    pub fn print(&self, line: &str) {
        if self.quiet {
            return;
        }
        let mut stdout = io::stdout().lock();
        // A closed pipe is not worth reporting
        let _ = stdout
            .write_all(line.as_bytes())
            .and_then(|_| stdout.write_all(b"\n"));
    }

    /// Print an error or warning line to stderr, also when quiet.
    pub fn error(&self, line: &str) {
        let mut stderr = io::stderr().lock();
        let _ = stderr
            .write_all(line.as_bytes())
            .and_then(|_| stderr.write_all(b"\n"));
    }
}

fn color_wanted() -> bool {
    if env::var_os("FORCE_COLOR").is_some_and(|value| !value.is_empty() && value != "0") {
        return true;
    }
    if env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    if env::var_os("TERM").is_some_and(|term| term == "dumb") {
        return false;
    }
    io::stdout().is_terminal()
}
