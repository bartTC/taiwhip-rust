# tailwhip-rust

A Rust port of [tailwhip](https://github.com/bartTC/tailwhip), the Tailwind
CSS class sorter for HTML, CSS and template files, written to test one
question: how much faster can the same tool be in Rust?

**This repository exists for testing only.** It is an experiment, not a
release. There is no package, no versioning promise and no support. The
Python tool is the one to use.

The port keeps the command line, the TOML configuration and its precedence,
and produces the same output as the Python tool for every file it was tried
on. The one-line stdin call an editor makes on save takes 3.3 ms instead of
67 ms.

```bash
$ cargo build --release
$ echo '<div class="p-4 m-2 bg-white">' | ./target/release/tailwhip
<div class="m-2 p-4 bg-white">

$ tailwhip templates/            # dry run: report files that would change
$ tailwhip templates/ --write    # sort in place
$ tailwhip templates/ -vv        # show a diff per file
$ tailwhip "static/**/*.{css,scss}" index.html
```

## Results

Measured on 3 October 2026 with `bench/compare.py` on an M-series Mac with
16 cores: wall clock including process start, median of 30 runs, lower is
better. "Real templates" are two Django projects.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/wall-clock-dark.svg">
  <img alt="Wall clock per run, Python versus Rust: 67.0 versus 3.3 ms for a one-line stdin call, 195.6 versus 51.7 ms for a 3.8 MB file, 284.3 versus 14.9 ms for 525 files, 115.0 versus 7.0 ms for 165 templates, 138.5 versus 18.3 ms for 252 templates" src="docs/wall-clock.svg">
</picture>

| Case                             | Python median | Python min | Rust median | Rust min | Speedup |
|----------------------------------|--------------:|-----------:|------------:|---------:|--------:|
| One-line stdin call              |       67.0 ms |    65.9 ms |      3.3 ms |   3.2 ms |   20.2x |
| 3.8 MB file on stdin             |      195.6 ms |   193.5 ms |     51.7 ms |  49.2 ms |    3.8x |
| Dry run, 525 synthetic files     |      284.3 ms |   281.7 ms |     14.9 ms |  14.5 ms |   19.1x |
| Dry run, 165 real templates      |      115.0 ms |   113.0 ms |      7.0 ms |   6.5 ms |   16.4x |
| Dry run, 252 real templates      |      138.5 ms |   137.3 ms |     18.3 ms |  17.4 ms |    7.6x |

### Same output

Every file was piped through both tools and the outputs compared byte for
byte. Nothing differed, 0 mismatches in 942 files:

- **525 synthetic files** (3.9 MB): Django-style templates built from the
  87 hand-ordered class groups of the Python test suite, shuffled, with
  template expressions, single and double quotes, multi-line attributes and
  `@apply` rules.
- **165 real templates** from a Django project.
- **252 real templates** from a second Django project.

The Rust test suite adds 86 tests, including the 87 golden groups under
reversal, every rotation and 25 random shuffles each.

## How a run works

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/workflow-dark.svg">
  <img alt="Flow of one run: CLI arguments, load configuration, then either read stdin, sort class lists and write stdout, or find files and, per file on all cores, read the file, find class lists, sort each list, splice and compare, write or report, then print a summary" src="docs/workflow.svg">
</picture>

The stdin branch runs the same three stages as a file (find class lists,
sort each list, splice) on one thread. The dashed region is where the thread
pool works: every file is one task, and a file with at least 256 class lists
sorts them on the pool as well. Sorting a list means splitting it on
whitespace, parsing each class into variants, prefix and components, ranking
those by their position in the configured lists, sorting, dropping
duplicates and joining.

## Threads in Python and in Rust

Both tools use a thread pool. The difference is what the threads are allowed
to do at the same time.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/threads-dark.svg">
  <img alt="Python threads each read a file, then queue at the interpreter lock, so only one sorts at a time. Rust threads each read, scan and sort on their own core, reading shared patterns and tables, with one result cache per thread" src="docs/threads.svg">
</picture>

Python's lock is released while a thread waits for the disk, so file reads
overlap, but the sort is Python bytecode and runs under the lock, one thread
at a time. Rust threads run native code with no such lock; they share the
compiled patterns and lookup tables by reference and each keeps its own
cache of sorted lists.

|                       | Python 0.14.0                                              | Rust port                                                                           |
|-----------------------|------------------------------------------------------------|-------------------------------------------------------------------------------------|
| Workers               | `ThreadPoolExecutor`, up to 32 threads                     | `rayon` pool, one thread per core                                                   |
| Runs in parallel      | File reads and writes. The sort holds the interpreter lock | Reading, scanning and sorting, all of it                                            |
| Shared state          | Configuration and the result cache, guarded by the lock    | Patterns and tables, read-only. One result cache per thread, so no lock is taken    |
| Within one file       | Sequential                                                 | Class lists sorted on the pool once a file has 256 or more                          |
| Pool start            | Every run, including stdin                                 | About 2.5 ms, only when there is enough work. The one-line stdin call never starts it |
| 525 files, measured   | 284 ms. More cores do not help                             | 62.6 ms on one thread, 5.5 ms on the pool                                           |

Neither tool uses processes. Processes would have been the Python way around
the lock, but on macOS each worker process starts by importing the tool
again, about 60 ms per worker, and every result travels back through
pickling. For a job that takes 100 ms in total that is a loss. Rust does not
need the workaround: threads get full parallelism and shared memory for free.

## Where the time goes

**The one-line call**

1. Python spends nearly all of its 67 ms importing dependencies before
   reading stdin.
2. The Rust binary spends about 1.5 ms of its own: 0.4 ms parsing the
   embedded default configuration and about 1 ms compiling three regexes.
3. The remaining 1 to 2 ms is process start, the same for any native tool.

**The 3.8 MB file**

1. Processing takes 29 ms: about 12 ms of regex scans, the rest sorting
   24,690 class lists on all cores.
2. The other 20 ms of the measured 52 ms is piping 7.6 MB in and out through
   the harness, which the Python tool pays too.
3. Python caches sorted attribute values, which is why it stays within 4x on
   repetitive input.

## What the port changed to get here

Processing time of the same 2.3 MB file at each step:

| Time   | Step |
|-------:|------|
| 82 ms  | First port, a literal translation. The HTML pattern used a backreference for the quote character, which needs a backtracking regex engine, and that scan alone took 52 ms. |
| 46 ms  | Patterns rewritten for the linear-time `regex` crate: double and single quotes are two patterns, and the sorted classes are spliced into the matched span instead of rebuilt from a template. |
| 34 ms  | Single-pass byte-level class parser that keeps the list positions it finds, so building a sort key needs no second lookup. Unicode word boundaries replaced by ASCII ones, three times cheaper to scan. |
| 29 ms* | Class lists of a large input sorted on all cores; results cached per thread. Directory discovery stopped resolving every path through the file system, which was 25 µs per file. *Measured on the 3.8 MB file. |

## Same output, different internals

The command line, messages, exit codes, TOML keys and precedence are
unchanged. These details differ from the Python tool on purpose:

- **Class patterns are a regex with a `classes` group, no template.** Only
  that group's text is replaced. Quotes, spacing and letter case around it
  stay as written, where Python normalized `class = "x"` to `class="x"`. The
  regex syntax is the Rust `regex` crate's; backreferences and lookaround are
  not supported, so single and double quotes are separate patterns.
- **Configuration-file values for `verbosity` and `write_mode` are
  honored.** Python always overrode them with the command line defaults.
- **Errors go to stderr** and are still shown with `--quiet`.
- **Line endings and byte order marks are preserved.** Python rewrote CRLF
  files as LF.
- **Reported paths are absolute** but symbolic links are not resolved.
- Invalid UTF-8 on stdin is passed through unchanged with exit code 1
  instead of a traceback.
- Not supported: `TAILWHIP_*` environment variables (undocumented Dynaconf
  behavior) and extended glob syntax such as `@(a|b)`.

## Configuration

Settings are loaded in this order, later sources overriding earlier ones,
and every key replaces its default as a whole:

1. the built-in defaults in `configuration.toml`, embedded in the binary
2. the `[tool.tailwhip]` section of the nearest `pyproject.toml`
3. a file passed with `--configuration FILE`
4. command line flags

`configuration.toml` documents every key: output settings, file globs,
template expressions to skip, the ordered lists that define the sort order,
and the class patterns.

## Reproduce

```bash
cd tailwhip-rust
cargo build --release
python3 bench/compare.py --runs 30 --files 500 path/to/templates ...
```

The harness generates the synthetic corpus, checks conformance for every
file, then times both tools. It expects the Python tool's virtualenv in a
sibling `tailwhip` directory. `bench/graphs.py` writes the graphs above to
`docs/` from the measured numbers.

```bash
cargo test                      # unit, integration and doc tests
cargo clippy --all-targets
cargo run --release --example profile -- bench/big.html      # stage timings
cargo run --release --example profile_dir -- bench/corpus    # file mode timings
```

## License

MIT, see `LICENSE`.
