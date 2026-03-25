"""Tests for camera policy system. No rendering — pure numpy assertions."""
import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).parent))
from camera_policy import (
    LUNAR_PITCH_FRACTION,
    CameraSchedule,
    build_camera_schedule,
    build_trajectory_context,
    compute_policy_weights,
)


@pytest.fixture
def apollo_ctx():
    """Build TrajectoryContext from actual Apollo 11 data (requires RRD)."""
    from plot_common import compute_derived, load_data
    sat, moon = load_data()
    d = compute_derived(sat, moon)
    N = len(d["sx"])
    # Use 1000 frames for fast tests
    r_moon_s = d["r_moon"] / 1000.0
    in_orbit = r_moon_s < 10.0
    orbit_start = int(np.argmax(in_orbit)) if in_orbit.any() else N
    orbit_end = N - int(np.argmax(in_orbit[::-1])) if in_orbit.any() else N
    idx_float = np.concatenate([
        np.linspace(0, max(orbit_start - 1, 0), 200),
        np.linspace(orbit_start, min(orbit_end - 1, N - 1), 600),
        np.linspace(min(orbit_end, N - 1), N - 1, 200),
    ])
    return build_trajectory_context(d, idx_float)


@pytest.fixture
def apollo_schedule(apollo_ctx):
    return build_camera_schedule(apollo_ctx)


# --- Policy weight tests ---

class TestPolicyWeights:
    def test_weights_sum_to_one(self, apollo_ctx):
        w = compute_policy_weights(apollo_ctx)
        np.testing.assert_allclose(w.sum(axis=1), 1.0, atol=1e-6)

    def test_weights_non_negative(self, apollo_ctx):
        w = compute_policy_weights(apollo_ctx)
        assert np.all(w >= -1e-10)

    def test_weights_continuous(self, apollo_ctx):
        """Weight change rate should be bounded (considering frame time step)."""
        w = compute_policy_weights(apollo_ctx)
        dt = np.diff(apollo_ctx.t_hours)
        dt = np.maximum(dt, 1e-6)
        w_rate = np.max(np.abs(np.diff(w, axis=0)) / dt[:, None], axis=1)
        max_rate = w_rate.max()
        assert max_rate < 1.0, f"Weight rate {max_rate:.3f}/h exceeds 1.0/h"

    def test_earth_orbit_dominant_in_leo(self, apollo_ctx):
        """First few frames (LEO) should be dominated by earth_orbit policy."""
        w = compute_policy_weights(apollo_ctx)
        # Frame 0 is in LEO
        assert w[0, 0] > 0.7, f"Earth orbit weight at start: {w[0, 0]:.3f}"

    def test_lunar_orbit_dominant_near_moon(self, apollo_ctx):
        """Frames deep in lunar orbit should be dominated by lunar_orbit policy."""
        w = compute_policy_weights(apollo_ctx)
        # Middle of schedule (frames 400-600 in our 1000-frame allocation)
        mid = len(w) // 2
        assert w[mid, 2] > 0.9, f"Lunar orbit weight at mid: {w[mid, 2]:.3f}"


# --- Schedule smoothness tests ---

class TestScheduleSmoothness:
    def test_look_direction_normalized(self, apollo_schedule):
        dirs = apollo_schedule.look_direction()
        norms = np.linalg.norm(dirs, axis=1)
        np.testing.assert_allclose(norms, 1.0, atol=1e-6)

    def test_up_vector_normalized(self, apollo_schedule):
        norms = np.linalg.norm(apollo_schedule.up, axis=1)
        np.testing.assert_allclose(norms, 1.0, atol=1e-6)

    def test_angular_rate_bounded(self, apollo_ctx, apollo_schedule):
        """Look direction change per second should be bounded.

        With 1000 test frames over ~200h, each frame spans ~0.7h.
        Scale the threshold by frame duration.
        """
        rates = apollo_schedule.angular_rate()
        # Degrees per hour (approximate)
        dt_hours = np.diff(apollo_ctx.t_hours)
        dt_hours = np.maximum(dt_hours, 1e-6)
        rate_per_hour = rates / dt_hours
        max_rate_h = rate_per_hour.max()
        # Lunar orbit rotates ~180°/h. Allow some headroom for transitions.
        assert max_rate_h < 700.0, f"Max angular rate {max_rate_h:.1f}°/h exceeds 700°/h"

    def test_fov_transitions_smooth(self, apollo_ctx, apollo_schedule):
        """FOV change per hour should be bounded."""
        fov_diff = np.abs(np.diff(apollo_schedule.fov))
        dt_hours = np.diff(apollo_ctx.t_hours)
        dt_hours = np.maximum(dt_hours, 1e-6)
        fov_rate = fov_diff / dt_hours
        max_rate = fov_rate.max()
        assert max_rate < 100.0, f"FOV rate {max_rate:.1f}°/h exceeds 100°/h"

    def test_ambient_transitions_smooth(self, apollo_schedule):
        """Ambient should not jump by more than 0.05 between frames."""
        amb_diff = np.abs(np.diff(apollo_schedule.ambient))
        max_jump = amb_diff.max()
        assert max_jump < 0.05, f"Ambient jump {max_jump:.3f} exceeds 0.05"


# --- Lunar orbit specific tests ---

class TestLunarOrbit:
    def test_policy_horizon_angle(self, apollo_ctx):
        """Lunar orbit POLICY output (pre-EMA) should have predictable nadir angle.

        For the nadir-based pitch algorithm, look·nadir = sin(horizon/2).
        This tests the geometric correctness of the policy independent of EMA.
        """
        from camera_policy import _lunar_orbit_policy
        focal, up, fov, ambient = _lunar_orbit_policy(apollo_ctx)

        for fi in range(apollo_ctx.n_frames):
            if apollo_ctx.r_moon[fi] > 3.0:
                continue
            nadir = apollo_ctx.moon[fi] - apollo_ctx.sc[fi]
            nadir /= np.linalg.norm(nadir)
            if abs(np.dot(apollo_ctx.vel_dir[fi], nadir)) > 0.3:
                continue

            look = focal[fi] - apollo_ctx.sc[fi]
            look /= np.linalg.norm(look)
            dot_nadir = np.dot(look, nadir)

            # Expected: sin(horizon_below / 2)
            safe_ratio = min(apollo_ctx.mr / max(apollo_ctx.r_moon[fi], apollo_ctx.mr + 0.001), 1.0)
            horizon_below = 90.0 - np.degrees(np.arcsin(safe_ratio))
            expected = np.sin(np.radians(horizon_below * LUNAR_PITCH_FRACTION))

            assert abs(dot_nadir - expected) < 0.05, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"look·nadir={dot_nadir:.3f}, expected={expected:.3f}"
            )

    def test_policy_horizon_level(self, apollo_ctx):
        """Radial-up guarantees nadir projects straight down: zero right-axis component.

        If the up vector is derived from radial outward (away from Moon), nadir
        will project to exactly the vertical screen axis, making the horizon
        appear perfectly horizontal. The mathematical condition is:
        nadir · right = 0, where right = look × up.
        """
        from camera_policy import _lunar_orbit_policy
        focal, up, fov, ambient = _lunar_orbit_policy(apollo_ctx)

        for fi in range(apollo_ctx.n_frames):
            if apollo_ctx.r_moon[fi] > 3.0:
                continue
            nadir = apollo_ctx.moon[fi] - apollo_ctx.sc[fi]
            nadir /= np.linalg.norm(nadir)

            look = focal[fi] - apollo_ctx.sc[fi]
            look /= np.linalg.norm(look)

            right = np.cross(look, up[fi])
            rn = np.linalg.norm(right)
            if rn < 1e-10:
                continue
            right /= rn

            nadir_right = abs(np.dot(nadir, right))
            assert nadir_right < 0.01, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"|nadir · right| = {nadir_right:.4f}, horizon is tilted"
            )

    def test_schedule_horizon_tilt_stable(self, apollo_ctx, apollo_schedule):
        """Post-EMA horizon tilt should not oscillate during lunar orbit.

        Even with EMA smoothing, the horizon tilt angle between consecutive
        lunar orbit frames should change by less than 2°.
        """
        tilts = []
        for fi in range(apollo_ctx.n_frames):
            if apollo_ctx.r_moon[fi] > 3.0:
                continue
            nadir = apollo_ctx.moon[fi] - apollo_ctx.sc[fi]
            nadir /= np.linalg.norm(nadir)

            look = apollo_schedule.focal_point[fi] - apollo_schedule.position[fi]
            look /= np.linalg.norm(look)

            right = np.cross(look, apollo_schedule.up[fi])
            rn = np.linalg.norm(right)
            if rn < 1e-10:
                continue
            right /= rn
            tilts.append(np.degrees(np.arcsin(np.clip(np.dot(nadir, right), -1, 1))))

        tilts = np.array(tilts)
        if len(tilts) > 1:
            max_tilt_change = np.max(np.abs(np.diff(tilts)))
            assert max_tilt_change < 2.0, (
                f"Horizon tilt changes by up to {max_tilt_change:.2f}° "
                f"between consecutive lunar frames"
            )

    def test_schedule_horizon_visible(self, apollo_ctx, apollo_schedule):
        """Final schedule (post-EMA) should keep horizon in view during ALL of lunar orbit.

        This explicitly includes periapsis/apoapsis frames where vel_dir is
        nearly radial — the camera must handle these without filling the screen
        with Moon surface.
        """
        for fi in range(apollo_ctx.n_frames):
            if apollo_ctx.r_moon[fi] > 3.0:
                continue
            nadir = apollo_ctx.moon[fi] - apollo_ctx.sc[fi]
            nadir /= np.linalg.norm(nadir)

            look = apollo_schedule.focal_point[fi] - apollo_schedule.position[fi]
            look /= np.linalg.norm(look)
            dot_nadir = np.dot(look, nadir)

            # The horizon is visible if the top edge of the FOV extends above
            # the horizon line. Geometric condition:
            #   angle_from_nadir + FOV/2 > horizon_angle_from_nadir
            angle_from_nadir = np.arccos(np.clip(dot_nadir, -1, 1))
            safe_r = min(apollo_ctx.mr / max(apollo_ctx.r_moon[fi], apollo_ctx.mr + 0.001), 1.0)
            horizon_from_nadir = np.arcsin(safe_r)
            fov_half_rad = np.radians(apollo_schedule.fov[fi] / 2.0)
            top_of_fov = angle_from_nadir + fov_half_rad
            assert top_of_fov > horizon_from_nadir, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"look·nadir={dot_nadir:.3f}, top_of_fov={np.degrees(top_of_fov):.1f}° "
                f"< horizon={np.degrees(horizon_from_nadir):.1f}° from nadir "
                f"— horizon not in view"
            )


# --- Consistency test ---

class TestTransit:
    def test_looks_at_earth_early_outbound(self, apollo_ctx, apollo_schedule):
        """Early outbound transit should look at Earth (still nearby and visible)."""
        for fi in range(apollo_ctx.n_frames):
            # Early outbound: Earth closer, well into transit zone
            if apollo_ctx.r_earth[fi] < 40.0 or apollo_ctx.r_earth[fi] > 150.0:
                continue  # too close (LEO/transition) or past midpoint
            if apollo_ctx.r_moon[fi] < 25.0:
                continue
            if apollo_ctx.r_earth[fi] > apollo_ctx.r_moon[fi]:
                continue  # past midpoint
            # Skip near-midpoint frames where the camera is rotating between bodies
            if abs(apollo_ctx.r_moon[fi] - apollo_ctx.r_earth[fi]) < 20.0:
                continue
            earth_dir = apollo_ctx.earth[fi] - apollo_ctx.sc[fi]
            earth_dir /= np.linalg.norm(earth_dir)
            look = apollo_schedule.focal_point[fi] - apollo_schedule.position[fi]
            look /= np.linalg.norm(look)
            angle = np.degrees(np.arccos(np.clip(np.dot(look, earth_dir), -1, 1)))
            fov_half = apollo_schedule.fov[fi] / 2.0
            assert angle < fov_half + 15.0, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"Earth at {angle:.1f}° from look, should be in view"
            )

    def test_looks_at_moon_late_outbound(self, apollo_ctx, apollo_schedule):
        """Late outbound transit (past midpoint) should look at Moon."""
        for fi in range(apollo_ctx.n_frames):
            # Past midpoint: Moon closer than Earth, far from both
            if apollo_ctx.r_moon[fi] < 25.0 or apollo_ctx.r_earth[fi] < 35.0:
                continue
            if apollo_ctx.r_moon[fi] > apollo_ctx.r_earth[fi]:
                continue  # before midpoint
            # Skip near-midpoint frames where the camera is rotating between bodies
            if abs(apollo_ctx.r_moon[fi] - apollo_ctx.r_earth[fi]) < 20.0:
                continue
            moon_dir = apollo_ctx.moon[fi] - apollo_ctx.sc[fi]
            moon_dir /= np.linalg.norm(moon_dir)
            look = apollo_schedule.focal_point[fi] - apollo_schedule.position[fi]
            look /= np.linalg.norm(look)
            angle = np.degrees(np.arccos(np.clip(np.dot(look, moon_dir), -1, 1)))
            fov_half = apollo_schedule.fov[fi] / 2.0
            assert angle < fov_half + 15.0, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"Moon at {angle:.1f}° from look, should be in view"
            )

    def test_nearer_body_in_view(self, apollo_ctx, apollo_schedule):
        """During pure transit, the nearer body should be roughly in view."""
        for fi in range(apollo_ctx.n_frames):
            # Only check pure transit: both bodies far, well past any transition
            if apollo_ctx.r_moon[fi] < 40.0 or apollo_ctx.r_earth[fi] < 50.0:
                continue
            # Skip near-midpoint frames where the camera is rotating between bodies
            if abs(apollo_ctx.r_moon[fi] - apollo_ctx.r_earth[fi]) < 20.0:
                continue
            nearer = apollo_ctx.moon[fi] if apollo_ctx.r_moon[fi] < apollo_ctx.r_earth[fi] else apollo_ctx.earth[fi]
            body_dir = nearer - apollo_ctx.sc[fi]
            body_dir /= np.linalg.norm(body_dir)
            look = apollo_schedule.focal_point[fi] - apollo_schedule.position[fi]
            look /= np.linalg.norm(look)
            angle = np.degrees(np.arccos(np.clip(np.dot(look, body_dir), -1, 1)))
            fov_half = apollo_schedule.fov[fi] / 2.0
            assert angle < fov_half + 15.0, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"nearer body at {angle:.1f}° from look, FOV/2={fov_half:.0f}°"
            )


class TestGeometry:
    def test_camera_outside_bodies(self, apollo_ctx):
        """Camera must never be inside the Moon or Earth sphere.

        Linear interpolation of XYZ positions along a curved orbit "cuts
        corners", shortening the distance to the central body. The trajectory
        context builder must correct this.
        """
        for fi in range(apollo_ctx.n_frames):
            moon_dist = np.linalg.norm(apollo_ctx.sc[fi] - apollo_ctx.moon[fi])
            moon_margin = (moon_dist - apollo_ctx.mr) * 1000.0
            assert moon_margin > -1.0, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"camera is {-moon_margin:.1f} km inside Moon"
            )
            earth_dist = np.linalg.norm(apollo_ctx.sc[fi] - apollo_ctx.earth[fi])
            earth_margin = (earth_dist - apollo_ctx.er) * 1000.0
            assert earth_margin > -1.0, (
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"camera is {-earth_margin:.1f} km inside Earth"
            )

    def test_camera_distance_matches_nearest_body(self, apollo_ctx):
        """SC distance to nearest body must match interpolated distance."""
        for fi in range(apollo_ctx.n_frames):
            if apollo_ctx.r_moon[fi] < apollo_ctx.r_earth[fi]:
                dist = np.linalg.norm(apollo_ctx.sc[fi] - apollo_ctx.moon[fi])
                expected = apollo_ctx.r_moon[fi]
                body = "Moon"
            else:
                dist = np.linalg.norm(apollo_ctx.sc[fi] - apollo_ctx.earth[fi])
                expected = apollo_ctx.r_earth[fi]
                body = "Earth"
            np.testing.assert_allclose(dist, expected, rtol=1e-6, err_msg=(
                f"Frame {fi} (GET {apollo_ctx.t_hours[fi]:.1f}h): "
                f"SC-{body} distance {dist:.6f} != expected {expected:.6f}"
            ))


class TestConsistency:
    def test_single_frame_matches_schedule(self, apollo_ctx, apollo_schedule):
        """Camera state from full schedule should match what single-frame would get."""
        # Just verify the schedule is self-consistent
        for fi in [0, len(apollo_schedule) // 4, len(apollo_schedule) // 2,
                    3 * len(apollo_schedule) // 4, len(apollo_schedule) - 1]:
            cam = apollo_schedule[fi]
            # Position should match trajectory
            np.testing.assert_allclose(cam.position, apollo_ctx.sc[fi], atol=1e-10)
            # FOV should be positive
            assert cam.fov > 0
            # Ambient should be in [0, 1]
            assert 0 <= cam.ambient <= 1
