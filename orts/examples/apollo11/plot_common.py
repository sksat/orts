# /// script
# requires-python = ">=3.10"
# dependencies = ["numpy>=1.24"]
# ///
"""Shared data loading and constants for Apollo 11 visualization scripts."""
import subprocess
import sys
from datetime import datetime
from pathlib import Path

import numpy as np

R_EARTH = 6378.137
R_MOON = 1737.4
EPOCH_UTC = datetime(1969, 7, 16, 13, 43, 49)
# J2000 epoch as Julian Date
EPOCH_JD = 2440423.0 + (13 + 43 / 60 + 49 / 3600) / 24  # 1969-07-16T13:43:49 UTC

SCRIPT_DIR = Path(__file__).parent
CACHE_DIR = SCRIPT_DIR / ".cache"
OUTPUT_DIR = SCRIPT_DIR
RRD_PATH = SCRIPT_DIR / "apollo11.rrd"

# Texture URLs per quality level.
# "low" uses cached 2K JPGs (fast rendering, no download needed if cached).
# "high" uses NASA 8K/16K TIFs (auto-converted to JPG on first download).
TEXTURE_URLS = {
    "low": {
        "earth": ("https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57735/land_ocean_ice_cloud_2048.jpg", "earth_2k.jpg"),
        "moon": ("https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_1k.jpg", "moon_1k.jpg"),
    },
    "high": {
        "earth": ("https://eoimages.gsfc.nasa.gov/images/imagerecords/57000/57735/land_ocean_ice_cloud_8192.tif", "earth_8k.tif"),
        "moon": ("https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720/lroc_color_poles_16k.tif", "moon_16k.tif"),
    },
}
DEFAULT_QUALITY = "high"


def load_data():
    """Load trajectory CSV from RRD via orts CLI."""
    # Run from workspace root so cargo finds the correct Cargo.toml
    workspace_root = SCRIPT_DIR.parents[2]  # orts/examples/apollo11 → orts
    result = subprocess.run(
        ["cargo", "run", "--bin", "orts", "-q", "--",
         "convert", str(RRD_PATH), "--format", "csv"],
        capture_output=True, text=True, timeout=60,
        cwd=str(workspace_root),
    )
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        sys.exit(1)
    sat_rows, moon_rows = [], []
    for line in result.stdout.strip().split("\n"):
        if line.startswith("#") or not line:
            continue
        p = line.split(",")
        t, x, y, z = float(p[0]), float(p[1]), float(p[2]), float(p[3])
        vx, vy, vz = float(p[4]), float(p[5]), float(p[6])
        (moon_rows if abs(vx) + abs(vy) + abs(vz) < 1e-12 else sat_rows).append((t, x, y, z))
    sat = np.array(sat_rows)
    moon = np.array(moon_rows)
    step = max(1, len(sat) // 1200)
    return sat[::step], moon


def compute_derived(sat, moon):
    """Compute all derived quantities from raw trajectory data."""
    t_h = sat[:, 0] / 3600
    sx, sy, sz = sat[:, 1], sat[:, 2], sat[:, 3]
    mt = moon[:, 0]
    mx = np.interp(sat[:, 0], mt, moon[:, 1])
    my = np.interp(sat[:, 0], mt, moon[:, 2])
    mz = np.interp(sat[:, 0], mt, moon[:, 3])
    r_earth = np.sqrt(sx**2 + sy**2 + sz**2)
    r_moon = np.sqrt((sx - mx)**2 + (sy - my)**2 + (sz - mz)**2)
    # Earth-Moon rotating frame
    em_dist = np.sqrt(mx**2 + my**2 + mz**2)
    ex = np.column_stack([mx / em_dist, my / em_dist, mz / em_dist])
    emv = np.column_stack([np.gradient(mx, t_h * 3600),
                           np.gradient(my, t_h * 3600),
                           np.gradient(mz, t_h * 3600)])
    ez = np.cross(np.column_stack([mx, my, mz]), emv)
    ez = ez / np.linalg.norm(ez, axis=1, keepdims=True)
    ey = np.cross(ez, ex)
    s_eci = np.column_stack([sx, sy, sz])
    rot_x = np.sum(s_eci * ex, axis=1) / em_dist
    rot_y = np.sum(s_eci * ey, axis=1) / em_dist
    # Moon-centered (time-varying)
    mc_x, mc_y, mc_z = sx - mx, sy - my, sz - mz
    return dict(
        t_h=t_h, sx=sx, sy=sy, sz=sz, mx=mx, my=my, mz=mz,
        r_earth=r_earth, r_moon=r_moon, rot_x=rot_x, rot_y=rot_y,
        mc_x=mc_x, mc_y=mc_y, mc_z=mc_z,
    )


def sun_direction_eci(t_hours):
    """Sun direction (unit vector) in ECI frame at epoch + t_hours.

    Port of arika::sun::sun_direction_eci (Meeus Ch.25).
    Accuracy ~1 arcminute — sufficient for lighting.
    """
    jd = EPOCH_JD + t_hours / 24.0
    # Julian centuries from J2000.0
    t = (jd - 2451545.0) / 36525.0
    # Mean longitude [deg]
    l0 = 280.46646 + 36000.76983 * t
    # Mean anomaly [rad]
    m = np.radians(357.52911 + 35999.05029 * t)
    # Equation of center [deg]
    c = (1.9146 - 0.004817 * t) * np.sin(m) + 0.019993 * np.sin(2 * m)
    # Ecliptic longitude [rad]
    lam = np.radians(l0 + c)
    # Obliquity [rad]
    eps = np.radians(23.439291 - 0.0130042 * t)
    # ECI unit vector
    x = np.cos(lam)
    y = np.cos(eps) * np.sin(lam)
    z = np.sin(eps) * np.sin(lam)
    norm = np.sqrt(x**2 + y**2 + z**2)
    return np.array([x / norm, y / norm, z / norm])


def download_cached(url, name):
    """Download a file with local caching.  TIF files are auto-converted to JPG."""
    import urllib.request

    # For TIF URLs, convert to JPG after download
    jpg_name = name.rsplit(".", 1)[0] + ".jpg" if name.endswith(".tif") else None
    if jpg_name:
        jpg_path = CACHE_DIR / jpg_name
        if jpg_path.exists():
            return str(jpg_path)

    path = CACHE_DIR / name
    if path.exists() and jpg_name is None:
        return str(path)

    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        print(f"  Downloading {name}...")
        urllib.request.urlretrieve(url, path)

    if jpg_name:
        from PIL import Image
        print(f"  Converting {name} -> {jpg_name}...")
        img = Image.open(path)
        img.save(jpg_path, "JPEG", quality=95)
        return str(jpg_path)

    return str(path)


def get_textures(quality=None):
    """Download and return (earth_tex_path, moon_tex_path) for the given quality.

    Args:
        quality: "low" (2K, fast) or "high" (8K/16K, slow). Defaults to DEFAULT_QUALITY.
    """
    q = quality or DEFAULT_QUALITY
    urls = TEXTURE_URLS[q]
    earth = download_cached(urls["earth"][0], urls["earth"][1])
    moon = download_cached(urls["moon"][0], urls["moon"][1])
    return earth, moon


def parse_quality_arg():
    """Parse --low / --high from sys.argv. Returns quality string."""
    import sys
    if "--low" in sys.argv:
        sys.argv.remove("--low")
        return "low"
    if "--high" in sys.argv:
        sys.argv.remove("--high")
        return "high"
    return DEFAULT_QUALITY
