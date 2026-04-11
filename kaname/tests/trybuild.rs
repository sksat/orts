//! Compile-fail tests that pin the "NullEop is rejected by precise APIs"
//! guarantee. Each `.rs` file in `tests/trybuild/` must fail to compile for
//! the documented reason; the corresponding `.stderr` file captures the
//! expected diagnostic.
//!
//! Run with: `cargo test -p kaname --test trybuild`.
//!
//! Updating stderr files: set `TRYBUILD=overwrite` and rerun the test.

#[test]
fn null_eop_is_rejected_by_precise_apis() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/trybuild/null_eop_in_to_ut1.rs");
}
