# Tailwhip (Rust)

A Rust port of [tailwhip](https://github.com/bartTC/tailwhip), the Tailwind
CSS class sorter for HTML, CSS and template files. It produces the same
output as the Python tool and keeps its command line and TOML configuration,
and it starts about 20 times faster, which is what matters when an editor
pipes a buffer through it.

```bash
$ cargo build --release
$ echo '<div class="p-4 m-2 bg-white">' | ./target/release/tailwhip
<div class="m-2 p-4 bg-white">

$ tailwhip templates/            # dry run: report files that would change
$ tailwhip templates/ --write    # sort in place
$ tailwhip templates/ -vv        # show a diff per file
$ tailwhip "static/**/*.{css,scss}" index.html
```

See `tailwhip --help` for all options. Verbosity, write mode, file globs,
template expressions to skip and the sort order are configured in TOML, with
this precedence: built-in defaults < `[tool.tailwhip]` in the nearest
`pyproject.toml` < `--configuration FILE` < command line flags. Every key
replaces its default as a whole. See `configuration.toml` for all keys.

## Performance

Measured with `bench/compare.py` on an M-series Mac, wall clock including
process start, median of 30 runs. "Templates" and "map" are two real Django
projects.

| Case                             | Python  | Rust    | Speedup |
|----------------------------------|--------:|--------:|--------:|
| one-line stdin call              | 67.0 ms |  3.3 ms |   20.2x |
| 3.8 MB file on stdin             | 196 ms  | 51.7 ms |    3.8x |
| dry run over 525 synthetic files | 284 ms  | 14.9 ms |   19.1x |
| dry run over 165 templates       | 115 ms  |  7.0 ms |   16.4x |
| dry run over 252 templates (map) | 139 ms  | 18.3 ms |    7.6x |

Output was byte-identical to the Python tool for all 942 files involved.
Reproduce with the Python tool checked out next to this directory:

```bash
$ python3 bench/compare.py --runs 30 --files 500 path/to/templates ...
```

Where the time goes, and what the port does about it:

- **Startup** is about 1.5 ms inside the binary: parsing the embedded
  default configuration and compiling three regexes. The Python tool spends
  most of its 67 ms importing dependencies.
- **Finding class lists** uses the linear-time `regex` crate. The original
  pattern needed a backreference for the quote character, which costs a
  backtracking engine 10 times the scan time, so the quote styles are two
  patterns instead (see below).
- **Sorting** parses each class once with byte-level scanning and a single
  lookup per token. Sorted class lists are cached per thread, since the same
  attribute values repeat across a codebase.
- **Parallelism**: files are processed on all cores, and a single large
  input sorts its class lists on all cores too.

## Differences from the Python tool

The command line, messages, exit codes and sort order are the same. These
details differ:

- **Class patterns have no template.** A pattern is a regex with a `classes`
  group, and only the text that group matched is replaced. Everything around
  it, including quotes, spacing and letter case, stays as written. The Python
  tool rebuilt the whole match from a template and normalized
  `class = "..."` to `class="..."` as a side effect. The regex syntax is the
  Rust `regex` crate's; backreferences and lookaround are not supported, so
  single and double quotes are separate patterns.
- `verbosity` and `write_mode` from configuration files are honored. The
  Python tool always overrode them with the command line defaults.
- Errors and warnings go to stderr and are shown in `--quiet` mode.
- Line endings and byte order marks are preserved. The Python tool converted
  CRLF files to LF when writing.
- Reported file paths are absolute but symbolic links are not resolved.
- Invalid UTF-8 on stdin is passed through unchanged with exit code 1
  instead of a traceback.
- Not supported: `TAILWHIP_*` environment variables (undocumented Dynaconf
  behavior) and extended glob syntax such as `@(a|b)`.

## Development

```bash
$ cargo test                    # unit, integration and doc tests
$ cargo clippy --all-targets
$ cargo run --release --example profile -- bench/big.html      # stage timings
$ cargo run --release --example profile_dir -- bench/corpus    # file mode timings
```

`tests/fixtures/class_groups.txt` holds the 87 hand-ordered class groups
from the Python test suite; every group must survive reversal, every
rotation and 25 random shuffles.
