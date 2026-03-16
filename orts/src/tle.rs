use std::f64::consts::PI;
use std::fmt;

use kaname::epoch::Epoch;

use crate::kepler::{KeplerianElements, mean_to_true_anomaly};

/// Error type for TLE parsing failures.
#[derive(Debug, Clone)]
pub enum TleParseError {
    /// Not enough lines in the input.
    InsufficientLines,
    /// Line 1 does not start with '1'.
    InvalidLine1Prefix,
    /// Line 2 does not start with '2'.
    InvalidLine2Prefix,
    /// A numeric field could not be parsed.
    InvalidField {
        line: u8,
        field: &'static str,
        value: String,
    },
}

impl fmt::Display for TleParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TleParseError::InsufficientLines => write!(f, "TLE requires at least 2 lines"),
            TleParseError::InvalidLine1Prefix => write!(f, "TLE line 1 must start with '1'"),
            TleParseError::InvalidLine2Prefix => write!(f, "TLE line 2 must start with '2'"),
            TleParseError::InvalidField { line, field, value } => {
                write!(f, "Invalid {field} on line {line}: '{value}'")
            }
        }
    }
}

impl std::error::Error for TleParseError {}

/// Parsed Two-Line Element set (TLE).
#[derive(Debug, Clone)]
pub struct Tle {
    /// Optional satellite name (from line 0).
    pub name: Option<String>,
    /// NORAD catalog number.
    pub satellite_number: u32,
    /// 4-digit epoch year.
    pub epoch_year: u32,
    /// Fractional day of year (1.0 = Jan 1 00:00 UTC).
    pub epoch_day: f64,
    /// Inclination \[rad\].
    pub inclination: f64,
    /// Right ascension of ascending node \[rad\].
    pub raan: f64,
    /// Eccentricity (dimensionless).
    pub eccentricity: f64,
    /// Argument of perigee \[rad\].
    pub argument_of_perigee: f64,
    /// Mean anomaly \[rad\].
    pub mean_anomaly: f64,
    /// Mean motion \[rad/s\].
    pub mean_motion: f64,
    /// B* drag term \[1/R_e\].
    pub bstar: f64,
}

impl Tle {
    /// Parse a TLE from text (2-line or 3-line format).
    ///
    /// Accepts:
    /// - 2 lines: line 1 + line 2
    /// - 3 lines: name + line 1 + line 2
    pub fn parse(text: &str) -> Result<Self, TleParseError> {
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        let (name, line1, line2) = match lines.len() {
            0 | 1 => return Err(TleParseError::InsufficientLines),
            2 => (None, lines[0], lines[1]),
            _ => {
                // Check if first line is a name or line 1
                if lines[0].starts_with('1') {
                    (None, lines[0], lines[1])
                } else {
                    (Some(lines[0].to_string()), lines[1], lines[2])
                }
            }
        };

        // Validate prefixes
        if !line1.starts_with('1') {
            return Err(TleParseError::InvalidLine1Prefix);
        }
        if !line2.starts_with('2') {
            return Err(TleParseError::InvalidLine2Prefix);
        }

        // Parse line 1
        let satellite_number = Self::parse_field::<u32>(line1, 2, 7, 1, "satellite_number")?;
        let epoch_year_2digit = Self::parse_field::<u32>(line1, 18, 20, 1, "epoch_year")?;
        let epoch_day = Self::parse_field::<f64>(line1, 20, 32, 1, "epoch_day")?;

        // Convert 2-digit year to 4-digit
        let epoch_year = if epoch_year_2digit >= 57 {
            1900 + epoch_year_2digit
        } else {
            2000 + epoch_year_2digit
        };

        // Parse B* drag term (columns 53-61, assumed decimal point notation)
        // Format: " NNNNN±E" where value = 0.NNNNN * 10^(±E)
        let bstar = Self::parse_assumed_decimal(line1, 53, 61, 1, "bstar")?;

        // Parse line 2
        let inclination_deg = Self::parse_field::<f64>(line2, 8, 16, 2, "inclination")?;
        let raan_deg = Self::parse_field::<f64>(line2, 17, 25, 2, "raan")?;

        // Eccentricity: implied leading decimal point (e.g., "0007417" → 0.0007417)
        let ecc_str = line2.get(26..33).ok_or(TleParseError::InvalidField {
            line: 2,
            field: "eccentricity",
            value: String::new(),
        })?;
        let eccentricity: f64 =
            format!("0.{}", ecc_str.trim())
                .parse()
                .map_err(|_| TleParseError::InvalidField {
                    line: 2,
                    field: "eccentricity",
                    value: ecc_str.to_string(),
                })?;

        let arg_perigee_deg = Self::parse_field::<f64>(line2, 34, 42, 2, "argument_of_perigee")?;
        let mean_anomaly_deg = Self::parse_field::<f64>(line2, 43, 51, 2, "mean_anomaly")?;
        let mean_motion_rev_day = Self::parse_field::<f64>(line2, 52, 63, 2, "mean_motion")?;

        Ok(Tle {
            name,
            satellite_number,
            epoch_year,
            epoch_day,
            inclination: inclination_deg.to_radians(),
            raan: raan_deg.to_radians(),
            eccentricity,
            argument_of_perigee: arg_perigee_deg.to_radians(),
            mean_anomaly: mean_anomaly_deg.to_radians(),
            mean_motion: mean_motion_rev_day * 2.0 * PI / 86400.0, // rev/day → rad/s
            bstar,
        })
    }

    /// Parse a fixed-width field from a TLE line.
    fn parse_field<T: std::str::FromStr>(
        line: &str,
        start: usize,
        end: usize,
        line_num: u8,
        field: &'static str,
    ) -> Result<T, TleParseError> {
        let s = line.get(start..end).ok_or(TleParseError::InvalidField {
            line: line_num,
            field,
            value: String::new(),
        })?;
        s.trim().parse().map_err(|_| TleParseError::InvalidField {
            line: line_num,
            field,
            value: s.to_string(),
        })
    }

    /// Parse a field in "assumed decimal point" notation (e.g., "30000-4" → 0.30000e-4).
    ///
    /// TLE uses this format for B* and second derivative of mean motion.
    /// Format: " NNNNN±E" or "+NNNNN±E" or "-NNNNN±E"
    fn parse_assumed_decimal(
        line: &str,
        start: usize,
        end: usize,
        line_num: u8,
        field: &'static str,
    ) -> Result<f64, TleParseError> {
        let s = line.get(start..end).ok_or(TleParseError::InvalidField {
            line: line_num,
            field,
            value: String::new(),
        })?;
        let s = s.trim();

        if s == "00000-0" || s == "00000+0" || s.is_empty() {
            return Ok(0.0);
        }

        // Find the exponent sign (last '+' or '-' that isn't the leading sign)
        let (mantissa_str, exp_str) = if let Some(pos) = s[1..].rfind(['+', '-']) {
            let pos = pos + 1; // adjust for the [1..] offset
            (&s[..pos], &s[pos..])
        } else {
            return Err(TleParseError::InvalidField {
                line: line_num,
                field,
                value: s.to_string(),
            });
        };

        // Prepend "0." to mantissa to get the assumed decimal
        let mantissa: f64 = format!("0.{}", mantissa_str.trim_start_matches(['+', '-', ' ']))
            .parse()
            .map_err(|_| TleParseError::InvalidField {
                line: line_num,
                field,
                value: s.to_string(),
            })?;

        let exp: i32 = exp_str.parse().map_err(|_| TleParseError::InvalidField {
            line: line_num,
            field,
            value: s.to_string(),
        })?;

        let sign = if mantissa_str.starts_with('-') {
            -1.0
        } else {
            1.0
        };

        Ok(sign * mantissa * 10.0_f64.powi(exp))
    }

    /// Compute the TLE epoch as an [`Epoch`] (Julian Date).
    pub fn epoch(&self) -> Epoch {
        let year_2digit = if self.epoch_year >= 2000 {
            self.epoch_year - 2000
        } else {
            self.epoch_year - 1900
        };
        Epoch::from_tle_epoch(year_2digit, self.epoch_day)
    }

    /// Compute semi-major axis from mean motion: `a = (μ/n²)^(1/3)`.
    pub fn semi_major_axis(&self, mu: f64) -> f64 {
        (mu / (self.mean_motion * self.mean_motion)).cbrt()
    }

    /// Convert TLE to classical Keplerian elements.
    ///
    /// Converts mean anomaly → true anomaly using Kepler's equation solver,
    /// and computes semi-major axis from mean motion.
    pub fn to_keplerian_elements(&self, mu: f64) -> KeplerianElements {
        let a = self.semi_major_axis(mu);
        let true_anomaly = mean_to_true_anomaly(self.mean_anomaly, self.eccentricity);
        KeplerianElements {
            semi_major_axis: a,
            eccentricity: self.eccentricity,
            inclination: self.inclination,
            raan: self.raan,
            argument_of_periapsis: self.argument_of_perigee,
            true_anomaly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaname::body::KnownBody;
    use kaname::constants::MU_EARTH;

    const ISS_TLE: &str = "\
ISS (ZARYA)
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000";

    const ISS_TLE_2LINE: &str = "\
1 25544U 98067A   24079.50000000  .00016717  00000-0  30000-4 0  9993
2 25544  51.6400 208.6520 0007417  35.3910 324.7580 15.49561654480000";

    // GEO satellite (INTELSAT 10-02)
    const GEO_TLE: &str = "\
1 28358U 04022A   24079.50000000  .00000012  00000-0  00000+0 0  9993
2 28358   0.0300 275.4700 0003500 135.2000 224.8000  1.00271000 72000";

    #[test]
    fn parse_iss_3line() {
        let tle = Tle::parse(ISS_TLE).unwrap();
        assert_eq!(tle.name.as_deref(), Some("ISS (ZARYA)"));
        assert_eq!(tle.satellite_number, 25544);
        assert_eq!(tle.epoch_year, 2024);
        assert!((tle.epoch_day - 79.5).abs() < 1e-6);
        assert!((tle.inclination.to_degrees() - 51.64).abs() < 0.01);
        assert!((tle.raan.to_degrees() - 208.652).abs() < 0.01);
        assert!((tle.eccentricity - 0.0007417).abs() < 1e-8);
        assert!((tle.argument_of_perigee.to_degrees() - 35.391).abs() < 0.01);
        assert!((tle.mean_anomaly.to_degrees() - 324.758).abs() < 0.01);
        // Mean motion: 15.4956 rev/day
        let mm_rev_day = tle.mean_motion * 86400.0 / (2.0 * PI);
        assert!(
            (mm_rev_day - 15.4956165).abs() < 0.001,
            "mean motion: {mm_rev_day} rev/day"
        );
    }

    #[test]
    fn parse_iss_2line() {
        let tle = Tle::parse(ISS_TLE_2LINE).unwrap();
        assert!(tle.name.is_none());
        assert_eq!(tle.satellite_number, 25544);
        assert!((tle.inclination.to_degrees() - 51.64).abs() < 0.01);
    }

    #[test]
    fn parse_geo_satellite() {
        let tle = Tle::parse(GEO_TLE).unwrap();
        assert_eq!(tle.satellite_number, 28358);
        assert!(
            tle.inclination.to_degrees() < 1.0,
            "GEO should have near-zero inclination"
        );
        // Mean motion ~1.0 rev/day → near GEO
        let mm_rev_day = tle.mean_motion * 86400.0 / (2.0 * PI);
        assert!(
            (mm_rev_day - 1.0027).abs() < 0.01,
            "GEO mean motion: {mm_rev_day} rev/day"
        );
    }

    #[test]
    fn parse_error_insufficient_lines() {
        let result = Tle::parse("only one line");
        assert!(result.is_err());
    }

    #[test]
    fn parse_error_invalid_prefix() {
        let result = Tle::parse("X invalid line 1\n2 25544  51.6400 ...");
        assert!(result.is_err());
    }

    #[test]
    fn iss_epoch() {
        let tle = Tle::parse(ISS_TLE).unwrap();
        let epoch = tle.epoch();
        let dt = epoch.to_datetime();
        assert_eq!(dt.year, 2024);
        assert_eq!(dt.month, 3);
        assert_eq!(dt.day, 19);
        assert_eq!(dt.hour, 12);
    }

    #[test]
    fn iss_semi_major_axis() {
        let tle = Tle::parse(ISS_TLE).unwrap();
        let a = tle.semi_major_axis(MU_EARTH);
        // ISS altitude ~400km → a ≈ 6778 km
        let earth_radius = KnownBody::Earth.properties().radius;
        let altitude = a - earth_radius;
        assert!(
            (400.0 - altitude).abs() < 30.0,
            "ISS altitude should be ~400km, got {altitude:.1}km (a={a:.1}km)"
        );
    }

    #[test]
    fn iss_keplerian_elements() {
        let tle = Tle::parse(ISS_TLE).unwrap();
        let elements = tle.to_keplerian_elements(MU_EARTH);

        // Check that semi-major axis matches
        let a = tle.semi_major_axis(MU_EARTH);
        assert!((elements.semi_major_axis - a).abs() < 1e-6);

        // Check inclination matches
        assert!((elements.inclination - tle.inclination).abs() < 1e-12);

        // Eccentricity should match
        assert!((elements.eccentricity - tle.eccentricity).abs() < 1e-12);
    }

    #[test]
    fn iss_state_vector_plausible() {
        let tle = Tle::parse(ISS_TLE).unwrap();
        let elements = tle.to_keplerian_elements(MU_EARTH);
        let (pos, vel) = elements.to_state_vector(MU_EARTH);

        let r = pos.magnitude();
        let v = vel.magnitude();
        let earth_radius = KnownBody::Earth.properties().radius;

        // Altitude should be ~400 km
        let altitude = r - earth_radius;
        assert!(
            (400.0 - altitude).abs() < 30.0,
            "ISS altitude from state vector: {altitude:.1} km"
        );

        // Velocity should be ~7.7 km/s for LEO
        assert!((v - 7.66).abs() < 0.2, "ISS velocity: {v:.3} km/s");

        // Verify energy conservation: ε = v²/2 - μ/r = -μ/(2a)
        let energy = v * v / 2.0 - MU_EARTH / r;
        let expected_energy = -MU_EARTH / (2.0 * elements.semi_major_axis);
        assert!(
            (energy - expected_energy).abs() / expected_energy.abs() < 1e-10,
            "Energy mismatch: {energy} vs {expected_energy}"
        );
    }

    #[test]
    fn geo_semi_major_axis() {
        let tle = Tle::parse(GEO_TLE).unwrap();
        let a = tle.semi_major_axis(MU_EARTH);
        // GEO altitude ~35786 km → a ≈ 42164 km
        assert!(
            (a - 42164.0).abs() < 50.0,
            "GEO semi-major axis should be ~42164km, got {a:.1}km"
        );
    }

    #[test]
    fn three_line_and_two_line_produce_same_result() {
        let tle3 = Tle::parse(ISS_TLE).unwrap();
        let tle2 = Tle::parse(ISS_TLE_2LINE).unwrap();

        assert_eq!(tle3.satellite_number, tle2.satellite_number);
        assert!((tle3.inclination - tle2.inclination).abs() < 1e-15);
        assert!((tle3.raan - tle2.raan).abs() < 1e-15);
        assert!((tle3.eccentricity - tle2.eccentricity).abs() < 1e-15);
        assert!((tle3.mean_motion - tle2.mean_motion).abs() < 1e-15);
    }

    #[test]
    fn iss_bstar() {
        // ISS TLE has "30000-4" → 0.30000e-4 = 3.0e-5
        let tle = Tle::parse(ISS_TLE).unwrap();
        assert!(
            (tle.bstar - 3.0e-5).abs() < 1e-10,
            "ISS B* should be 3.0e-5, got {:.6e}",
            tle.bstar
        );
    }

    #[test]
    fn geo_bstar_zero() {
        // GEO TLE has "00000+0" → 0.0
        let tle = Tle::parse(GEO_TLE).unwrap();
        assert_eq!(tle.bstar, 0.0, "GEO B* should be 0.0, got {}", tle.bstar);
    }
}
