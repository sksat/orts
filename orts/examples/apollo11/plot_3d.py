# /// script
# requires-python = ">=3.10"
# dependencies = ["plotly>=5.18", "kaleido>=0.2", "numpy>=1.24", "pillow>=10.0"]
# ///
"""Apollo 11 — 3D visualization (Plotly).

Usage: uv run orts/examples/apollo11/plot_3d.py

Outputs:
  apollo11_3d.html             — interactive 4-panel static view
  apollo11_3d.png              — static image
  apollo11_3d_animation.html   — interactive animated 4-panel
  apollo11_spacecraft_view.html — animated view from the spacecraft
"""
import sys
from datetime import timedelta
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from plot_common import (EARTH_TEX_URL, EPOCH_UTC, MOON_TEX_URL, OUTPUT_DIR,
                         R_EARTH, R_MOON, compute_derived, download_cached,
                         load_data)


def load_texture(url, name):
    from PIL import Image
    path = download_cached(url, name)
    return np.array(Image.open(path).convert("RGB")) / 255.0


def make_sphere_mesh(center, radius, texture_img, n=80):
    import plotly.graph_objects as go
    phi = np.linspace(0, np.pi, n)
    theta = np.linspace(0, 2 * np.pi, n)
    phi_g, theta_g = np.meshgrid(phi, theta)
    cx, cy, cz = center
    x = (cx + radius * np.sin(phi_g) * np.cos(theta_g)).flatten()
    y = (cy + radius * np.sin(phi_g) * np.sin(theta_g)).flatten()
    z = (cz + radius * np.cos(phi_g)).flatten()
    ii, jj, kk = [], [], []
    for i in range(n - 1):
        for j in range(n - 1):
            v00, v01 = i * n + j, i * n + j + 1
            v10, v11 = (i + 1) * n + j, (i + 1) * n + j + 1
            ii.extend([v00, v00]); jj.extend([v01, v10]); kk.extend([v10, v11])
    h, w = texture_img.shape[:2]
    u = theta_g.flatten() / (2 * np.pi)
    v = phi_g.flatten() / np.pi
    tc = np.clip((u * (w - 1)).astype(int), 0, w - 1)
    tr = np.clip((v * (h - 1)).astype(int), 0, h - 1)
    colors = texture_img[tr, tc]
    vc = [f'rgb({int(c[0]*255)},{int(c[1]*255)},{int(c[2]*255)})' for c in colors]
    return go.Mesh3d(x=x, y=y, z=z, i=ii, j=jj, k=kk, vertexcolor=vc,
                     flatshading=False, lighting=dict(ambient=0.6, diffuse=0.4),
                     hoverinfo='skip', showlegend=False)


SCENE_DARK = dict(
    bgcolor="#0a0a1a",
    xaxis=dict(showbackground=True, backgroundcolor="#0a0a20", gridcolor="#333", color="gray"),
    yaxis=dict(showbackground=True, backgroundcolor="#0a0a20", gridcolor="#333", color="gray"),
    zaxis=dict(showbackground=True, backgroundcolor="#0a0a20", gridcolor="#333", color="gray"),
    aspectmode="cube",
)


def plot_3d_static(d, earth_img, moon_img):
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots

    print("Generating 3D static...")
    s = 1000.0
    sx, sy, sz = d["sx"]/s, d["sy"]/s, d["sz"]/s
    mx, my, mz = d["mx"]/s, d["my"]/s, d["mz"]/s
    mc_x, mc_y, mc_z = d["mc_x"]/s, d["mc_y"]/s, d["mc_z"]/s
    er, mr = R_EARTH / s, R_MOON / s
    mi = np.argmin(d["r_moon"])
    moon_c = (mx[mi], my[mi], mz[mi])

    fig = make_subplots(
        rows=2, cols=2,
        specs=[[{"type": "scene"}]*2]*2,
        subplot_titles=["Overview", "Earth (true scale)", "Moon-centered (true scale)", "Moon approach"],
        vertical_spacing=0.05, horizontal_spacing=0.03,
    )
    ts = dict(mode="lines", line=dict(color="#66aaff", width=2), hoverinfo="skip", showlegend=False)

    # P1: Overview
    fig.add_trace(go.Scatter3d(x=sx, y=sy, z=sz, name="Apollo 11", **{**ts, "showlegend": True}), row=1, col=1)
    fig.add_trace(make_sphere_mesh((0,0,0), er*2, earth_img, n=50), row=1, col=1)
    fig.add_trace(make_sphere_mesh(moon_c, mr*2, moon_img, n=40), row=1, col=1)
    rng = max(abs(sx).max(), abs(sy).max(), abs(sz).max()) * 1.1

    # P2: Earth
    fig.add_trace(go.Scatter3d(x=sx, y=sy, z=sz, **ts), row=1, col=2)
    fig.add_trace(make_sphere_mesh((0,0,0), er, earth_img, n=80), row=1, col=2)

    # P3: Moon-centered
    fig.add_trace(go.Scatter3d(x=mc_x, y=mc_y, z=mc_z, **ts), row=2, col=1)
    fig.add_trace(make_sphere_mesh((0,0,0), mr, moon_img, n=80), row=2, col=1)

    # P4: Moon approach
    w = 80
    sl = slice(max(0, mi-w), min(len(sx), mi+w))
    fig.add_trace(go.Scatter3d(x=sx[sl], y=sy[sl], z=sz[sl], **ts), row=2, col=2)
    fig.add_trace(make_sphere_mesh(moon_c, mr, moon_img, n=80), row=2, col=2)
    appr = d["r_moon"][mi] / s * 1.2
    acx, acy, acz = (sx[mi]+moon_c[0])/2, (sy[mi]+moon_c[1])/2, (sz[mi]+moon_c[2])/2

    fig.update_layout(
        scene=dict(**SCENE_DARK, xaxis_range=[-rng,rng], yaxis_range=[-rng,rng], zaxis_range=[-rng,rng]),
        scene2=dict(**SCENE_DARK, xaxis_range=[-30,30], yaxis_range=[-30,30], zaxis_range=[-30,30]),
        scene3=dict(**SCENE_DARK, xaxis_range=[-10,10], yaxis_range=[-10,10], zaxis_range=[-10,10]),
        scene4=dict(**SCENE_DARK, xaxis_range=[acx-appr,acx+appr], yaxis_range=[acy-appr,acy+appr], zaxis_range=[acz-appr,acz+appr]),
        title=dict(text="Apollo 11 — 3D Trajectory Views", font=dict(color="white", size=16)),
        paper_bgcolor="#0a0a1a", legend=dict(font=dict(color="white")),
        height=1000, width=1400,
    )
    fig.write_html(OUTPUT_DIR / "apollo11_3d.html")
    fig.write_image(OUTPUT_DIR / "apollo11_3d.png", scale=2)
    print(f"Saved apollo11_3d.html + .png")


def plot_3d_animation(d, earth_img, moon_img):
    import plotly.graph_objects as go
    from plotly.subplots import make_subplots

    print("Generating 3D animation...")
    s = 1000.0
    sx, sy, sz = d["sx"]/s, d["sy"]/s, d["sz"]/s
    mx, my, mz = d["mx"]/s, d["my"]/s, d["mz"]/s
    mc_x, mc_y, mc_z = d["mc_x"]/s, d["mc_y"]/s, d["mc_z"]/s
    er, mr = R_EARTH / s, R_MOON / s
    mi = np.argmin(d["r_moon"])
    moon_c = (mx[mi], my[mi], mz[mi])

    fig = make_subplots(
        rows=2, cols=2, specs=[[{"type":"scene"}]*2]*2,
        subplot_titles=["Overview", "Earth", "Moon-centered", "Moon approach"],
        vertical_spacing=0.05, horizontal_spacing=0.03,
    )
    ts = dict(mode="lines", line=dict(color="#66aaff", width=2), hoverinfo="skip", showlegend=False)
    ms = dict(mode="markers", marker=dict(color="yellow", size=4), hoverinfo="skip", showlegend=False)

    # Static spheres
    fig.add_trace(make_sphere_mesh((0,0,0), er*2, earth_img, n=50), row=1, col=1)  # 0
    fig.add_trace(make_sphere_mesh(moon_c, mr*2, moon_img, n=40), row=1, col=1)    # 1
    fig.add_trace(make_sphere_mesh((0,0,0), er, earth_img, n=60), row=1, col=2)    # 2
    fig.add_trace(make_sphere_mesh((0,0,0), mr, moon_img, n=60), row=2, col=1)     # 3
    fig.add_trace(make_sphere_mesh(moon_c, mr, moon_img, n=60), row=2, col=2)      # 4
    ns = 5

    # Animated traces: traj+marker × 4 panels
    for row, col in [(1,1),(1,2),(2,1),(2,2)]:
        fig.add_trace(go.Scatter3d(x=[0], y=[0], z=[0], **ts), row=row, col=col)
        fig.add_trace(go.Scatter3d(x=[0], y=[0], z=[0], **ms), row=row, col=col)
    anim_ids = list(range(ns, ns + 8))

    N = len(sx)
    n_frames = 150
    idx_map = np.linspace(0, N-1, n_frames).astype(int)

    frames = []
    for i in idx_map:
        sl = slice(0, i+1)
        utc = EPOCH_UTC + timedelta(seconds=float(d["t_h"][i]*3600))
        frames.append(go.Frame(
            data=[
                go.Scatter3d(x=sx[sl], y=sy[sl], z=sz[sl]),
                go.Scatter3d(x=[sx[i]], y=[sy[i]], z=[sz[i]]),
                go.Scatter3d(x=sx[sl], y=sy[sl], z=sz[sl]),
                go.Scatter3d(x=[sx[i]], y=[sy[i]], z=[sz[i]]),
                go.Scatter3d(x=mc_x[sl], y=mc_y[sl], z=mc_z[sl]),
                go.Scatter3d(x=[mc_x[i]], y=[mc_y[i]], z=[mc_z[i]]),
                go.Scatter3d(x=sx[sl], y=sy[sl], z=sz[sl]),
                go.Scatter3d(x=[sx[i]], y=[sy[i]], z=[sz[i]]),
            ],
            traces=anim_ids,
            name=f"GET {d['t_h'][i]:.0f}h",
            layout=dict(title=dict(text=f"Apollo 11 — GET {d['t_h'][i]:.1f}h  {utc.strftime('%Y-%m-%d %H:%M UTC')}"))
        ))
    fig.frames = frames

    rng = max(abs(sx).max(), abs(sy).max(), abs(sz).max()) * 1.1
    appr = d["r_moon"][mi] / s * 1.2
    acx, acy, acz = (sx[mi]+moon_c[0])/2, (sy[mi]+moon_c[1])/2, (sz[mi]+moon_c[2])/2

    fig.update_layout(
        scene=dict(**SCENE_DARK, xaxis_range=[-rng,rng], yaxis_range=[-rng,rng], zaxis_range=[-rng,rng]),
        scene2=dict(**SCENE_DARK, xaxis_range=[-30,30], yaxis_range=[-30,30], zaxis_range=[-30,30]),
        scene3=dict(**SCENE_DARK, xaxis_range=[-10,10], yaxis_range=[-10,10], zaxis_range=[-10,10]),
        scene4=dict(**SCENE_DARK, xaxis_range=[acx-appr,acx+appr], yaxis_range=[acy-appr,acy+appr], zaxis_range=[acz-appr,acz+appr]),
        title=dict(text="Apollo 11", font=dict(color="white", size=14)),
        paper_bgcolor="#0a0a1a", height=1000, width=1400, showlegend=False,
        updatemenus=[dict(type="buttons", showactive=False, x=0.05, y=0.02, buttons=[
            dict(label="▶ Play", method="animate",
                 args=[None, dict(frame=dict(duration=100, redraw=True), fromcurrent=True)]),
            dict(label="⏸ Pause", method="animate",
                 args=[[None], dict(frame=dict(duration=0, redraw=False), mode="immediate")]),
        ])],
        sliders=[dict(active=0, x=0.05, len=0.9, y=-0.02,
                      currentvalue=dict(prefix="", font=dict(color="white")),
                      font=dict(color="gray"),
                      steps=[dict(args=[[f.name], dict(frame=dict(duration=0, redraw=True), mode="immediate")],
                                  method="animate", label=f.name) for f in frames])],
    )
    fig.write_html(OUTPUT_DIR / "apollo11_3d_animation.html")
    print("Saved apollo11_3d_animation.html")


def plot_spacecraft_view(d, earth_img, moon_img):
    """Animated view FROM the spacecraft — positions in spacecraft-centered coords."""
    import plotly.graph_objects as go

    print("Generating spacecraft view...")
    s = 1000.0
    sx, sy, sz = d["sx"] / s, d["sy"] / s, d["sz"] / s
    mx, my, mz = d["mx"] / s, d["my"] / s, d["mz"] / s
    er, mr = R_EARTH / s, R_MOON / s

    N = len(sx)
    n_frames = 150
    idx_map = np.linspace(0, N - 1, n_frames).astype(int)

    # In spacecraft-centered coords:
    #   Earth at (-sx, -sy, -sz), Moon at (mx-sx, my-sy, mz-sz)
    # Scene range adapts to show the nearest body well.

    # Initial frame: spacecraft-centered positions
    i0 = idx_map[0]
    earth_sc = (-sx[i0], -sy[i0], -sz[i0])
    moon_sc = (mx[i0] - sx[i0], my[i0] - sy[i0], mz[i0] - sz[i0])

    fig = go.Figure()

    # Earth sphere (trace 0) — updated per frame
    fig.add_trace(make_sphere_mesh(earth_sc, er, earth_img, n=60))
    # Moon sphere (trace 1) — updated per frame
    fig.add_trace(make_sphere_mesh(moon_sc, mr, moon_img, n=50))
    # Spacecraft marker at origin
    fig.add_trace(go.Scatter3d(
        x=[0], y=[0], z=[0], mode="markers",
        marker=dict(color="yellow", size=5, symbol="diamond"),
        name="Apollo 11", hoverinfo="skip",
    ))
    # Direction to Earth line
    fig.add_trace(go.Scatter3d(
        x=[0, earth_sc[0]], y=[0, earth_sc[1]], z=[0, earth_sc[2]],
        mode="lines", line=dict(color="cyan", width=1, dash="dot"),
        name="→ Earth", hoverinfo="skip",
    ))
    # Direction to Moon line
    fig.add_trace(go.Scatter3d(
        x=[0, moon_sc[0]], y=[0, moon_sc[1]], z=[0, moon_sc[2]],
        mode="lines", line=dict(color="gray", width=1, dash="dot"),
        name="→ Moon", hoverinfo="skip",
    ))

    frames = []
    for i in idx_map:
        utc = EPOCH_UTC + timedelta(seconds=float(d["t_h"][i] * 3600))
        ex, ey, ez = -sx[i], -sy[i], -sz[i]
        mcx, mcy, mcz = mx[i] - sx[i], my[i] - sy[i], mz[i] - sz[i]

        # Scale spheres: true scale but ensure visibility
        # Use distance-proportional size for visual reference
        dist_e = np.sqrt(ex**2 + ey**2 + ez**2)
        dist_m = np.sqrt(mcx**2 + mcy**2 + mcz**2)
        near_dist = min(dist_e, dist_m)

        # Scene range: show the nearer body comfortably
        scene_range = near_dist * 1.5

        frames.append(go.Frame(
            data=[
                make_sphere_mesh((ex, ey, ez), er, earth_img, n=40),
                make_sphere_mesh((mcx, mcy, mcz), mr, moon_img, n=30),
                go.Scatter3d(x=[0], y=[0], z=[0]),  # spacecraft
                go.Scatter3d(x=[0, ex * 0.8], y=[0, ey * 0.8], z=[0, ez * 0.8]),  # → Earth
                go.Scatter3d(x=[0, mcx * 0.8], y=[0, mcy * 0.8], z=[0, mcz * 0.8]),  # → Moon
            ],
            traces=[0, 1, 2, 3, 4],
            name=f"GET {d['t_h'][i]:.0f}h",
            layout=dict(
                title=dict(text=(
                    f"View from Apollo 11 — GET {d['t_h'][i]:.1f}h  "
                    f"{utc.strftime('%Y-%m-%d %H:%M UTC')}<br>"
                    f"Earth: {dist_e * s:.0f} km  Moon: {dist_m * s:.0f} km"
                )),
                scene=dict(
                    xaxis_range=[-scene_range, scene_range],
                    yaxis_range=[-scene_range, scene_range],
                    zaxis_range=[-scene_range, scene_range],
                ),
            ),
        ))

    fig.frames = frames

    init_range = max(abs(sx[0]), abs(sy[0]), abs(sz[0])) * 1.2
    fig.update_layout(
        scene=dict(**SCENE_DARK,
                   xaxis_range=[-init_range, init_range],
                   yaxis_range=[-init_range, init_range],
                   zaxis_range=[-init_range, init_range]),
        title=dict(text="View from Apollo 11", font=dict(color="white", size=14)),
        paper_bgcolor="#0a0a1a", height=800, width=1000,
        legend=dict(font=dict(color="white")),
        updatemenus=[dict(type="buttons", showactive=False, x=0.05, y=0.05, buttons=[
            dict(label="▶ Play", method="animate",
                 args=[None, dict(frame=dict(duration=200, redraw=True), fromcurrent=True)]),
            dict(label="⏸ Pause", method="animate",
                 args=[[None], dict(frame=dict(duration=0, redraw=False), mode="immediate")]),
        ])],
        sliders=[dict(active=0, x=0.05, len=0.9, y=0,
                      currentvalue=dict(prefix="", font=dict(color="white")),
                      font=dict(color="gray"),
                      steps=[dict(args=[[f.name], dict(frame=dict(duration=0, redraw=True), mode="immediate")],
                                  method="animate", label=f.name) for f in frames])],
    )
    fig.write_html(OUTPUT_DIR / "apollo11_spacecraft_view.html")
    print("Saved apollo11_spacecraft_view.html")


if __name__ == "__main__":
    sat, moon = load_data()
    d = compute_derived(sat, moon)
    earth_img = load_texture(EARTH_TEX_URL, "earth_2k.jpg")
    moon_img = load_texture(MOON_TEX_URL, "moon_1k.jpg")

    plot_3d_static(d, earth_img, moon_img)
    plot_3d_animation(d, earth_img, moon_img)
    plot_spacecraft_view(d, earth_img, moon_img)
