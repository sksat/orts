//! OMM JSON parser (CelesTrak / Space-Track "GP" JSON).
//!
//! Deserializes a single CCSDS OMM keyword-value object into [`Omm`]. The
//! catalog number is already numeric in JSON (no Alpha-5 5-character limit),
//! and angles arrive in degrees / mean motion in rev/day — converted to the
//! `Omm` conventions (radians, rad/s) here.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::f64::consts::PI;
use core::fmt;

// `.to_radians()` resolves via libm in no_std; under std the inherent shadows it.
#[allow(unused_imports)]
use crate::math::F64Ext;

use serde::Deserialize;

use crate::omm::Omm;

/// Error type for OMM JSON parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum JsonParseError {
    /// The input was not a JSON object with the required OMM fields.
    Malformed(String),
    /// `EPOCH` was not a parseable ISO-8601 UTC calendar timestamp.
    InvalidEpoch(String),
}

impl fmt::Display for JsonParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonParseError::Malformed(e) => write!(f, "malformed OMM JSON: {e}"),
            JsonParseError::InvalidEpoch(s) => write!(f, "invalid OMM EPOCH: '{s}'"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for JsonParseError {}

/// CCSDS OMM fields as they appear in CelesTrak / Space-Track JSON
/// (SCREAMING_SNAKE_CASE keys). Angles in degrees, mean motion in rev/day.
#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct OmmJson {
    object_name: Option<String>,
    object_id: Option<String>,
    norad_cat_id: u32,
    epoch: String,
    mean_motion: f64,
    eccentricity: f64,
    inclination: f64,
    ra_of_asc_node: f64,
    arg_of_pericenter: f64,
    mean_anomaly: f64,
    #[serde(default)]
    bstar: f64,
}

/// Parse an OMM JSON document — a single object or a 1-element array — into an [`Omm`].
pub fn parse(json: &str) -> Result<Omm, JsonParseError> {
    // Accept a single OMM object, or a 1-element array — some producers
    // (incl. CelesTrak single-satellite GP queries) wrap the object in a JSON
    // array. Reject empty / multi-element arrays with a clear error.
    let raw: OmmJson = if json.trim_start().starts_with('[') {
        let arr: Vec<OmmJson> =
            serde_json::from_str(json).map_err(|e| JsonParseError::Malformed(e.to_string()))?;
        let mut it = arr.into_iter();
        match (it.next(), it.next()) {
            (Some(obj), None) => obj,
            (None, _) => {
                return Err(JsonParseError::Malformed(String::from(
                    "OMM JSON array is empty",
                )));
            }
            (Some(_), Some(_)) => {
                return Err(JsonParseError::Malformed(String::from(
                    "OMM JSON array has multiple objects; expected exactly one",
                )));
            }
        }
    } else {
        serde_json::from_str(json).map_err(|e| JsonParseError::Malformed(e.to_string()))?
    };

    let epoch = crate::omm::parse_epoch(&raw.epoch)
        .ok_or_else(|| JsonParseError::InvalidEpoch(raw.epoch.clone()))?;

    Ok(Omm {
        object_name: raw.object_name,
        object_id: raw.object_id,
        norad_cat_id: raw.norad_cat_id,
        epoch,
        mean_motion: raw.mean_motion * 2.0 * PI / 86400.0, // rev/day → rad/s
        eccentricity: raw.eccentricity,
        inclination: raw.inclination.to_radians(),
        raan: raw.ra_of_asc_node.to_radians(),
        argument_of_perigee: raw.arg_of_pericenter.to_radians(),
        mean_anomaly: raw.mean_anomaly.to_radians(),
        bstar: raw.bstar,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earth::MU as MU_EARTH;

    // ISS OMM JSON (CelesTrak GP format). EPOCH 2024-03-19T12:00 ≡ TLE 2024-079.5,
    // so the values match the ISS_TLE fixture in `crate::tle`.
    const ISS_OMM_JSON: &str = r#"{
        "OBJECT_NAME": "ISS (ZARYA)",
        "OBJECT_ID": "1998-067A",
        "EPOCH": "2024-03-19T12:00:00.000000",
        "MEAN_MOTION": 15.49561654,
        "ECCENTRICITY": 0.0007417,
        "INCLINATION": 51.6400,
        "RA_OF_ASC_NODE": 208.6520,
        "ARG_OF_PERICENTER": 35.3910,
        "MEAN_ANOMALY": 324.7580,
        "BSTAR": 0.00003,
        "NORAD_CAT_ID": 25544,
        "ELEMENT_SET_NO": 999
    }"#;

    #[test]
    fn parse_iss_omm_json() {
        let omm = parse(ISS_OMM_JSON).unwrap();
        assert_eq!(omm.object_name.as_deref(), Some("ISS (ZARYA)"));
        assert_eq!(omm.object_id.as_deref(), Some("1998-067A"));
        assert_eq!(omm.norad_cat_id, 25544);

        let dt = omm.epoch.to_datetime();
        assert_eq!((dt.year, dt.month, dt.day, dt.hour), (2024, 3, 19, 12));

        assert!((omm.inclination.to_degrees() - 51.64).abs() < 1e-9);
        assert!((omm.raan.to_degrees() - 208.652).abs() < 1e-9);
        assert!((omm.eccentricity - 0.0007417).abs() < 1e-12);
        assert!((omm.argument_of_perigee.to_degrees() - 35.391).abs() < 1e-9);
        assert!((omm.mean_anomaly.to_degrees() - 324.758).abs() < 1e-9);
        let mm_rev_day = omm.mean_motion * 86400.0 / (2.0 * PI);
        assert!((mm_rev_day - 15.49561654).abs() < 1e-8);
        assert!((omm.bstar - 3.0e-5).abs() < 1e-10);

        // Semi-major axis consistent with the ISS (~6796 km).
        assert!((omm.semi_major_axis(MU_EARTH) - 6796.0).abs() < 5.0);
    }

    #[test]
    fn epoch_without_z_suffix_is_accepted() {
        // CelesTrak omits the trailing 'Z'; ensure both forms parse identically.
        let with_z = ISS_OMM_JSON.replace("12:00:00.000000", "12:00:00.000000Z");
        let a = parse(ISS_OMM_JSON).unwrap();
        let b = parse(&with_z).unwrap();
        assert_eq!(a.epoch, b.epoch);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(
            parse("{ not json"),
            Err(JsonParseError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_bad_epoch() {
        let j = r#"{
            "NORAD_CAT_ID": 1,
            "EPOCH": "not-a-date",
            "MEAN_MOTION": 1.0,
            "ECCENTRICITY": 0.0,
            "INCLINATION": 0.0,
            "RA_OF_ASC_NODE": 0.0,
            "ARG_OF_PERICENTER": 0.0,
            "MEAN_ANOMALY": 0.0
        }"#;
        assert!(matches!(parse(j), Err(JsonParseError::InvalidEpoch(_))));
    }

    #[test]
    fn parse_single_element_array() {
        // CelesTrak single-satellite OMM JSON is sometimes a 1-element array.
        let arr = ["[", ISS_OMM_JSON, "]"].concat();
        assert_eq!(parse(&arr).unwrap().norad_cat_id, 25544);
    }

    #[test]
    fn rejects_empty_and_multi_element_arrays() {
        assert!(matches!(parse("[]"), Err(JsonParseError::Malformed(_))));
        let two = ["[", ISS_OMM_JSON, ",", ISS_OMM_JSON, "]"].concat();
        assert!(matches!(parse(&two), Err(JsonParseError::Malformed(_))));
    }
}
