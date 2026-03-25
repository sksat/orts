# /// script
# requires-python = ">=3.10"
# dependencies = [
#     "pyvista>=0.43",
#     "numpy>=1.24",
#     "pillow>=10.0",
#     "imageio[ffmpeg]>=2.31",
# ]
# ///
"""Apollo 11 — 3D visualization with PyVista (GPU-rendered).

Usage: uv run orts/examples/apollo11/plot_3d_pyvista.py

Outputs (in orts/examples/apollo11/):
  apollo11_3d_overview.png
  apollo11_3d_earth.png
  apollo11_3d_moon.png
  apollo11_3d_approach.png
  apollo11_spacecraft.mp4    — animated spacecraft POV
  apollo11_overview.mp4      — animated overview
  apollo11_combined.mp4      — side-by-side overview + spacecraft
"""
import sys
import time
from datetime import timedelta
from pathlib import Path

import numpy as np
import pyvista as pv
import vtk

sys.path.insert(0, str(Path(__file__).parent))
from plot_common import (EPOCH_UTC, OUTPUT_DIR, R_EARTH, R_MOON,
                         compute_derived, get_textures, load_data,
                         parse_quality_arg, sun_direction_eci)
from camera_policy import (CameraSchedule, build_camera_schedule,
                           build_trajectory_context)

pv.global_theme.background = "black"
pv.global_theme.font.color = "white"

# Earth sidereal rotation rate [deg/hour]
EARTH_ROT_RATE = 360.0 / 23.9345
# GMST at epoch (1969-07-16T13:43:49 UTC) — needed to align texture with ECI frame.
# Without this, the prime meridian is ~325° off from the vernal equinox direction (+X).
GMST_AT_EPOCH = 324.61  # degrees


def make_textured_sphere(center, radius, texture_path, n_lat=100, n_lon=200):
    """Create a UV-mapped sphere without texture seam artifacts.

    Builds the mesh from a parametric lat/lon grid with n_lon+1 columns
    so the seam (lon=0 and lon=2π) has separate vertices with u=0 and u=1.
    This prevents triangles from spanning the UV discontinuity.
    """
    cx, cy, cz = center

    lats = np.linspace(-np.pi / 2, np.pi / 2, n_lat)
    # Start at -π so u=0.5 (texture center) maps to +X direction,
    # matching the convention used by the RotateZ tidal-lock and GMST rotations.
    lons = np.linspace(-np.pi, np.pi, n_lon + 1)  # +1 for seam
    lon_g, lat_g = np.meshgrid(lons, lats)

    x = cx + radius * np.cos(lat_g) * np.cos(lon_g)
    y = cy + radius * np.cos(lat_g) * np.sin(lon_g)
    z = cz + radius * np.sin(lat_g)
    points = np.column_stack([x.ravel(), y.ravel(), z.ravel()])

    # UV: u along longitude [0, 1], v along latitude [0, 1]
    u_arr = np.linspace(0, 1, n_lon + 1)
    v_arr = np.linspace(0, 1, n_lat)
    u_g, v_g = np.meshgrid(u_arr, v_arr)
    uvs = np.column_stack([u_g.ravel(), v_g.ravel()])

    # Build triangle faces (vectorized, two triangles per quad)
    cols = n_lon + 1
    ii, jj = np.meshgrid(np.arange(n_lat - 1), np.arange(n_lon), indexing="ij")
    ii, jj = ii.ravel(), jj.ravel()
    p00 = ii * cols + jj
    p01 = ii * cols + jj + 1
    p10 = (ii + 1) * cols + jj
    p11 = (ii + 1) * cols + jj + 1
    n_quads = len(p00)
    faces = np.empty((n_quads * 2, 4), dtype=np.intp)
    faces[0::2] = np.column_stack([np.full(n_quads, 3), p00, p10, p11])
    faces[1::2] = np.column_stack([np.full(n_quads, 3), p00, p11, p01])

    mesh = pv.PolyData(points, faces.ravel())
    mesh.active_texture_coordinates = uvs
    mesh.compute_normals(inplace=True)

    tex = pv.read_texture(texture_path)
    tex.SetInterpolate(True)
    return mesh, tex


def setup_sun_light(pl, t_hours, ambient_intensity=0.5):
    """Replace default lights with a directional sun light + ambient.

    Ambient is applied via VTK's ambient lighting coefficient on actors,
    not a headlight (which only illuminates surfaces facing the camera).

    Returns (sun_light, ambient_value) so they can be updated per frame.
    """
    pl.remove_all_lights()
    sun_dir = sun_direction_eci(t_hours)
    light_pos = tuple(sun_dir * 500.0)
    light = pv.Light(position=light_pos, focal_point=(0, 0, 0),
                     intensity=0.6, light_type="scene light")
    light.positional = False
    pl.add_light(light)
    return light, ambient_intensity


def _set_actor_ambient(actor, value):
    """Set ambient lighting coefficient on an actor's property."""
    actor.GetProperty().SetAmbient(value)
    actor.GetProperty().SetDiffuse(0.6)   # reduce diffuse to prevent wash-out
    actor.GetProperty().SetSpecular(0.0)   # no specular highlights


def _progress(fi, n_frames, t_start, label=""):
    """Print progress with ETA, overwriting the same line."""
    elapsed = time.time() - t_start
    if fi == 0:
        eta_str = "..."
    else:
        eta = elapsed / fi * (n_frames - fi)
        eta_str = f"{eta:.0f}s"
    pct = fi * 100 // n_frames
    bar = "█" * (pct // 4) + "░" * (25 - pct // 4)
    print(f"\r  {label} {bar} {pct:3d}% ({fi}/{n_frames}) ETA {eta_str}   ", end="", flush=True)


def _interp_arrays(idx_float, *arrays):
    """Linearly interpolate multiple arrays at fractional indices."""
    x = np.arange(len(arrays[0]))
    return [np.interp(idx_float, x, a) for a in arrays]



def render_static_views(d, earth_tex_path, moon_tex_path):
    """Render 4 static views as PNG."""
    print("Rendering static 3D views...")
    s = 1000.0
    sx, sy, sz = d["sx"] / s, d["sy"] / s, d["sz"] / s
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    mc_x, mc_y, mc_z = d["mc_x"] / s, d["mc_y"] / s, d["mc_z"] / s
    er, mr = R_EARTH / s, R_MOON / s
    mi = np.argmin(d["r_moon"])
    moon_c = (mx[mi], my[mi], mz[mi])

    trajectory = np.column_stack([sx, sy, sz])
    mc_trajectory = np.column_stack([mc_x, mc_y, mc_z])

    views = [
        {
            "name": "overview",
            "title": "Apollo 11 — Overview",
            "traj": trajectory,
            "earth": {"center": (0, 0, 0), "r": er * 2},
            "moon": {"center": moon_c, "r": mr * 2},
            "camera": {
                "position": (moon_c[0]/2, moon_c[1]/2 - 500, moon_c[2]/2 + 300),
                "focal_point": (moon_c[0]/2, moon_c[1]/2, moon_c[2]/2),
            },
        },
        {
            "name": "earth",
            "title": "Apollo 11 — Earth (true scale)",
            "traj": trajectory,
            "earth": {"center": (0, 0, 0), "r": er},
            "moon": None,
            "camera": {"position": (0, -50, 25), "focal_point": (0, 0, 0)},
        },
        {
            "name": "moon",
            "title": "Apollo 11 — Moon-centered (true scale)",
            "traj": mc_trajectory,
            "earth": None,
            "moon": {"center": (0, 0, 0), "r": mr},
            "camera": {"position": (0, -15, 8), "focal_point": (0, 0, 0)},
        },
        {
            "name": "approach",
            "title": "Apollo 11 — Moon approach",
            "traj": trajectory[max(0, mi - 80):min(len(sx), mi + 80)],
            "earth": None,
            "moon": {"center": moon_c, "r": mr},
            "camera": {
                "position": (
                    (sx[mi] + moon_c[0]) / 2 + 5,
                    (sy[mi] + moon_c[1]) / 2 - 5,
                    (sz[mi] + moon_c[2]) / 2 + 3,
                ),
                "focal_point": moon_c,
            },
        },
    ]

    for view in views:
        pl = pv.Plotter(off_screen=True, window_size=[1600, 1200])
        pl.set_background("black")
        # Mid-mission sun direction for static views
        setup_sun_light(pl, d["t_h"][mi], ambient_intensity=0.6)

        # Trajectory
        traj_line = pv.Spline(view["traj"], n_points=len(view["traj"]))
        pl.add_mesh(traj_line, color="#66aaff", line_width=2)

        # Earth
        if view["earth"]:
            e = view["earth"]
            sphere, tex = make_textured_sphere(e["center"], e["r"], earth_tex_path)
            pl.add_mesh(sphere, texture=tex, smooth_shading=True)

        # Moon
        if view["moon"]:
            m = view["moon"]
            sphere, tex = make_textured_sphere(m["center"], m["r"], moon_tex_path)
            pl.add_mesh(sphere, texture=tex, smooth_shading=True)

        # Start/end markers
        if view["name"] == "overview":
            pl.add_points(np.array([[sx[0], sy[0], sz[0]]]),
                          color="green", point_size=10, render_points_as_spheres=True)
            pl.add_points(np.array([[sx[-1], sy[-1], sz[-1]]]),
                          color="red", point_size=10, render_points_as_spheres=True)

        pl.add_text(view["title"], font_size=14, color="white")
        cam = view["camera"]
        pl.camera.position = cam["position"]
        pl.camera.focal_point = cam["focal_point"]

        out = OUTPUT_DIR / f"apollo11_3d_{view['name']}.png"
        pl.screenshot(str(out))
        pl.close()
        print(f"  Saved {out.name}")


def render_overview_animation(d, earth_tex_path, moon_tex_path, idx_float, fps=30):
    """Animated overview MP4 — Earth added once, Moon transformed."""
    print("Rendering overview animation...")
    s = 1000.0
    sx, sy, sz = d["sx"] / s, d["sy"] / s, d["sz"] / s
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    er, mr = R_EARTH / s, R_MOON / s
    N = len(sx)
    mi = np.argmin(d["r_moon"])
    moon_c = (mx[mi], my[mi], mz[mi])

    n_frames = len(idx_float)
    # Interpolate all positions for smooth motion
    sxi, syi, szi = _interp_arrays(idx_float, sx, sy, sz)
    mxi, myi, mzi = _interp_arrays(idx_float, mx, my, mz)
    t_hi = np.interp(idx_float, np.arange(N), d["t_h"])

    e_mesh, e_tex = make_textured_sphere((0, 0, 0), er * 2, earth_tex_path, 40, 100)
    m_mesh, m_tex = make_textured_sphere((0, 0, 0), mr * 2, moon_tex_path, 30, 60)

    mid = (moon_c[0] / 2, moon_c[1] / 2, moon_c[2] / 2)

    out = OUTPUT_DIR / "apollo11_overview.mp4"
    pl = pv.Plotter(off_screen=True, window_size=[1280, 896])
    pl.set_background("black")
    pl.open_movie(str(out), framerate=int(round(fps)), quality=8)

    # Static: Earth
    earth_actor = pl.add_mesh(e_mesh, texture=e_tex, smooth_shading=True)
    # Moon: transform each frame
    moon_actor = pl.add_mesh(m_mesh, texture=m_tex, smooth_shading=True)
    sun_light, _ambient = setup_sun_light(pl, t_hi[0], ambient_intensity=0.7)
    # Trajectory line actor (updated each frame)
    traj_actor = None
    dot_actor = None
    t_start = time.time()

    # Pre-create text mapper for fast updates
    ov_title_mapper = vtk.vtkTextMapper()
    ov_title_mapper.GetTextProperty().SetFontSize(24)
    ov_title_mapper.GetTextProperty().SetColor(1, 1, 1)
    ov_title_a2d = vtk.vtkActor2D()
    ov_title_a2d.SetMapper(ov_title_mapper)
    ov_title_a2d.GetPositionCoordinate().SetCoordinateSystemToNormalizedDisplay()
    ov_title_a2d.GetPositionCoordinate().SetValue(0.01, 0.95)
    pl.renderer.AddActor(ov_title_a2d)

    for fi in range(n_frames):
        if fi % 20 == 0:
            _progress(fi, n_frames, t_start, "overview")

        # Update sun light direction
        if fi % 50 == 0:
            sun_dir = sun_direction_eci(t_hi[fi])
            sun_light.position = tuple(sun_dir * 500.0)

        # Earth rotation
        et = vtk.vtkTransform()
        et.RotateZ(GMST_AT_EPOCH + EARTH_ROT_RATE * t_hi[fi])
        earth_actor.SetUserTransform(et)

        # Moon position + tidal lock rotation
        mt = vtk.vtkTransform()
        mt.Translate(mxi[fi], myi[fi], mzi[fi])
        # Tidal lock: orient Moon so 0° lon faces Earth
        angle = np.degrees(np.arctan2(-myi[fi], -mxi[fi]))
        mt.RotateZ(angle)
        moon_actor.SetUserTransform(mt)

        # Growing trajectory line (use PolyData line, not Spline — Spline is O(n²))
        if traj_actor is not None:
            pl.remove_actor(traj_actor)
        if dot_actor is not None:
            pl.remove_actor(dot_actor)
        end = fi + 1
        if end >= 2:
            step = max(1, end // 500)
            pts = np.column_stack([sxi[:end:step], syi[:end:step], szi[:end:step]])
            line = pv.PolyData(pts)
            line.lines = np.hstack([len(pts), np.arange(len(pts))])
            traj_actor = pl.add_mesh(line, color="#66aaff", line_width=2)
        dot_actor = pl.add_points(
            np.array([[sxi[fi], syi[fi], szi[fi]]]),
            color="yellow", point_size=8, render_points_as_spheres=True,
        )

        utc = EPOCH_UTC + timedelta(seconds=float(t_hi[fi] * 3600))
        ov_title_mapper.SetInput(
            f"Apollo 11 — GET {t_hi[fi]:.1f}h  {utc.strftime('%Y-%m-%d %H:%M UTC')}"
        )

        pl.camera.position = (mid[0], mid[1] - 700, mid[2] + 400)
        pl.camera.focal_point = mid
        pl.write_frame()

    _progress(n_frames, n_frames, t_start, "overview")
    elapsed = time.time() - t_start
    print()
    pl.close()
    print(f"  Saved {out.name} ({elapsed:.0f}s)")


def render_spacecraft_view(d, earth_tex_path, moon_tex_path, idx_float, fps=30):
    """Animated spacecraft POV — camera driven by CameraSchedule."""
    print("Rendering spacecraft view animation...")
    s = 1000.0
    er, mr = R_EARTH / s, R_MOON / s

    # Build camera schedule (all camera logic happens here, not in the render loop)
    ctx = build_trajectory_context(d, idx_float)
    schedule = build_camera_schedule(ctx)
    n_frames = len(schedule)

    # Interpolated positions for mesh transforms
    N = len(d["sx"])
    sx, sy, sz = d["sx"] / s, d["sy"] / s, d["sz"] / s
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    mxi, myi, mzi = _interp_arrays(idx_float, mx, my, mz)

    e_mesh, e_tex = make_textured_sphere((0, 0, 0), er, earth_tex_path, 100, 200)
    m_mesh, m_tex = make_textured_sphere((0, 0, 0), mr, moon_tex_path, 120, 240)

    # --- Render loop ---
    out = OUTPUT_DIR / "apollo11_spacecraft.mp4"
    pl = pv.Plotter(off_screen=True, window_size=[1280, 896])
    pl.set_background("black")
    pl.open_movie(str(out), framerate=int(round(fps)), quality=8)

    earth_actor = pl.add_mesh(e_mesh, texture=e_tex, smooth_shading=True)
    moon_actor = pl.add_mesh(m_mesh, texture=m_tex, smooth_shading=True)
    sun_light, _ambient_val = setup_sun_light(pl, ctx.t_hours[0], ambient_intensity=0.3)
    t_start = time.time()

    # VTK text mappers for fast per-frame updates
    title_mapper = vtk.vtkTextMapper()
    title_mapper.GetTextProperty().SetFontSize(24)
    title_mapper.GetTextProperty().SetColor(1, 1, 1)
    title_a2d = vtk.vtkActor2D()
    title_a2d.SetMapper(title_mapper)
    title_a2d.GetPositionCoordinate().SetCoordinateSystemToNormalizedDisplay()
    title_a2d.GetPositionCoordinate().SetValue(0.01, 0.95)
    pl.renderer.AddActor(title_a2d)

    info_mapper = vtk.vtkTextMapper()
    info_mapper.GetTextProperty().SetFontSize(22)
    info_mapper.GetTextProperty().SetColor(0.7, 0.7, 0.7)
    info_a2d = vtk.vtkActor2D()
    info_a2d.SetMapper(info_mapper)
    info_a2d.GetPositionCoordinate().SetCoordinateSystemToNormalizedDisplay()
    info_a2d.GetPositionCoordinate().SetValue(0.01, 0.02)
    pl.renderer.AddActor(info_a2d)

    for fi in range(n_frames):
        if fi % 20 == 0:
            _progress(fi, n_frames, t_start, "spacecraft")

        cam = schedule[fi]
        utc = EPOCH_UTC + timedelta(seconds=float(ctx.t_hours[fi] * 3600))

        # Sun light (update slowly)
        _set_actor_ambient(earth_actor, cam.ambient)
        _set_actor_ambient(moon_actor, cam.ambient)
        if fi % 50 == 0:
            sun_dir = sun_direction_eci(ctx.t_hours[fi])
            sun_light.position = tuple(sun_dir * 500.0)

        # Earth rotation
        et = vtk.vtkTransform()
        et.RotateZ(GMST_AT_EPOCH + EARTH_ROT_RATE * ctx.t_hours[fi])
        earth_actor.SetUserTransform(et)

        # Moon: translate + tidal lock
        mt = vtk.vtkTransform()
        mt.Translate(mxi[fi], myi[fi], mzi[fi])
        angle = np.degrees(np.arctan2(-myi[fi], -mxi[fi]))
        mt.RotateZ(angle)
        moon_actor.SetUserTransform(mt)

        # Camera (entirely from schedule — no logic here)
        pl.camera.position = tuple(cam.position)
        pl.camera.focal_point = tuple(cam.focal_point)
        pl.camera.up = tuple(cam.up)
        pl.camera.clipping_range = (0.001, 10000.0)
        pl.camera.view_angle = cam.fov

        # Text
        title_mapper.SetInput(
            f"View from Apollo 11 — GET {ctx.t_hours[fi]:.1f}h  "
            f"{utc.strftime('%Y-%m-%d %H:%M UTC')}"
        )
        info_mapper.SetInput(
            f"Earth: {ctx.r_earth_km[fi]:.0f} km  |  Moon: {ctx.r_moon_km[fi]:.0f} km"
        )

        pl.write_frame()

    _progress(n_frames, n_frames, t_start, "spacecraft")
    elapsed = time.time() - t_start
    print()
    pl.close()
    print(f"  Saved {out.name} ({elapsed:.0f}s)")


def combine_videos():
    """Merge overview and spacecraft MP4s side by side using ffmpeg."""
    import subprocess

    print("Combining overview + spacecraft MP4s...")
    ov_path = OUTPUT_DIR / "apollo11_overview.mp4"
    sc_path = OUTPUT_DIR / "apollo11_spacecraft.mp4"
    if not ov_path.exists() or not sc_path.exists():
        print("  Skipping: source MP4s not found")
        return

    out = OUTPUT_DIR / "apollo11_combined.mp4"
    subprocess.run([
        "ffmpeg", "-y",
        "-i", str(ov_path), "-i", str(sc_path),
        "-filter_complex", "hstack=inputs=2",
        "-c:v", "libx264", "-crf", "20", "-preset", "medium",
        str(out),
    ], check=True, capture_output=True)
    print(f"  Saved {out.name}")


def compute_shared_time_mapping(d, draft=False):
    """Compute a shared frame→data index mapping for synchronized animations.

    Returns idx_float array (length n_frames) mapping each video frame to a
    fractional data index.  The orbit phase gets more frames for detail.
    """
    s = 1000.0
    r_moon_s = d["r_moon"] / s
    N = len(r_moon_s)

    in_orbit = r_moon_s < 10.0
    orbit_start = int(np.argmax(in_orbit)) if in_orbit.any() else N
    orbit_end = N - int(np.argmax(in_orbit[::-1])) if in_orbit.any() else N

    # Total frames depend on quality; framerate is adjusted so duration = 3 min.
    # Orbit phase gets 60% of frames for detail.
    if draft:
        n_out, n_orb, n_ret = 250, 500, 250  # 1000 frames, fps = 1000/180 ≈ 5.6
    else:
        n_out, n_orb, n_ret = 1080, 3240, 1080  # 5400 frames, fps = 30
    idx_float = np.concatenate([
        np.linspace(0, max(orbit_start - 1, 0), n_out),
        np.linspace(orbit_start, min(orbit_end - 1, N - 1), n_orb),
        np.linspace(min(orbit_end, N - 1), N - 1, n_ret),
    ])
    # fps must be integer for video codec. Adjust frame counts so
    # total / fps = exactly 180 seconds, preserving the phase ratio.
    n_total = n_out + n_orb + n_ret
    fps = max(1, int(n_total / 180.0))
    n_exact = fps * 180
    if n_exact < n_total:
        # Scale down each phase proportionally
        scale = n_exact / n_total
        n_out = int(n_out * scale)
        n_orb = int(n_orb * scale)
        n_ret = n_exact - n_out - n_orb  # remainder to return phase
        idx_float = np.concatenate([
            np.linspace(0, max(orbit_start - 1, 0), n_out),
            np.linspace(orbit_start, min(orbit_end - 1, N - 1), n_orb),
            np.linspace(min(orbit_end, N - 1), N - 1, n_ret),
        ])
    return idx_float, fps


def render_single_frame(d, earth_tex_path, moon_tex_path, idx_float, get_h):
    """Render a single spacecraft-view frame for debugging. Uses same CameraSchedule as animation."""
    print(f"Rendering single frame at GET {get_h:.1f}h...")
    s = 1000.0
    er, mr = R_EARTH / s, R_MOON / s

    ctx = build_trajectory_context(d, idx_float)
    schedule = build_camera_schedule(ctx)

    fi = int(np.argmin(np.abs(ctx.t_hours - get_h)))
    cam = schedule[fi]
    print(f"  Frame {fi}/{len(schedule)}, actual GET {ctx.t_hours[fi]:.1f}h")

    N = len(d["sx"])
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    mxi, myi, mzi = _interp_arrays(idx_float, mx, my, mz)

    e_mesh, e_tex = make_textured_sphere((0, 0, 0), er, earth_tex_path, 100, 200)
    m_mesh, m_tex = make_textured_sphere((0, 0, 0), mr, moon_tex_path, 120, 240)

    pl = pv.Plotter(off_screen=True, window_size=[1280, 896])
    pl.set_background("black")
    setup_sun_light(pl, ctx.t_hours[fi], ambient_intensity=0.3)

    ea = pl.add_mesh(e_mesh, texture=e_tex, smooth_shading=True)
    _set_actor_ambient(ea, cam.ambient)
    et = vtk.vtkTransform()
    et.RotateZ(GMST_AT_EPOCH + EARTH_ROT_RATE * ctx.t_hours[fi])
    ea.SetUserTransform(et)

    ma = pl.add_mesh(m_mesh, texture=m_tex, smooth_shading=True)
    _set_actor_ambient(ma, cam.ambient)
    mt = vtk.vtkTransform()
    mt.Translate(mxi[fi], myi[fi], mzi[fi])
    mt.RotateZ(np.degrees(np.arctan2(-myi[fi], -mxi[fi])))
    ma.SetUserTransform(mt)

    pl.camera.position = tuple(cam.position)
    pl.camera.focal_point = tuple(cam.focal_point)
    pl.camera.up = tuple(cam.up)
    pl.camera.clipping_range = (0.001, 10000.0)
    pl.camera.view_angle = cam.fov

    out = OUTPUT_DIR / f"test_get{get_h:.0f}.png"
    pl.screenshot(str(out))
    pl.close()
    print(f"  Saved {out.name}")


if __name__ == "__main__":
    quality = parse_quality_arg()
    draft = "--draft" in sys.argv
    if draft:
        sys.argv.remove("--draft")

    # --frame GET_HOURS: render a single frame for debugging
    frame_get = None
    if "--frame" in sys.argv:
        i = sys.argv.index("--frame")
        frame_get = float(sys.argv[i + 1])
        sys.argv.pop(i + 1)
        sys.argv.pop(i)

    print(f"Quality: {quality}{' (draft)' if draft else ''}")

    sat, moon = load_data()
    d = compute_derived(sat, moon)

    earth_tex, moon_tex = get_textures(quality)
    idx_float, fps = compute_shared_time_mapping(d, draft=draft)
    print(f"Frames: {len(idx_float)}, FPS: {fps:.1f}, Duration: {len(idx_float)/fps:.0f}s")

    if frame_get is not None:
        render_single_frame(d, earth_tex, moon_tex, idx_float, frame_get)
    else:
        render_static_views(d, earth_tex, moon_tex)
        render_overview_animation(d, earth_tex, moon_tex, idx_float, fps)
        render_spacecraft_view(d, earth_tex, moon_tex, idx_float, fps)
        combine_videos()
