//! JPL Horizons vector-table parser and (optionally) fetcher.
//!
//! Provides tools for parsing state-vector tables exported from the JPL
//! Horizons system (<https://ssd.jpl.nasa.gov/horizons/>) and querying them
//! via Hermite interpolation. The parser is always available. When compiled
//! with the `fetch-horizons` feature, [`HorizonsTable::fetch_vector_table`]
//! downloads tables over HTTP and caches them on disk.
//!
//! ## Supported input format
//!
//! CSV-format vector-table output delimited by `$$SOE` / `$$EOE` markers,
//! as produced by the Horizons API with
//! `EPHEM_TYPE=VECTORS`, `VEC_TABLE=2`, `CSV_FORMAT=YES`, `OUT_UNITS=KM-S`.
//! Each row has the shape:
//!
//! ```text
//! JDTDB, Calendar Date, X, Y, Z, VX, VY, VZ,
//! ```
//!
//! Position is in km, velocity in km/s. The parser accepts either
//! whitespace- or comma-separated rows after splitting on commas.

use std::fmt;

use nalgebra::Vector3;

use crate::epoch::Epoch;

/// One state-vector sample from a Horizons vector table.
#[derive(Debug, Clone, Copy)]
pub struct HorizonsSample {
    /// Epoch (reconstructed from the JDTDB column).
    ///
    /// Note: Horizons JDTDB is in the TDB time scale, while `Epoch::from_jd`
    /// is agnostic. The difference between TDB and UTC is ≤ ~69 seconds for
    /// modern epochs. For sub-kilometer accuracy at Moon distance, callers
    /// should be aware of this time-scale mismatch — see README.
    pub epoch: Epoch,
    /// Position in ECI/J2000 [km].
    pub position: Vector3<f64>,
    /// Velocity in ECI/J2000 [km/s].
    pub velocity: Vector3<f64>,
}

/// A parsed Horizons vector table with Hermite interpolation support.
#[derive(Debug, Clone)]
pub struct HorizonsTable {
    /// Samples in ascending epoch order.
    samples: Vec<HorizonsSample>,
}

/// Errors produced by the Horizons parser / fetcher.
#[derive(Debug)]
pub enum HorizonsError {
    /// `$$SOE` start-of-ephemeris marker not found.
    MissingStartMarker,
    /// `$$EOE` end-of-ephemeris marker not found.
    MissingEndMarker,
    /// A row in the ephemeris block had too few comma-separated fields.
    RowTooShort { line: usize, fields: usize },
    /// A numeric field could not be parsed.
    ParseField {
        line: usize,
        field: &'static str,
        value: String,
    },
    /// No samples were found between `$$SOE` and `$$EOE`.
    NoData,
    /// Samples are not in ascending epoch order.
    NotSorted { index: usize },
    /// I/O error while reading a file.
    Io(String),
    /// HTTP / fetch error (only produced with the `fetch-horizons` feature).
    Fetch(String),
}

impl fmt::Display for HorizonsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStartMarker => write!(f, "Horizons CSV missing $$SOE marker"),
            Self::MissingEndMarker => write!(f, "Horizons CSV missing $$EOE marker"),
            Self::RowTooShort { line, fields } => {
                write!(
                    f,
                    "Horizons CSV row at line {line}: only {fields} fields (need at least 8)"
                )
            }
            Self::ParseField { line, field, value } => {
                write!(
                    f,
                    "Horizons CSV line {line}: failed to parse {field}: {value:?}"
                )
            }
            Self::NoData => write!(
                f,
                "Horizons CSV contained no samples between $$SOE and $$EOE"
            ),
            Self::NotSorted { index } => {
                write!(
                    f,
                    "Horizons CSV samples not sorted in ascending epoch order at index {index}"
                )
            }
            Self::Io(msg) => write!(f, "Horizons I/O error: {msg}"),
            Self::Fetch(msg) => write!(f, "Horizons fetch error: {msg}"),
        }
    }
}

impl std::error::Error for HorizonsError {}

impl HorizonsTable {
    /// Parse a Horizons CSV vector-table string.
    ///
    /// The input must contain `$$SOE` / `$$EOE` markers bracketing CSV rows
    /// of the form `JDTDB, Calendar, X, Y, Z, VX, VY, VZ`.
    pub fn parse_csv(text: &str) -> Result<Self, HorizonsError> {
        // Locate the $$SOE and $$EOE markers.
        let lines: Vec<&str> = text.lines().collect();

        let soe_idx = lines
            .iter()
            .position(|l| l.trim_start().starts_with("$$SOE"))
            .ok_or(HorizonsError::MissingStartMarker)?;
        let eoe_idx = lines
            .iter()
            .skip(soe_idx + 1)
            .position(|l| l.trim_start().starts_with("$$EOE"))
            .map(|rel| soe_idx + 1 + rel)
            .ok_or(HorizonsError::MissingEndMarker)?;

        let mut samples = Vec::with_capacity(eoe_idx.saturating_sub(soe_idx + 1));
        for (offset, raw) in lines[soe_idx + 1..eoe_idx].iter().enumerate() {
            let line_number = soe_idx + 2 + offset; // 1-based source line
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }

            let fields: Vec<&str> = trimmed.split(',').map(str::trim).collect();
            if fields.len() < 8 {
                return Err(HorizonsError::RowTooShort {
                    line: line_number,
                    fields: fields.len(),
                });
            }

            let jd = parse_field(fields[0], "JDTDB", line_number)?;
            // fields[1] is the calendar date string — we ignore it and use JDTDB.
            let x = parse_field(fields[2], "X", line_number)?;
            let y = parse_field(fields[3], "Y", line_number)?;
            let z = parse_field(fields[4], "Z", line_number)?;
            let vx = parse_field(fields[5], "VX", line_number)?;
            let vy = parse_field(fields[6], "VY", line_number)?;
            let vz = parse_field(fields[7], "VZ", line_number)?;

            samples.push(HorizonsSample {
                epoch: Epoch::from_jd(jd),
                position: Vector3::new(x, y, z),
                velocity: Vector3::new(vx, vy, vz),
            });
        }

        if samples.is_empty() {
            return Err(HorizonsError::NoData);
        }

        // Enforce ascending epoch order.
        for i in 1..samples.len() {
            if samples[i].epoch.jd() < samples[i - 1].epoch.jd() {
                return Err(HorizonsError::NotSorted { index: i });
            }
        }

        Ok(Self { samples })
    }

    /// Load a Horizons CSV table from a file on disk.
    pub fn from_file(path: impl AsRef<std::path::Path>) -> Result<Self, HorizonsError> {
        let text =
            std::fs::read_to_string(path.as_ref()).map_err(|e| HorizonsError::Io(e.to_string()))?;
        Self::parse_csv(&text)
    }

    /// All samples, in ascending epoch order.
    pub fn samples(&self) -> &[HorizonsSample] {
        &self.samples
    }

    /// Number of samples in the table.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Returns `true` if the table has no samples (parser guarantees this
    /// cannot be constructed from [`parse_csv`] — reserved for defensive use).
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// First and last epochs in the table, if any.
    pub fn date_range(&self) -> Option<(Epoch, Epoch)> {
        Some((self.samples.first()?.epoch, self.samples.last()?.epoch))
    }

    /// Interpolate state at `epoch` using cubic Hermite interpolation with
    /// position and velocity at the bracketing samples.
    ///
    /// Returns `None` if `epoch` is outside the table's range.
    pub fn interpolate(&self, epoch: &Epoch) -> Option<HorizonsSample> {
        let t = epoch.jd();
        let first = self.samples.first()?;
        let last = self.samples.last()?;

        // Out-of-range guard.
        if t < first.epoch.jd() || t > last.epoch.jd() {
            return None;
        }

        // Exact-boundary fast path.
        if t == first.epoch.jd() {
            return Some(*first);
        }
        if t == last.epoch.jd() {
            return Some(*last);
        }

        // Binary search for the upper bound.
        let idx_hi = self
            .samples
            .partition_point(|s| s.epoch.jd() <= t)
            .min(self.samples.len() - 1)
            .max(1);
        let idx_lo = idx_hi - 1;
        let s0 = &self.samples[idx_lo];
        let s1 = &self.samples[idx_hi];

        // Hermite interpolation with position + velocity at each endpoint.
        // Convert JD difference to seconds for numerical stability: 1 day = 86400 s.
        let dt_days = s1.epoch.jd() - s0.epoch.jd();
        let dt_s = dt_days * 86_400.0;
        if dt_s <= 0.0 {
            return Some(*s0);
        }

        let tau = ((t - s0.epoch.jd()) * 86_400.0) / dt_s; // normalized [0,1]
        let tau2 = tau * tau;
        let tau3 = tau2 * tau;

        // Hermite basis:
        //   h00 = 2τ³ − 3τ² + 1
        //   h10 = τ³ − 2τ² + τ
        //   h01 = −2τ³ + 3τ²
        //   h11 = τ³ − τ²
        let h00 = 2.0 * tau3 - 3.0 * tau2 + 1.0;
        let h10 = tau3 - 2.0 * tau2 + tau;
        let h01 = -2.0 * tau3 + 3.0 * tau2;
        let h11 = tau3 - tau2;

        // Interpolated position: p(τ) = h00·p0 + h10·Δt·v0 + h01·p1 + h11·Δt·v1
        let position = s0.position * h00
            + s0.velocity * (h10 * dt_s)
            + s1.position * h01
            + s1.velocity * (h11 * dt_s);

        // Derivative (velocity) of the Hermite polynomial:
        //   h00' = 6τ² − 6τ
        //   h10' = 3τ² − 4τ + 1
        //   h01' = −6τ² + 6τ
        //   h11' = 3τ² − 2τ
        // Divide by dt_s because τ = (t - t0) / dt_s.
        let dh00 = 6.0 * tau2 - 6.0 * tau;
        let dh10 = 3.0 * tau2 - 4.0 * tau + 1.0;
        let dh01 = -6.0 * tau2 + 6.0 * tau;
        let dh11 = 3.0 * tau2 - 2.0 * tau;

        let velocity = (s0.position * dh00
            + s0.velocity * (dh10 * dt_s)
            + s1.position * dh01
            + s1.velocity * (dh11 * dt_s))
            / dt_s;

        Some(HorizonsSample {
            epoch: *epoch,
            position,
            velocity,
        })
    }
}

fn parse_field(s: &str, field: &'static str, line: usize) -> Result<f64, HorizonsError> {
    s.parse::<f64>().map_err(|_| HorizonsError::ParseField {
        line,
        field,
        value: s.to_string(),
    })
}

// ---------------------------------------------------------------------------
// HTTP fetch + disk cache (optional, feature-gated)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "fetch-horizons", not(target_arch = "wasm32")))]
mod fetch_impl {
    use std::hash::{Hash, Hasher};
    use std::time::{Duration, SystemTime};

    use super::*;

    /// Default cache max age (7 days) — spacecraft ephemerides rarely change
    /// after the mission ends, so a long TTL is safe.
    const DEFAULT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

    /// JPL Horizons API endpoint (REST).
    const HORIZONS_API: &str = "https://ssd.jpl.nasa.gov/api/horizons.api";

    /// Horizons `TIME_TYPE` parameter value.
    ///
    /// Controls two things about the Horizons response that matter for
    /// downstream correctness:
    /// 1. How `START_TIME` / `STOP_TIME` query strings are interpreted
    ///    (as TDB wall clock or UT wall clock).
    /// 2. Which time scale the JD column in the CSV response uses
    ///    (`JDTDB` vs `JDUT`).
    ///
    /// We fix this to `"UT"` because [`Epoch::from_iso8601`] is a
    /// UTC-only parser (it has no concept of TDB) and
    /// [`epoch_to_iso`] renders epochs via `DateTime`'s display which
    /// the same parser round-trips with UTC semantics. Horizons in UT
    /// mode interprets the strings we send as UTC wall clocks and
    /// returns JDUT samples whose numerical JD matches the
    /// literal-wall-clock JD that `arika::Epoch` computes via
    /// `from_gregorian`, so cached interpolation lookups land on the
    /// **physical state the caller actually asked for** rather than a
    /// state 69 s off.
    ///
    /// A previous revision used `"TDB"` here which made Horizons
    /// interpret the same strings as TDB wall clocks, producing states
    /// systematically 69 s earlier in physical time than the caller
    /// intended. That was invisible in coast phases but added
    /// |Δv| × 69 s ≈ 7-10 km of position error to every Artemis 1
    /// impulsive burn verification.
    ///
    /// Because the same wall-clock ISO string maps to **different
    /// physical instants** under different `TIME_TYPE` values (TDB − UTC
    /// ≈ 69.184 s for modern epochs), the cached CSV content depends on
    /// this value — so [`cache_key_for`] includes it in the hash so a
    /// hypothetical future switch back to TDB (or addition of a runtime
    /// choice) does not silently serve wrong-scale data from an old
    /// cache entry.
    const TIME_TYPE: &str = "UT";

    impl HorizonsTable {
        /// Fetch a vector table from JPL Horizons with disk caching.
        ///
        /// * `target` — Horizons target ID (e.g. `"-1023"` for Artemis 1, `"301"` for Moon).
        /// * `center` — coordinate center in Horizons syntax (e.g. `"500@399"` for Earth geocenter).
        /// * `start` / `stop` — epoch range bracketing the ephemeris.
        /// * `step` — Horizons STEP_SIZE string (e.g. `"10m"`, `"1h"`, `"1d"`).
        /// * `max_age` — disk-cache max age; defaults to 7 days.
        ///
        /// The response is cached at
        /// `~/.cache/orts/horizons/<hash>.csv` so repeat calls are free.
        pub fn fetch_vector_table(
            target: &str,
            center: &str,
            start: &Epoch,
            stop: &Epoch,
            step: &str,
            max_age: Option<Duration>,
        ) -> Result<Self, HorizonsError> {
            let max_age = max_age.unwrap_or(DEFAULT_MAX_AGE);

            let start_iso = epoch_to_iso(start);
            let stop_iso = epoch_to_iso(stop);
            let cache_key = cache_key_for(target, center, &start_iso, &stop_iso, step, TIME_TYPE);
            let cache_path =
                cache_file_path(&cache_key).map_err(|e| HorizonsError::Io(e.to_string()))?;

            // --- Serve from cache if fresh ---
            if let Ok(metadata) = std::fs::metadata(&cache_path)
                && let Ok(modified) = metadata.modified()
                && SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or(Duration::MAX)
                    < max_age
            {
                eprintln!("Using cached Horizons data: {}", cache_path.display());
                return Self::from_file(&cache_path);
            }

            // --- Fetch from HTTP ---
            eprintln!(
                "Fetching Horizons vector table: target={target}, center={center}, \
                 {start_iso} → {stop_iso} step={step}"
            );

            // Build query with all required parameters.
            // NOTE: we use `format=text` to get the classic CSV output.
            let mut response = ureq::get(HORIZONS_API)
                .query("format", "text")
                .query("COMMAND", format!("'{target}'"))
                .query("OBJ_DATA", "NO")
                .query("MAKE_EPHEM", "YES")
                .query("EPHEM_TYPE", "VECTORS")
                .query("CENTER", format!("'{center}'"))
                .query("START_TIME", format!("'{start_iso}'"))
                .query("STOP_TIME", format!("'{stop_iso}'"))
                .query("STEP_SIZE", format!("'{step}'"))
                .query("VEC_TABLE", "2")
                .query("OUT_UNITS", "KM-S")
                .query("CSV_FORMAT", "YES")
                .query("REF_SYSTEM", "ICRF")
                .query("REF_PLANE", "FRAME")
                .query("TIME_TYPE", TIME_TYPE)
                .call()
                .map_err(|e| HorizonsError::Fetch(format!("HTTP request failed: {e}")))?;

            let body = response
                .body_mut()
                .read_to_string()
                .map_err(|e| HorizonsError::Fetch(format!("reading response: {e}")))?;

            // Validate by parsing before caching.
            let table = Self::parse_csv(&body)?;

            // Write to disk cache.
            if let Some(parent) = cache_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| HorizonsError::Io(e.to_string()))?;
            }
            std::fs::write(&cache_path, &body).map_err(|e| HorizonsError::Io(e.to_string()))?;
            eprintln!("Cached {} samples to {}", table.len(), cache_path.display());

            Ok(table)
        }
    }

    /// Render an Epoch as an ISO-8601 string Horizons understands:
    /// `YYYY-MM-DD HH:MM:SS` in the JD-inferred calendar (UTC-ish).
    ///
    /// Uses `DateTime`'s `Display` formatter so that sub-microsecond JD
    /// round-off (≈50 µs at modern epochs) does not produce nonsense like
    /// `HH:59:60` by carrying overflow into the next minute/hour/day.
    /// Horizons accepts second precision; padding on either side of the
    /// requested window handles the rounding.
    fn epoch_to_iso(epoch: &Epoch) -> String {
        let dt = epoch.to_datetime();
        // Reuse DateTime's Display (`YYYY-MM-DDTHH:MM:SSZ`) and strip the
        // trailing `Z` + swap the `T` for a space to match the legacy
        // Horizons textual format.
        let iso = format!("{dt}");
        // Expected format: "2022-11-26T00:00:00Z" (20 chars).
        iso.replace('T', " ").trim_end_matches('Z').to_string()
    }

    /// Deterministic cache key derived from the query parameters.
    ///
    /// `time_type` is included because the same `(target, center, start,
    /// stop, step)` tuple produces **different** state vectors under
    /// different `TIME_TYPE` values: the START / STOP ISO strings are
    /// interpreted in the given time scale, so the returned samples
    /// correspond to physical instants that differ by the TDB − UTC
    /// offset (~69 s). A cache entry tagged only by the first five
    /// fields would silently serve the wrong-scale data across a switch
    /// and mask the bug behind cache hits. Including the time scale in
    /// the hash makes each scale live in its own cache slot.
    fn cache_key_for(
        target: &str,
        center: &str,
        start_iso: &str,
        stop_iso: &str,
        step: &str,
        time_type: &str,
    ) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        target.hash(&mut hasher);
        center.hash(&mut hasher);
        start_iso.hash(&mut hasher);
        stop_iso.hash(&mut hasher);
        step.hash(&mut hasher);
        time_type.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    fn cache_file_path(key: &str) -> Result<std::path::PathBuf, String> {
        let home = std::env::var("HOME").map_err(|_| "HOME environment variable not set")?;
        Ok(std::path::PathBuf::from(home)
            .join(".cache")
            .join("orts")
            .join("horizons")
            .join(format!("{key}.csv")))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn epoch_to_iso_round_midnight() {
            // A clean ISO epoch should render identically (modulo the space
            // separator and dropped `Z`) — this is the common case used by
            // the Artemis 1 DRO spike.
            let epoch = Epoch::from_iso8601("2022-11-26T00:00:00Z").unwrap();
            assert_eq!(epoch_to_iso(&epoch), "2022-11-26 00:00:00");
        }

        #[test]
        fn epoch_to_iso_regression_second_overflow_carries_to_minute() {
            // Regression guard for the bug that blocked the DRO spike:
            // `DateTime::sec = 59.9999999` formatted with the old custom
            // `{:06.3}` specifier became `60.000`, producing illegal
            // strings like `HH:59:60` that Horizons rejected.
            //
            // We test `DateTime::Display` directly (not via an
            // `Epoch` round-trip) so the test exercises the carry logic
            // in isolation, regardless of how JD-to-DateTime conversion
            // accumulates round-off on any particular input.
            let dt = crate::epoch::DateTime::new(2024, 6, 15, 12, 59, 59.9999999);
            let formatted = format!("{dt}");
            // Must carry into the minute: 12:59:60 is illegal, the
            // correct result is 13:00:00.
            assert!(
                !formatted.contains(":60"),
                "DateTime::Display failed to carry sec=59.9999999: {formatted:?}"
            );
            // And must specifically normalise to the next minute/hour.
            assert_eq!(formatted, "2024-06-15T13:00:00Z");

            // And when fed through `epoch_to_iso`'s string-munging, the
            // result is still carry-safe (the munging only strips the
            // trailing `Z` and swaps `T` for a space — it does not
            // reintroduce the carry bug).
            let epoch = crate::epoch::Epoch::from_iso8601("2024-06-15T13:00:00Z").unwrap();
            assert_eq!(epoch_to_iso(&epoch), "2024-06-15 13:00:00");
        }

        #[test]
        fn epoch_to_iso_near_hour_boundary_after_add_seconds() {
            // End-to-end regression: the `add_seconds(3600.0)` the spike
            // uses for Moon-ephemeris padding must produce a cleanly
            // formatted "HH:MM:00" string. JD ULP at 2022 epochs is ~50 µs
            // which should round cleanly, but we assert the *exact* output
            // so a future regression in either `add_seconds` or the
            // formatter is caught loudly.
            let base = Epoch::from_iso8601("2022-12-01T00:00:00Z").unwrap();
            let one_hour_later = base.add_seconds(3600.0);
            assert_eq!(epoch_to_iso(&one_hour_later), "2022-12-01 01:00:00");
        }

        #[test]
        fn cache_key_is_deterministic_for_identical_params() {
            // The on-disk cache is keyed by this hash, so two consecutive
            // calls with identical parameters must produce identical
            // strings — otherwise the cache is write-only and every
            // request re-fetches. This trivially-passing test guards
            // against a future refactor breaking that property.
            let a = cache_key_for(
                "301",
                "500@399",
                "2022-11-26 00:00:00",
                "2022-11-27 00:00:00",
                "1h",
                "TDB",
            );
            let b = cache_key_for(
                "301",
                "500@399",
                "2022-11-26 00:00:00",
                "2022-11-27 00:00:00",
                "1h",
                "TDB",
            );
            assert_eq!(a, b);
        }

        #[test]
        fn cache_key_changes_with_time_type() {
            // Regression guard for the bug this test was added to prevent:
            // a previous version of `cache_key_for` hashed only five
            // fields and left `TIME_TYPE` out. Because the same ISO
            // window returns physically different state vectors under
            // TDB vs UT (states shift by TDB − UTC ≈ 69 s), sharing a
            // cache entry across time types silently serves wrong-scale
            // data.
            let tdb = cache_key_for(
                "301",
                "500@399",
                "2022-11-26 00:00:00",
                "2022-11-27 00:00:00",
                "1h",
                "TDB",
            );
            let ut = cache_key_for(
                "301",
                "500@399",
                "2022-11-26 00:00:00",
                "2022-11-27 00:00:00",
                "1h",
                "UT",
            );
            assert_ne!(
                tdb, ut,
                "TIME_TYPE must participate in cache key to prevent cross-scale cache pollution"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal three-row fixture (not real Horizons data — synthetic values
    /// chosen so interpolation behavior can be verified analytically).
    const FIXTURE_CSV: &str = "\
*******************************************************************************
 Revised: some header
*******************************************************************************
$$SOE
2459000.0, A.D. 2020-May-31 12:00:00.0000,  1.000000000000000E+05,  0.000000000000000E+00,  0.000000000000000E+00,  0.000000000000000E+00,  1.000000000000000E+00,  0.000000000000000E+00,
2459000.5, A.D. 2020-Jun-01 00:00:00.0000,  1.432000000000000E+05,  0.000000000000000E+00,  0.000000000000000E+00,  0.000000000000000E+00,  1.000000000000000E+00,  0.000000000000000E+00,
2459001.0, A.D. 2020-Jun-01 12:00:00.0000,  1.864000000000000E+05,  0.000000000000000E+00,  0.000000000000000E+00,  0.000000000000000E+00,  1.000000000000000E+00,  0.000000000000000E+00,
$$EOE
*******************************************************************************
";

    #[test]
    fn parse_minimal_fixture() {
        let table = HorizonsTable::parse_csv(FIXTURE_CSV).unwrap();
        assert_eq!(table.len(), 3);
        let s0 = &table.samples()[0];
        assert_eq!(s0.epoch.jd(), 2459000.0);
        assert_eq!(s0.position.x, 100_000.0);
        assert_eq!(s0.velocity.y, 1.0);
        let s2 = &table.samples()[2];
        assert_eq!(s2.epoch.jd(), 2459001.0);
        assert_eq!(s2.position.x, 186_400.0);
    }

    #[test]
    fn parse_missing_soe_errors() {
        let bad = "no markers here\n2459000.0, x, 1, 2, 3, 4, 5, 6,\n";
        assert!(matches!(
            HorizonsTable::parse_csv(bad),
            Err(HorizonsError::MissingStartMarker)
        ));
    }

    #[test]
    fn parse_missing_eoe_errors() {
        let bad = "$$SOE\n2459000.0, x, 1, 2, 3, 4, 5, 6,\n";
        assert!(matches!(
            HorizonsTable::parse_csv(bad),
            Err(HorizonsError::MissingEndMarker)
        ));
    }

    #[test]
    fn parse_empty_block_errors() {
        let bad = "$$SOE\n$$EOE\n";
        assert!(matches!(
            HorizonsTable::parse_csv(bad),
            Err(HorizonsError::NoData)
        ));
    }

    #[test]
    fn parse_short_row_errors() {
        let bad = "$$SOE\n2459000.0, x, 1, 2\n$$EOE\n";
        assert!(matches!(
            HorizonsTable::parse_csv(bad),
            Err(HorizonsError::RowTooShort { .. })
        ));
    }

    #[test]
    fn parse_bad_numeric_errors() {
        let bad = "$$SOE\n2459000.0, x, notanumber, 2, 3, 4, 5, 6,\n$$EOE\n";
        assert!(matches!(
            HorizonsTable::parse_csv(bad),
            Err(HorizonsError::ParseField { .. })
        ));
    }

    #[test]
    fn interpolate_at_sample_returns_sample() {
        let table = HorizonsTable::parse_csv(FIXTURE_CSV).unwrap();
        let sample = table.interpolate(&Epoch::from_jd(2459000.5)).unwrap();
        // Boundary point should match exactly.
        assert!((sample.position.x - 143_200.0).abs() < 1e-9);
        assert!((sample.velocity.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn interpolate_out_of_range_returns_none() {
        let table = HorizonsTable::parse_csv(FIXTURE_CSV).unwrap();
        assert!(table.interpolate(&Epoch::from_jd(2458999.0)).is_none());
        assert!(table.interpolate(&Epoch::from_jd(2459002.0)).is_none());
    }

    #[test]
    fn interpolate_linear_motion_recovers_analytic() {
        // For a body moving along +y with constant velocity 1 km/s along +y
        // (x changes per our fixture are a pure artifact — let's use a cleaner
        // synthetic table: constant velocity along +x).
        let csv = "\
$$SOE
2459000.0, A, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459000.5, A, 43200.0, 0.0, 0.0, 1.0, 0.0, 0.0,
2459001.0, A, 86400.0, 0.0, 0.0, 1.0, 0.0, 0.0,
$$EOE
";
        let table = HorizonsTable::parse_csv(csv).unwrap();

        // Query a point 1/4 of the way between samples 0 and 1
        // (jd = 2459000.0 + 0.125 = 2459000.125).
        // Expected position: x = (0.125 days × 86400 s/day) × 1 km/s = 10800 km.
        let interp = table.interpolate(&Epoch::from_jd(2459000.125)).unwrap();
        assert!(
            (interp.position.x - 10_800.0).abs() < 1e-6,
            "expected 10800 km, got {:.6}",
            interp.position.x
        );
        // Velocity should recover the constant rate.
        assert!(
            (interp.velocity.x - 1.0).abs() < 1e-9,
            "expected velocity 1.0 km/s, got {:.9}",
            interp.velocity.x
        );
    }

    #[test]
    fn date_range_returns_endpoints() {
        let table = HorizonsTable::parse_csv(FIXTURE_CSV).unwrap();
        let (first, last) = table.date_range().unwrap();
        assert_eq!(first.jd(), 2459000.0);
        assert_eq!(last.jd(), 2459001.0);
    }
}
