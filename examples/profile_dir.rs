//! Where does the time go in file mode? Usage:
//! cargo run --release --example profile_dir -- bench/corpus
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use tailwhip::config::Config;
use tailwhip::console::Console;
use tailwhip::files::{Options, apply_changes, find_files};
use tailwhip::process::Processor;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("directory argument"));
    let config = Config::default();
    let processor = Processor::new(&config).unwrap();

    let t = Instant::now();
    let files = find_files(std::slice::from_ref(&dir), &config.default_globs, true);
    println!(
        "find_files, incl. thread pool start: {:?} ({} files)",
        t.elapsed(),
        files.len()
    );

    let t = Instant::now();
    let files = find_files(std::slice::from_ref(&dir), &config.default_globs, true);
    println!(
        "find_files again: {:?} ({} files)",
        t.elapsed(),
        files.len()
    );

    let t = Instant::now();
    let texts: Vec<String> = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    println!(
        "read all files sequentially: {:?} ({} bytes)",
        t.elapsed(),
        texts.iter().map(String::len).sum::<usize>()
    );

    let t = Instant::now();
    let changed = texts
        .iter()
        .filter(|text| processor.process_text(text) != text.as_str())
        .count();
    println!(
        "process sequentially: {:?} ({changed} changed)",
        t.elapsed()
    );

    let t = Instant::now();
    let changed = texts
        .par_iter()
        .filter(|text| processor.process_text(text) != text.as_str())
        .count();
    println!("process in parallel: {:?} ({changed} changed)", t.elapsed());

    let console = Console::with_color(true, false);
    let t = Instant::now();
    let summary = apply_changes(
        &[dir],
        &Options {
            processor: &processor,
            console: &console,
            default_globs: &config.default_globs,
            skip_ignored: true,
            write_mode: false,
            verbosity: 1,
        },
    );
    println!(
        "apply_changes, find and process overlapped: {:?} ({} changed)",
        t.elapsed(),
        summary.changed
    );
}
