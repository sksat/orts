//! CCSDS Orbit Mean-Elements Message (OMM) format decoders.
//!
//! OMM is the CCSDS standard that Space-Track and CelesTrak recommend as the
//! overflow-proof successor to the legacy TLE catalog (whose 5-digit number is
//! only extended, not fixed, by Alpha-5 — see [`crate::tle`]). Its
//! `NORAD_CAT_ID` has no fixed width.
//!
//! These submodules decode the three OMM serializations — [`json`], [`kvn`],
//! [`xml`] — into the shared [`crate::elements::Sgp4Elements`] record. The
//! format-detecting [`crate::elements::parse`] entry point dispatches to them.
//!
//! Angles are converted to **radians** and mean motion to **rad/s** (orts
//! conventions) from each format's native units (degrees, rev/day) at parse
//! time.

pub mod json;
pub mod kvn;
pub mod xml;
