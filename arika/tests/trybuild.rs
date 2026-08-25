//! Compile-fail tests that pin type-level guarantees about EOP data: that
//! `NullEop` is rejected by the precise APIs, and that the EOP-free
//! `EarthOrientation` constructor is unavailable to a frame that needs EOP.
//! Each `.rs` file in `tests/trybuild/` must fail to compile for the documented
//! reason; the corresponding `.stderr` file captures the expected diagnostic.
//!
//! Run with: `cargo test -p arika --test trybuild`.
//!
//! Updating stderr files: set `TRYBUILD=overwrite` and rerun the test.

#[test]
fn null_eop_is_rejected_by_precise_apis() {
    let t = trybuild::TestCases::new();
    // Phase 2: NullEop → Epoch::<Utc>::to_ut1
    t.compile_fail("tests/trybuild/null_eop_in_to_ut1.rs");
    // Phase 3B: NullEop → Rotation::<Gcrs, Cirs>::iau2006
    t.compile_fail("tests/trybuild/null_eop_in_iau2006.rs");
    // Phase 3B: NullEop → Rotation::<Tirs, Itrs>::polar_motion
    t.compile_fail("tests/trybuild/null_eop_in_polar_motion.rs");
    // Phase 3B: NullEop → Rotation::<Gcrs, Itrs>::iau2006_full_from_utc
    t.compile_fail("tests/trybuild/null_eop_in_iau2006_full_from_utc.rs");
}

#[test]
fn eop_free_orientation_is_rejected_for_a_frame_that_needs_eop() {
    let t = trybuild::TestCases::new();
    // EarthOrientation::simple is `EopStorage = ()`-only, so Gcrs cannot use it.
    t.compile_fail("tests/trybuild/earth_orientation_simple_for_gcrs.rs");
}
