//! Styled terminal output.
//!
//! Colors are used when stdout is a terminal, unless `NO_COLOR` is set or
//! `TERM` is `dumb`; `FORCE_COLOR` turns them on regardless. The 256-color
//! codes are the ones Rich picks for the Python tool's theme.

use std::env;
use std::fmt::{self, Display};
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

/// A value that displays in a style, without building a string first.
#[derive(Debug, Clone, Copy)]
pub struct Styled<'a, T: ?Sized> {
    style: Option<&'static str>,
    value: &'a T,
}

impl<T: Display + ?Sized> Display for Styled<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.style {
            Some(style) => write!(f, "\x1b[{style}m{}\x1b[0m", self.value),
            None => self.value.fmt(f),
        }
    }
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

    /// `value` in the style, or plain without colors.
    pub fn style<'a, T: Display + ?Sized>(
        &self,
        style: &'static str,
        value: &'a T,
    ) -> Styled<'a, T> {
        Styled {
            style: self.color.then_some(style),
            value,
        }
    }

    /// Print a line of regular output, unless quiet.
    pub fn print(&self, line: impl Display) {
        if !self.quiet {
            // A closed pipe is not worth reporting
            let _ = writeln!(io::stdout().lock(), "{line}");
        }
    }

    /// Print an error or warning line to stderr, also when quiet.
    pub fn error(&self, line: impl Display) {
        let _ = writeln!(io::stderr().lock(), "{line}");
    }

    /// Write text of any number of lines to stdout with a single call,
    /// unless quiet.
    pub fn write_output(&self, text: &str) {
        if !self.quiet && !text.is_empty() {
            let _ = io::stdout().lock().write_all(text.as_bytes());
        }
    }

    /// Write text of any number of lines to stderr with a single call, also
    /// when quiet.
    pub fn write_errors(&self, text: &str) {
        if !text.is_empty() {
            let _ = io::stderr().lock().write_all(text.as_bytes());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_apply_only_with_colors() {
        let plain = Console::with_color(false, false);
        let colored = Console::with_color(false, true);
        assert_eq!(plain.style(style::RED, "x").to_string(), "x");
        assert_eq!(
            colored.style(style::RED, "x").to_string(),
            "\x1b[31mx\x1b[0m"
        );
        assert_eq!(
            colored
                .style(style::DIM, &format_args!("{} files", 3))
                .to_string(),
            "\x1b[2m3 files\x1b[0m"
        );
    }
}
