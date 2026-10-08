//! The moment MTQ rods put out over time, after each command.
//!
//! A rod does not follow a new command at once: its drive (the coil current,
//! as the moment it would make) approaches the command with a first-order
//! response, the core's remanence follows that drive, and the rod's moment is
//! the drive blended with the remanence (DESIGN.md "MTQ の rod の moment は、遅れて追従する駆動と残留磁化から決める").
//! [`MtqMomentProfile`] is that over one held
//! command, evaluated at any time; [`MtqMomentDrive`] owns the rods' state
//! across commands.

use super::mtq::{MtqAssemblyCore, MtqCommand};
use super::remanence::{MtqRemanence, RemanencePlay};

/// The rods over the span one command is held for: each rod's drive goes from
/// its value at `start_t` towards the clamped command with time constant
/// `τ`, the cores' remanence follows the drive, and the moment blends the two.
///
/// Within the span the drive moves monotonically towards the command, and a
/// play operator driven monotonically ends where its last input puts it, so
/// the remanence at any time is the remanence at the start driven once to the
/// drive at that time. Evaluating is therefore a pure function of time, which
/// an integrator may revisit at will.
#[derive(Debug, Clone)]
pub struct MtqMomentProfile {
    core: MtqAssemblyCore,
    /// When the command was applied [s].
    start_t: f64,
    /// Each rod's drive at `start_t` [A·m²].
    drive_start: Vec<f64>,
    /// Each rod's clamped command [A·m²].
    drive_target: Vec<f64>,
    /// Response time constant [s].
    time_constant: f64,
    /// The cores' remanence at `start_t`; `None` keeps none.
    remanence: Option<MtqRemanence>,
}

impl MtqMomentProfile {
    /// Rods of `core` off, with no remanence, from `start_t` on.
    pub fn off(core: MtqAssemblyCore, start_t: f64) -> Self {
        let off = vec![0.0; core.num_mtqs()];
        Self {
            core,
            start_t,
            drive_start: off.clone(),
            drive_target: off,
            time_constant: 0.0,
            remanence: None,
        }
    }

    /// The rods this profile is for.
    pub fn core(&self) -> &MtqAssemblyCore {
        &self.core
    }

    /// When the command was applied [s].
    pub fn start_t(&self) -> f64 {
        self.start_t
    }

    /// Each rod's drive at time `t` [A·m²]: the start before `start_t`, the
    /// command from it on with no time constant.
    pub fn drive_at(&self, t: f64) -> Vec<f64> {
        if t < self.start_t {
            return self.drive_start.clone();
        }
        if self.time_constant == 0.0 {
            return self.drive_target.clone();
        }
        let left = (-(t - self.start_t) / self.time_constant).exp();
        // A weighted mean of the two ends: no difference of them is formed,
        // so ends of opposite sign near the largest finite value cannot
        // overflow, and the result stays between them.
        self.drive_start
            .iter()
            .zip(&self.drive_target)
            .map(|(&start, &target)| start * left + target * (1.0 - left))
            .collect()
    }

    /// The cores' remanence at time `t`, `None` with no remanence.
    fn remanence_at(&self, t: f64) -> Option<MtqRemanence> {
        let mut remanence = self.remanence.clone()?;
        remanence.apply(&self.drive_at(t));
        Some(remanence)
    }

    /// Each rod's moment at time `t` [A·m²]: the drive blended with the
    /// remanence ([`MtqAssemblyCore::rod_moments_with_remanence`]).
    pub fn at(&self, t: f64) -> Vec<f64> {
        let drive = self.drive_at(t);
        match self.remanence_at(t) {
            Some(remanence) => self
                .core
                .rod_moments_with_remanence(&drive, remanence.residual()),
            None => drive,
        }
    }
}

/// The MTQ rods' state across commands: the profile of the command held now,
/// which carries the drive and the remanence where that command took over.
///
/// It is changed only by [`Self::apply`], when a command is applied; the
/// moments at any time come from [`Self::moments_at`], which both the torque
/// (through [`Self::profile`]) and a coupled magnetometer read. Applying the
/// same command again at a later time leaves the moments the same, so how
/// many controller ticks a command is held for does not matter: an
/// exponential restarted from where it is follows the same curve, and the
/// remanence driven along it ends in the same state.
#[derive(Debug, Clone)]
pub struct MtqMomentDrive {
    time_constant: f64,
    profile: MtqMomentProfile,
}

impl MtqMomentDrive {
    /// Rods with no remanence that follow each command at once, off from
    /// `start_t` on.
    ///
    /// # Panics
    /// If `start_t` is non-finite.
    pub fn new(core: MtqAssemblyCore, start_t: f64) -> Self {
        assert!(start_t.is_finite(), "start_t must be finite, got {start_t}");
        Self {
            time_constant: 0.0,
            profile: MtqMomentProfile::off(core, start_t),
        }
    }

    /// The same rods with demagnetized cores whose remanence follows `plays`
    /// (`plays[i]` for rod `i`), built against these rods' limits so that no
    /// other core's can apply. A builder step: any remanence so far is reset.
    ///
    /// # Panics
    /// As [`MtqRemanence::demagnetized_with_plays`].
    pub fn with_remanence_plays(mut self, plays: Vec<Vec<RemanencePlay>>) -> Self {
        self.profile.remanence = Some(MtqRemanence::demagnetized_with_plays(
            &self.profile.core,
            plays,
        ));
        self
    }

    /// The same rods responding to a command with time constant
    /// `time_constant` [s].
    ///
    /// # Panics
    /// If `time_constant` is negative or non-finite.
    pub fn with_time_constant(mut self, time_constant: f64) -> Self {
        assert!(
            time_constant.is_finite() && time_constant >= 0.0,
            "time_constant must be finite and non-negative, got {time_constant}"
        );
        self.time_constant = time_constant;
        self
    }

    /// The rods' geometry and limits.
    pub fn core(&self) -> &MtqAssemblyCore {
        &self.profile.core
    }

    /// Response time constant [s].
    pub fn time_constant(&self) -> f64 {
        self.time_constant
    }

    /// Each rod's moment at time `t` [A·m²], under the command applied last.
    pub fn moments_at(&self, t: f64) -> Vec<f64> {
        self.profile.at(t)
    }

    /// The profile of the command applied last, for an
    /// [`MtqAssembly`](super::MtqAssembly) to evaluate.
    pub fn profile(&self) -> &MtqMomentProfile {
        &self.profile
    }

    /// Apply `command` at time `t`: the drive and the remanence carry on from
    /// where they are at `t`, and the drive heads for the clamped command.
    ///
    /// A non-finite command realizes a non-finite moment while it is held, as
    /// [`MtqAssemblyCore::realized_rod_moments`] does; a drive it leaves
    /// non-finite starts the next command from that command itself, so it
    /// does not outlast its own span.
    ///
    /// # Panics
    /// If the command's length differs from the number of MTQs, `t` is
    /// non-finite, or `t` is before the last command's: the state carries on
    /// from the last command, so commands come in time order.
    pub fn apply(&mut self, t: f64, command: &MtqCommand) {
        assert!(t.is_finite(), "a command's time must be finite, got {t}");
        assert!(
            t >= self.profile.start_t,
            "commands come in time order: {t} is before the last command's {}",
            self.profile.start_t
        );
        let drive_target = self.profile.core.realized_rod_moments(command);
        let drive_start = self
            .profile
            .drive_at(t)
            .into_iter()
            .zip(&drive_target)
            .map(|(d, &c)| if d.is_finite() { d } else { c })
            .collect();
        let remanence = self.profile.remanence_at(t);
        self.profile = MtqMomentProfile {
            core: self.profile.core.clone(),
            start_t: t,
            drive_start,
            drive_target,
            time_constant: self.time_constant,
            remanence,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacecraft::Mtq;
    use nalgebra::Vector3;

    const MAX: f64 = 10.0;
    const TAU: f64 = 0.05;
    const REMANENCE: f64 = 0.06;

    fn one_rod() -> MtqAssemblyCore {
        MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), MAX)])
    }

    fn remanent(tau: f64) -> MtqMomentDrive {
        MtqMomentDrive::new(one_rod(), 0.0)
            .with_remanence_plays(vec![RemanencePlay::from_residual_moment(REMANENCE)])
            .with_time_constant(tau)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-12,
            "expected {expected}, got {actual}"
        );
    }

    /// With no time constant a command takes effect at once, exactly the
    /// clamped command, as before the response was modelled.
    #[test]
    fn without_a_time_constant_the_command_takes_effect_at_once() {
        let mut drive = MtqMomentDrive::new(one_rod(), 0.0);
        assert_eq!(drive.moments_at(0.5), vec![0.0], "off before any command");
        drive.apply(1.0, &MtqCommand::Moments(vec![12.0]));
        assert_eq!(drive.moments_at(1.0), vec![MAX]);
        assert_eq!(drive.moments_at(1.3), vec![MAX]);
    }

    /// Switched off after a full drive, the moment falls towards 0 with the
    /// time constant: e^-1 of it is left after one τ.
    #[test]
    fn switching_off_decays_with_the_time_constant() {
        let mut drive = MtqMomentDrive::new(one_rod(), 0.0).with_time_constant(TAU);
        drive.apply(0.0, &MtqCommand::Moments(vec![MAX]));
        drive.apply(1.0, &MtqCommand::Moments(vec![0.0]));
        // 1 s after switching on, 20 τ: the rod is at 10 within e^-20.
        let on = drive.moments_at(1.0)[0];
        assert!((on - MAX).abs() < MAX * 1e-8, "{on}");
        assert_close(drive.moments_at(1.0 + TAU)[0], on * (-1.0_f64).exp());
        assert_close(drive.moments_at(1.0 + 3.0 * TAU)[0], on * (-3.0_f64).exp());
    }

    /// Switched off after a full drive held for 20 τ, the moment settles to
    /// the remanence: the drive decays, and the moment is the drive blended
    /// with what the core keeps.
    #[test]
    fn switching_off_settles_to_the_remanence() {
        let mut drive = remanent(TAU);
        drive.apply(0.0, &MtqCommand::Moments(vec![MAX]));
        drive.apply(1.0, &MtqCommand::Moments(vec![0.0]));
        let reached = drive.profile().drive_at(1.0)[0];
        // From demagnetized, the one operator keeps (v - 1/2) / (1/2) of the
        // remanence for a drive v above half the limit.
        let kept = REMANENCE * (reached / MAX - 0.5) / 0.5;
        let d = reached * (-2.0_f64).exp();
        assert_close(
            drive.moments_at(1.0 + 2.0 * TAU)[0],
            d + kept * (1.0 - d / MAX),
        );
        let settled = drive.moments_at(1.0 + 40.0 * TAU)[0];
        assert!((settled - kept).abs() < 1e-12, "{settled}");
        assert!((kept - REMANENCE).abs() < 1e-8, "a 20 τ drive saturates");
    }

    /// The remanence follows the drive the coil reached, not the command: a
    /// full command switched off after a tenth of τ drives the core to under
    /// a tenth of the limit, below the operator's width of a half, which
    /// leaves nothing.
    #[test]
    fn a_pulse_whose_drive_stays_below_every_width_leaves_no_remanence() {
        let mut drive = remanent(0.1);
        drive.apply(0.0, &MtqCommand::Moments(vec![MAX]));
        drive.apply(0.01, &MtqCommand::Moments(vec![0.0]));
        assert!(
            drive.moments_at(10.0)[0].abs() < 1e-12,
            "{:?}",
            drive.moments_at(10.0)
        );
        // Held long enough, the same command leaves the remanence.
        let mut held = remanent(0.1);
        held.apply(0.0, &MtqCommand::Moments(vec![MAX]));
        held.apply(5.0, &MtqCommand::Moments(vec![0.0]));
        assert!((held.moments_at(15.0)[0] - REMANENCE).abs() < 1e-12);
    }

    /// Applying the same command again later, as each controller tick of a
    /// held command does, leaves the moments on the same curve, the
    /// remanence included.
    #[test]
    fn reapplying_the_same_command_keeps_the_curve() {
        let make = || {
            let mut d = remanent(TAU);
            d.apply(0.0, &MtqCommand::Moments(vec![MAX]));
            d.apply(1.0, &MtqCommand::Moments(vec![-6.0]));
            d
        };
        let once = make();
        let mut ticked = make();
        for k in 1..=10 {
            ticked.apply(1.0 + 0.01 * k as f64, &MtqCommand::Moments(vec![-6.0]));
        }
        for t in [1.1, 1.2, 1.5] {
            let (a, b) = (once.moments_at(t)[0], ticked.moments_at(t)[0]);
            assert!((a - b).abs() < 1e-12, "t {t}: {a} vs {b}");
        }
    }

    /// The profile is pure: evaluating a time again, or one before the
    /// command, gives the same moments, as a rejected integrator step does.
    #[test]
    fn the_profile_is_pure_and_holds_its_start_before_it() {
        let mut drive = MtqMomentDrive::new(one_rod(), 0.0).with_time_constant(TAU);
        drive.apply(2.0, &MtqCommand::Moments(vec![MAX]));
        let profile = drive.profile().clone();
        let first = profile.at(2.03);
        let _ = profile.at(2.5);
        assert_eq!(profile.at(2.03), first);
        assert_eq!(profile.at(1.0), vec![0.0], "before the command: its start");
    }

    /// With no time constant too, a time before the command gives the moment
    /// from before it, and the command from its own time on.
    #[test]
    fn a_step_profile_holds_its_start_before_the_command() {
        let mut drive = MtqMomentDrive::new(one_rod(), 0.0);
        drive.apply(2.0, &MtqCommand::Moments(vec![MAX]));
        assert_eq!(drive.moments_at(1.999), vec![0.0]);
        assert_eq!(drive.moments_at(2.0), vec![MAX]);
    }

    /// Reversing from one full-scale end to the other near f64::MAX: the
    /// drive is a weighted mean of the ends, so it stays finite and between
    /// them.
    #[test]
    fn a_full_scale_reversal_at_huge_limits_stays_finite() {
        let huge = 1e308;
        let core = MtqAssemblyCore::new(vec![Mtq::new(Vector3::x(), huge)]);
        let mut drive = MtqMomentDrive::new(core, 0.0).with_time_constant(1.0);
        drive.apply(0.0, &MtqCommand::Moments(vec![huge]));
        drive.apply(1000.0, &MtqCommand::Moments(vec![-huge]));
        for t in [1000.0, 1000.5, 1001.0, 1010.0] {
            let u = drive.moments_at(t)[0];
            assert!(u.is_finite() && u.abs() <= huge, "t {t}: {u}");
        }
    }

    /// A non-finite command realizes a non-finite moment while it is held,
    /// and the next command starts clean instead of carrying it on.
    #[test]
    fn a_non_finite_command_does_not_outlast_its_span() {
        let mut drive = remanent(0.1);
        drive.apply(0.0, &MtqCommand::Moments(vec![f64::NAN]));
        assert!(drive.moments_at(0.05)[0].is_nan());
        drive.apply(1.0, &MtqCommand::Moments(vec![0.0]));
        assert_eq!(drive.moments_at(1.05), vec![0.0]);
    }

    #[test]
    #[should_panic(expected = "time_constant")]
    fn a_negative_time_constant_is_refused() {
        let _ = MtqMomentDrive::new(one_rod(), 0.0).with_time_constant(-1.0);
    }

    #[test]
    #[should_panic(expected = "time order")]
    fn a_command_before_the_last_one_is_refused() {
        let mut drive = MtqMomentDrive::new(one_rod(), 0.0);
        drive.apply(2.0, &MtqCommand::Moments(vec![MAX]));
        drive.apply(1.0, &MtqCommand::Moments(vec![0.0]));
    }

    #[test]
    #[should_panic(expected = "start_t")]
    fn a_non_finite_start_is_refused() {
        let _ = MtqMomentDrive::new(one_rod(), f64::NAN);
    }

    #[test]
    #[should_panic(expected = "MTQ count")]
    fn remanence_plays_for_another_rod_count_are_refused() {
        let _ = MtqMomentDrive::new(one_rod(), 0.0).with_remanence_plays(vec![vec![]; 3]);
    }

    /// The remanence is built against the drive's own rods: plays whose
    /// remanence exceeds this rod's limit are refused when attached, not when
    /// a later command would realize them.
    #[test]
    #[should_panic(expected = "max_moment")]
    fn remanence_plays_above_the_drives_limits_are_refused() {
        let _ = MtqMomentDrive::new(one_rod(), 0.0)
            .with_remanence_plays(vec![RemanencePlay::from_residual_moment(20.0)]);
    }
}
