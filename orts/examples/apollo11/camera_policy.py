"""Camera policy system for Apollo 11 spacecraft POV visualization.

Separates camera logic from rendering. The render loop only reads from a
pre-computed CameraSchedule — no camera decisions at render time.

Usage:
    ctx = build_trajectory_context(d, idx_float)
    schedule = build_camera_schedule(ctx)
    cam = schedule[fi]  # CameraState for frame fi
"""
from dataclasses import dataclass

import numpy as np

# ---------------------------------------------------------------------------
# Data structures
# ---------------------------------------------------------------------------


@dataclass
class CameraState:
    """Complete camera state for a single frame."""
    position: np.ndarray      # (3,) camera/spacecraft ECI position
    focal_point: np.ndarray   # (3,) look-at point
    up: np.ndarray            # (3,) up vector
    fov: float                # vertical FOV [degrees]
    ambient: float            # ambient lighting coefficient [0, 1]


@dataclass
class CameraSchedule:
    """Pre-computed camera state for every frame (struct-of-arrays)."""
    position: np.ndarray      # (N, 3)
    focal_point: np.ndarray   # (N, 3)
    up: np.ndarray            # (N, 3)
    fov: np.ndarray           # (N,)
    ambient: np.ndarray       # (N,)

    def __len__(self) -> int:
        return len(self.fov)

    def __getitem__(self, i: int) -> CameraState:
        return CameraState(
            position=self.position[i],
            focal_point=self.focal_point[i],
            up=self.up[i],
            fov=float(self.fov[i]),
            ambient=float(self.ambient[i]),
        )

    def look_direction(self) -> np.ndarray:
        """(N, 3) unit vectors from position to focal_point."""
        d = self.focal_point - self.position
        norms = np.linalg.norm(d, axis=1, keepdims=True)
        norms = np.maximum(norms, 1e-12)
        return d / norms

    def angular_rate(self) -> np.ndarray:
        """(N-1,) angle change in degrees between consecutive look directions."""
        dirs = self.look_direction()
        dots = np.sum(dirs[:-1] * dirs[1:], axis=1).clip(-1, 1)
        return np.degrees(np.arccos(dots))


@dataclass
class TrajectoryContext:
    """Per-frame geometric quantities, pre-computed from trajectory data."""
    n_frames: int
    sc: np.ndarray          # (N, 3) spacecraft ECI [×1000 km]
    moon: np.ndarray        # (N, 3) Moon ECI [×1000 km]
    earth: np.ndarray       # (N, 3) Earth ECI (zeros)
    vel_dir: np.ndarray     # (N, 3) unit velocity direction
    r_moon: np.ndarray      # (N,) distance to Moon center [×1000 km]
    r_earth: np.ndarray     # (N,) distance to Earth center [×1000 km]
    r_moon_km: np.ndarray   # (N,) distance to Moon [km]
    r_earth_km: np.ndarray  # (N,) distance to Earth [km]
    t_hours: np.ndarray     # (N,) GET in hours
    mr: float               # Moon radius [×1000 km]
    er: float               # Earth radius [×1000 km]


# ---------------------------------------------------------------------------
# Sigmoid utility
# ---------------------------------------------------------------------------

# Fraction of horizon depression angle used for pitch toward nadir.
# 0.5 = horizon at frame center; higher = more surface visible.
LUNAR_PITCH_FRACTION = 0.65
# Earth orbit: lower pitch + narrower FOV because LEO altitude/radius
# ratio (~3%) is much smaller than lunar orbit (~6%), so Earth fills
# more of the frame at the same settings.
EARTH_PITCH_FRACTION = 0.35
EARTH_ORBIT_FOV = 40.0

def _sigmoid(x, center, width):
    """Smooth step: ~0 when x << center, ~1 when x >> center."""
    return 1.0 / (1.0 + np.exp(-(x - center) / width))


def _normalize_rows(v):
    """Normalize each row of (N, 3) array to unit length."""
    norms = np.linalg.norm(v, axis=1, keepdims=True)
    norms = np.maximum(norms, 1e-12)
    return v / norms


# ---------------------------------------------------------------------------
# TrajectoryContext builder
# ---------------------------------------------------------------------------

def _interp_arrays(idx_float, *arrays):
    x = np.arange(len(arrays[0]))
    return [np.interp(idx_float, x, a) for a in arrays]


def build_trajectory_context(d: dict, idx_float: np.ndarray) -> TrajectoryContext:
    """Build TrajectoryContext from trajectory data dict and frame indices."""
    from plot_common import R_EARTH, R_MOON
    s = 1000.0
    N = len(d["sx"])
    n_frames = len(idx_float)

    sx, sy, sz = d["sx"] / s, d["sy"] / s, d["sz"] / s
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    r_moon_s = d["r_moon"] / s
    r_earth_s = d["r_earth"] / s
    mr, er = R_MOON / s, R_EARTH / s

    sxi, syi, szi = _interp_arrays(idx_float, sx, sy, sz)
    mxi, myi, mzi = _interp_arrays(idx_float, mx, my, mz)
    r_moon_i, r_earth_i = _interp_arrays(idx_float, r_moon_s, r_earth_s)
    t_hi = np.interp(idx_float, np.arange(N), d["t_h"])
    r_moon_km_i = np.interp(idx_float, np.arange(N), d["r_moon"])
    r_earth_km_i = np.interp(idx_float, np.arange(N), d["r_earth"])

    sc = np.column_stack([sxi, syi, szi])
    moon = np.column_stack([mxi, myi, mzi])
    earth = np.zeros_like(sc)

    # Fix interpolation-induced position error.
    # Linear interpolation of XYZ positions along a curved orbit "cuts
    # corners" through the orbit interior, making the interpolated distance
    # to the central body shorter than the true distance (up to ~130 km in
    # LEO, ~60 km at lunar periapsis). This causes the camera to penetrate
    # the body sphere in the scene.
    # Correct by scaling the SC position relative to the NEAREST body to
    # match the independently interpolated distance.
    r_moon_arr = np.array(r_moon_i)
    r_earth_arr = np.array(r_earth_i)
    near_moon = r_moon_arr < r_earth_arr
    body_pos = np.where(near_moon[:, None], moon, earth)
    target_dist = np.where(near_moon, r_moon_arr, r_earth_arr)
    sc2body = body_pos - sc
    interp_dist = np.linalg.norm(sc2body, axis=1, keepdims=True)
    interp_dist = np.maximum(interp_dist, 1e-12)
    sc = body_pos - (sc2body / interp_dist) * target_dist[:, None]

    # Velocity: Moon-relative near Moon, ECI elsewhere
    rel = sc - moon
    t_sec = t_hi * 3600.0
    vel_dir = np.zeros_like(sc)
    for fi in range(n_frames):
        near_moon = r_moon_i[fi] < 10.0
        di = max(1, int(0.5 + (8.0 if near_moon else 3.0) * n_frames / N))
        fi_prev = max(0, fi - di)
        fi_next = min(n_frames - 1, fi + di)
        dt = t_sec[fi_next] - t_sec[fi_prev]
        if dt > 0:
            if near_moon:
                v = (rel[fi_next] - rel[fi_prev]) / dt
            else:
                v = (sc[fi_next] - sc[fi_prev]) / dt
        else:
            v = np.array([1.0, 0.0, 0.0])
        vn = np.linalg.norm(v)
        vel_dir[fi] = v / vn if vn > 1e-12 else np.array([1.0, 0.0, 0.0])

    return TrajectoryContext(
        n_frames=n_frames,
        sc=sc, moon=moon, earth=earth,
        vel_dir=vel_dir,
        r_moon=np.array(r_moon_i), r_earth=np.array(r_earth_i),
        r_moon_km=np.array(r_moon_km_i), r_earth_km=np.array(r_earth_km_i),
        t_hours=t_hi,
        mr=mr, er=er,
    )


# ---------------------------------------------------------------------------
# Policy functions — each returns (focal, up, fov, ambient) arrays for ALL frames
# ---------------------------------------------------------------------------

def _earth_orbit_policy(ctx: TrajectoryContext):
    """Earth horizon view — same approach as lunar orbit but for Earth.

    Orbital tangent + pitch toward nadir so Earth's horizon is centered,
    with radial outward as up (Earth surface always "below").
    """
    nadir = ctx.earth - ctx.sc
    nadir = _normalize_rows(nadir)

    # Orbital tangent: velocity projected perpendicular to nadir
    dot_vn = np.sum(ctx.vel_dir * nadir, axis=1, keepdims=True)
    vel_tangent = ctx.vel_dir - dot_vn * nadir
    vt_norm = np.linalg.norm(vel_tangent, axis=1, keepdims=True)

    # Fallback for degenerate frames (velocity parallel to nadir)
    r_rel = ctx.sc - ctx.earth
    L = np.cross(r_rel, ctx.vel_dir)
    fallback = np.cross(L, nadir)
    fb_norm = np.linalg.norm(fallback, axis=1, keepdims=True)
    fb_norm = np.maximum(fb_norm, 1e-12)
    fallback = fallback / fb_norm

    degenerate = (vt_norm < 0.3).astype(float)
    vt_norm = np.maximum(vt_norm, 1e-12)
    vel_tangent = vel_tangent / vt_norm * (1.0 - degenerate) + fallback * degenerate
    vel_tangent = _normalize_rows(vel_tangent)

    # Pitch toward Earth nadir
    safe_ratio = np.clip(ctx.er / np.maximum(ctx.r_earth, ctx.er + 0.001), 0, 1)
    horizon_below_deg = 90.0 - np.degrees(np.arcsin(safe_ratio))
    pitch_deg = horizon_below_deg * EARTH_PITCH_FRACTION
    pitch_rad = np.radians(pitch_deg)

    look = (vel_tangent * np.cos(pitch_rad)[:, None]
            + nadir * np.sin(pitch_rad)[:, None])
    look = _normalize_rows(look)
    focal = ctx.sc + look * 2.0

    # Up = radial outward from Earth
    radial_out = -nadir
    dot_rl = np.sum(radial_out * look, axis=1, keepdims=True)
    up = radial_out - dot_rl * look
    up = _normalize_rows(up)

    fov = np.full(ctx.n_frames, EARTH_ORBIT_FOV)
    ambient = np.full(ctx.n_frames, 0.35)
    return focal, up, fov, ambient


def _transit_policy(ctx: TrajectoryContext):
    """Look directly at the nearer body (Earth or Moon).

    During transit, Earth and Moon are ~145° apart as seen from the spacecraft.
    Blending their directions with a sigmoid produces a degenerate intermediate
    direction pointing at empty space. Instead, select one body per frame and
    let the EMA in build_camera_schedule smooth the ~145° rotation at the
    midpoint over several frames.
    """
    moon_dir = ctx.moon - ctx.sc
    moon_dir = _normalize_rows(moon_dir)
    earth_dir = ctx.earth - ctx.sc
    earth_dir = _normalize_rows(earth_dir)

    # Hard body selection: Moon when closer, Earth otherwise.
    # The discontinuity at ratio=0.5 is handled by the EMA smoother.
    ratio = ctx.r_earth / np.maximum(ctx.r_moon + ctx.r_earth, 1e-6)
    moon_closer = ratio >= 0.5
    look = np.where(moon_closer[:, None], moon_dir, earth_dir)
    focal = ctx.sc + look * 2.0

    z_up = np.tile(np.array([0.0, 0.0, 1.0]), (ctx.n_frames, 1))
    dot_zl = np.sum(z_up * look, axis=1, keepdims=True)
    up = z_up - dot_zl * look
    up = _normalize_rows(up)

    fov = np.full(ctx.n_frames, 30.0)
    ambient = np.full(ctx.n_frames, 0.35)
    return focal, up, fov, ambient


def _lunar_orbit_policy(ctx: TrajectoryContext):
    """Orbital tangent + pitch toward nadir so lunar horizon is centered.

    Projects velocity onto the plane perpendicular to nadir (removing radial
    component), then tilts toward nadir by half the horizon depression angle.
    This guarantees the horizon position is deterministic regardless of the
    radial velocity component (approach, elliptical orbit, etc.).
    """
    nadir = ctx.moon - ctx.sc
    nadir = _normalize_rows(nadir)

    # Project velocity onto plane perpendicular to nadir (= orbital tangent).
    # When velocity is nearly radial (periapsis/apoapsis), the tangent component
    # becomes degenerate. Use the orbital angular momentum cross product as
    # fallback to get a stable tangent direction.
    dot_vn = np.sum(ctx.vel_dir * nadir, axis=1, keepdims=True)
    vel_tangent = ctx.vel_dir - dot_vn * nadir
    vt_norm = np.linalg.norm(vel_tangent, axis=1, keepdims=True)

    # Fallback for degenerate frames: use L × nadir (perpendicular to both
    # angular momentum and nadir = guaranteed tangent to orbit)
    r_rel = ctx.sc - ctx.moon
    L = np.cross(r_rel, ctx.vel_dir)  # orbital angular momentum
    fallback = np.cross(L, nadir)     # tangent from L × nadir
    fb_norm = np.linalg.norm(fallback, axis=1, keepdims=True)
    fb_norm = np.maximum(fb_norm, 1e-12)
    fallback = fallback / fb_norm

    # Blend: use projection when reliable, fallback when degenerate
    degenerate = (vt_norm < 0.3).astype(float)  # smooth transition
    vt_norm = np.maximum(vt_norm, 1e-12)
    vel_tangent = vel_tangent / vt_norm * (1.0 - degenerate) + fallback * degenerate
    vel_tangent = _normalize_rows(vel_tangent)

    # Pitch angle: fraction of horizon depression.
    # 0.5 = horizon at frame center, 0.7 = horizon in upper third (more Moon visible).
    safe_ratio = np.clip(ctx.mr / np.maximum(ctx.r_moon, ctx.mr + 0.001), 0, 1)
    horizon_below_deg = 90.0 - np.degrees(np.arcsin(safe_ratio))
    pitch_deg = horizon_below_deg * LUNAR_PITCH_FRACTION
    pitch_rad = np.radians(pitch_deg)

    # Tilt from horizontal (vel_tangent) toward nadir
    look = (vel_tangent * np.cos(pitch_rad)[:, None]
            + nadir * np.sin(pitch_rad)[:, None])
    look = _normalize_rows(look)
    focal = ctx.sc + look * 2.0

    # Up = radial outward (away from Moon) projected perpendicular to look.
    # This keeps the Moon surface always "below" and the horizon always
    # horizontal, regardless of where the spacecraft is in its orbit.
    radial_out = -nadir  # nadir points SC→Moon, so -nadir = away from Moon
    dot_rl = np.sum(radial_out * look, axis=1, keepdims=True)
    up = radial_out - dot_rl * look
    up = _normalize_rows(up)

    fov = np.full(ctx.n_frames, 60.0)
    ambient = np.full(ctx.n_frames, 0.55)
    return focal, up, fov, ambient


# ---------------------------------------------------------------------------
# Policy weights
# ---------------------------------------------------------------------------

def compute_policy_weights(ctx: TrajectoryContext) -> np.ndarray:
    """Compute (N, 3) soft weights: [earth_orbit, transit, lunar_orbit].

    Weights are continuous (sigmoid-based) and sum to 1.0 per frame.
    """
    # Proximity signals (1 = near, 0 = far)
    # Wide sigmoid widths for gradual transitions (no abrupt weight jumps)
    # Very wide sigmoid for earth → transit transition to avoid jump at TLI
    w_earth_near = 1.0 - _sigmoid(ctx.r_earth, center=20.0, width=8.0)
    w_moon_near = 1.0 - _sigmoid(ctx.r_moon, center=15.0, width=5.0)

    w_earth = w_earth_near
    w_lunar = w_moon_near * (1.0 - w_earth_near)
    w_transit = np.maximum(0.0, 1.0 - w_earth - w_lunar)

    # Normalize to sum=1
    total = w_earth + w_transit + w_lunar
    total = np.maximum(total, 1e-12)
    weights = np.column_stack([w_earth / total, w_transit / total, w_lunar / total])
    return weights


# ---------------------------------------------------------------------------
# Schedule builder
# ---------------------------------------------------------------------------

# Per-phase EMA alpha values
_EMA_ALPHA = {
    "earth_orbit": 0.15,
    "transit": 0.50,  # smooth the ~145° rotation at the Earth/Moon midpoint
    "lunar_orbit": 0.60,
}


def build_camera_schedule(ctx: TrajectoryContext) -> CameraSchedule:
    """Compute the complete camera schedule for all frames.

    Pipeline:
    1. Compute per-frame weights for each policy
    2. Evaluate each policy for every frame
    3. Blend outputs using weights
    4. Apply EMA smoothing
    5. Return CameraSchedule
    """
    weights = compute_policy_weights(ctx)  # (N, 3)

    # Evaluate all policies
    policies = [_earth_orbit_policy, _transit_policy, _lunar_orbit_policy]
    results = [p(ctx) for p in policies]  # list of (focal, up, fov, ambient)

    # Blend on DIRECTIONS, not focal positions (which have incompatible scales).
    # Each policy's focal is at different distances from sc, so blending
    # positions gives wrong directions. Blend unit look directions instead.
    look_dirs = []
    up_dirs = []
    for f, u, v, a in results:
        d = f - ctx.sc
        look_dirs.append(_normalize_rows(d))
        up_dirs.append(u)

    blended_look = np.zeros((ctx.n_frames, 3))
    up_raw = np.zeros((ctx.n_frames, 3))
    fov = np.zeros(ctx.n_frames)
    ambient = np.zeros(ctx.n_frames)

    for i in range(len(results)):
        w = weights[:, i]
        blended_look += w[:, None] * look_dirs[i]
        up_raw += w[:, None] * up_dirs[i]
        fov += w * results[i][2]
        ambient += w * results[i][3]

    blended_look = _normalize_rows(blended_look)
    focal = ctx.sc + blended_look * 2.0
    up = _normalize_rows(up_raw)

    # Blended EMA alpha
    alpha_names = list(_EMA_ALPHA.keys())
    alpha = np.zeros(ctx.n_frames)
    for i, name in enumerate(alpha_names):
        alpha += weights[:, i] * _EMA_ALPHA[name]

    # Boost alpha during phase transitions to prevent stale values from
    # the previous phase dragging the camera in the wrong direction.
    if ctx.n_frames > 1:
        weight_change = np.zeros(ctx.n_frames)
        weight_change[1:] = np.max(np.abs(np.diff(weights, axis=0)), axis=1)
        # When weights change fast (>0.05/frame), push alpha toward 1.0
        transition_boost = np.clip(weight_change / 0.05, 0.0, 1.0)
        alpha = alpha * (1.0 - transition_boost) + 1.0 * transition_boost

    # EMA smoothing on DIRECTION (not focal position, which has scale issues
    # when blending near focal points (2 units) with distant ones (400 units)).
    look_dir = focal - ctx.sc
    look_dir = _normalize_rows(look_dir)

    smooth_look = look_dir.copy()
    smooth_up = up.copy()
    smooth_fov = fov.copy()
    smooth_ambient = ambient.copy()

    for fi in range(1, ctx.n_frames):
        a = alpha[fi]
        d = a * look_dir[fi] + (1 - a) * smooth_look[fi - 1]
        dn = np.linalg.norm(d)
        smooth_look[fi] = d / dn if dn > 1e-10 else smooth_look[fi - 1]

        u = a * up[fi] + (1 - a) * smooth_up[fi - 1]
        # Prevent 180° flip: if blended up opposes previous, negate before normalizing
        if np.dot(u, smooth_up[fi - 1]) < 0:
            u = -u
        un = np.linalg.norm(u)
        smooth_up[fi] = u / un if un > 1e-10 else smooth_up[fi - 1]

        smooth_fov[fi] = a * fov[fi] + (1 - a) * smooth_fov[fi - 1]
        smooth_ambient[fi] = a * ambient[fi] + (1 - a) * smooth_ambient[fi - 1]

    # Reconstruct focal points from smoothed directions
    smooth_focal = ctx.sc + smooth_look * 2.0

    return CameraSchedule(
        position=ctx.sc,
        focal_point=smooth_focal,
        up=smooth_up,
        fov=smooth_fov,
        ambient=smooth_ambient,
    )
