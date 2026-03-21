//! IGRF-14 Gauss coefficients (auto-generated at build time).
//!
//! Source: International Association of Geomagnetism and Aeronomy (IAGA)
//! Reference: Alken et al. (2025), "International Geomagnetic Reference Field:
//!   the fourteenth generation", Earth Planets Space, 77, 79.
//!
//! Coefficients are scientific factual data published by IAGA and distributed
//! by NOAA (<https://www.ngdc.noaa.gov/IAGA/vmod/igrf14coeffs.txt>).

#![allow(clippy::large_const_arrays)]

include!(concat!(env!("OUT_DIR"), "/igrf_generated.rs"));
