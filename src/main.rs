//! The tailwhip command line interface.

use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use lexopt::prelude::*;

use tailwhip::config::{Config, find_pyproject, verbosity};
use tailwhip::console::{Console, style};
use tailwhip::files::{Options, apply_changes};
use tailwhip::process::Processor;

const HELP: &str = "\
Sort Tailwind CSS classes in HTML and CSS files.

Usage: tailwhip [OPTIONS] [PATH]...

Arguments:
  [PATH]...                  Files or directories to process. Omit to read
                             from stdin. Directories are searched for HTML
                             and CSS files, skipping hidden files and what
                             .gitignore excludes.

Options:
  -w, --write                Write changes to files (default: dry-run mode).
  -q, --quiet                Suppress output except errors and warnings.
  -v, --verbose              Increase output verbosity (-v: unchanged files
                             and summary, -vv: diff). Repeatable.
  -c, --configuration FILE   Load a configuration file (overrides
                             pyproject.toml settings).
      --no-ignore            Also process hidden files and files excluded by
                             .gitignore when searching directories.
  -h, --help                 Show this help and exit.
  -V, --version              Show the version and exit.

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
  echo '<div class=\"mt-4 p-2 bg-blue-500\"></div>' | tailwhip
";

/// The command line arguments.
#[derive(Debug, Default)]
struct Cli {
    paths: Vec<PathBuf>,
    write: bool,
    no_ignore: bool,
    quiet: bool,
    verbose: u8,
    configuration: Option<PathBuf>,
}

/// What the command line asks for.
enum Command {
    Run(Cli),
    Help,
    Version,
}

impl Cli {
    /// Parse the arguments of this process. A hand-written parser over
    /// `lexopt` instead of clap, which built its whole model of the command
    /// line, help texts included, on every start.
    fn parse() -> Result<Command, lexopt::Error> {
        let mut cli = Cli::default();
        let mut parser = lexopt::Parser::from_env();
        while let Some(arg) = parser.next()? {
            match arg {
                Short('w') | Long("write") => cli.write = true,
                Short('q') | Long("quiet") => cli.quiet = true,
                Short('v') | Long("verbose") => cli.verbose = cli.verbose.saturating_add(1),
                Short('c') | Long("configuration") => {
                    cli.configuration = Some(parser.value()?.into());
                }
                Long("no-ignore") => cli.no_ignore = true,
                Short('h') | Long("help") => return Ok(Command::Help),
                Short('V') | Long("version") => return Ok(Command::Version),
                Value(path) => cli.paths.push(path.into()),
                _ => return Err(arg.unexpected()),
            }
        }
        Ok(Command::Run(cli))
    }
}

fn main() -> ExitCode {
    let cli = match Cli::parse() {
        Ok(Command::Run(cli)) => cli,
        Ok(Command::Help) => {
            print!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("tailwhip {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!(
                "error: {error}\n\nUsage: tailwhip [OPTIONS] [PATH]...\nFor more information, try '--help'."
            );
            return ExitCode::from(2);
        }
    };

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
        console.error(console.style(
            style::RED,
            "Error: No paths provided. Provide file paths or pipe content to stdin.",
        ));
        return ExitCode::FAILURE;
    }

    // Handle file mode --------------------------------------------------------

    configure_thread_pool();
    let start = Instant::now();
    let summary = apply_changes(
        &cli.paths,
        &Options {
            processor: &processor,
            console: &console,
            default_globs: &config.default_globs,
            skip_ignored: !cli.no_ignore,
            write_mode: config.write_mode,
            verbosity: config.verbosity,
        },
    );
    let duration = start.elapsed();

    if !summary.found_any {
        console.error(console.style(style::RED, "Error: No files found"));
        return ExitCode::FAILURE;
    }

    if config.verbosity >= verbosity::VERBOSE {
        if !config.write_mode {
            console.print(format_args!(
                "\n⚠ Dry Run. No files were actually written. Use {} to write changes.",
                console.style(style::IMPORTANT, " --write ")
            ));
        }
        console.print(format_args!(
            "⏱ Completed in {} for {} files. {}",
            console.style(
                style::HIGHLIGHT,
                &format_args!("{:.3}s", duration.as_secs_f64())
            ),
            summary.changed,
            console.style(style::DIM, &format_args!("({} skipped)", summary.skipped)),
        ));
    }

    if summary.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Threads for file mode, unless `RAYON_NUM_THREADS` says otherwise. Most
/// of the work in a real project is reading directories and files, and on
/// macOS that got slower with more threads: 250 templates in a tree of
/// 5,000 files took 7 ms on six threads and 12 ms on sixteen. Sorting the
/// classes of a large file keeps using every core: stdin mode leaves the
/// pool alone.
const MAX_FILE_THREADS: usize = 6;

fn configure_thread_pool() {
    if std::env::var_os("RAYON_NUM_THREADS").is_some() {
        return;
    }
    let threads = std::thread::available_parallelism()
        .map_or(1, |cores| cores.get())
        .min(MAX_FILE_THREADS);
    // Fails only if the pool already exists, which leaves it as it is
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global();
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
