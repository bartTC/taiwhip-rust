//! Turns the default configuration into Rust code.
//!
//! `configuration.toml` stays the single source of truth, but parsing it on
//! every start took longer than sorting the classes an editor sends on save.
//! The generated code is a `Config { ... }` expression with one field per
//! top-level key, so a key without a field, or a field without a key, fails
//! the build instead of being skipped.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=configuration.toml");
    let text = fs::read_to_string("configuration.toml").expect("configuration.toml is readable");
    let table: toml::Table = text.parse().expect("configuration.toml is valid TOML");

    let mut code = String::from("Config {\n");
    for (key, value) in &table {
        code.push_str(&format!("    {key}: {},\n", expression(key, value)));
    }
    code.push('}');

    let out =
        Path::new(&env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("default_config.rs");
    fs::write(out, code).expect("OUT_DIR is writable");
}

/// The value as a Rust expression. Debug formatting of a string is a valid
/// Rust string literal.
fn expression(key: &str, value: &toml::Value) -> String {
    match value {
        toml::Value::Integer(number) => number.to_string(),
        toml::Value::Boolean(flag) => flag.to_string(),
        toml::Value::String(text) => format!("String::from({text:?})"),
        toml::Value::Array(items) if items.is_empty() => "Vec::new()".to_string(),
        toml::Value::Array(items) if items.iter().all(toml::Value::is_str) => {
            let items: Vec<String> = items
                .iter()
                .map(|item| format!("{:?}", item.as_str().unwrap()))
                .collect();
            format!("strings(&[{}])", items.join(", "))
        }
        // Arrays of tables are the class patterns
        toml::Value::Array(items) => {
            let specs: Vec<String> = items
                .iter()
                .map(|item| {
                    let Some(table) = item.as_table() else {
                        panic!("{key}: expected an array of tables");
                    };
                    let fields: Vec<String> = table
                        .iter()
                        .map(|(field, value)| format!("{field}: {}", expression(field, value)))
                        .collect();
                    format!("PatternSpec {{ {} }}", fields.join(", "))
                })
                .collect();
            format!("vec![{}]", specs.join(", "))
        }
        other => panic!("{key}: unsupported value {other:?}"),
    }
}
