//! GFZ space weather data parser.
//!
//! Parses the GFZ Helmholtz Centre Kp/ap/Ap/SN/F10.7 format
//! (`Kp_ap_Ap_SN_F107_since_1932.txt`) and converts to [`CssiData`]
//! for use with [`CssiSpaceWeather`].
//!
//! ## Data source
//!
//! - Kp/Ap: GFZ Helmholtz Centre for Geosciences (CC BY 4.0)
//!   Matzka et al. (2021), doi:10.1029/2020SW002641
//! - F10.7: NRCan DRAO / NOAA SWPC (public domain)
//! - Sunspot Number: WDC-SILSO (CC BY-NC 4.0)

use std::fmt;

use arika::epoch::Epoch;

use crate::cssi::{CssiDailyRecord, CssiData};

/// Parse error for GFZ space weather files.
#[derive(Debug)]
pub enum GfzParseError {
    /// Failed to parse a numeric field.
    ParseField {
        line_number: usize,
        field: &'static str,
        value: String,
    },
    /// No data records found.
    NoData,
}

impl fmt::Display for GfzParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParseField {
                line_number,
                field,
                value,
            } => write!(f, "line {line_number}: failed to parse {field}: {value:?}"),
            Self::NoData => write!(f, "no data records found in GFZ file"),
        }
    }
}

impl std::error::Error for GfzParseError {}

/// Parse GFZ `Kp_ap_Ap_SN_F107_*.txt` format into [`CssiData`].
///
/// The GFZ format is space-separated with 40 header lines starting with `#`.
/// Each data line contains: YYYY MM DD days days_m Bsr dB Kp1..Kp8 ap1..ap8 Ap SN F10.7obs F10.7adj D
///
/// Missing values are indicated by -1 (int) or -1.0 (float).
///
/// # Limitations
///
/// - Pre-1947 records have no F10.7 data. These are backfilled with the
///   centered average when possible, but very early records (1932–1946)
///   may retain F10.7=0.0 which is physically invalid for NRLMSISE-00.
///   Use CelesTrak CSSI format for recent data (post-2000) to avoid this.
/// - The 81-day centered average uses a truncated window at dataset edges,
///   which slightly biases `f107_avg` for the first/last 40 days.
pub fn parse_gfz(text: &str) -> Result<CssiData, GfzParseError> {
    let mut records = Vec::new();

    for (i, line) in text.lines().enumerate() {
        let line_number = i + 1;
        let trimmed = line.trim();

        // Skip header lines (start with #) and empty lines
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        // Expected: YYYY MM DD days days_m Bsr dB Kp1..8 ap1..8 Ap SN F10.7obs F10.7adj D
        // Indices:  0    1  2  3    4      5   6  7..14  15..22 23 24 25       26       27
        if fields.len() < 27 {
            continue; // Skip short lines
        }

        let parse_int = |idx: usize, field: &'static str| -> Result<i64, GfzParseError> {
            fields[idx]
                .parse::<i64>()
                .map_err(|_| GfzParseError::ParseField {
                    line_number,
                    field,
                    value: fields[idx].to_string(),
                })
        };

        let parse_float = |idx: usize, field: &'static str| -> Result<f64, GfzParseError> {
            fields[idx]
                .parse::<f64>()
                .map_err(|_| GfzParseError::ParseField {
                    line_number,
                    field,
                    value: fields[idx].to_string(),
                })
        };

        let year = parse_int(0, "year")? as i32;
        let month = parse_int(1, "month")? as u32;
        let day = parse_int(2, "day")? as u32;

        // ap1..ap8 at indices 15..22
        let ap_3h_raw: [i64; 8] = [
            parse_int(15, "ap1")?,
            parse_int(16, "ap2")?,
            parse_int(17, "ap3")?,
            parse_int(18, "ap4")?,
            parse_int(19, "ap5")?,
            parse_int(20, "ap6")?,
            parse_int(21, "ap7")?,
            parse_int(22, "ap8")?,
        ];

        // Ap at index 23
        let ap_daily_raw = parse_int(23, "Ap")?;

        // F10.7obs at index 25
        let f107_obs = parse_float(25, "F10.7obs")?;

        // Handle missing data (-1 for int, -1.0 for float).
        // GFZ uses -1/-1.0 for early records (pre-1947 for F10.7) and rare gaps.
        // We preserve the record (daily continuity required by CssiSpaceWeather)
        // but use quiet-condition defaults. The F10.7 centered average
        // (computed below) excludes 0.0 values, so missing F10.7 days don't
        // corrupt the average used by NRLMSISE-00.
        let ap_3h = ap_3h_raw.map(|v| if v < 0 { 0.0 } else { v as f64 });
        let ap_daily = if ap_daily_raw < 0 {
            0.0
        } else {
            ap_daily_raw as f64
        };
        let f107_obs = if f107_obs < 0.0 { 0.0 } else { f107_obs };

        // GFZ doesn't provide 81-day centered average; computed after parsing
        let f107_obs_ctr81 = f107_obs;

        let jd_midnight = Epoch::from_gregorian(year, month, day, 0, 0, 0.0).jd();

        records.push(CssiDailyRecord {
            jd_midnight,
            year,
            month,
            day,
            ap_3h,
            ap_daily,
            f107_obs,
            f107_obs_ctr81,
        });
    }

    if records.is_empty() {
        return Err(GfzParseError::NoData);
    }

    records.sort_by(|a, b| a.jd_midnight.partial_cmp(&b.jd_midnight).unwrap());

    // Compute rolling 81-day centered average for F10.7
    compute_f107_centered_avg(&mut records);

    // Backfill missing F10.7 obs (0.0) with the computed centered average.
    // This prevents CssiSpaceWeather from returning 0.0 as f107_daily,
    // which is physically impossible and would corrupt NRLMSISE-00 results.
    for record in &mut records {
        if record.f107_obs <= 0.0 && record.f107_obs_ctr81 > 0.0 {
            record.f107_obs = record.f107_obs_ctr81;
        }
    }

    CssiData::from_records(records).map_err(|_| GfzParseError::NoData)
}

/// Compute 81-day centered average of F10.7 for each record.
fn compute_f107_centered_avg(records: &mut [CssiDailyRecord]) {
    let n = records.len();
    let f107_values: Vec<f64> = records.iter().map(|r| r.f107_obs).collect();

    for (i, record) in records.iter_mut().enumerate() {
        let start = i.saturating_sub(40);
        let end = (i + 41).min(n);
        let mut sum = 0.0;
        let mut count = 0;
        for val in &f107_values[start..end] {
            if *val > 0.0 {
                sum += val;
                count += 1;
            }
        }
        if count > 0 {
            record.f107_obs_ctr81 = sum / count as f64;
        }
    }
}

/// Detect whether text is CSSI or GFZ format.
pub fn detect_format(text: &str) -> SpaceWeatherFormat {
    for line in text.lines().take(5) {
        let trimmed = line.trim();
        if trimmed.starts_with("DATATYPE CssiSpaceWeather") {
            return SpaceWeatherFormat::Cssi;
        }
        if trimmed.starts_with("# PURPOSE:") || trimmed.starts_with("# LICENSE:") {
            return SpaceWeatherFormat::Gfz;
        }
    }
    // Default heuristic: if first non-empty line starts with #, assume GFZ
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            return if trimmed.starts_with('#') {
                SpaceWeatherFormat::Gfz
            } else {
                SpaceWeatherFormat::Cssi
            };
        }
    }
    SpaceWeatherFormat::Cssi
}

/// Detected space weather data format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceWeatherFormat {
    /// CelesTrak CSSI format (SW-Last5Years.txt, SW-All.txt).
    Cssi,
    /// GFZ Helmholtz Centre format (Kp_ap_Ap_SN_F107_*.txt).
    Gfz,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_test_fixture() -> String {
        std::fs::read_to_string("tests/fixtures/gfz_test_weather.txt").unwrap()
    }

    #[test]
    fn parse_gfz_test_fixture() {
        let text = load_test_fixture();
        let data = parse_gfz(&text).unwrap();
        assert_eq!(data.len(), 4); // 4 data lines in fixture
    }

    #[test]
    fn parse_gfz_first_record() {
        let text = load_test_fixture();
        let data = parse_gfz(&text).unwrap();
        let records = data.records();

        // 2024-03-20
        let r = &records[0];
        assert_eq!(r.year, 2024);
        assert_eq!(r.month, 3);
        assert_eq!(r.day, 20);
        assert_eq!(r.ap_daily, 7.0);
        assert!((r.f107_obs - 130.5).abs() < 0.1);
    }

    #[test]
    fn parse_gfz_ap_3h_values() {
        let text = load_test_fixture();
        let data = parse_gfz(&text).unwrap();
        let records = data.records();

        // 2024-03-20: ap1=4, ap2=5, ap3=7, ap4=9, ap5=12, ap6=9, ap7=7, ap8=6
        let r = &records[0];
        assert_eq!(r.ap_3h, [4.0, 5.0, 7.0, 9.0, 12.0, 9.0, 7.0, 6.0]);
    }

    #[test]
    fn parse_gfz_records_sorted_by_date() {
        let text = load_test_fixture();
        let data = parse_gfz(&text).unwrap();
        let records = data.records();

        for i in 1..records.len() {
            assert!(
                records[i].jd_midnight > records[i - 1].jd_midnight,
                "Records must be sorted by date"
            );
        }
    }

    #[test]
    fn parse_gfz_f107_centered_avg_computed() {
        let text = load_test_fixture();
        let data = parse_gfz(&text).unwrap();
        let records = data.records();

        // With only 4 records, the centered average should be close to the mean of all
        for r in records {
            assert!(
                r.f107_obs_ctr81 > 0.0,
                "F10.7 centered average should be computed"
            );
        }
    }

    #[test]
    fn detect_cssi_format() {
        assert_eq!(
            detect_format("DATATYPE CssiSpaceWeather\nVERSION 1.2\n"),
            SpaceWeatherFormat::Cssi
        );
    }

    #[test]
    fn detect_gfz_format() {
        assert_eq!(
            detect_format("# PURPOSE: This file distributes...\n# LICENSE: CC BY 4.0\n"),
            SpaceWeatherFormat::Gfz
        );
    }

    #[test]
    fn detect_gfz_format_from_fixture() {
        let text = load_test_fixture();
        assert_eq!(detect_format(&text), SpaceWeatherFormat::Gfz);
    }
}
