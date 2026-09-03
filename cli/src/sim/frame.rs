//! The inertial frames `orts run` can propagate in, and what a frame needs to
//! be one.
//!
//! The propagation frame is a *type* in orts (`OrbitalSystem<F>`,
//! `OrbitalState<F>`) but a *value* on the command line, so the CLI resolves
//! [`FrameChoice`] once and dispatches into a generic body — see
//! `commands::run::run_simulation`. [`RunFrame`] collects the three things
//! that body needs and that differ per frame: how to build the frame's EOP
//! storage from the loaded table, how to rotate an SGP4 TEME state into it,
//! and which choice it implements.
//!
//! Only orbit-only propagation is frame-generic. Attitude, the plugin
//! controller ABI and `orts serve` are `SimpleEci`-locked (see
//! `orts::plugin::tick_input`), which is why `--frame gcrs` is refused there
//! instead of quietly falling back.

use std::sync::Arc;

use arika::earth::eop::{ClampedEop, EopTable};
use arika::earth::transform::EphemerisFrameBridge;
use arika::earth::{EarthFixedTransform, GcrsEopStorage};
use arika::epoch::{Epoch, Utc};
use arika::frame::{FrameTransform, Gcrs, SimpleEci, Teme};
use clap::ValueEnum;

/// Which inertial frame to propagate in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FrameChoice {
    /// ERA-only Earth rotation, no EOP: the historical default, adequate for
    /// visualization and mission-design work.
    SimpleEci,
    /// The IAU 2006/2000A CIO chain with observed EOP (precession, nutation,
    /// ERA, polar motion): the metre-class path. Requires an EOP source.
    Gcrs,
}

impl FrameChoice {
    /// The spelling used on the command line and in config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SimpleEci => "simple-eci",
            Self::Gcrs => "gcrs",
        }
    }
}

/// A frame `orts run` can propagate an orbit in.
///
/// Implemented for exactly the two [`FrameChoice`] variants; the bounds are
/// what the installed models demand (`EarthFixedTransform` for drag and the
/// spherical-harmonic field, `EphemerisFrameBridge` for third-body and SRP).
///
/// The mapping back to a [`FrameChoice`] lives in the `match` that dispatches
/// (`commands::run::run_simulation`), so adding a frame there is a
/// non-exhaustive-match error rather than a silently unused impl.
pub trait RunFrame: EarthFixedTransform + EphemerisFrameBridge {
    /// Build this frame's EOP storage from the table the CLI loaded.
    ///
    /// `Some` only for frames that use EOP; a `Gcrs` run without a table is
    /// refused before reaching here (`validate` / `validate_sim_args`), so the
    /// `None` case is the model-only CIP fallback that `--eop zero` selects.
    fn eop_storage(table: Option<&Arc<EopTable>>) -> Self::EopStorage;

    /// Rotate an SGP4 TEME position/velocity into this frame.
    fn teme_transform(epoch: &Epoch<Utc>) -> FrameTransform<Teme, Self>;
}

impl RunFrame for SimpleEci {
    fn eop_storage(_table: Option<&Arc<EopTable>>) -> Self::EopStorage {}

    fn teme_transform(epoch: &Epoch<Utc>) -> FrameTransform<Teme, Self> {
        FrameTransform::<Teme, SimpleEci>::teme_to_simple_eci(&epoch.to_ut1_naive())
    }
}

impl RunFrame for Gcrs {
    /// One table backs every model: each gets its own clamping provider over a
    /// shared handle, so a 60-year series is not copied per satellite.
    ///
    /// Without a table the storage is the zero provider — the IAU 2006 *model*
    /// CIP with no observed dUT1 or polar motion, which is what `--eop zero`
    /// asks for.
    fn eop_storage(table: Option<&Arc<EopTable>>) -> Self::EopStorage {
        match table {
            Some(table) => GcrsEopStorage::new(ClampedEop::new(Arc::clone(table))),
            None => GcrsEopStorage::new(ZeroEop),
        }
    }

    fn teme_transform(epoch: &Epoch<Utc>) -> FrameTransform<Teme, Self> {
        FrameTransform::<Teme, Gcrs>::teme_to_gcrs(&epoch.to_tt())
    }
}

/// EOP provider with every correction zero: the IAU 2006 model chain only.
///
/// Not an accuracy claim — zeroing dUT1 alone displaces ERA by up to ~0.4
/// arcsecond (~12 m at the equator). It exists so `--eop zero` can give a
/// reproducible `Gcrs` run without shipping a table, and it is the one thing
/// `--frame gcrs` accepts instead of real EOP.
struct ZeroEop;

impl arika::earth::eop::Ut1Offset for ZeroEop {
    fn dut1(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
}

impl arika::earth::eop::PolarMotion for ZeroEop {
    fn x_pole(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
    fn y_pole(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
}

impl arika::earth::eop::NutationCorrections for ZeroEop {
    fn dx(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
    fn dy(&self, _utc_mjd: f64) -> f64 {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arika::earth::{EarthOrientation, EarthRotationPole};
    use arika::frame::Vec3;

    fn epoch() -> Epoch<Utc> {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    fn eop_table() -> Arc<EopTable> {
        Arc::new(
            EopTable::from_finals2000a(include_str!(
                "../../../orts/tests/fixtures/finals2000A.sample"
            ))
            .expect("EOP fixture parses"),
        )
    }

    #[test]
    fn frame_choice_spellings_round_trip_through_clap() {
        for choice in [FrameChoice::SimpleEci, FrameChoice::Gcrs] {
            let parsed = FrameChoice::from_str(choice.as_str(), true).expect("spelling parses");
            assert_eq!(parsed, choice);
        }
        assert!(FrameChoice::from_str("eme2000", true).is_err());
    }

    /// The two frames really are different: the ERA-only pole is `+Z`, the
    /// `Gcrs` one is the CIP, ~0.1° away in 2024.
    #[test]
    fn the_two_frames_disagree_about_the_pole() {
        let simple = SimpleEci::earth_pole(&epoch()).into_inner();
        let gcrs = Gcrs::earth_pole(&epoch()).into_inner();
        assert_eq!(simple, nalgebra::Vector3::new(0.0, 0.0, 1.0));
        let angle_deg = gcrs.angle(&simple).to_degrees();
        assert!(
            (0.01..1.0).contains(&angle_deg),
            "CIP offset {angle_deg} deg"
        );
    }

    /// A real table must reach the transform: the same position converts to a
    /// different geodetic longitude with observed EOP than with zeros, by
    /// about the dUT1 the fixture carries (a few tenths of a second of time,
    /// so a few arcseconds of rotation).
    #[test]
    fn gcrs_eop_storage_carries_the_table() {
        let pos = Vec3::<Gcrs>::new(6948.0, 0.0, 0.0);
        let with_table = Gcrs::eop_storage(Some(&eop_table()));
        let without = Gcrs::eop_storage(None);
        let lon = |eop: &GcrsEopStorage| {
            Gcrs::to_geodetic(&pos, &EarthOrientation::new(epoch(), eop)).longitude
        };
        let diff_arcsec = (lon(&with_table) - lon(&without)).to_degrees() * 3600.0;
        assert!(
            (0.1..60.0).contains(&diff_arcsec.abs()),
            "observed EOP should move the longitude by arcseconds, got {diff_arcsec}\""
        );
    }

    /// `SimpleEci` needs no EOP at all, which is why its storage is `()`.
    #[test]
    fn simple_eci_storage_is_unit_regardless_of_the_table() {
        let _: () = SimpleEci::eop_storage(None);
        let _: () = SimpleEci::eop_storage(Some(&eop_table()));
    }

    /// Both TEME bridges preserve the magnitude and differ from each other by
    /// the frame-bias + precession/nutation the simple frame omits.
    #[test]
    fn teme_transforms_agree_on_magnitude_and_differ_in_direction() {
        let r = Vec3::<Teme>::new(4000.0, -3000.0, 5000.0);
        let v = Vec3::<Teme>::new(1.0, 7.0, 0.5);
        let (r_simple, _) = SimpleEci::teme_transform(&epoch()).transform_state(&r, &v);
        let (r_gcrs, _) = Gcrs::teme_transform(&epoch()).transform_state(&r, &v);
        let n_simple = r_simple.into_inner().norm();
        let n_gcrs = r_gcrs.into_inner().norm();
        assert!((n_simple - r.into_inner().norm()).abs() < 1e-9);
        assert!((n_gcrs - r.into_inner().norm()).abs() < 1e-9);
        let sep = (r_simple.into_inner() - r_gcrs.into_inner()).norm();
        assert!(
            (0.1..100.0).contains(&sep),
            "TEME->SimpleEci vs TEME->Gcrs differ by {sep} km"
        );
    }
}
