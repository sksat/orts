#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["pyerfa>=2.0.0"]
# ///
"""Generate an ERFA reference fixture for the arika `iau2006` module.

The fixture is written as JSON and loaded by integration tests in
`arika/tests/iau2006_vs_erfa.rs`. It lets the Rust implementation be
cross-validated against the ERFA C library (a BSD-licensed, SOFA-board-
blessed fork of IAU SOFA) without making ERFA a runtime dependency of
arika or CI.

Usage (run from the repository root):

    uv run arika/tools/generate_iau2006_reference.py

Re-run whenever the reference set of `t` values is changed, the pyerfa
version is bumped, or new quantities need to be added to the fixture.
The generator is stable — running it twice produces identical output.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import erfa

# ─── Sample set ──────────────────────────────────────────────────
#
# TT Julian centuries since J2000.0. The samples deliberately cover
# a broad range so that non-trivial polynomial coefficients are exercised:
#
# - ± 1 century: bounds of IAU 2000A validity
# - ± 0.5 century: typical long-term propagation
# - ± 0.24 century: roughly the 2024 ↔ 1976 range (modern satellite era)
# - ± 0.1 / 0.01 century: nearby reference epochs
# - 0.0 exactly (J2000.0): captures constant terms and sign of t-linear
SAMPLES: tuple[float, ...] = (
    -1.0,
    -0.5,
    -0.24,
    -0.1,
    -0.01,
    0.0,
    0.01,
    0.1,
    0.2,
    0.24,
    0.5,
    1.0,
)


def fundamental_arguments(t: float) -> dict[str, float]:
    """Delaunay (F1..F5) and planetary longitudes (F6..F14) in radians."""
    return {
        "l": float(erfa.fal03(t)),
        "l_prime": float(erfa.falp03(t)),
        "f": float(erfa.faf03(t)),
        "d": float(erfa.fad03(t)),
        "omega": float(erfa.faom03(t)),
        "l_me": float(erfa.fame03(t)),
        "l_ve": float(erfa.fave03(t)),
        "l_e": float(erfa.fae03(t)),
        "l_ma": float(erfa.fama03(t)),
        "l_j": float(erfa.faju03(t)),
        "l_sa": float(erfa.fasa03(t)),
        "l_u": float(erfa.faur03(t)),
        "l_ne": float(erfa.fane03(t)),
        "p_a": float(erfa.fapa03(t)),
    }


def precession_fukushima_williams(t: float) -> dict[str, float]:
    """IAU 2006 precession Fukushima-Williams angles from `erfa.pfw06`.

    `erfa.pfw06(date1, date2)` takes a two-part TT Julian Date so we
    decompose `t` (TT centuries) into the J2000 reference JD (2451545.0)
    plus the offset in days (`t × 36525`). Returns radians.
    """
    J2000_JD = 2451545.0
    offset_days = t * 36525.0
    gamb, phib, psib, epsa = erfa.pfw06(J2000_JD, offset_days)
    return {
        "gamma_bar": float(gamb),
        "phi_bar": float(phib),
        "psi_bar": float(psib),
        "eps_a": float(epsa),
    }


def cip_xys(t: float) -> dict[str, float]:
    """IAU 2006/2000A_R06 CIP coordinates `X`, `Y` and CIO locator `s`.

    - `X`, `Y` are returned by `erfa.xy06(date1, date2)` in radians, and
      are computed from Tables 5.2a / 5.2b of IERS Conventions 2010 TN36
    - `s` is returned by `erfa.s06(date1, date2, x, y)` in radians, and
      is computed from Table 5.2d plus an `−X·Y/2` subtraction

    All three quantities take a two-part TT Julian Date, decomposed as
    `(J2000, t × 36525)` to minimise f64 cancellation.
    """
    J2000_JD = 2451545.0
    offset_days = t * 36525.0
    x, y = erfa.xy06(J2000_JD, offset_days)
    s = erfa.s06(J2000_JD, offset_days, x, y)
    return {
        "x": float(x),
        "y": float(y),
        "s": float(s),
    }


def gcrs_to_cirs_matrix(t: float) -> list[list[float]]:
    """3×3 celestial-to-intermediate rotation matrix at TT centuries `t`.

    Computed by `erfa.c2ixys(x, y, s)` where `(x, y, s)` come from the
    same `xy06` / `s06` call that [`cip_xys`] uses. The returned matrix
    is row-major, consistent with SOFA / ERFA layout: applying it to a
    column vector in GCRS gives the same vector in CIRS, i.e.
    `v_cirs = M · v_gcrs`.
    """
    J2000_JD = 2451545.0
    offset_days = t * 36525.0
    x, y = erfa.xy06(J2000_JD, offset_days)
    s = erfa.s06(J2000_JD, offset_days, x, y)
    m = erfa.c2ixys(x, y, s)
    return [[float(v) for v in row] for row in m]


def gcrs_to_itrs_matrix_zero_eop(t: float) -> dict:
    """Full GCRS→ITRS rotation matrix at TT centuries `t`, with every
    EOP quantity held at zero (`dUT1 = xp = yp = dX = dY = 0`).

    Computed by combining the ERFA pieces individually so that the
    Rust test can compare against a chain built from arika's
    `Rotation<Gcrs, Cirs>::iau2006`, `Rotation<Cirs, Tirs>::from_era`,
    and `Rotation<Tirs, Itrs>::polar_motion` each given the same
    zero-valued mock provider. We do NOT call `erfa.c2t06a` because
    that routine uses SOFA's internal IAU 2000A nutation series
    routing, which differs from the `xy06 + s06 + c2ixys` path we
    wired into arika in Phase 3A.

    With `dUT1 = 0` the UT1 and UTC Julian Dates coincide, so
    `era00(ut1) = era00(utc)`.
    """
    J2000_JD = 2451545.0
    offset_days = t * 36525.0

    # Q: GCRS → CIRS via c2ixys
    x, y = erfa.xy06(J2000_JD, offset_days)
    s = erfa.s06(J2000_JD, offset_days, x, y)
    rc2i = erfa.c2ixys(x, y, s)

    # R: CIRS → TIRS via −ERA about z
    era = erfa.era00(J2000_JD, offset_days)
    # iauRz(era, r): r := R_z(era) · r — equivalent to applying R_3(era)
    # which is the passive rotation matching arika's `rotation_z`.
    rc2tirs = erfa.rz(era, rc2i)

    # W: TIRS → ITRS via `iauPom00(xp=0, yp=0, sp=sp00(tt))`
    sp = erfa.sp00(J2000_JD, offset_days)
    rpom = erfa.pom00(0.0, 0.0, sp)

    # ITRS = W · (CIRS → TIRS) · (GCRS → CIRS) = W · rc2tirs
    rc2t = rpom @ rc2tirs

    return {
        "era": float(era),
        "sp": float(sp),
        "matrix": [[float(v) for v in row] for row in rc2t],
    }


def main() -> None:
    samples = []
    for t in SAMPLES:
        samples.append(
            {
                "t_tt_centuries_from_j2000": t,
                "fundamental_arguments": fundamental_arguments(t),
                "precession_fukushima_williams": precession_fukushima_williams(t),
                "cip_xys": cip_xys(t),
                "gcrs_to_cirs_matrix": gcrs_to_cirs_matrix(t),
                "gcrs_to_itrs_matrix_zero_eop": gcrs_to_itrs_matrix_zero_eop(t),
            }
        )

    fixture = {
        "description": (
            "IAU 2006 / 2000A_R06 reference values generated from ERFA "
            "(liberfa/erfa), a BSD-3-Clause fork of IAU SOFA. Used by "
            "arika/tests/iau2006_vs_erfa.rs as an independent oracle "
            "for the pure-Rust IAU 2006 implementation in "
            "arika/src/earth/iau2006/."
        ),
        "source": f"pyerfa {erfa.__version__}",
        "generator": "arika/tools/generate_iau2006_reference.py",
        "convention": "IAU 2006 / 2000A_R06, IERS Conventions 2010 (TN36)",
        "independent_variable": "t = TT Julian centuries since J2000.0",
        "units": {
            "fundamental_arguments": "rad (each value is fmod'd modulo 2*pi)",
            "precession_fukushima_williams": "rad",
            "cip_xys": "rad",
            "gcrs_to_cirs_matrix": "dimensionless, row-major 3x3 orthogonal",
            "gcrs_to_itrs_matrix_zero_eop": "dimensionless matrix + era (rad) + sp (rad)",
        },
        "samples": samples,
    }

    # `Path(__file__).resolve().parent.parent` = `orts/arika/` since this
    # script lives at `arika/tools/generate_iau2006_reference.py`.
    arika_root = Path(__file__).resolve().parent.parent
    out_path = arika_root / "tests" / "fixtures" / "iau2006_erfa_reference.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    # Emit JSON with Biome-compatible numeric formatting: the project-wide
    # `biome format` rule strips leading zeros from float exponents
    # (`e-05` → `e-5`), but Python's `json.dumps` uses `repr(float)`
    # which zero-pads. Normalise here so `pnpm biome check` stays clean.
    raw = json.dumps(fixture, indent=2)
    normalised = re.sub(r"(e[+-])0(\d)", r"\1\2", raw)
    out_path.write_text(normalised + "\n")
    print(f"Wrote {len(samples)} samples to {out_path}")


if __name__ == "__main__":
    main()
