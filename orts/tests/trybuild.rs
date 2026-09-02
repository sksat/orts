//! Compile-fail tests that pin the frame-typing of attitude values (#332):
//! a quaternion's components depend on the frame it is expressed against, so a
//! reading or a pointing target from one inertial frame must not be usable
//! where another is required.
//!
//! Each `.rs` file in `tests/trybuild/` must fail to compile for the documented
//! reason; the corresponding `.stderr` file captures the expected diagnostic.
//!
//! Run with: `cargo test -p orts --test trybuild`.
//!
//! Updating stderr files: set `TRYBUILD=overwrite` and rerun the test.

#[test]
fn a_non_simple_eci_attitude_cannot_be_used_where_simple_eci_is_required() {
    let t = trybuild::TestCases::new();
    // The v0 plugin WIT payload is defined as body→simple-ECI, so the named
    // boundary operation exists only for a `SimpleEci`-tagged reading.
    t.compile_fail("tests/trybuild/gcrs_attitude_at_wit_v0_boundary.rs");
    // A whole sensor bundle evaluated in `Gcrs` cannot be handed to a plugin.
    t.compile_fail("tests/trybuild/gcrs_sensors_into_tick_input.rs");
    // An `InertialPointing` target expressed in `Gcrs` cannot steer a system
    // integrated in `SimpleEci`.
    t.compile_fail("tests/trybuild/gcrs_inertial_pointing_in_simple_eci.rs");

    // A model carrying its own frame must be installed for a state in that frame.
    t.compile_fail("tests/trybuild/frame_capability_model_in_another_frame.rs");
    // Same seam for the spherical-harmonic field: its longitude-dependent
    // terms are rotated through `F`'s Earth-fixed chain, so a `Gcrs` field
    // cannot be installed for a `SimpleEci` state.
    t.compile_fail("tests/trybuild/spherical_harmonic_gravity_in_another_frame.rs");

    // A burn Delta-v given in one inertial frame cannot be flown in another.
    t.compile_fail("tests/trybuild/simple_eci_burn_in_gcrs_system.rs");

    // A burn direction can only be held fixed in a frame whose axes are.
    t.compile_fail("tests/trybuild/constant_thrust_in_of_date_frame.rs");
}
