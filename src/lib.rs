//! Tailwhip: sort Tailwind CSS classes in HTML and CSS files.
//!
//! The crate mirrors the Python package of the same name. The binary in
//! `main.rs` is the CLI; these modules are the library it is built on:
//!
//! - [`config`]: the TOML configuration and its precedence rules
//! - [`sorting`]: parsing a class name into components and ordering classes
//! - [`process`]: finding class attributes in text and rewriting them
//! - [`scan`]: the built-in finders for class attributes and `@apply`
//! - [`files`]: file discovery and applying changes to files
//! - [`console`]: styled terminal output

pub mod config;
pub mod console;
pub mod files;
pub mod process;
pub mod scan;
pub mod sorting;
