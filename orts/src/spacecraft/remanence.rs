//! Remanent magnetization of MTQ rods' cores.
//!
//! A rod's core keeps part of its magnetization after the drive is switched
//! off, and how much depends on how it was driven. This model keeps it fixed
//! between drives: the slow relaxation of a real core after switch-off
//! (magnetic viscosity) and its temperature dependence are left out. Each rod's
//! remanence is a weighted sum of play operators of different widths (a
//! Prandtl–Ishlinskii model) on the normalized drive `v = c / max_moment`
//! (DESIGN.md "MTQ の rod の moment は、遅れて追従する駆動と残留磁化から決める").

use nalgebra::{DMatrix, DVector};

use super::mtq::MtqAssemblyCore;

/// One play operator of a rod's remanence.
///
/// Its state follows `z ← clamp(z, v − width, v + width)` on the normalized
/// drive `v`; what it leaves when the rod is switched off is
/// `weight · clamp(z, −width, width) / width` [A·m²].
#[derive(Debug, Clone, PartialEq)]
pub struct RemanencePlay {
    /// Half the play, in units of the rod's `max_moment`, in (0, 1).
    pub width: f64,
    /// The remanence this operator leaves at its fullest [A·m²].
    pub weight: f64,
}

/// Widths of the operators a remanence curve is fitted with: `j / FIT_WIDTHS`
/// for `j = 1 .. FIT_WIDTHS`. 32 steps of 1/32 of the limit keep the fitted
/// curve within a few percent of a smooth measured one.
const FIT_WIDTHS: usize = 32;
/// Points the linearly interpolated curve is sampled at for the fit, evenly
/// spaced in the drive; four per operator width.
const FIT_SAMPLES: usize = 4 * FIT_WIDTHS;
/// Most points a remanence curve may have. A measured curve has a few to a
/// few tens; past this, 31 operators follow it no better, and a curve taken
/// from a network request (`orts serve`'s `add_satellite`) cannot make the
/// fit arbitrarily costly.
pub const MAX_REMANENCE_CURVE_POINTS: usize = 64;

impl RemanencePlay {
    /// The one operator a datasheet's residual moment `m_r` [A·m²] gives: the
    /// remanence after a full drive is `m_r`, and from a demagnetized rod a
    /// drive above half the limit leaves a remanence growing linearly to it.
    pub fn from_residual_moment(m_r: f64) -> Vec<Self> {
        vec![Self {
            width: 0.5,
            weight: m_r,
        }]
    }

    /// What a demagnetized rod keeps after a drive `v` (normalized, in
    /// [0, 1]) and switching off, through this operator alone [A·m²].
    fn remanence_after_drive(&self, v: f64) -> f64 {
        self.weight * (v - self.width).clamp(0.0, self.width) / self.width
    }

    /// Operators reproducing a measured remanence curve: `curve` holds
    /// `(v, R)` pairs, `R` [A·m²] being what a demagnetized rod keeps after a
    /// drive of `v` times its limit and switching off.
    ///
    /// The operators' widths are spread evenly over (0, 1), and their weights
    /// fitted by non-negative least squares to the curve interpolated
    /// linearly from `(0, 0)` through the points. Returns the operators with
    /// a non-zero weight and the largest difference [A·m²] between the
    /// fitted curve and the points, which the caller judges.
    ///
    /// The fit runs on the curve divided by its largest remanence, and the
    /// weights are scaled back, so a curve and the same curve times any
    /// positive factor give the same operators up to that factor.
    ///
    /// # Errors
    /// If the curve is empty or has more than [`MAX_REMANENCE_CURVE_POINTS`]
    /// points, a drive is outside (0, 1] or not increasing,
    /// the last drive is not 1 (the full drive, without which the remanence
    /// after it would be extrapolated), or a remanence is negative or
    /// non-finite.
    pub fn fit_remanence_curve(curve: &[(f64, f64)]) -> Result<(Vec<Self>, f64), String> {
        if curve.is_empty() {
            return Err("the remanence curve has no points".into());
        }
        if curve.len() > MAX_REMANENCE_CURVE_POINTS {
            return Err(format!(
                "the remanence curve has {} points, more than {MAX_REMANENCE_CURVE_POINTS}",
                curve.len()
            ));
        }
        let mut last_v = 0.0;
        for &(v, r) in curve {
            if !(v.is_finite() && v > last_v && v <= 1.0) {
                return Err(format!(
                    "the remanence curve's drives must increase within (0, 1], got {v} after {last_v}"
                ));
            }
            if !(r.is_finite() && r >= 0.0) {
                return Err(format!(
                    "the remanence curve's remanences must be finite and non-negative, got {r}"
                ));
            }
            last_v = v;
        }
        if last_v != 1.0 {
            return Err(format!(
                "the remanence curve must end at the full drive, v = 1; it ends at {last_v}"
            ));
        }
        // Fitted on the curve scaled to a largest remanence of 1, so the
        // solver's tolerances do not depend on the curve's units.
        let scale = curve.iter().map(|&(_, r)| r).fold(0.0, f64::max);
        if scale == 0.0 {
            return Ok((Vec::new(), 0.0));
        }
        let interpolated = |v: f64| -> f64 {
            let mut prev = (0.0, 0.0);
            for &(cv, cr) in curve {
                if v <= cv {
                    return prev.1 + (cr - prev.1) * (v - prev.0) / (cv - prev.0);
                }
                prev = (cv, cr);
            }
            prev.1
        };
        let basis: Vec<Self> = (1..FIT_WIDTHS)
            .map(|j| Self {
                width: j as f64 / FIT_WIDTHS as f64,
                weight: 1.0,
            })
            .collect();
        let drives: Vec<f64> = (1..=FIT_SAMPLES)
            .map(|s| last_v * s as f64 / FIT_SAMPLES as f64)
            .chain(curve.iter().map(|&(v, _)| v))
            .collect();
        let a = DMatrix::from_fn(drives.len(), basis.len(), |i, j| {
            basis[j].remanence_after_drive(drives[i])
        });
        let b = DVector::from_iterator(
            drives.len(),
            drives.iter().map(|&v| interpolated(v) / scale),
        );
        let weights = nnls(&a, &b);
        let plays: Vec<Self> = basis
            .into_iter()
            .zip(weights.iter())
            .filter(|(_, w)| **w > 0.0)
            .map(|(p, &w)| Self {
                weight: w * scale,
                ..p
            })
            .collect();
        // A wide operator keeps only part of its weight after a full drive,
        // so its weight can exceed the curve's largest remanence; scaled back
        // near f64::MAX it may not be finite.
        if plays.iter().any(|p| !p.weight.is_finite()) {
            return Err(
                "the remanence curve's operators need a weight larger than the largest finite \
                 value"
                    .into(),
            );
        }
        let misfit = curve
            .iter()
            .map(|&(v, r)| {
                let fitted: f64 = plays.iter().map(|p| p.remanence_after_drive(v)).sum();
                (fitted - r).abs()
            })
            .fold(0.0, f64::max);
        Ok((plays, misfit))
    }
}

/// Non-negative least squares, `min ‖A x − b‖` subject to `x ≥ 0`, by the
/// active-set method of Lawson & Hanson (Solving Least Squares Problems,
/// 1974, ch. 23).
fn nnls(a: &DMatrix<f64>, b: &DVector<f64>) -> DVector<f64> {
    // Relative to the problem's scale: below it a gradient or a weight is 0.
    let tol = 1e-12 * (a.norm() * b.norm()).max(f64::MIN_POSITIVE);
    let n = a.ncols();
    let mut x = DVector::zeros(n);
    let mut passive = vec![false; n];
    let solve_passive = |passive: &[bool]| -> DVector<f64> {
        let cols: Vec<usize> = (0..n).filter(|&j| passive[j]).collect();
        let sub = a.select_columns(&cols);
        let s_p = sub
            .svd(true, true)
            .solve(b, 1e-14)
            .expect("the SVD was computed with both factors");
        let mut s = DVector::zeros(n);
        for (k, &j) in cols.iter().enumerate() {
            s[j] = s_p[k];
        }
        s
    };
    // Each outer step adds a column; 3n bounds the steps, as in the reference.
    for _ in 0..3 * n {
        let w = a.transpose() * (b - a * &x);
        let Some((t, _)) = (0..n)
            .filter(|&j| !passive[j] && w[j] > tol)
            .map(|j| (j, w[j]))
            .max_by(|p, q| p.1.total_cmp(&q.1))
        else {
            break;
        };
        passive[t] = true;
        loop {
            let s = solve_passive(&passive);
            if (0..n).filter(|&j| passive[j]).all(|j| s[j] > tol) {
                x = s;
                break;
            }
            // Step towards s until the first passive weight reaches zero.
            let alpha = (0..n)
                .filter(|&j| passive[j] && s[j] <= tol)
                .map(|j| x[j] / (x[j] - s[j]))
                .fold(f64::INFINITY, f64::min);
            x += (s - &x) * alpha;
            for j in 0..n {
                if passive[j] && x[j] <= tol {
                    passive[j] = false;
                    x[j] = 0.0;
                }
            }
        }
    }
    x
}

/// The remanent magnetization each MTQ rod's core keeps after a drive.
///
/// Per rod, a weighted sum of [`RemanencePlay`]s on the normalized drive. It
/// changes only when a command is applied, and applying the same command
/// again changes nothing, so the remanence does not depend on how many
/// controller ticks a command is held for.
///
/// The state belongs to whoever applies the commands; [`MtqAssemblyCore`]
/// stays stateless, and an [`MtqAssembly`](super::MtqAssembly) gets a
/// snapshot of [`Self::residual`].
#[derive(Debug, Clone)]
pub struct MtqRemanence {
    /// Each rod's operators.
    plays: Vec<Vec<RemanencePlay>>,
    /// Each rod's operator states `z`, normalized: `|z| ≤ 1 + width`.
    states: Vec<Vec<f64>>,
    /// Remanent moment each rod would keep if switched off now [A·m²].
    residual: Vec<f64>,
    /// Each rod's `max_moment` [A·m²], kept from the core this was built for
    /// so that no other core's limits can be applied to it.
    max_moments: Vec<f64>,
}

impl MtqRemanence {
    /// Demagnetized rods, each keeping `residual_moment[i]` [A·m²] after a full
    /// drive: one operator per rod ([`RemanencePlay::from_residual_moment`]).
    ///
    /// # Panics
    /// As [`Self::demagnetized_with_plays`].
    pub fn demagnetized(core: &MtqAssemblyCore, residual_moment: Vec<f64>) -> Self {
        let plays = residual_moment
            .into_iter()
            .map(RemanencePlay::from_residual_moment)
            .collect();
        Self::demagnetized_with_plays(core, plays)
    }

    /// Demagnetized rods with `plays[i]` the operators of rod `i`.
    ///
    /// # Panics
    /// Panics if the length differs from the number of MTQs, a width is
    /// outside (0, 1), a weight is negative or non-finite, or a rod's weights
    /// sum above its `max_moment` (its blended moment would then exceed it).
    pub fn demagnetized_with_plays(core: &MtqAssemblyCore, plays: Vec<Vec<RemanencePlay>>) -> Self {
        assert_eq!(
            plays.len(),
            core.num_mtqs(),
            "remanence length ({}) != MTQ count ({})",
            plays.len(),
            core.num_mtqs()
        );
        for (rod, mtq) in plays.iter().zip(core.mtqs()) {
            for p in rod {
                assert!(
                    p.width > 0.0 && p.width < 1.0,
                    "a remanence play's width must be within (0, 1), got {}",
                    p.width
                );
                assert!(
                    p.weight.is_finite() && p.weight >= 0.0,
                    "a remanence play's weight must be finite and non-negative, got {}",
                    p.weight
                );
            }
            let sum: f64 = rod.iter().map(|p| p.weight).sum();
            assert!(
                sum <= mtq.max_moment,
                "remanence must be within [0, max_moment = {}], got {sum}",
                mtq.max_moment
            );
        }
        let states = plays.iter().map(|rod| vec![0.0; rod.len()]).collect();
        let residual = vec![0.0; plays.len()];
        let max_moments = core.mtqs().iter().map(|m| m.max_moment).collect();
        Self {
            plays,
            states,
            residual,
            max_moments,
        }
    }

    /// Remanent moment each rod would keep if switched off now [A·m²], in the
    /// order of [`MtqAssemblyCore::mtqs`].
    pub fn residual(&self) -> &[f64] {
        &self.residual
    }

    /// Drive the rods at the clamped moments `clamped` [A·m²] (from
    /// [`MtqAssemblyCore::realized_rod_moments`] of the core this was built
    /// for).
    ///
    /// A non-finite moment is no drive the core can follow, and leaves that
    /// rod's remanence as it was.
    ///
    /// # Panics
    /// Panics if the length differs from the number of MTQs.
    pub fn apply(&mut self, clamped: &[f64]) {
        assert_eq!(
            clamped.len(),
            self.residual.len(),
            "clamped moments length != MTQ count"
        );
        for (rod, &c) in clamped.iter().enumerate() {
            let max_moment = self.max_moments[rod];
            if !c.is_finite() || max_moment <= 0.0 {
                continue;
            }
            // Normalized, so the states stay within 1 + width whatever the
            // limit: large limits cannot overflow.
            let v = (c / max_moment).clamp(-1.0, 1.0);
            let mut r = 0.0;
            for (p, z) in self.plays[rod].iter().zip(&mut self.states[rod]) {
                *z = z.clamp(v - p.width, v + p.width);
                r += p.weight * (z.clamp(-p.width, p.width) / p.width);
            }
            self.residual[rod] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacecraft::Mtq;
    use nalgebra::Vector3;

    // A 10 A·m² rod keeping 0.06 A·m² (0.6 %) after a full drive.
    const REM_MAX: f64 = 10.0;
    const REM_SAT: f64 = 0.06;

    fn one_rod() -> MtqRemanence {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), REM_MAX)]);
        MtqRemanence::demagnetized(&core, vec![REM_SAT])
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn a_full_drive_leaves_the_saturation_remanence_and_off_keeps_it() {
        let mut rem = one_rod();
        rem.apply(&[REM_MAX]);
        assert_close(rem.residual()[0], REM_SAT);
        rem.apply(&[0.0]);
        assert_close(rem.residual()[0], REM_SAT);
        rem.apply(&[-REM_MAX]);
        assert_close(rem.residual()[0], -REM_SAT);
    }

    /// The table in the PR: a reverse drive of a quarter of the limit halves
    /// the remanence instead of flipping it.
    #[test]
    fn a_weak_reverse_drive_reduces_the_remanence_without_flipping_it() {
        let mut rem = one_rod();
        rem.apply(&[REM_MAX]);
        rem.apply(&[-REM_MAX / 4.0]);
        assert_close(rem.residual()[0], REM_SAT / 2.0);
    }

    /// From a demagnetized core, a drive up to half the limit leaves nothing,
    /// and above it the remanence grows linearly to the saturation value.
    #[test]
    fn a_drive_from_demagnetized_leaves_remanence_only_above_half_the_limit() {
        for (drive, expected) in [(0.5, 0.0), (0.75, 0.5), (1.0, 1.0)] {
            let mut rem = one_rod();
            rem.apply(&[drive * REM_MAX]);
            assert_close(rem.residual()[0], expected * REM_SAT);
        }
    }

    /// Holding a command over one tick or over ten leaves the same remanence:
    /// the result must not depend on the controller's period. With several
    /// operators of different widths too.
    #[test]
    fn applying_the_same_command_again_changes_nothing() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), REM_MAX)]);
        let several = vec![
            RemanencePlay {
                width: 0.1,
                weight: 0.02,
            },
            RemanencePlay {
                width: 0.4,
                weight: 0.03,
            },
            RemanencePlay {
                width: 0.8,
                weight: 0.05,
            },
        ];
        for rem in [
            one_rod(),
            MtqRemanence::demagnetized_with_plays(&core, vec![several]),
        ] {
            for start in [REM_MAX, -REM_MAX, 0.0] {
                for c in [-10.0, -6.0, -2.5, 0.0, 3.0, 7.0, 10.0] {
                    let mut once = rem.clone();
                    once.apply(&[start]);
                    let mut many = once.clone();
                    once.apply(&[c]);
                    for _ in 0..10 {
                        many.apply(&[c]);
                    }
                    assert_eq!(once.residual(), many.residual(), "start {start}, c {c}");
                }
            }
        }
    }

    /// Driving back and forth with a decreasing amplitude demagnetizes the
    /// core: 10, -9, 8, ..., the play operator reaches 0 at the -5 step.
    #[test]
    fn an_alternating_decaying_drive_demagnetizes() {
        let mut rem = one_rod();
        for k in 0..10 {
            let amplitude = REM_MAX - k as f64;
            let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
            rem.apply(&[sign * amplitude]);
        }
        assert_close(rem.residual()[0], 0.0);
    }

    /// A non-finite command is not a drive the core can follow; the
    /// remanence keeps its value rather than becoming NaN.
    #[test]
    fn a_non_finite_command_leaves_the_remanence_alone() {
        let mut rem = one_rod();
        rem.apply(&[REM_MAX]);
        for c in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut r = rem.clone();
            r.apply(&[c]);
            assert_eq!(r.residual(), rem.residual(), "c {c}");
        }
    }

    /// A rod with a zero limit has no drive range; it keeps no remanence and
    /// produces no NaN.
    #[test]
    fn a_rod_with_a_zero_limit_keeps_no_remanence() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), 0.0)]);
        let mut rem = MtqRemanence::demagnetized(&core, vec![0.0]);
        rem.apply(&[0.0]);
        assert_eq!(rem.residual(), &[0.0]);
        assert_eq!(
            core.rod_moments_with_remanence(&[0.0], rem.residual()),
            vec![0.0]
        );
    }

    /// Limits near f64::MAX: the states are normalized, so neither the
    /// remanence nor the blended moment overflows.
    #[test]
    fn huge_limits_do_not_overflow() {
        let huge = 1e308;
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), huge)]);
        let mut rem = MtqRemanence::demagnetized(&core, vec![huge]);
        rem.apply(&[huge]);
        assert_eq!(rem.residual(), &[huge]);
        rem.apply(&[-huge]);
        assert_eq!(rem.residual(), &[-huge]);
        let u = core.rod_moments_with_remanence(&[huge], rem.residual());
        assert_eq!(u, vec![huge]);
    }

    /// A saturated operator keeps its whole weight however small the weight
    /// and its width: the state is normalized before the weight scales it.
    #[test]
    fn a_tiny_operator_keeps_its_weight() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), 1.0)]);
        let mut rem = MtqRemanence::demagnetized_with_plays(
            &core,
            vec![vec![RemanencePlay {
                width: 1e-100,
                weight: 1e-300,
            }]],
        );
        rem.apply(&[1.0]);
        assert_eq!(rem.residual(), &[1e-300]);
    }

    #[test]
    #[should_panic(expected = "remanence")]
    fn a_remanence_above_the_rod_limit_is_refused() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), 1.0)]);
        MtqRemanence::demagnetized(&core, vec![1.5]);
    }

    #[test]
    #[should_panic(expected = "width")]
    fn a_play_width_outside_the_unit_interval_is_refused() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), 1.0)]);
        MtqRemanence::demagnetized_with_plays(
            &core,
            vec![vec![RemanencePlay {
                width: 1.0,
                weight: 0.1,
            }]],
        );
    }

    /// Several operators turn the single operator's kinked curve into a
    /// smooth one: from a demagnetized rod the remanence after a drive is the
    /// sum of each operator's ramp, which starts at its width.
    #[test]
    fn several_plays_sum_their_ramps() {
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), REM_MAX)]);
        let plays = vec![
            RemanencePlay {
                width: 0.2,
                weight: 0.02,
            },
            RemanencePlay {
                width: 0.6,
                weight: 0.04,
            },
        ];
        for v in [0.1, 0.3, 0.5, 0.7, 1.0] {
            let mut rem = MtqRemanence::demagnetized_with_plays(&core, vec![plays.clone()]);
            rem.apply(&[v * REM_MAX]);
            // 0.2-wide: (v - 0.2) / 0.2 capped at 1; 0.6-wide: (v - 0.6) / 0.6.
            let expected =
                0.02 * ((v - 0.2) / 0.2).clamp(0.0, 1.0) + 0.04 * ((v - 0.6) / 0.6).clamp(0.0, 1.0);
            assert_close(rem.residual()[0], expected);
        }
    }

    /// A curve one operator makes exactly is fitted back to that operator's
    /// remanence after a full drive, and the fit stays on the points.
    #[test]
    fn a_curve_of_one_play_fits_back_to_it() {
        let truth = RemanencePlay {
            width: 0.25,
            weight: 0.05,
        };
        let curve: Vec<(f64, f64)> = [0.25, 0.375, 0.5, 0.75, 1.0]
            .iter()
            .map(|&v| (v, truth.remanence_after_drive(v)))
            .collect();
        let (plays, misfit) = RemanencePlay::fit_remanence_curve(&curve).unwrap();
        assert!(misfit < 1e-9, "misfit {misfit}");
        let full: f64 = plays.iter().map(|p| p.remanence_after_drive(1.0)).sum();
        assert_close(full, 0.05);
    }

    /// A smooth measured curve (here R = 0.06 v², sampled at five drives) is
    /// fitted within a few percent of its largest value, with the fitted
    /// remanence growing from small drives on instead of from half the limit.
    #[test]
    fn a_smooth_curve_is_fitted_closely() {
        let curve: Vec<(f64, f64)> = [0.2, 0.4, 0.6, 0.8, 1.0]
            .iter()
            .map(|&v| (v, 0.06 * v * v))
            .collect();
        let (plays, misfit) = RemanencePlay::fit_remanence_curve(&curve).unwrap();
        assert!(misfit < 0.03 * 0.06, "misfit {misfit}");
        let at = |v: f64| -> f64 { plays.iter().map(|p| p.remanence_after_drive(v)).sum() };
        assert!(at(0.3) > 0.0, "remanence below half the limit: {}", at(0.3));
        assert!(
            plays
                .iter()
                .all(|p| p.weight > 0.0 && p.width > 0.0 && p.width < 1.0)
        );
    }

    #[test]
    fn a_curve_out_of_order_or_out_of_range_is_refused() {
        for bad in [
            vec![],
            vec![(0.5, 0.01), (0.4, 0.02)],
            vec![(0.0, 0.0)],
            vec![(1.5, 0.01)],
            vec![(0.5, -0.01)],
            vec![(0.5, f64::NAN)],
            vec![(0.5, 0.01)],
        ] {
            assert!(RemanencePlay::fit_remanence_curve(&bad).is_err(), "{bad:?}");
        }
    }

    /// A curve and the same curve scaled to near f64::MAX fit to the same
    /// operators up to the scale; a curve of zeros leaves none.
    #[test]
    fn the_fit_does_not_depend_on_the_curves_scale() {
        let curve: Vec<(f64, f64)> = [0.2, 0.4, 0.6, 0.8, 1.0]
            .iter()
            .map(|&v| (v, 0.06 * v * v))
            .collect();
        let (small, _) = RemanencePlay::fit_remanence_curve(&curve).unwrap();
        // The largest remanence becomes 1e306.
        let factor = 1e306 / 0.06;
        let big: Vec<(f64, f64)> = curve.iter().map(|&(v, r)| (v, r * factor)).collect();
        let (large, misfit) = RemanencePlay::fit_remanence_curve(&big).unwrap();
        assert!(misfit < 0.03 * 1e306, "misfit {misfit}");
        assert_eq!(small.len(), large.len());
        for (s, l) in small.iter().zip(&large) {
            assert_eq!(s.width, l.width);
            assert!(
                (l.weight / factor - s.weight).abs() <= 1e-9 * s.weight,
                "{s:?} {l:?}"
            );
        }
        // A single full-drive point at the largest finite scale fits too.
        let (plays, misfit) = RemanencePlay::fit_remanence_curve(&[(1.0, 1e308)]).unwrap();
        assert!(
            !plays.is_empty() && misfit < 0.05 * 1e308,
            "misfit {misfit}"
        );
        // A curve rising only just before the full drive needs wide operators
        // with weights above its largest remanence: near f64::MAX they cannot
        // be finite, which is an error rather than an infinite operator.
        // Only operators about 0.97 wide start that late, and they keep about
        // 3% of their weight, so the weight would be some 30 times 1e308.
        let err = RemanencePlay::fit_remanence_curve(&[(0.97, 0.0), (1.0, 1e308)]).unwrap_err();
        assert!(err.contains("largest finite"), "{err}");
        let zeros = [(0.5, 0.0), (1.0, 0.0)];
        assert_eq!(
            RemanencePlay::fit_remanence_curve(&zeros).unwrap(),
            (vec![], 0.0)
        );
    }

    #[test]
    fn a_curve_with_too_many_points_is_refused() {
        let n = MAX_REMANENCE_CURVE_POINTS + 1;
        let curve: Vec<(f64, f64)> = (1..=n).map(|k| (k as f64 / n as f64, 0.01)).collect();
        let err = RemanencePlay::fit_remanence_curve(&curve).unwrap_err();
        assert!(err.contains("points"), "{err}");
        let ok: Vec<(f64, f64)> = curve[1..]
            .iter()
            .map(|&(_, r)| r)
            .enumerate()
            .map(|(k, r)| ((k + 1) as f64 / (n - 1) as f64, r))
            .collect();
        assert!(RemanencePlay::fit_remanence_curve(&ok).is_ok());
    }

    /// NNLS reaches the least residual over all non-negative x, compared with
    /// a brute force over every set of free columns (unconstrained least
    /// squares on the set, kept when it is non-negative). The matrices have
    /// correlated columns, so the active-set loop has to drop columns again.
    #[test]
    fn nnls_matches_a_brute_force_over_the_active_sets() {
        // A fixed linear congruential sequence: reproducible, no dependency.
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for _ in 0..40 {
            let (rows, cols) = (6, 4);
            let base: Vec<f64> = (0..rows).map(|_| next()).collect();
            let a = DMatrix::from_fn(rows, cols, |i, _| base[i] + 0.3 * next());
            let b = DVector::from_fn(rows, |_, _| next());
            let x = nnls(&a, &b);
            assert!(x.iter().all(|&v| v >= 0.0), "{x}");
            let best = (0u32..1 << cols)
                .filter_map(|mask| {
                    let set: Vec<usize> = (0..cols).filter(|j| mask >> j & 1 == 1).collect();
                    if set.is_empty() {
                        return Some(b.norm());
                    }
                    let sub = a.select_columns(&set);
                    let s = sub.clone().svd(true, true).solve(&b, 1e-14).unwrap();
                    s.iter().all(|&v| v >= 0.0).then(|| (&b - &sub * &s).norm())
                })
                .fold(f64::INFINITY, f64::min);
            let got = (&b - &a * &x).norm();
            assert!((got - best).abs() < 1e-9, "nnls {got} vs best {best}");
        }
    }

    /// NNLS on a small problem with a known non-negative solution recovers it,
    /// and clips a component the unconstrained solution would make negative.
    #[test]
    fn nnls_recovers_a_non_negative_solution_and_clips_negative_ones() {
        let a = DMatrix::from_row_slice(3, 2, &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        let x = nnls(&a, &DVector::from_row_slice(&[1.0, 2.0, 3.0]));
        assert!(
            (x[0] - 1.0).abs() < 1e-12 && (x[1] - 2.0).abs() < 1e-12,
            "{x}"
        );
        // Unconstrained: x = (-1, 2); with x ≥ 0 the first is 0.
        let a = DMatrix::from_row_slice(2, 2, &[1.0, 0.0, 0.0, 1.0]);
        let x = nnls(&a, &DVector::from_row_slice(&[-1.0, 2.0]));
        assert_eq!(x[0], 0.0);
        assert!((x[1] - 2.0).abs() < 1e-12);
    }
}
