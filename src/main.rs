//! The tailwhip command line interface.

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use clap::{ArgAction, Parser};

use tailwhip::config::{Config, find_pyproject, verbosity};
use tailwhip::console::{Console, style};
use tailwhip::files::{Options, apply_changes, find_files};
use tailwhip::process::Processor;

const LONG_ABOUT: &str = "\
Sort Tailwind CSS classes in HTML and CSS files.

Automatically discovers and sorts Tailwind classes according to a consistent
ordering. Supports HTML, CSS, and template files with Tailwind @apply directives.";

const EXAMPLES: &str = "\
Examples:

  # Check a single file (dry-run by default)
  tailwhip index.html

  # Sort classes in multiple files
  tailwhip file1.html file2.html styles.css

  # Process all HTML and CSS files in a directory
  tailwhip src/templates/

  # Actually write changes to files
  tailwhip src/ --write

  # Preview detailed diff before writing
  tailwhip index.html -vv

  # Read from stdin and output to stdout
  echo '<div class=\"mt-4 p-2 bg-blue-500\"></div>' | tailwhip";

#[derive(Debug, Parser)]
#[command(
    name = "tailwhip",
    version,
    about = "Sort Tailwind CSS classes in HTML and CSS files.",
    long_about = LONG_ABOUT,
    after_help = EXAMPLES,
    disable_version_flag = true
)]
struct Cli {
    #[arg(
        value_name = "PATH",
        help = "Files or directories to process. Omit to read from stdin."
    )]
    paths: Vec<PathBuf>,

    #[arg(short = 'V', long = "version", action = ArgAction::Version, help = "Show version and exit.")]
    version: (),

    #[arg(
        short = 'w',
        long = "write",
        help = "Write changes to files (default: dry-run mode)."
    )]
    write: bool,

    #[arg(
        short = 'q',
        long = "quiet",
        help = "Suppress output except errors and warnings."
    )]
    quiet: bool,

    #[arg(
        short = 'v',
        long = "verbose",
        action = ArgAction::Count,
        help = "Increase output verbosity (-v: changes, -vv: diff, -vvv: debug)."
    )]
    verbose: u8,

    #[arg(
        short = 'c',
        long = "configuration",
        value_name = "FILE",
        help = "Load custom configuration file (overrides pyproject.toml settings)."
    )]
    configuration: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Setup configuration values ---------------------------------------------

    if let Some(file) = &cli.configuration
        && !file.exists()
    {
        eprintln!("Custom configuration file {} not found.", file.display());
        return ExitCode::FAILURE;
    }

    let mut config = Config::default();

    // 1. The nearest pyproject.toml overrides the defaults
    if let Some(pyproject) = std::env::current_dir()
        .ok()
        .and_then(|cwd| find_pyproject(&cwd))
        && let Err(error) = config.apply_pyproject(&pyproject)
    {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    // 2. A custom configuration file overrides pyproject.toml
    if let Some(file) = &cli.configuration
        && let Err(error) = config.apply_file(file)
    {
        eprintln!("{error}");
        return ExitCode::FAILURE;
    }

    // 3. Command line options override everything
    if cli.write {
        config.write_mode = true;
    }
    if cli.quiet {
        config.verbosity = verbosity::QUIET;
    } else if cli.verbose > 0 {
        config.verbosity = cli.verbose.saturating_add(1);
    }

    let processor = match Processor::new(&config) {
        Ok(processor) => processor,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    // Handle stdin mode -------------------------------------------------------

    // Without paths and with stdin being piped (not a TTY), process stdin to stdout
    if cli.paths.is_empty() && !io::stdin().is_terminal() {
        return run_stdin(&processor);
    }

    let console = Console::new(cli.quiet);

    if cli.paths.is_empty() {
        console.error(&console.style(
            style::RED,
            "Error: No paths provided. Provide file paths or pipe content to stdin.",
        ));
        return ExitCode::FAILURE;
    }

    // Handle file mode --------------------------------------------------------

    let start = Instant::now();
    let files = find_files(&cli.paths, &config.default_globs);
    let summary = apply_changes(
        &files,
        &Options {
            processor: &processor,
            console: &console,
            write_mode: config.write_mode,
            verbosity: config.verbosity,
        },
    );
    let duration = start.elapsed();

    if !summary.found_any {
        console.error(&console.style(style::RED, "Error: No files found"));
        return ExitCode::FAILURE;
    }

    if config.verbosity >= verbosity::VERBOSE {
        if !config.write_mode {
            console.print(&format!(
                "\n⚠ Dry Run. No files were actually written. Use {} to write changes.",
                console.style(style::IMPORTANT, " --write ")
            ));
        }
        console.print(&format!(
            "⏱ Completed in {} for {} files. {}",
            console.style(style::HIGHLIGHT, &format!("{:.3}s", duration.as_secs_f64())),
            summary.changed,
            console.style(style::DIM, &format!("({} skipped)", summary.skipped)),
        ));
    }

    if summary.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Read all of stdin, sort its classes and write the result to stdout.
/// Input that is not valid UTF-8 is passed through unchanged.
fn run_stdin(processor: &Processor) -> ExitCode {
    let mut input = Vec::new();
    if let Err(error) = io::stdin().lock().read_to_end(&mut input) {
        eprintln!("Unable to read stdin: {error}");
        return ExitCode::FAILURE;
    }

    let (output, status) = match std::str::from_utf8(&input) {
        Ok(text) => (
            processor.process_text(text).into_owned().into_bytes(),
            ExitCode::SUCCESS,
        ),
        Err(_) => {
            eprintln!("Error: stdin is not valid UTF-8; passing it through unchanged.");
            (input, ExitCode::FAILURE)
        }
    };

    let mut stdout = io::stdout().lock();
    if stdout
        .write_all(&output)
        .and_then(|_| stdout.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    status
}
