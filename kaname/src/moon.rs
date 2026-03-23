use nalgebra::Vector3;

use crate::epoch::Epoch;

/// Moon position vector in ECI (J2000) frame [km].
///
/// Uses the analytical model from Meeus "Astronomical Algorithms" Chapter 47
/// with full periodic term tables (60 terms for longitude/distance, 60 for latitude),
/// E correction factor, and planetary perturbation corrections.
///
/// Accuracy: ~10" in ecliptic longitude, ~1% in distance (~4,000 km).
pub fn moon_position_eci(epoch: &Epoch) -> Vector3<f64> {
    let t = epoch.centuries_since_j2000();
    let t2 = t * t;
    let t3 = t2 * t;
    let t4 = t3 * t;

    // Fundamental arguments (degrees), full polynomial
    let lp = 218.3164477 + 481267.88123421 * t - 0.0015786 * t2 + t3 / 538841.0 - t4 / 65194000.0;
    let d = 297.8501921 + 445267.1114034 * t - 0.0018819 * t2 + t3 / 545868.0 - t4 / 113065000.0;
    let m = 357.5291092 + 35999.0502909 * t - 0.0001535 * t2 + t3 / 24490000.0;
    let mp = 134.9633964 + 477198.8675055 * t + 0.0087414 * t2 + t3 / 69699.0 - t4 / 14712000.0;
    let f = 93.2720950 + 483202.0175233 * t - 0.0036539 * t2 - t3 / 3526000.0 + t4 / 863310000.0;

    // Additional arguments for planetary corrections
    let a1 = (119.75 + 131.849 * t).to_radians();
    let a2 = (53.09 + 479264.290 * t).to_radians();
    let a3 = (313.45 + 481266.484 * t).to_radians();

    let d_r = d.to_radians();
    let m_r = m.to_radians();
    let mp_r = mp.to_radians();
    let f_r = f.to_radians();
    let lp_r = lp.to_radians();

    // E correction for solar terms
    let e = 1.0 - 0.002516 * t - 0.0000074 * t2;
    let e2 = e * e;

    // Table 47.A: periodic terms for longitude (Σl) and distance (Σr)
    // Each row: (D, M, M', F, Σl, Σr)
    // Σl in units of 0.000001 degrees (multiply sin(arg))
    // Σr in units of 0.001 km (multiply cos(arg))
    #[rustfmt::skip]
    const TABLE_A: [(i32, i32, i32, i32, f64, f64); 60] = [
        ( 0,  0,  1,  0,  6288774.0, -20905355.0),
        ( 2,  0, -1,  0,  1274027.0,  -3699111.0),
        ( 2,  0,  0,  0,   658314.0,  -2955968.0),
        ( 0,  0,  2,  0,   213618.0,   -569925.0),
        ( 0,  1,  0,  0,  -185116.0,     48888.0),
        ( 0,  0,  0,  2,  -114332.0,     -3149.0),
        ( 2,  0, -2,  0,    58793.0,    246158.0),
        ( 2, -1, -1,  0,    57066.0,   -152138.0),
        ( 2,  0,  1,  0,    53322.0,   -170733.0),
        ( 2, -1,  0,  0,    45758.0,   -204586.0),
        ( 0,  1, -1,  0,   -40923.0,   -129620.0),
        ( 1,  0,  0,  0,   -34720.0,    108743.0),
        ( 0,  1,  1,  0,   -30383.0,    104755.0),
        ( 2,  0,  0, -2,    15327.0,     10321.0),
        ( 0,  0,  1,  2,   -12528.0,         0.0),
        ( 0,  0,  1, -2,    10980.0,     79661.0),
        ( 4,  0, -1,  0,    10675.0,    -34782.0),
        ( 0,  0,  3,  0,    10034.0,    -23210.0),
        ( 4,  0, -2,  0,     8548.0,    -21636.0),
        ( 2,  1, -1,  0,    -7888.0,     24208.0),
        ( 2,  1,  0,  0,    -6766.0,     30824.0),
        ( 1,  0, -1,  0,    -5163.0,     -8379.0),
        ( 1,  1,  0,  0,     4987.0,    -16675.0),
        ( 2, -1,  1,  0,     4036.0,    -12831.0),
        ( 2,  0,  2,  0,     3994.0,    -10445.0),
        ( 4,  0,  0,  0,     3861.0,    -11650.0),
        ( 2,  0, -3,  0,     3665.0,     14403.0),
        ( 0,  1, -2,  0,    -2689.0,     -7003.0),
        ( 2,  0, -1,  2,    -2602.0,         0.0),
        ( 2, -1, -2,  0,     2390.0,     10056.0),
        ( 1,  0,  1,  0,    -2348.0,      6322.0),
        ( 2, -2,  0,  0,     2236.0,     -9884.0),
        ( 0,  1,  2,  0,    -2120.0,      5751.0),
        ( 0,  2,  0,  0,    -2069.0,         0.0),
        ( 2, -2, -1,  0,     2048.0,     -4950.0),
        ( 2,  0,  1, -2,    -1773.0,      4130.0),
        ( 2,  0,  0,  2,    -1595.0,         0.0),
        ( 4, -1, -1,  0,     1215.0,     -3958.0),
        ( 0,  0,  2,  2,    -1110.0,         0.0),
        ( 3,  0, -1,  0,     -892.0,      3258.0),
        ( 2,  1,  1,  0,     -810.0,      2616.0),
        ( 4, -1, -2,  0,      759.0,     -1897.0),
        ( 0,  2, -1,  0,     -713.0,     -2117.0),
        ( 2,  2, -1,  0,     -700.0,      2354.0),
        ( 2,  1, -2,  0,      691.0,         0.0),
        ( 2, -1,  0, -2,      596.0,         0.0),
        ( 4,  0,  1,  0,      549.0,     -1423.0),
        ( 0,  0,  4,  0,      537.0,     -1117.0),
        ( 4, -1,  0,  0,      520.0,     -1571.0),
        ( 1,  0, -2,  0,     -487.0,     -1739.0),
        ( 2,  1,  0, -2,     -399.0,         0.0),
        ( 0,  0,  2, -2,     -381.0,     -4421.0),
        ( 1,  1,  1,  0,      351.0,         0.0),
        ( 3,  0, -2,  0,     -340.0,         0.0),
        ( 4,  0, -3,  0,      330.0,         0.0),
        ( 2, -1,  2,  0,      327.0,         0.0),
        ( 0,  2,  1,  0,     -323.0,      1165.0),
        ( 1,  1, -1,  0,      299.0,         0.0),
        ( 2,  0,  3,  0,      294.0,         0.0),
        ( 2,  0, -1, -2,        0.0,      8752.0),
    ];

    // Table 47.B: periodic terms for latitude (Σb)
    // Each row: (D, M, M', F, Σb)
    // Σb in units of 0.000001 degrees (multiply sin(arg))
    #[rustfmt::skip]
    const TABLE_B: [(i32, i32, i32, i32, f64); 60] = [
        ( 0,  0,  0,  1,  5128122.0),
        ( 0,  0,  1,  1,   280602.0),
        ( 0,  0,  1, -1,   277693.0),
        ( 2,  0,  0, -1,   173237.0),
        ( 2,  0, -1,  1,    55413.0),
        ( 2,  0, -1, -1,    46271.0),
        ( 2,  0,  0,  1,    32573.0),
        ( 0,  0,  2,  1,    17198.0),
        ( 2,  0,  1, -1,     9266.0),
        ( 0,  0,  2, -1,     8822.0),
        ( 2, -1,  0, -1,     8216.0),
        ( 2,  0, -2, -1,     4324.0),
        ( 2,  0,  1,  1,     4200.0),
        ( 2,  1,  0, -1,    -3359.0),
        ( 2, -1, -1,  1,     2463.0),
        ( 2, -1,  0,  1,     2211.0),
        ( 2, -1, -1, -1,     2065.0),
        ( 0,  1, -1, -1,    -1870.0),
        ( 4,  0, -1, -1,     1828.0),
        ( 0,  1,  0,  1,    -1794.0),
        ( 0,  0,  0,  3,    -1749.0),
        ( 0,  1, -1,  1,    -1565.0),
        ( 1,  0,  0,  1,    -1491.0),
        ( 0,  1,  1,  1,    -1475.0),
        ( 0,  1,  1, -1,    -1410.0),
        ( 0,  1,  0, -1,    -1344.0),
        ( 1,  0,  0, -1,    -1335.0),
        ( 0,  0,  3,  1,     1107.0),
        ( 4,  0,  0, -1,     1021.0),
        ( 4,  0, -1,  1,      833.0),
        ( 0,  0,  1, -3,      777.0),
        ( 4,  0, -2,  1,      671.0),
        ( 2,  0,  0, -3,      607.0),
        ( 2,  0,  2, -1,      596.0),
        ( 2, -1,  1, -1,      491.0),
        ( 2,  0, -2,  1,     -451.0),
        ( 0,  0,  3, -1,      439.0),
        ( 2,  0,  2,  1,      422.0),
        ( 2,  0, -3, -1,      421.0),
        ( 2,  1, -1,  1,     -366.0),
        ( 2,  1,  0,  1,     -351.0),
        ( 4,  0,  0,  1,      331.0),
        ( 2, -1,  1,  1,      315.0),
        ( 2, -2,  0, -1,      302.0),
        ( 0,  0,  1,  3,     -283.0),
        ( 2,  1,  1, -1,     -229.0),
        ( 1,  1,  0, -1,      223.0),
        ( 1,  1,  0,  1,      223.0),
        ( 0,  1, -2, -1,     -220.0),
        ( 2,  1, -1, -1,     -220.0),
        ( 1,  0,  1,  1,     -185.0),
        ( 2, -1, -2, -1,      181.0),
        ( 0,  1,  2,  1,     -177.0),
        ( 4,  0, -2, -1,      176.0),
        ( 4, -1, -1, -1,      166.0),
        ( 1,  0,  1, -1,     -164.0),
        ( 4,  0,  1, -1,      132.0),
        ( 1,  0, -1, -1,     -119.0),
        ( 4, -1,  0, -1,      115.0),
        ( 2, -2,  0,  1,      107.0),
    ];

    // Compute Σl, Σr from Table 47.A
    let mut sigma_l: f64 = 0.0;
    let mut sigma_r: f64 = 0.0;
    for &(cd, cm, cmp, cf, sl, sr) in &TABLE_A {
        let arg = cd as f64 * d_r + cm as f64 * m_r + cmp as f64 * mp_r + cf as f64 * f_r;
        let e_factor = match cm.abs() {
            1 => e,
            2 => e2,
            _ => 1.0,
        };
        sigma_l += sl * e_factor * arg.sin();
        sigma_r += sr * e_factor * arg.cos();
    }

    // Compute Σb from Table 47.B
    let mut sigma_b: f64 = 0.0;
    for &(cd, cm, cmp, cf, sb) in &TABLE_B {
        let arg = cd as f64 * d_r + cm as f64 * m_r + cmp as f64 * mp_r + cf as f64 * f_r;
        let e_factor = match cm.abs() {
            1 => e,
            2 => e2,
            _ => 1.0,
        };
        sigma_b += sb * e_factor * arg.sin();
    }

    // Additive corrections
    sigma_l += 3958.0 * a1.sin() + 1962.0 * (lp_r - f_r).sin() + 318.0 * a2.sin();

    sigma_b += -2235.0 * lp_r.sin()
        + 382.0 * a3.sin()
        + 175.0 * (a1 - f_r).sin()
        + 175.0 * (a1 + f_r).sin()
        + 127.0 * (lp_r - mp_r).sin()
        - 115.0 * (lp_r + mp_r).sin();

    // Final coordinates
    let lambda = (lp + sigma_l * 1e-6).to_radians();
    let beta = (sigma_b * 1e-6).to_radians();
    let distance_km = 385000.56 + sigma_r * 0.001;

    // Ecliptic → Equatorial (ECI) conversion
    let epsilon = (23.439291 - 0.0130042 * t).to_radians();

    let cos_lam = lambda.cos();
    let sin_lam = lambda.sin();
    let cos_beta = beta.cos();
    let sin_beta = beta.sin();
    let cos_eps = epsilon.cos();
    let sin_eps = epsilon.sin();

    let x = distance_km * cos_beta * cos_lam;
    let y = distance_km * (cos_eps * cos_beta * sin_lam - sin_eps * sin_beta);
    let z = distance_km * (sin_eps * cos_beta * sin_lam + cos_eps * sin_beta);

    Vector3::new(x, y, z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moon_distance_range() {
        // Moon distance varies between ~356,500 km (perigee) and ~406,700 km (apogee)
        let dates = [
            Epoch::from_gregorian(2024, 3, 1, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 7, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 14, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 21, 0, 0, 0.0),
            Epoch::from_gregorian(2024, 3, 28, 0, 0, 0.0),
        ];

        for epoch in &dates {
            let pos = moon_position_eci(epoch);
            let dist = pos.magnitude();
            assert!(
                dist > 340_000.0 && dist < 420_000.0,
                "Moon distance at JD {} should be 340k-420k km, got {dist:.0} km",
                epoch.jd()
            );
        }
    }

    #[test]
    fn moon_mean_distance() {
        let n = 30;
        let epoch0 = Epoch::from_gregorian(2024, 1, 1, 0, 0, 0.0);
        let total_dist: f64 = (0..n)
            .map(|i| {
                let epoch = epoch0.add_seconds(i as f64 * 86400.0);
                moon_position_eci(&epoch).magnitude()
            })
            .sum();
        let mean_dist = total_dist / n as f64;

        assert!(
            (mean_dist - 384_400.0).abs() < 5_000.0,
            "Mean moon distance should be ~384,400 km, got {mean_dist:.0} km"
        );
    }

    #[test]
    fn moon_orbital_period() {
        let epoch0 = Epoch::from_gregorian(2024, 3, 10, 0, 0, 0.0);
        let pos0 = moon_position_eci(&epoch0).normalize();

        let epoch1 = epoch0.add_seconds(27.3 * 86400.0);
        let pos1 = moon_position_eci(&epoch1).normalize();

        let dot = pos0.dot(&pos1);
        assert!(
            dot > 0.9,
            "Moon should return near starting direction after ~27.3 days, dot={dot:.3}"
        );

        let epoch_half = epoch0.add_seconds(13.7 * 86400.0);
        let pos_half = moon_position_eci(&epoch_half).normalize();
        let dot_half = pos0.dot(&pos_half);
        assert!(
            dot_half < 0.0,
            "Moon should be roughly opposite after half orbit, dot={dot_half:.3}"
        );
    }

    #[test]
    fn moon_not_in_ecliptic() {
        let epoch = Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0);
        let pos = moon_position_eci(&epoch);
        let z_frac = pos.z.abs() / pos.magnitude();

        let mut max_z_frac = 0.0_f64;
        for i in 0..28 {
            let ep = epoch.add_seconds(i as f64 * 86400.0);
            let p = moon_position_eci(&ep);
            max_z_frac = max_z_frac.max(p.z.abs() / p.magnitude());
        }
        assert!(
            max_z_frac > 0.1,
            "Moon should have significant z-component at some point, max z/r = {max_z_frac:.3}"
        );
        let _ = z_frac;
    }

    #[test]
    fn moon_distance_apollo11_epoch() {
        // Apollo 11 TLI: 1969-07-16. Moon was near apogee, ~394,000 km (JPL Horizons)
        let epoch = Epoch::from_iso8601("1969-07-16T16:22:03Z").unwrap();
        let dist = moon_position_eci(&epoch).magnitude();
        // Meeus analytical model has ~2-3% distance error
        assert!(
            (dist - 394_000.0).abs() < 15_000.0,
            "Moon distance at Apollo 11 TLI should be ~394,000 km, got {dist:.0} km"
        );
    }
}
