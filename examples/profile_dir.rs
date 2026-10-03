//! Where does the time go in file mode? Usage:
//! cargo run --release --example profile_dir -- bench/corpus
use std::path::PathBuf;
use std::time::Instant;

use rayon::prelude::*;
use tailwhip::config::Config;
use tailwhip::files::find_files;
use tailwhip::process::Processor;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("directory argument"));
    let config = Config::default();
    let processor = Processor::new(&config).unwrap();

    let t = Instant::now();
    let files = find_files(&[dir], &config.default_globs);
    println!("find_files: {:?} ({} files)", t.elapsed(), files.len());

    let t = Instant::now();
    let n = files.iter().filter(|f| f.canonicalize().is_ok()).count();
    println!(
        "canonicalize all files (absolute): {:?} ({n} files)",
        t.elapsed()
    );

    let dir = std::env::args().nth(1).unwrap();
    let patterns: Vec<String> = config
        .default_globs
        .iter()
        .map(|g| format!("{dir}/{g}"))
        .collect();
    let t = Instant::now();
    let unresolved = tailwhip::files::expand_globs(&patterns);
    println!(
        "expand_globs: {:?} ({} files)",
        t.elapsed(),
        unresolved.len()
    );
    let t = Instant::now();
    let n = unresolved
        .iter()
        .filter(|f| f.canonicalize().is_ok())
        .count();
    println!(
        "canonicalize (relative, sequential): {:?} ({n} files)",
        t.elapsed()
    );
    let t = Instant::now();
    let n = unresolved
        .par_iter()
        .filter(|f| f.canonicalize().is_ok())
        .count();
    println!(
        "canonicalize (relative, parallel): {:?} ({n} files)",
        t.elapsed()
    );
    let t = Instant::now();
    let n = unresolved
        .iter()
        .filter(|f| std::path::absolute(f).is_ok())
        .count();
    println!("std::path::absolute: {:?} ({n} files)", t.elapsed());

    let t = Instant::now();
    let n = walkdir::WalkDir::new(std::env::args().nth(1).unwrap())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count();
    println!("plain walk: {:?} ({n} files)", t.elapsed());

    let t = Instant::now();
    let texts: Vec<String> = files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    println!(
        "read all files: {:?} ({} bytes)",
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
    println!(
        "process with rayon (incl. pool start): {:?} ({changed} changed)",
        t.elapsed()
    );

    let t = Instant::now();
    let changed = texts
        .par_iter()
        .filter(|text| processor.process_text(text) != text.as_str())
        .count();
    println!(
        "process with rayon again: {:?} ({changed} changed)",
        t.elapsed()
    );

    let t = Instant::now();
    let changed = files
        .par_iter()
        .filter(|f| {
            let text = std::fs::read_to_string(f).unwrap();
            processor.process_text(&text) != text.as_str()
        })
        .count();
    println!(
        "read + process with rayon: {:?} ({changed} changed)",
        t.elapsed()
    );
}
