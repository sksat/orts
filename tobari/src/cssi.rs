//! CSSI space weather data parser and provider.
//!
//! Parses CelesTrak CSSI-format space weather files (SW-Last5Years.txt, SW-All.txt)
//! and provides historical F10.7 and Ap data via [`SpaceWeatherProvider`].
//!
//! ## Data sources
//!
//! - Kp/Ap geomagnetic indices: GFZ Helmholtz Centre for Geosciences (CC BY 4.0)
//! - F10.7 solar radio flux: NOAA SWPC / NRCan DRAO (public domain)
//! - Aggregated by CelesTrak (<https://celestrak.org/SpaceData/>)

use std::fmt;
use std::sync::Arc;

use kaname::epoch::Epoch;

use crate::space_weather::{SpaceWeather, SpaceWeatherProvider};

/// A single daily record from CSSI space weather data.
#[derive(Debug, Clone)]
pub struct CssiDailyRecord {
    /// Julian Date at 00:00 UTC of this day.
    pub jd_midnight: f64,
    /// Year.
    pub year: i32,
    /// Month (1-12).
    pub month: u32,
    /// Day (1-31).
    pub day: u32,
    /// 8 three-hourly ap values: \[00-03, 03-06, 06-09, 09-12, 12-15, 15-18, 18-21, 21-24\] UT.
    pub ap_3h: [f64; 8],
    /// Daily average Ap.
    pub ap_daily: f64,
    /// Observed F10.7 \[SFU\].
    pub f107_obs: f64,
    /// Observed 81-day centered average F10.7 \[SFU\].
    pub f107_obs_ctr81: f64,
}

/// Parse error for CSSI space weather files.
#[derive(Debug)]
pub enum CssiParseError {
    /// Line too short for required fields.
    LineTooShort { line_number: usize, length: usize },
    /// Failed to parse a numeric field.
    ParseField {
        line_number: usize,
        field: &'static str,
        value: String,
    },
    /// No data records found in file.
    NoData,
}

impl fmt::Display for CssiParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineTooShort {
                line_number,
                length,
            } => {
                write!(
                    f,
                    "line {line_number}: too short ({length} chars, need at least 124)"
                )
            }
            Self::ParseField {
                line_number,
                field,
                value,
            } => {
                write!(f, "line {line_number}: failed to parse {field}: {value:?}")
            }
            Self::NoData => write!(f, "no data records found in file"),
        }
    }
}

impl std::error::Error for CssiParseError {}

/// Parsed CSSI space weather data, sorted by date.
#[derive(Debug, Clone)]
pub struct CssiData {
    /// Daily records sorted by JD (ascending).
    records: Vec<CssiDailyRecord>,
}

impl CssiData {
    /// Parse CSSI space weather data from text content.
    ///
    /// Accepts the full content of SW-Last5Years.txt or SW-All.txt.
    /// Parses OBSERVED and DAILY_PREDICTED sections.
    /// Records are sorted by date after parsing; duplicates from overlapping
    /// sections are resolved in favor of OBSERVED data.
    pub fn parse(text: &str) -> Result<Self, CssiParseError> {
        let mut records = Vec::new();
        let mut in_section = false;
        let mut is_observed_section = false;
        let mut observed_jds = std::collections::HashSet::new();

        for (i, line) in text.lines().enumerate() {
            let line_number = i + 1;
            let trimmed = line.trim();

            if trimmed == "BEGIN OBSERVED" {
                in_section = true;
                is_observed_section = true;
                continue;
            }
            if trimmed == "BEGIN DAILY_PREDICTED" || trimmed == "BEGIN MONTHLY_PREDICTED" {
                in_section = true;
                is_observed_section = false;
                continue;
            }
            if trimmed.starts_with("END ") {
                in_section = false;
                continue;
            }

            if !in_section || trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Data line — must be at least 124 chars (through Ctr81 obs)
            if line.len() < 124 {
                // Some predicted lines may be shorter; skip gracefully
                continue;
            }

            match Self::parse_line(line, line_number) {
                Ok(record) => {
                    let jd_key = record.jd_midnight.to_bits();
                    if is_observed_section {
                        observed_jds.insert(jd_key);
                        records.push(record);
                    } else if !observed_jds.contains(&jd_key) {
                        // Only add predicted data if no observed data for this date
                        records.push(record);
                    }
                }
                Err(_) => {
                    // Skip unparseable lines in predicted sections
                    if is_observed_section {
                        // In observed section, this is unexpected but we still skip
                        // to be robust against format variations
                    }
                    continue;
                }
            }
        }

        if records.is_empty() {
            return Err(CssiParseError::NoData);
        }

        records.sort_by(|a, b| a.jd_midnight.partial_cmp(&b.jd_midnight).unwrap());

        Ok(Self { records })
    }

    /// Parse a single CSSI data line.
    ///
    /// Fixed-width format (Fortran): `FORMAT(I4,I3,I3,I5,I3,8I3,I4,8I4,I4,F4.1,I2,I4,F6.1,I2,5F6.1)`
    fn parse_line(line: &str, line_number: usize) -> Result<CssiDailyRecord, CssiParseError> {
        let parse_int =
            |start: usize, end: usize, field: &'static str| -> Result<i64, CssiParseError> {
                let s = line[start..end].trim();
                s.parse::<i64>().map_err(|_| CssiParseError::ParseField {
                    line_number,
                    field,
                    value: s.to_string(),
                })
            };

        let parse_float =
            |start: usize, end: usize, field: &'static str| -> Result<f64, CssiParseError> {
                let s = line[start..end].trim();
                if s.is_empty() {
                    return Ok(0.0);
                }
                s.parse::<f64>().map_err(|_| CssiParseError::ParseField {
                    line_number,
                    field,
                    value: s.to_string(),
                })
            };

        // Date fields
        let year = parse_int(0, 4, "year")? as i32;
        let month = parse_int(4, 7, "month")? as u32;
        let day = parse_int(7, 10, "day")? as u32;

        // 8x ap values (cols 46-78, each I4)
        let ap_3h = [
            parse_int(46, 50, "ap0")? as f64,
            parse_int(50, 54, "ap3")? as f64,
            parse_int(54, 58, "ap6")? as f64,
            parse_int(58, 62, "ap9")? as f64,
            parse_int(62, 66, "ap12")? as f64,
            parse_int(66, 70, "ap15")? as f64,
            parse_int(70, 74, "ap18")? as f64,
            parse_int(74, 78, "ap21")? as f64,
        ];

        // Daily Ap average (cols 78-82, I4)
        let ap_daily = parse_int(78, 82, "ap_daily")? as f64;

        // F10.7 observed (cols 112-118, F6.1)
        let f107_obs = parse_float(112, 118, "f107_obs")?;

        // F10.7 observed 81-day centered average (cols 118-124, F6.1)
        let f107_obs_ctr81 = parse_float(118, 124, "f107_obs_ctr81")?;

        // Compute JD at midnight UTC
        let jd_midnight = Epoch::from_gregorian(year, month, day, 0, 0, 0.0).jd();

        Ok(CssiDailyRecord {
            jd_midnight,
            year,
            month,
            day,
            ap_3h,
            ap_daily,
            f107_obs,
            f107_obs_ctr81,
        })
    }

    /// Number of daily records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the dataset is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Date range as (first_epoch, last_epoch).
    pub fn date_range(&self) -> Option<(Epoch, Epoch)> {
        if self.records.is_empty() {
            return None;
        }
        Some((
            Epoch::from_jd(self.records.first().unwrap().jd_midnight),
            Epoch::from_jd(self.records.last().unwrap().jd_midnight),
        ))
    }

    /// Access the underlying records.
    pub fn records(&self) -> &[CssiDailyRecord] {
        &self.records
    }
}

/// Behavior when the requested epoch is outside the data range.
#[derive(Debug, Clone, Copy)]
pub enum OutOfRangeBehavior {
    /// Use the nearest (first or last) record's values.
    Clamp,
    /// Panic with an error message.
    Panic,
}

/// Space weather provider backed by CSSI historical data.
///
/// Looks up the correct daily record for a given epoch using binary search,
/// then computes the NRLMSISE-00 7-element ap history array from the
/// 3-hourly ap values across day boundaries.
#[derive(Clone)]
pub struct CssiSpaceWeather {
    data: Arc<CssiData>,
    out_of_range: OutOfRangeBehavior,
}

impl CssiSpaceWeather {
    /// Create a provider from parsed CSSI data.
    pub fn new(data: CssiData) -> Self {
        Self {
            data: Arc::new(data),
            out_of_range: OutOfRangeBehavior::Clamp,
        }
    }

    /// Create a provider by parsing a CSSI file from a path.
    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        let data = CssiData::parse(&text)?;
        Ok(Self::new(data))
    }

    /// Set out-of-range behavior (builder pattern).
    pub fn with_out_of_range(mut self, behavior: OutOfRangeBehavior) -> Self {
        self.out_of_range = behavior;
        self
    }

    /// Access the underlying data.
    pub fn data(&self) -> &CssiData {
        &self.data
    }
}

/// Get a 3-hourly ap value at a given number of slots before the reference position.
///
/// Each day has 8 slots (3 hours each). This function navigates backward
/// across day boundaries in the sorted record array.
fn ap_at_offset(
    records: &[CssiDailyRecord],
    day_idx: usize,
    current_slot: usize,
    slots_back: usize,
) -> f64 {
    let total_current = day_idx * 8 + current_slot;
    if slots_back > total_current {
        // Not enough history; use daily Ap of first available day
        return records[0].ap_daily;
    }
    let total_target = total_current - slots_back;
    let target_day = total_target / 8;
    let target_slot = total_target % 8;
    records[target_day].ap_3h[target_slot]
}

impl SpaceWeatherProvider for CssiSpaceWeather {
    fn get(&self, epoch: &Epoch) -> SpaceWeather {
        let jd = epoch.jd();
        let records = &self.data.records;

        // Binary search for the day containing this epoch.
        // Each record represents midnight UTC; we want the record whose
        // jd_midnight <= jd < jd_midnight + 1.
        let idx = match records.binary_search_by(|r| r.jd_midnight.partial_cmp(&jd).unwrap()) {
            Ok(i) => i,
            Err(i) => {
                if i == 0 {
                    match self.out_of_range {
                        OutOfRangeBehavior::Clamp => 0,
                        OutOfRangeBehavior::Panic => panic!(
                            "epoch JD {jd} is before CSSI data range (starts JD {})",
                            records[0].jd_midnight
                        ),
                    }
                } else {
                    i - 1
                }
            }
        };

        let idx = idx.min(records.len() - 1);

        // Check if past end of data
        if jd > records.last().unwrap().jd_midnight + 1.0 {
            match self.out_of_range {
                OutOfRangeBehavior::Clamp => {} // will use last record
                OutOfRangeBehavior::Panic => panic!(
                    "epoch JD {jd} is after CSSI data range (ends JD {})",
                    records.last().unwrap().jd_midnight
                ),
            }
        }

        let day = &records[idx];
        let ut_hours = (jd - day.jd_midnight) * 24.0;
        let current_slot = (ut_hours / 3.0).floor().clamp(0.0, 7.0) as usize;

        // Build NRLMSISE-00 7-element ap history array
        let ap_array = [
            day.ap_daily,
            ap_at_offset(records, idx, current_slot, 0), // current 3-hr
            ap_at_offset(records, idx, current_slot, 1), // 3 hr ago
            ap_at_offset(records, idx, current_slot, 2), // 6 hr ago
            ap_at_offset(records, idx, current_slot, 3), // 9 hr ago
            // Average of 12-33 hours before (8 slots: 4..=11)
            (4..=11)
                .map(|s| ap_at_offset(records, idx, current_slot, s))
                .sum::<f64>()
                / 8.0,
            // Average of 36-57 hours before (8 slots: 12..=19)
            (12..=19)
                .map(|s| ap_at_offset(records, idx, current_slot, s))
                .sum::<f64>()
                / 8.0,
        ];

        // F10.7: NRLMSISE-00 uses previous day's observed value
        let f107_daily = if idx > 0 {
            records[idx - 1].f107_obs
        } else {
            day.f107_obs
        };

        // 81-day centered average; fall back to daily if unavailable
        let f107_avg = if day.f107_obs_ctr81 > 0.0 {
            day.f107_obs_ctr81
        } else {
            f107_daily
        };

        SpaceWeather {
            f107_daily,
            f107_avg,
            ap_daily: day.ap_daily,
            ap_3hour_history: ap_array,
        }
    }
}

// --- fetch feature: HTTP download + cache ---

#[cfg(feature = "fetch")]
mod fetch_impl {
    use super::*;
    use std::time::{Duration, SystemTime};

    /// CelesTrak SW-Last5Years.txt URL.
    const CELESTRAK_SW_URL: &str = "https://celestrak.org/SpaceData/SW-Last5Years.txt";

    /// Default cache max age (24 hours).
    const DEFAULT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

    impl CssiSpaceWeather {
        /// Fetch space weather data from CelesTrak with local caching.
        ///
        /// Downloads SW-Last5Years.txt and caches it at
        /// `~/.cache/orts/SW-Last5Years.txt`.
        /// If the cache file exists and is newer than `max_age`, it is reused.
        /// Pass `None` for the default max age of 24 hours.
        pub fn fetch(max_age: Option<Duration>) -> Result<Self, Box<dyn std::error::Error>> {
            let max_age = max_age.unwrap_or(DEFAULT_MAX_AGE);
            let cache_path = cache_file_path()?;

            // Check cache
            if let Ok(metadata) = std::fs::metadata(&cache_path)
                && let Ok(modified) = metadata.modified()
                && SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or(Duration::MAX)
                    < max_age
            {
                eprintln!("Using cached space weather data: {}", cache_path.display());
                let text = std::fs::read_to_string(&cache_path)?;
                let data = CssiData::parse(&text)?;
                return Ok(Self::new(data));
            }

            // Download
            eprintln!("Downloading space weather data from CelesTrak...");
            let body = ureq::get(CELESTRAK_SW_URL)
                .call()
                .map_err(|e| format!("HTTP request failed: {e}"))?
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("Failed to read response body: {e}"))?;

            // Parse before caching (ensures data is valid)
            let data = CssiData::parse(&body)?;

            // Cache to disk
            if let Some(parent) = cache_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&cache_path, &body)?;
            eprintln!("Cached {} records to {}", data.len(), cache_path.display());

            Ok(Self::new(data))
        }

        /// Fetch with default settings (24-hour cache).
        pub fn fetch_default() -> Result<Self, Box<dyn std::error::Error>> {
            Self::fetch(None)
        }
    }

    /// Determine the cache file path: `~/.cache/orts/SW-Last5Years.txt`
    fn cache_file_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
        let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
        Ok(std::path::PathBuf::from(home)
            .join(".cache")
            .join("orts")
            .join("SW-Last5Years.txt"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal CSSI data fragment for testing.
    const CSSI_FRAGMENT: &str = "\
DATATYPE CssiSpaceWeather
VERSION 1.2
# Test data
NUM_OBSERVED_POINTS 3
BEGIN OBSERVED
2024 01 01 2567  1  7  7  3  3  7  3  3  3  37   3   3   2   2   3   2   2   2   2 0.1 1  52 144.2 0 155.9 155.0 148.7 161.5 160.4
2024 01 02 2567  2  3  7  3  7  3  0  0  3  27   2   3   2   3   2   0   0   2   2 0.0 0  46 152.3 0 155.6 155.2 158.2 161.1 160.8
2024 01 03 2567  3  0  3  3  3  3  3  7  7  30   0   2   2   2   2   2   3   3   2 0.0 0  47 157.0 0 155.6 155.4 161.3 161.0 161.0
END OBSERVED
";

    #[test]
    fn parse_cssi_fragment() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        assert_eq!(data.len(), 3);

        let r0 = &data.records()[0];
        assert_eq!(r0.year, 2024);
        assert_eq!(r0.month, 1);
        assert_eq!(r0.day, 1);
        assert_eq!(r0.ap_3h, [3.0, 3.0, 2.0, 2.0, 3.0, 2.0, 2.0, 2.0]);
        assert!((r0.ap_daily - 2.0).abs() < 0.01);
        assert!((r0.f107_obs - 148.7).abs() < 0.1);
        assert!((r0.f107_obs_ctr81 - 161.5).abs() < 0.1);

        let r2 = &data.records()[2];
        assert_eq!(r2.year, 2024);
        assert_eq!(r2.month, 1);
        assert_eq!(r2.day, 3);
        assert!((r2.f107_obs - 161.3).abs() < 0.1);
    }

    #[test]
    fn parse_empty_gives_error() {
        let result = CssiData::parse("# empty file\n");
        assert!(matches!(result, Err(CssiParseError::NoData)));
    }

    #[test]
    fn date_range() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let (first, last) = data.date_range().unwrap();
        // 2024-01-01 to 2024-01-03
        let dt_first = first.to_datetime();
        assert_eq!(dt_first.year, 2024);
        assert_eq!(dt_first.month, 1);
        assert_eq!(dt_first.day, 1);
        let dt_last = last.to_datetime();
        assert_eq!(dt_last.year, 2024);
        assert_eq!(dt_last.month, 1);
        assert_eq!(dt_last.day, 3);
    }

    #[test]
    fn provider_lookup_mid_day() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let provider = CssiSpaceWeather::new(data);

        // 2024-01-02 12:00 UTC → should find day 2024-01-02
        let epoch = Epoch::from_gregorian(2024, 1, 2, 12, 0, 0.0);
        let sw = provider.get(&epoch);

        // ap_daily of 2024-01-02 is 2
        assert!((sw.ap_daily - 2.0).abs() < 0.01);

        // f107_daily = previous day (2024-01-01) observed F10.7
        assert!((sw.f107_daily - 148.7).abs() < 0.1);

        // f107_avg = 2024-01-02 Ctr81 observed
        assert!((sw.f107_avg - 161.1).abs() < 0.1);
    }

    #[test]
    fn provider_3hr_slot_mapping() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let provider = CssiSpaceWeather::new(data);

        // 2024-01-02 01:30 UTC → slot 0 (00-03 UT)
        let epoch = Epoch::from_gregorian(2024, 1, 2, 1, 30, 0.0);
        let sw = provider.get(&epoch);

        // ap_array[1] = current slot = ap_3h[0] of 2024-01-02 = 2
        assert!((sw.ap_3hour_history[1] - 2.0).abs() < 0.01);

        // ap_array[2] = 3hr ago = slot 7 of 2024-01-01 = 2
        assert!((sw.ap_3hour_history[2] - 2.0).abs() < 0.01);
    }

    #[test]
    fn provider_clamp_before_data() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let provider = CssiSpaceWeather::new(data);

        // Query before data range
        let epoch = Epoch::from_gregorian(2023, 12, 31, 12, 0, 0.0);
        let sw = provider.get(&epoch);

        // Should clamp to first record (2024-01-01)
        assert!((sw.ap_daily - 2.0).abs() < 0.01);
    }

    #[test]
    fn provider_clamp_after_data() {
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let provider = CssiSpaceWeather::new(data);

        // Query after data range
        let epoch = Epoch::from_gregorian(2024, 1, 10, 12, 0, 0.0);
        let sw = provider.get(&epoch);

        // Should clamp to last record (2024-01-03)
        assert!((sw.ap_daily - 2.0).abs() < 0.01);
    }

    #[test]
    fn predicted_section_parsed() {
        let text = "\
DATATYPE CssiSpaceWeather
VERSION 1.2
BEGIN OBSERVED
2024 01 01 2567  1  7  7  3  3  7  3  3  3  37   3   3   2   2   3   2   2   2   2 0.1 1  52 144.2 0 155.9 155.0 148.7 161.5 160.4
END OBSERVED
BEGIN DAILY_PREDICTED
2024 01 02 2567  2  3  7  3  7  3  0  0  3  27   2   3   2   3   2   0   0   2   2 0.0 0  46 152.3 0 155.6 155.2 158.2 161.1 160.8
END DAILY_PREDICTED
";
        let data = CssiData::parse(text).unwrap();
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn observed_takes_precedence_over_predicted() {
        let text = "\
DATATYPE CssiSpaceWeather
VERSION 1.2
BEGIN OBSERVED
2024 01 01 2567  1  7  7  3  3  7  3  3  3  37   3   3   2   2   3   2   2   2   2 0.1 1  52 144.2 0 155.9 155.0 148.7 161.5 160.4
END OBSERVED
BEGIN DAILY_PREDICTED
2024 01 01 2567  1  0  0  0  0  0  0  0  0   0   0   0   0   0   0   0   0   0   0 0.0 0   0 100.0 0 100.0 100.0 100.0 100.0 100.0
END DAILY_PREDICTED
";
        let data = CssiData::parse(text).unwrap();
        // Only 1 record (observed wins, predicted duplicate skipped)
        assert_eq!(data.len(), 1);
        assert!((data.records()[0].f107_obs - 148.7).abs() < 0.1);
    }

    #[test]
    fn ap_history_averaging() {
        // With 3 days of data and query at 2024-01-03 12:00 (slot 4),
        // slots_back 4..11 spans back 12-33 hours → from slot 0 of day 3
        // backwards through day 2.
        let data = CssiData::parse(CSSI_FRAGMENT).unwrap();
        let provider = CssiSpaceWeather::new(data);

        let epoch = Epoch::from_gregorian(2024, 1, 3, 12, 0, 0.0);
        let sw = provider.get(&epoch);

        // ap_array[5] should be average of 8 values (12-33 hr before)
        // At slot 4 of day 3 (idx=2), slots_back 4..11:
        // slot_back 4 → day3 slot0 = 0
        // slot_back 5 → day2 slot7 = 2
        // slot_back 6 → day2 slot6 = 0
        // slot_back 7 → day2 slot5 = 0
        // slot_back 8 → day2 slot4 = 2
        // slot_back 9 → day2 slot3 = 3
        // slot_back 10 → day2 slot2 = 2
        // slot_back 11 → day2 slot1 = 3
        let expected_avg = (0.0 + 2.0 + 0.0 + 0.0 + 2.0 + 3.0 + 2.0 + 3.0) / 8.0;
        assert!(
            (sw.ap_3hour_history[5] - expected_avg).abs() < 0.01,
            "ap_array[5] = {}, expected {}",
            sw.ap_3hour_history[5],
            expected_avg
        );
    }
}
