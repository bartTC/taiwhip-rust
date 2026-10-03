//! Where does the time go? Times the stages of processing one file.
//! Usage: cargo run --release --example profile -- bench/corpus/big.html

use std::time::Instant;

use tailwhip::config::Config;
use tailwhip::process::Processor;
use tailwhip::sorting::Sorter;

fn main() {
    let path = std::env::args().nth(1).expect("file argument");
    let text = std::fs::read_to_string(&path).unwrap();

    let t = Instant::now();
    let config = Config::default();
    println!("parse default configuration: {:?}", t.elapsed());

    for spec in &config.class_patterns {
        let t = Instant::now();
        let _ = regex::Regex::new(&spec.regex).unwrap();
        println!("  compile {}: {:?}", spec.name, t.elapsed());
    }
    let t = Instant::now();
    let sorter = Sorter::new(&config);
    println!("  build sorter tables: {:?}", t.elapsed());
    let t = Instant::now();
    let processor = Processor::new(&config).unwrap();
    println!("build processor (regexes + tables): {:?}", t.elapsed());

    let t = Instant::now();
    let mut attrs: Vec<&str> = Vec::new();
    for spec in &config.class_patterns {
        let regex = regex::Regex::new(&spec.regex).unwrap();
        let before = attrs.len();
        for caps in regex.captures_iter(&text) {
            attrs.push(caps.name("classes").unwrap().as_str());
        }
        println!("  scan {}: {} matches", spec.name, attrs.len() - before);
    }
    println!(
        "regex scans only: {:?} ({} matches)",
        t.elapsed(),
        attrs.len()
    );

    let t = Instant::now();
    let mut total = 0;
    for attr in &attrs {
        if let Some(s) = processor.sort_class_string(attr) {
            total += s.len();
        }
    }
    println!(
        "sort_class_string over all attrs, uncached: {:?} ({} bytes)",
        t.elapsed(),
        total
    );

    let t = Instant::now();
    let mut classes = 0;
    for attr in &attrs {
        for class in attr.split_whitespace() {
            let key = sorter.sort_key(class);
            classes += key.original.len();
        }
    }
    println!(
        "sort_key for every class: {:?} ({} class bytes)",
        t.elapsed(),
        classes
    );

    let t = Instant::now();
    let out = processor.process_text(&text);
    println!(
        "process_text total, cold cache: {:?} ({} bytes out)",
        t.elapsed(),
        out.len()
    );

    let t = Instant::now();
    let out = processor.process_text(&text);
    println!(
        "process_text total, warm cache: {:?} ({} bytes out)",
        t.elapsed(),
        out.len()
    );
}
