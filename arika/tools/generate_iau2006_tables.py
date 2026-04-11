#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Generate Rust const tables for the IAU 2006 CIP X, Y and CIO locator s
series from the IERS Conventions 2010 (TN36) Chapter 5 electronic tables.

The generator fetches `tab5.2a.txt`, `tab5.2b.txt`, and `tab5.2d.txt`
from the IERS Conventions Centre (with a BIPM FTP fallback), parses the
five power-of-`t` groups (`j = 0..4`) in each table, and emits a single
Rust module at `arika/src/earth/iau2006/tables_gen.rs` containing:

- Polynomial part coefficients (microarcsec), one `[f64; 6]` per table
- One `CipTerm` / `SxyTerm` slice per power of `t`, containing the
  `sin` / `cos` amplitudes (microarcsec) and the 14 integer multipliers
  of the fundamental arguments

Everything is committed to the repository, so CI does not need network
access. Re-run this script whenever:

- The IERS Conventions Centre publishes an updated Chapter 5 bundle
- A new quantity is added to the arika IAU 2006 module

Usage (run from the repository root):

    uv run arika/tools/generate_iau2006_tables.py

The generator is stable — running it twice produces identical output
when the upstream tables are unchanged.

# License / attribution

The numerical coefficients in Tables 5.2a / 5.2b / 5.2d are scientific
data published by the IERS Conventions Centre as the definitive CIP
X, Y and CIO locator s series for IAU 2006 + IAU 2000A_R06 (Petit &
Luzum eds., IERS Conventions 2010). arika's LICENSE remains MIT; the
generated `tables_gen.rs` carries an attribution header naming the
IERS source files.
"""

from __future__ import annotations

import re
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

# ─── Upstream sources ────────────────────────────────────────────
#
# `iers-conventions.obspm.fr` is the Conventions Centre web mirror;
# BIPM FTP is the original host. We try the HTTPS mirror first so the
# generator works on machines that block outbound FTP.

SOURCE_URLS: dict[str, list[str]] = {
    "tab5.2a.txt": [
        "https://iers-conventions.obspm.fr/content/chapter5/additional_info/tab5.2a.txt",
        "ftp://tai.bipm.org/iers/conv2010/chapter5/tab5.2a.txt",
    ],
    "tab5.2b.txt": [
        "https://iers-conventions.obspm.fr/content/chapter5/additional_info/tab5.2b.txt",
        "ftp://tai.bipm.org/iers/conv2010/chapter5/tab5.2b.txt",
    ],
    "tab5.2d.txt": [
        "https://iers-conventions.obspm.fr/content/chapter5/additional_info/tab5.2d.txt",
        "ftp://tai.bipm.org/iers/conv2010/chapter5/tab5.2d.txt",
    ],
}

# Expected per-power row counts — a structural sanity check. If any
# group length diverges from these numbers, the IERS table has changed
# and the generator aborts before writing output.
EXPECTED_COUNTS: dict[str, list[int]] = {
    "tab5.2a.txt": [1306, 253, 36, 4, 1],
    "tab5.2b.txt": [962, 277, 30, 5, 1],
    "tab5.2d.txt": [33, 3, 25, 4, 1],
}

# Polynomial-part literal strings from the TN36 tables, parsed below
# by a small shared regex. Keeping these as expected values lets the
# generator verify the literal the table claims matches what we emit.
EXPECTED_POLY_LITERALS: dict[str, str] = {
    "tab5.2a.txt": "- 16617. + 2004191898. t - 429782.9 t^2 - 198618.34 t^3 + 7.578 t^4 + 5.9285 t^5",
    "tab5.2b.txt": "- 6951. - 25896. t - 22407274.7 t^2 + 1900.59 t^3 + 1112.526 t^4 + 0.1358 t^5",
    "tab5.2d.txt": "94.0 + 3808.65 t - 122.68 t^2 - 72574.11 t^3 + 27.98 t^4 + 15.62 t^5",
}


@dataclass
class Table:
    """Parsed contents of one IERS CIP/CIO table file."""

    name: str  # e.g. "tab5.2a.txt"
    polynomial_uas: list[float]  # [c0..c5], microarcseconds
    groups: list[list[tuple[float, float, list[int]]]]  # per-power terms


def fetch(name: str) -> str:
    """Fetch a table file, trying each mirror in turn."""
    last_err: Exception | None = None
    for url in SOURCE_URLS[name]:
        try:
            with urllib.request.urlopen(url, timeout=60) as resp:
                return resp.read().decode("ascii", errors="strict")
        except (urllib.error.URLError, urllib.error.HTTPError, OSError) as e:
            last_err = e
            print(f"  [{name}] {url} failed: {e}", file=sys.stderr)
    raise RuntimeError(f"all mirrors failed for {name}; last error: {last_err}")


POLY_RE = re.compile(
    r"""^\s*
        (?P<c0>[-+]?\s*[\d.]+)\.? \s* \+?
        \s* (?P<c1>[-+]?\s*[\d.]+)\.? \s* t \s*
        \+? \s* (?P<c2>[-+]?\s*[\d.]+)\.? \s* t\^2 \s*
        \+? \s* (?P<c3>[-+]?\s*[\d.]+)\.? \s* t\^3 \s*
        \+? \s* (?P<c4>[-+]?\s*[\d.]+)\.? \s* t\^4 \s*
        \+? \s* (?P<c5>[-+]?\s*[\d.]+)\.? \s* t\^5
        """,
    re.VERBOSE,
)


def parse_poly_literal(literal: str) -> list[float]:
    """Parse the six coefficients of a polynomial part line.

    The TN36 literal has the form
    ``c0 + c1 t + c2 t^2 + c3 t^3 + c4 t^4 + c5 t^5`` with any subset of
    signs (handled by the regex) and trailing periods after integers
    (e.g. ``16617.``). The regex is deliberately permissive about
    whitespace.
    """
    # Collapse ``- 16617.`` to ``-16617.`` etc. so the regex sees a
    # single token per coefficient.
    cleaned = re.sub(r"([-+])\s+", r"\1", literal.strip())
    m = POLY_RE.match(cleaned)
    if not m:
        raise ValueError(f"could not parse polynomial literal: {literal!r}")
    return [float(m.group(f"c{i}")) for i in range(6)]


def parse_table(name: str, text: str) -> Table:
    """Parse one IERS CIP/CIO table file into a `Table`."""
    # Polynomial literal
    poly_match = re.search(
        r"Polynomial part \(unit microarcsecond\)\s*\n\s*(.+)",
        text,
    )
    if not poly_match:
        raise ValueError(f"{name}: polynomial part line not found")
    literal = poly_match.group(1).strip()

    if literal != EXPECTED_POLY_LITERALS[name]:
        raise ValueError(
            f"{name}: polynomial literal changed\n"
            f"  expected: {EXPECTED_POLY_LITERALS[name]!r}\n"
            f"  got:      {literal!r}"
        )
    poly_coeffs = parse_poly_literal(literal)

    # Data rows per `j = k` section. We split the file on the `j = k`
    # markers and parse each block as a sequence of rows.
    groups: list[list[tuple[float, float, list[int]]]] = []
    section_pattern = re.compile(r"^\s*j\s*=\s*(\d+)\s+Number of terms\s*=\s*(\d+)", re.M)
    markers = list(section_pattern.finditer(text))
    if not markers:
        raise ValueError(f"{name}: no `j = k` section markers found")

    for idx, m in enumerate(markers):
        start = m.end()
        end = markers[idx + 1].start() if idx + 1 < len(markers) else len(text)
        block = text[start:end]
        terms: list[tuple[float, float, list[int]]] = []
        for line in block.splitlines():
            line = line.strip()
            if not line or line.startswith("-"):
                continue
            parts = line.split()
            # Row shape: i  sin_coef  cos_coef  14 integer multipliers
            if len(parts) != 17:
                # skip dash separators, empty lines, decorative rows
                continue
            try:
                sin_coef = float(parts[1])
                cos_coef = float(parts[2])
                mult = [int(x) for x in parts[3:17]]
            except ValueError:
                continue
            terms.append((sin_coef, cos_coef, mult))

        expected_n = int(m.group(2))
        if len(terms) != expected_n:
            raise ValueError(
                f"{name}: j={m.group(1)} expected {expected_n} terms, parsed {len(terms)}"
            )
        groups.append(terms)

    # Cross-check against hardcoded expected counts.
    expected = EXPECTED_COUNTS[name]
    actual = [len(g) for g in groups]
    if actual != expected:
        raise ValueError(
            f"{name}: group counts {actual} differ from expected {expected}"
        )

    return Table(name=name, polynomial_uas=poly_coeffs, groups=groups)


def format_term_array(const_name: str, terms: list[tuple[float, float, list[int]]]) -> str:
    """Emit a `pub(crate) const NAME: &[CipTerm] = &[...]` literal.

    Each generated const carries `#[rustfmt::skip]` because the compact
    single-line `CipTerm { ... }` layout is easier to diff against the
    upstream TN36 text than rustfmt's multi-line rewrite. Matches
    tobari's `data/igrf14_generated.rs` convention.
    """
    lines: list[str] = []
    lines.append("#[rustfmt::skip]")
    lines.append(f"pub(crate) const {const_name}: &[CipTerm] = &[")
    for sin_uas, cos_uas, mult in terms:
        mult_str = ", ".join(f"{m:>3}" for m in mult)
        lines.append(
            f"    CipTerm {{ sin_uas: {sin_uas:>18}, cos_uas: {cos_uas:>18}, "
            f"arg: [{mult_str}] }},"
        )
    lines.append("];")
    return "\n".join(lines)


def format_poly_const(const_name: str, coeffs: list[float]) -> str:
    """Emit a `pub(crate) const NAME: [f64; 6] = [...]` literal.

    `#[rustfmt::skip]` keeps the one-line-per-coefficient layout stable
    across regenerations.
    """
    body = ",\n    ".join(f"{c}" for c in coeffs)
    return (
        f"#[rustfmt::skip]\npub(crate) const {const_name}: [f64; 6] = "
        f"[\n    {body},\n];"
    )


def emit_rust(x: Table, y: Table, s: Table) -> str:
    """Generate the final `tables_gen.rs` file contents."""
    header = '''// GENERATED FILE — DO NOT EDIT BY HAND
//
// Source: IERS Conventions (2010), Technical Note No. 36, Chapter 5
//   electronic tables 5.2a, 5.2b, and 5.2d, edited by G. Petit and
//   B. Luzum (International Earth Rotation and Reference Systems
//   Service, Verlag des Bundesamts für Kartographie und Geodäsie,
//   Frankfurt am Main, 2010).
//
// Mirrored at:
//   - https://iers-conventions.obspm.fr/content/chapter5/additional_info/
//   - ftp://tai.bipm.org/iers/conv2010/chapter5/
//
// Generator: arika/tools/generate_iau2006_tables.py
//
// Re-generate with (from the repository root):
//   uv run arika/tools/generate_iau2006_tables.py
//
// The numerical coefficients below are scientific data published by the
// IERS Conventions Centre as the definitive CIP X, Y and CIO locator s
// series for IAU 2006 + IAU 2000A_R06. The concept of the tables, their
// selection, and their layout originate with the IERS Conventions 2010;
// this file only reformats them from fixed-width text into Rust `const`
// arrays for consumption by `arika::earth::iau2006`. See the top-level
// arika DESIGN.md section on the IAU 2006 CIO chain.

// Phase 3A-2 only lands the tables; the evaluator in Phase 3A-3 will
// be the real consumer. Suppress unused-item warnings crate-wide for
// this one generated file so `cargo clippy -- -D warnings` stays clean.
#![allow(dead_code)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::unreadable_literal)]
// The tables contain amplitude coefficients in microarcseconds; some
// (e.g. 3.14, 2.72) happen to coincide with math constants but are
// physical values, not approximations. Silence the false positive.
#![allow(clippy::approx_constant)]

/// One non-polynomial term of a CIP / CIO series.
///
/// `sin_uas` and `cos_uas` are the amplitudes of the sine / cosine
/// components in **microarcseconds**; `arg` holds the 14 integer
/// multipliers of the fundamental arguments, in the TN36 column order:
/// `[l, l\', F, D, Ω, L_Me, L_Ve, L_E, L_Ma, L_J, L_Sa, L_U, L_Ne, p_A]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CipTerm {
    pub sin_uas: f64,
    pub cos_uas: f64,
    pub arg: [i8; 14],
}
'''

    # `header` is a triple-quoted string ending in `}\n`; strip the
    # trailing newline so the `\n\n` join inserts exactly one blank line
    # between the `CipTerm` struct and the first table section.
    body_parts: list[str] = [header.rstrip()]

    def add_table(prefix: str, label: str, table: Table) -> None:
        body_parts.append(f"// ─── {label} ───────────────────────────────")
        body_parts.append(
            format_poly_const(f"{prefix}_POLY_UAS", table.polynomial_uas)
        )
        for j, terms in enumerate(table.groups):
            body_parts.append(format_term_array(f"{prefix}_TERMS_{j}", terms))

    add_table("X", "Table 5.2a: CIP X coordinate (IAU 2006 + 2000A_R06)", x)
    add_table("Y", "Table 5.2b: CIP Y coordinate (IAU 2006 + 2000A_R06)", y)
    add_table("SXY2", "Table 5.2d: CIO locator s + X·Y/2", s)

    return "\n\n".join(body_parts) + "\n"


def main() -> None:
    print("Fetching IERS Conventions 2010 Chapter 5 tables…")
    tables: dict[str, Table] = {}
    for name in SOURCE_URLS:
        print(f"  {name}…")
        text = fetch(name)
        tables[name] = parse_table(name, text)

    x = tables["tab5.2a.txt"]
    y = tables["tab5.2b.txt"]
    s = tables["tab5.2d.txt"]

    print("Sanity checks:")
    for name, table in tables.items():
        total = sum(len(g) for g in table.groups)
        print(f"  {name}: poly={len(table.polynomial_uas)} coeffs, "
              f"non-poly={total} terms across {len(table.groups)} power groups")

    rust = emit_rust(x, y, s)
    # `Path(__file__).resolve().parent.parent` = `orts/arika/` since this
    # script lives at `arika/tools/generate_iau2006_tables.py`.
    arika_root = Path(__file__).resolve().parent.parent
    out_path = arika_root / "src" / "earth" / "iau2006" / "tables_gen.rs"
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(rust)
    print(f"Wrote {out_path} ({len(rust)} bytes)")


if __name__ == "__main__":
    main()
