#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# dependencies = ["pyerfa>=2.0.0", "numpy>=1.26"]
# ///
"""Generate an ERFA reference fixture for the arika TEME↔GCRS reduction.

The fixture is JSON, loaded by `arika/tests/teme_vs_erfa.rs`, and lets the
Rust IAU-76/FK5 equinox-based reduction (GMST1982, equation of the
equinoxes, IAU-80 nutation, IAU-76 precession, and the composed TEME→J2000
rotation) be cross-validated against ERFA (a BSD-licensed SOFA fork) without
making ERFA a runtime/CI dependency.

This is the *component* oracle (formula-level, ~1e-11). The composed
TEME→J2000 matrix sign/convention is additionally anchored against Orekit
(see `generate_teme_orekit_reference.py`), which implements an authoritative
TEME frame.

Usage (from the repository root):

    uv run arika/tools/generate_teme_reference.py
"""

from __future__ import annotations

import json
from pathlib import Path

import erfa
import numpy as np

J2000_JD = 2451545.0

# TT Julian centuries since J2000.0, spanning the modern satellite era and
# beyond so polynomial terms are exercised. GMST82 is evaluated treating the
# same JD as UT1 (the test validates the formula, not a UT1 value).
SAMPLES = (-1.0, -0.5, -0.24, -0.1, -0.01, 0.0, 0.01, 0.1, 0.2, 0.24, 0.5, 1.0)

# A fixed, arbitrary TEME position [km] used to validate the composed rotation
# acting on a vector (catches matrix-layout / quaternion bugs).
TEME_VEC = [4500.0, -3000.0, 5000.0]


def rot3(theta: float) -> np.ndarray:
    """Passive rotation about the z-axis by `theta` (ERFA / SOFA convention)."""
    return np.array(erfa.rz(theta, erfa.ir()))


def sample(t: float) -> dict:
    offset = t * 36525.0
    # IAU-80 nutation (Δψ, Δε) and IAU-80 mean obliquity ε̄, all TT-based.
    dpsi, deps = (float(v) for v in erfa.nut80(J2000_JD, offset))
    epsa = float(erfa.obl80(J2000_JD, offset))
    # Equation of the equinoxes (1994 model: Δψ·cos ε̄ + small Ω terms).
    eqeq = float(erfa.eqeq94(J2000_JD, offset))
    # IAU-76 precession matrix (J2000 → mean-of-date) and the combined
    # IAU-76/80 precession-nutation matrix (J2000 → true-of-date).
    pmat76 = np.array(erfa.pmat76(J2000_JD, offset))
    pnm80 = np.array(erfa.pnm80(J2000_JD, offset))
    # GMST 1982 (UT1-based; here the same JD is treated as UT1 to validate the
    # polynomial form independent of any dUT1).
    gmst82 = float(erfa.gmst82(J2000_JD, offset))

    # TEME → J2000 (≈GCRS, frame bias ~tens of mas neglected):
    #   r_TOD   = ROT3(-Eqe) · r_TEME           (TEME differs from TOD by Eqe)
    #   r_J2000 = pnm80ᵀ · r_TOD
    # so M = pnm80ᵀ · ROT3(-Eqe). (The -Eqe sign is anchored by Orekit.)
    teme_to_j2000 = pnm80.T @ rot3(-eqeq)
    j2000_vec = (teme_to_j2000 @ np.array(TEME_VEC)).tolist()

    return {
        "t": t,
        "gmst82": gmst82,
        "equation_of_equinoxes": eqeq,
        "nutation": {"dpsi": dpsi, "deps": deps},
        "mean_obliquity": epsa,
        "precession_matrix_pmat76": [[float(v) for v in row] for row in pmat76],
        "prec_nut_matrix_pnm80": [[float(v) for v in row] for row in pnm80],
        "teme_to_j2000": [[float(v) for v in row] for row in teme_to_j2000],
        "j2000_vec": [float(v) for v in j2000_vec],
    }


def main() -> None:
    fixture = {
        "_comment": (
            "ERFA (pyerfa) reference for the arika TEME↔GCRS IAU-76/FK5 "
            "reduction. Regenerate with: uv run "
            "arika/tools/generate_teme_reference.py"
        ),
        "erfa_version": erfa.__version__,
        "j2000_jd": J2000_JD,
        "teme_vec": TEME_VEC,
        "samples": [sample(t) for t in SAMPLES],
    }
    out = Path("arika/tests/fixtures/teme_erfa_reference.json")
    out.write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"wrote {out} ({len(fixture['samples'])} samples, erfa {erfa.__version__})")


if __name__ == "__main__":
    main()
