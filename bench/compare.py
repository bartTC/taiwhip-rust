#!/usr/bin/env python3
"""
Compare the Rust port against the Python tailwhip: output equality and speed.

1. Generates a deterministic synthetic corpus of HTML/CSS files from the
   golden class groups (bench/corpus/, git-ignored).
2. Checks that both tools produce byte-identical output for every corpus
   file, plus any extra directories passed on the command line.
3. Times both tools on: a one-line stdin call (editor integration), a large
   stdin input, and a dry run over the corpus directory.

Usage:
    python3 bench/compare.py [--runs N] [--files N] [EXTRA_DIR ...]
"""

from __future__ import annotations

import argparse
import os
import random
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
CORPUS = HERE / "corpus"
FIXTURE = ROOT / "tests" / "fixtures" / "class_groups.txt"

PYTHON_BIN = ROOT.parent / "tailwhip" / ".venv" / "bin" / "tailwhip"
RUST_BIN = ROOT / "target" / "release" / "tailwhip"

SMALL_INPUT = b'<div class="p-4 m-2 bg-white text-lg font-bold"></div>\n'


# ---------------------------------------------------------------------------
# Corpus generation
# ---------------------------------------------------------------------------


def class_groups() -> list[list[str]]:
    text = FIXTURE.read_text()
    return [g.split("\n") for g in text.strip().split("\n\n")]


def generate_corpus(file_count: int, seed: int = 42) -> list[Path]:
    rng = random.Random(seed)
    groups = class_groups()
    all_classes = [c for g in groups for c in g]
    CORPUS.mkdir(parents=True, exist_ok=True)
    for old in CORPUS.glob("**/*"):
        if old.is_file():
            old.unlink()

    files = []
    for i in range(file_count):
        lines = ["{% extends 'base.html' %}", "{% block content %}"]
        for _ in range(rng.randint(20, 80)):
            classes = list(rng.choice(groups))
            if rng.random() < 0.5:
                classes += rng.sample(all_classes, rng.randint(1, 6))
            rng.shuffle(classes)
            tag = rng.choice(["div", "span", "p", "a", "button", "li"])
            quote = rng.choice(['"', '"', '"', "'"])
            attr = " ".join(classes)
            if rng.random() < 0.1:
                attr += " {{ extra_classes }}"  # left untouched by both tools
            if rng.random() < 0.1:
                attr = attr.replace(" ", "\n        ", 3)  # multi-line attribute
            indent = "    " * rng.randint(1, 4)
            lines.append(f"{indent}<{tag} class={quote}{attr}{quote}>{rng.choice(['Text', '{{ item.name }}', ''])}</{tag}>")
        lines.append("{% endblock %}")
        subdir = CORPUS / rng.choice(["", "pages", "components", "pages/deep"])
        subdir.mkdir(parents=True, exist_ok=True)
        path = subdir / f"template_{i:04d}.html"
        path.write_text("\n".join(lines) + "\n")
        files.append(path)

    # A few CSS files with @apply
    for i in range(max(1, file_count // 20)):
        rules = []
        for j in range(rng.randint(5, 30)):
            classes = list(rng.choice(groups))
            rng.shuffle(classes)
            rules.append(f".rule-{j} {{\n    @apply {' '.join(classes)};\n}}")
        path = CORPUS / f"styles_{i:03d}.css"
        path.write_text("\n\n".join(rules) + "\n")
        files.append(path)

    # One large file for the big-stdin test: every template concatenated.
    # It lives outside the corpus directory so it does not dominate the
    # directory run.
    big = HERE / "big.html"
    big.write_text("".join(p.read_text() for p in files if p.suffix == ".html"))
    return files


# ---------------------------------------------------------------------------
# Running the tools
# ---------------------------------------------------------------------------


def run(binary: Path, args: list[str], stdin: bytes | None, cwd: Path) -> tuple[float, subprocess.CompletedProcess]:
    start = time.perf_counter()
    result = subprocess.run(
        [str(binary), *args],
        input=stdin if stdin is not None else b"",
        capture_output=True,
        cwd=cwd,
        env={**os.environ, "NO_COLOR": "1"},
        check=False,
    )
    return time.perf_counter() - start, result


def timings(binary: Path, args: list[str], stdin: bytes | None, cwd: Path, runs: int) -> list[float]:
    run(binary, args, stdin, cwd)  # warm up caches
    return [run(binary, args, stdin, cwd)[0] for _ in range(runs)]


def fmt_ms(seconds: float) -> str:
    return f"{seconds * 1000:.1f} ms"


# ---------------------------------------------------------------------------
# Conformance
# ---------------------------------------------------------------------------


def check_conformance(files: list[Path], cwd: Path, label: str) -> int:
    mismatches = 0
    for path in files:
        data = path.read_bytes()
        try:
            data.decode("utf-8")
        except UnicodeDecodeError:
            continue
        _, py = run(PYTHON_BIN, [], data, cwd)
        _, rs = run(RUST_BIN, [], data, cwd)
        if py.returncode != 0:
            continue  # Python crashed on this file; nothing to compare against
        if py.stdout != rs.stdout:
            mismatches += 1
            if mismatches <= 3:
                print(f"  MISMATCH {path}")
                show_first_difference(py.stdout, rs.stdout)
    print(f"  {label}: {len(files)} files compared, {mismatches} mismatches")
    return mismatches


def show_first_difference(a: bytes, b: bytes) -> None:
    a_lines, b_lines = a.split(b"\n"), b.split(b"\n")
    for i, (x, y) in enumerate(zip(a_lines, b_lines)):
        if x != y:
            print(f"    line {i + 1}:")
            print(f"      python: {x[:200]!r}")
            print(f"      rust:   {y[:200]!r}")
            return
    print(f"    lengths differ: python {len(a)} bytes, rust {len(b)} bytes")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("extra_dirs", nargs="*", type=Path, help="Extra directories of real templates to compare")
    parser.add_argument("--runs", type=int, default=30, help="Timing runs per measurement (default 30)")
    parser.add_argument("--files", type=int, default=500, help="Synthetic corpus size (default 500)")
    parser.add_argument("--skip-conformance", action="store_true")
    args = parser.parse_args()

    for binary in (PYTHON_BIN, RUST_BIN):
        if not binary.exists():
            print(f"missing: {binary}", file=sys.stderr)
            return 2

    print(f"python: {PYTHON_BIN}")
    print(f"rust:   {RUST_BIN}\n")

    print(f"Generating corpus of {args.files} files ...")
    files = generate_corpus(args.files)
    big = HERE / "big.html"
    total = sum(p.stat().st_size for p in files)
    print(f"  {len(files)} files, {total / 1e6:.1f} MB; big.html is {big.stat().st_size / 1e6:.1f} MB\n")

    failed = 0
    if not args.skip_conformance:
        print("Conformance (stdin output must be byte-identical):")
        failed += check_conformance(files, CORPUS, "synthetic corpus")
        for directory in args.extra_dirs:
            real = sorted(p for p in directory.rglob("*") if p.suffix in {".html", ".css", ".htm"} and p.is_file())
            failed += check_conformance(real, directory, str(directory))
        print()

    print(f"Timing ({args.runs} runs each, wall clock including process start):\n")
    cases = [
        ("one-line stdin", [], SMALL_INPUT, CORPUS),
        (f"big stdin ({big.stat().st_size / 1e6:.1f} MB)", [], big.read_bytes(), CORPUS),
        (f"dry run over {len(files)} files", ["."], None, CORPUS),
    ]
    for directory in args.extra_dirs:
        count = sum(1 for p in directory.rglob("*") if p.suffix in {".html", ".css"} and p.is_file())
        cases.append((f"dry run over {count} files ({directory.name})", [str(directory)], None, directory))
    rows = []
    for label, cli, stdin, cwd in cases:
        runs = args.runs if stdin is None or len(stdin) < 10_000 else max(5, args.runs // 3)
        py = timings(PYTHON_BIN, cli, stdin, cwd, runs)
        rs = timings(RUST_BIN, cli, stdin, cwd, runs)
        rows.append((label, statistics.median(py), min(py), statistics.median(rs), min(rs)))

    print(f"| {'case':<40} | {'python median':>13} | {'python min':>10} | {'rust median':>11} | {'rust min':>8} | {'speedup':>7} |")
    print(f"|{'-' * 42}|{'-' * 15}|{'-' * 12}|{'-' * 13}|{'-' * 10}|{'-' * 9}|")
    for label, py_med, py_min, rs_med, rs_min in rows:
        print(f"| {label:<40} | {fmt_ms(py_med):>13} | {fmt_ms(py_min):>10} | {fmt_ms(rs_med):>11} | {fmt_ms(rs_min):>8} | {py_med / rs_med:>6.1f}x |")

    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
