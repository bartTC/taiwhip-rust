#!/usr/bin/env python3
"""
Write the README graphs to docs/ as standalone SVGs, one light and one dark
variant each, from the numbers measured by bench/compare.py.

Usage: python3 bench/graphs.py
"""

from __future__ import annotations

from pathlib import Path

DOCS = Path(__file__).resolve().parent.parent / "docs"

# Median and minimum wall clock in ms, Python then Rust
CASES = [
    ("One-line stdin call", "editor integration", (71.4, 69.1), (2.9, 2.5)),
    ("3.8 MB file on stdin", "24,690 class lists", (202.8, 200.4), (32.0, 31.0)),
    ("Dry run, 525 synthetic files", "3.9 MB", (296.7, 291.0), (11.4, 10.2)),
    ("Dry run, 165 real templates", "Django project", (122.8, 117.9), (4.9, 4.5)),
    ("Dry run, 252 real templates", "second Django project", (144.2, 139.8), (7.5, 6.8)),
]
AXIS_MAX = 300

THEMES = {
    "": {  # light
        "BG": "#ffffff", "TEXT": "#16191f", "MUTED": "#5d6474", "GRID": "#e6e8ee",
        "SURFACE": "#f7f8fa", "ACCENT": "#b5431c", "PYTHON": "#2a78d6", "RUST": "#eb6834",
    },
    "-dark": {
        "BG": "#1e2127", "TEXT": "#eceef2", "MUTED": "#a3a9b5", "GRID": "#2f343d",
        "SURFACE": "#262a31", "ACCENT": "#f08a5d", "PYTHON": "#3987e5", "RUST": "#d95926",
    },
}

FONT = 'font-family="IBM Plex Sans, Helvetica Neue, Arial, sans-serif"'


def wall_clock() -> str:
    label_w, bar_x, bar_w = 250, 270, 440
    row_h, top = 50, 58
    scale = bar_w / AXIS_MAX
    height = top + row_h * len(CASES) + 44
    out = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 760 {height}" {FONT} role="img" aria-label="Wall clock per run, Python versus Rust, median of 30 runs">']
    out.append('<rect width="760" height="%d" fill="__BG__"/>' % height)
    out.append('<text x="20" y="26" font-size="14" font-weight="600" fill="__TEXT__">Wall clock per run, median of 30, lower is better</text>')
    out.append('<rect x="20" y="36" width="12" height="12" rx="3" fill="__PYTHON__"/><text x="38" y="47" font-size="12" fill="__MUTED__">Python 0.14.0</text>')
    out.append('<rect x="140" y="36" width="12" height="12" rx="3" fill="__RUST__"/><text x="158" y="47" font-size="12" fill="__MUTED__">Rust port</text>')
    for tick in (0, 100, 200, 300):
        x = bar_x + tick * scale
        out.append(f'<line x1="{x:.1f}" y1="{top}" x2="{x:.1f}" y2="{top + row_h * len(CASES)}" stroke="__GRID__"/>')
        out.append(f'<text x="{x:.1f}" y="{top + row_h * len(CASES) + 18}" font-size="11" text-anchor="middle" fill="__MUTED__">{tick}</text>')
    out.append(f'<text x="{bar_x + bar_w}" y="{top + row_h * len(CASES) + 36}" font-size="11" text-anchor="end" fill="__MUTED__">milliseconds</text>')
    for i, (name, sub, py, rs) in enumerate(CASES):
        y = top + i * row_h
        if i:
            out.append(f'<line x1="20" y1="{y}" x2="{bar_x + bar_w}" y2="{y}" stroke="__GRID__"/>')
        out.append(f'<text x="20" y="{y + 22}" font-size="13" fill="__TEXT__">{name}</text>')
        out.append(f'<text x="20" y="{y + 38}" font-size="11" fill="__MUTED__">{sub}</text>')
        for j, (color, values) in enumerate((("__PYTHON__", py), ("__RUST__", rs))):
            by = y + 12 + j * 14
            w = values[0] * scale
            out.append(f'<path d="M{bar_x},{by} h{w - 4:.1f} a4,4 0 0 1 4,4 v4 a4,4 0 0 1 -4,4 h{-(w - 4):.1f} z" fill="{color}"/>')
            out.append(f'<text x="{bar_x + w + 6:.1f}" y="{by + 10}" font-size="11" fill="__MUTED__">{values[0]:.1f} ms</text>')
    out.append("</svg>")
    return "\n".join(out)


WORKFLOW = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 920 290" __FONT__ role="img" aria-label="A run loads the configuration, then either sorts piped stdin and writes it back, or finds files in parallel, skipping ignored ones, and processes each one on the thread pool as soon as it is found">
<rect width="920" height="290" fill="__BG__"/>
<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="__TEXT__"/></marker></defs>
<g fill="none" stroke="__TEXT__" stroke-width="1.2" marker-end="url(#arrow)">
<line x1="136" y1="46" x2="164" y2="46"/><line x1="356" y1="46" x2="384" y2="46"/><line x1="496" y1="46" x2="524" y2="46"/>
<line x1="622" y1="46" x2="650" y2="46"/><line x1="772" y1="46" x2="800" y2="46"/><path d="M441,68 V96 H78 V126"/>
<line x1="136" y1="150" x2="174" y2="150"/><line x1="262" y1="150" x2="286" y2="150"/><line x1="412" y1="150" x2="436" y2="150"/>
<line x1="556" y1="150" x2="580" y2="150"/><line x1="710" y1="150" x2="734" y2="150"/><line x1="811" y1="172" x2="811" y2="224"/>
</g>
<rect x="160" y="108" width="740" height="88" rx="8" fill="none" stroke="__ACCENT__" stroke-width="1.2" stroke-dasharray="5 4"/>
<text x="172" y="121" font-size="10" fill="__ACCENT__" font-weight="600" letter-spacing="0.06em">PER FILE, ON THE THREAD POOL, WHILE FILES ARE STILL BEING FOUND</text>
<g stroke="__TEXT__" stroke-width="1.2" fill="__SURFACE__">
<rect x="20" y="24" width="116" height="44" rx="6"/><rect x="166" y="24" width="190" height="44" rx="6"/><rect x="386" y="24" width="110" height="44" rx="22"/>
<rect x="526" y="24" width="96" height="44" rx="6"/><rect x="652" y="24" width="120" height="44" rx="6"/><rect x="802" y="24" width="98" height="44" rx="6"/>
<rect x="20" y="128" width="116" height="44" rx="6"/><rect x="176" y="128" width="86" height="44" rx="6"/><rect x="288" y="128" width="124" height="44" rx="6"/>
<rect x="438" y="128" width="118" height="44" rx="6"/><rect x="582" y="128" width="128" height="44" rx="6"/><rect x="736" y="128" width="150" height="44" rx="6"/>
<rect x="736" y="226" width="150" height="44" rx="6"/>
</g>
<g fill="__TEXT__" font-size="12" text-anchor="middle">
<text x="78" y="50">CLI arguments</text>
<text x="261" y="43">Load configuration</text><text x="261" y="58" font-size="10" fill="__MUTED__">defaults → pyproject → file → flags</text>
<text x="441" y="50">stdin piped?</text><text x="511" y="39" font-size="10" fill="__MUTED__">yes</text><text x="455" y="90" font-size="10" fill="__MUTED__">no</text>
<text x="574" y="50">Read stdin</text><text x="712" y="50">Sort class lists</text><text x="851" y="50">Write stdout</text>
<text x="78" y="147">Find files</text><text x="78" y="162" font-size="10" fill="__MUTED__">skips git-ignored</text>
<text x="219" y="154">Read file</text>
<text x="350" y="147">Find class lists</text><text x="350" y="162" font-size="10" fill="__MUTED__">scanners, added regexes</text>
<text x="497" y="147">Sort each list</text><text x="497" y="162" font-size="10" fill="__MUTED__">keys cached per thread</text>
<text x="646" y="154">Splice, compare</text>
<text x="811" y="147">Write or report</text><text x="811" y="162" font-size="10" fill="__MUTED__">--write, or "Would update"</text>
<text x="811" y="245">Print reports</text><text x="811" y="260" font-size="10" fill="__MUTED__">in path order, then summary</text>
</g>
</svg>
'''

THREADS = '''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 920 250" __FONT__ role="img" aria-label="Python threads queue at the interpreter lock so one sorts at a time; Rust threads each read, scan and sort on their own core">
<rect width="920" height="250" fill="__BG__"/>
<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="__TEXT__"/></marker></defs>
<line x1="460" y1="12" x2="460" y2="238" stroke="__GRID__"/>
<g fill="__TEXT__" font-size="12" font-weight="600">
<text x="20" y="22">Python 0.14.0: thread pool under the interpreter lock</text><text x="480" y="22">Rust port: rayon thread pool</text>
</g>
<g fill="none" stroke="__TEXT__" stroke-width="1.2" marker-end="url(#arrow)">
<line x1="108" y1="57" x2="234" y2="57"/><line x1="108" y1="97" x2="234" y2="97"/><line x1="108" y1="137" x2="234" y2="137"/><line x1="108" y1="177" x2="234" y2="177"/>
<line x1="280" y1="115" x2="318" y2="115"/>
<line x1="568" y1="57" x2="604" y2="57"/><line x1="568" y1="97" x2="604" y2="97"/><line x1="568" y1="137" x2="604" y2="137"/><line x1="568" y1="177" x2="604" y2="177"/>
<line x1="762" y1="117" x2="788" y2="117"/>
</g>
<g fill="none" stroke="__TEXT__" stroke-width="1.2">
<line x1="736" y1="57" x2="762" y2="57"/><line x1="736" y1="97" x2="762" y2="97"/><line x1="736" y1="137" x2="762" y2="137"/><line x1="736" y1="177" x2="762" y2="177"/>
<line x1="762" y1="57" x2="762" y2="177"/>
</g>
<rect x="236" y="40" width="44" height="150" rx="6" fill="__ACCENT__" fill-opacity="0.15" stroke="__ACCENT__" stroke-width="1.2"/>
<g stroke="__TEXT__" stroke-width="1.2" fill="__SURFACE__">
<rect x="20" y="44" width="88" height="26" rx="5"/><rect x="20" y="84" width="88" height="26" rx="5"/><rect x="20" y="124" width="88" height="26" rx="5"/><rect x="20" y="164" width="88" height="26" rx="5"/>
<rect x="320" y="93" width="120" height="44" rx="6"/>
<rect x="480" y="44" width="88" height="26" rx="5"/><rect x="480" y="84" width="88" height="26" rx="5"/><rect x="480" y="124" width="88" height="26" rx="5"/><rect x="480" y="164" width="88" height="26" rx="5"/>
<rect x="606" y="44" width="130" height="26" rx="5"/><rect x="606" y="84" width="130" height="26" rx="5"/><rect x="606" y="124" width="130" height="26" rx="5"/><rect x="606" y="164" width="130" height="26" rx="5"/>
<rect x="790" y="85" width="110" height="64" rx="6"/>
</g>
<g fill="__TEXT__" font-size="12" text-anchor="middle">
<text x="64" y="61">thread 1</text><text x="64" y="101">thread 2</text><text x="64" y="141">thread 3</text><text x="64" y="181">thread 4</text>
<text x="172" y="38" font-size="10" fill="__MUTED__">read file, then wait</text>
<text x="258" y="119" font-weight="600" fill="__ACCENT__">GIL</text>
<text x="380" y="112">sort</text><text x="380" y="127" font-size="10" fill="__MUTED__">one thread at a time</text>
<text x="524" y="61">thread 1</text><text x="524" y="101">thread 2</text><text x="524" y="141">thread 3</text><text x="524" y="181">thread 4</text>
<text x="671" y="61" font-size="11">read, scan, sort</text><text x="671" y="101" font-size="11">read, scan, sort</text><text x="671" y="141" font-size="11">read, scan, sort</text><text x="671" y="181" font-size="11">read, scan, sort</text>
<text x="775" y="110" font-size="10" fill="__MUTED__">reads</text>
<text x="845" y="112">patterns, tables</text><text x="845" y="127" font-size="10" fill="__MUTED__">shared, read-only</text><text x="845" y="139" font-size="10" fill="__MUTED__">caches per thread</text>
</g>
<g fill="__MUTED__" font-size="11">
<text x="20" y="226">16 cores, one of them sorting.</text><text x="480" y="226">Every thread sorting at once, no lock on the hot path.</text>
</g>
</svg>
'''


def main() -> None:
    DOCS.mkdir(exist_ok=True)
    graphs = {"wall-clock": wall_clock(), "workflow": WORKFLOW, "threads": THREADS}
    for suffix, colors in THEMES.items():
        for name, svg in graphs.items():
            text = svg.replace("__FONT__", FONT)
            for key, value in colors.items():
                text = text.replace(f"__{key}__", value)
            (DOCS / f"{name}{suffix}.svg").write_text(text)
            print("wrote", DOCS / f"{name}{suffix}.svg")


if __name__ == "__main__":
    main()
