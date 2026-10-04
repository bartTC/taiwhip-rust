//! Sorts every class list of the files given, over and over, for profiling.
//! Usage: cargo run --release --example sort_loop -- SECONDS FILE...
use std::time::{Duration, Instant};

use tailwhip::config::Config;
use tailwhip::process::Processor;
use tailwhip::sorting::KeyCache;

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds: f64 = args.next().expect("seconds").parse().unwrap();
    let config = Config::default();
    let processor = Processor::new(&config).unwrap();
    let texts: Vec<String> = args.map(|f| std::fs::read_to_string(f).unwrap()).collect();
    let lists: Vec<&str> = texts
        .iter()
        .flat_map(|text| {
            processor
                .patterns()
                .iter()
                .flat_map(|p| p.spans(text))
                .map(|(start, end)| &text[start..end])
        })
        .collect();
    let classes: usize = lists.iter().map(|l| l.split_whitespace().count()).sum();
    let sorter = processor.sorter();
    let mut cache = KeyCache::default();
    for cached in [false, true] {
        let start = Instant::now();
        let mut rounds = 0;
        while start.elapsed() < Duration::from_secs_f64(seconds / 2.0) {
            for list in &lists {
                if cached {
                    let split: Vec<&str> = list.split_whitespace().collect();
                    std::hint::black_box(sorter.sort_classes_cached(&split, &mut cache).join(" "));
                } else {
                    std::hint::black_box(processor.sort_class_string(list));
                }
            }
            rounds += 1;
        }
        let per_round = start.elapsed() / rounds;
        println!(
            "{} {} lists, {classes} classes: {per_round:?} per round, {:.1} ns per class",
            if cached {
                "cached keys:  "
            } else {
                "parsing each:"
            },
            lists.len(),
            per_round.as_nanos() as f64 / classes as f64
        );
    }
}
