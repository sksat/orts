//! Magnetic fields the spacecraft's own equipment produces at a magnetometer.
//!
//! A magnetometer reads the MTQs' fields as well as the geomagnetic one. The
//! coupling from each rod to a magnetometer is held by the magnetometer
//! ([`MtqCoupling`]); the rods' state at the sample is handed to the sensor
//! evaluation ([`OnboardMagneticSources`]). See DESIGN.md "搭載機器の磁場を磁気センサに入れる".

use nalgebra::Vector3;

use crate::spacecraft::Mtq;

/// μ₀ / 4π [T·m/A].
///
/// Exact before the 2019 SI redefinition; since then μ₀ is measured and
/// differs from 4π × 10⁻⁷ by ~10⁻¹⁰ relative, far below the point-dipole
/// approximation's own error.
const MU0_OVER_4PI: f64 = 1e-7;

/// Field the MTQ rods produce at one magnetometer, per unit rod moment.
///
/// Column `i` is the field \[T\] that rod `i` produces at the magnetometer, in
/// the body frame, per 1 A·m² of that rod's moment. With `u` the rods' realized
/// moments \[A·m²\] (see
/// [`MtqAssemblyCore::realized_rod_moments`](crate::spacecraft::MtqAssemblyCore::realized_rod_moments)),
/// the onboard field is `K · u`.
///
/// The columns are in the order of the MTQ assembly's rods.
#[derive(Debug, Clone, PartialEq)]
pub struct MtqCoupling {
    columns: Vec<Vector3<f64>>,
}

impl MtqCoupling {
    /// A coupling given column by column, e.g. as measured in a ground test.
    ///
    /// # Panics
    /// Panics if any component is non-finite.
    pub fn from_columns(columns: Vec<Vector3<f64>>) -> Self {
        assert!(
            columns.iter().all(|c| c.iter().all(|v| v.is_finite())),
            "MTQ coupling must be finite, got {columns:?}"
        );
        Self { columns }
    }

    /// A coupling computed by treating each rod as a point dipole.
    ///
    /// Rod `i` sits at `rod_positions_m[i]` with the axis of `mtqs[i]`; the
    /// magnetometer sits at `magnetometer_position_m`. Positions are in the body
    /// frame \[m\]. With `r` from the rod to the magnetometer, the column is
    ///
    /// ```text
    /// μ₀/(4π|r|³) · (3 r̂ (r̂ · a) − a)
    /// ```
    ///
    /// The point-dipole field is a far-field approximation: it degrades once
    /// the distance is comparable to the rod's length, which on a small
    /// spacecraft it often is. Prefer [`Self::from_columns`] with measured
    /// values when they exist.
    ///
    /// # Panics
    /// Panics if the position counts differ from the rod count, if a position
    /// is non-finite, or if a rod sits at the magnetometer.
    pub fn from_point_dipoles(
        magnetometer_position_m: Vector3<f64>,
        mtqs: &[Mtq],
        rod_positions_m: &[Vector3<f64>],
    ) -> Self {
        assert_eq!(
            rod_positions_m.len(),
            mtqs.len(),
            "rod position count ({}) != MTQ count ({})",
            rod_positions_m.len(),
            mtqs.len()
        );
        assert!(
            magnetometer_position_m.iter().all(|v| v.is_finite()),
            "magnetometer position must be finite, got {magnetometer_position_m:?}"
        );
        let columns = mtqs
            .iter()
            .zip(rod_positions_m)
            .map(|(mtq, rod)| {
                assert!(
                    rod.iter().all(|v| v.is_finite()),
                    "rod position must be finite, got {rod:?}"
                );
                let r = magnetometer_position_m - rod;
                let d = r.magnitude();
                assert!(d > 0.0, "a rod at {rod:?} sits at the magnetometer");
                let r_hat = r / d;
                let a = mtq.axis();
                MU0_OVER_4PI / d.powi(3) * (3.0 * r_hat * r_hat.dot(a) - a)
            })
            .collect();
        Self::from_columns(columns)
    }

    /// Number of rods the coupling has a column for.
    pub fn num_mtqs(&self) -> usize {
        self.columns.len()
    }

    /// The columns \[T per A·m²\], in rod order.
    pub fn columns(&self) -> &[Vector3<f64>] {
        &self.columns
    }

    /// The field \[T, body frame\] the rods produce at the magnetometer for the
    /// given realized rod moments \[A·m²\].
    ///
    /// # Panics
    /// Panics if `rod_moments.len() != self.num_mtqs()`.
    pub fn field(&self, rod_moments: &[f64]) -> Vector3<f64> {
        assert_eq!(
            rod_moments.len(),
            self.columns.len(),
            "rod moment count ({}) != MTQ coupling columns ({})",
            rod_moments.len(),
            self.columns.len()
        );
        self.columns
            .iter()
            .zip(rod_moments)
            .map(|(c, &u)| c * u)
            .sum()
    }
}

/// State of the onboard magnetic sources at a sensor sample.
///
/// There is deliberately no `Default`: a caller states that the spacecraft has
/// no sources with [`Self::none`], so forgetting to pass the MTQ state cannot
/// silently drop a configured coupling.
#[derive(Debug, Clone, PartialEq)]
pub struct OnboardMagneticSources {
    mtq_rod_moments: Option<Vec<f64>>,
}

impl OnboardMagneticSources {
    /// A spacecraft with no onboard magnetic sources.
    pub fn none() -> Self {
        Self {
            mtq_rod_moments: None,
        }
    }

    /// The MTQ rods' realized moments \[A·m²\] at the sample, in rod order.
    ///
    /// An MTQ assembly that is mounted but off is zeros, not [`Self::none`].
    pub fn with_mtq_rod_moments(mut self, rod_moments: Vec<f64>) -> Self {
        self.mtq_rod_moments = Some(rod_moments);
        self
    }

    /// The MTQ rods' moments, or `None` if the spacecraft has no MTQ.
    pub fn mtq_rod_moments(&self) -> Option<&[f64]> {
        self.mtq_rod_moments.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel_err(a: Vector3<f64>, b: Vector3<f64>) -> f64 {
        (a - b).magnitude() / b.magnitude()
    }

    /// On the dipole axis the field is `2 μ₀/(4π) m / d³` along the axis.
    #[test]
    fn point_dipole_on_axis_field() {
        let d = 0.1;
        let k = MtqCoupling::from_point_dipoles(
            Vector3::new(d, 0.0, 0.0),
            &[Mtq::new(Vector3::x(), 1.0)],
            &[Vector3::zeros()],
        );
        let expected = Vector3::new(2.0 * MU0_OVER_4PI / d.powi(3), 0.0, 0.0);
        assert!(rel_err(k.columns()[0], expected) < 1e-15);
    }

    /// On the equatorial plane the field is `−μ₀/(4π) m / d³`, antiparallel to
    /// the dipole.
    #[test]
    fn point_dipole_equatorial_field() {
        let d = 0.2;
        let k = MtqCoupling::from_point_dipoles(
            Vector3::new(0.0, 0.0, d) + Vector3::new(0.5, 0.5, 0.0),
            &[Mtq::new(Vector3::y(), 1.0)],
            &[Vector3::new(0.5, 0.5, 0.0)],
        );
        let expected = Vector3::new(0.0, -MU0_OVER_4PI / d.powi(3), 0.0);
        assert!(rel_err(k.columns()[0], expected) < 1e-15);
    }

    /// The column is per unit moment: a rod's `max_moment` does not enter it.
    #[test]
    fn point_dipole_column_ignores_max_moment() {
        let at = |max| {
            MtqCoupling::from_point_dipoles(
                Vector3::new(0.1, 0.05, -0.02),
                &[Mtq::new(Vector3::new(1.0, 1.0, 0.0), max)],
                &[Vector3::zeros()],
            )
        };
        assert_eq!(at(0.1), at(10.0));
    }

    #[test]
    fn field_is_the_columns_weighted_by_the_rod_moments() {
        let k = MtqCoupling::from_columns(vec![
            Vector3::new(1e-6, 0.0, 2e-7),
            Vector3::new(0.0, -3e-6, 0.0),
            Vector3::new(4e-7, 5e-7, 6e-6),
        ]);
        let b = k.field(&[2.0, -1.0, 0.5]);
        let expected = Vector3::new(2e-6 + 2e-7, 3e-6 + 2.5e-7, 4e-7 + 3e-6);
        assert!(rel_err(b, expected) < 1e-15);
    }

    #[test]
    #[should_panic(expected = "rod moment count (2) != MTQ coupling columns (3)")]
    fn field_rejects_a_rod_count_mismatch() {
        let k = MtqCoupling::from_columns(vec![Vector3::zeros(); 3]);
        let _ = k.field(&[1.0, 0.0]);
    }

    #[test]
    #[should_panic(expected = "MTQ coupling must be finite")]
    fn from_columns_rejects_non_finite() {
        let _ = MtqCoupling::from_columns(vec![Vector3::new(f64::NAN, 0.0, 0.0)]);
    }

    #[test]
    #[should_panic(expected = "sits at the magnetometer")]
    fn point_dipole_rejects_a_rod_at_the_magnetometer() {
        let _ = MtqCoupling::from_point_dipoles(
            Vector3::new(0.1, 0.0, 0.0),
            &[Mtq::new(Vector3::x(), 1.0)],
            &[Vector3::new(0.1, 0.0, 0.0)],
        );
    }

    #[test]
    #[should_panic(expected = "rod position must be finite")]
    fn point_dipole_rejects_a_non_finite_rod_position() {
        let _ = MtqCoupling::from_point_dipoles(
            Vector3::zeros(),
            &[Mtq::new(Vector3::x(), 1.0)],
            &[Vector3::new(f64::INFINITY, 0.0, 0.0)],
        );
    }

    #[test]
    #[should_panic(expected = "rod position count (1) != MTQ count (3)")]
    fn point_dipole_rejects_a_position_count_mismatch() {
        let _ = MtqCoupling::from_point_dipoles(
            Vector3::zeros(),
            &[
                Mtq::new(Vector3::x(), 1.0),
                Mtq::new(Vector3::y(), 1.0),
                Mtq::new(Vector3::z(), 1.0),
            ],
            &[Vector3::new(0.1, 0.0, 0.0)],
        );
    }
}
