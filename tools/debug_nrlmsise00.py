# /// script
# requires-python = ">=3.10"
# dependencies = ["pymsis", "numpy"]
# ///
"""Debug NRLMSISE-00: compare temperatures at spline node altitudes."""

import numpy as np
import pymsis

# Test condition: vernal equinox, solar_min, equatorial, noon
dates = np.array([np.datetime64("2003-03-20T12:00:00")])
lons = [0.0]
lats = [0.0]
f107s = [70.0]
f107as = [70.0]
aps = [[4.0] * 7]

# Temperature at spline node altitudes and key test altitudes
altitudes = [72.5, 80.0, 90.0, 100.0, 110.0, 120.0, 150.0, 200.0, 300.0, 400.0, 500.0, 700.0, 1000.0]

print("=== pymsis NRLMSISE-00 (version=0) ===")
print("Condition: vernal equinox, solar_min (F10.7=70, Ap=4), equatorial, noon")
print()

for alt in altitudes:
    result = pymsis.calculate(
        dates, lons, lats, [alt], f107s, f107as, aps, version=0
    )
    r = result.flatten()[:11]
    rho = r[0]  # kg/m3
    temp = r[10]  # K
    n2 = r[1]  # m-3
    o2 = r[2]
    o = r[3]
    he = r[4]
    h = r[5]
    ar = r[6]
    n_atom = r[7]
    anom_o = r[8]

    # Convert from m-3 to cm-3
    n2_cm = n2 * 1e-6
    o2_cm = o2 * 1e-6
    o_cm = o * 1e-6
    he_cm = he * 1e-6
    h_cm = h * 1e-6
    ar_cm = ar * 1e-6
    n_cm = n_atom * 1e-6
    anom_o_cm = anom_o * 1e-6
    rho_gcm3 = rho * 1e-3

    print(f"alt={alt:.1f}km: T={temp:.2f}K  rho={rho_gcm3:.4e} g/cm3")
    print(f"  N2={n2_cm:.4e} O2={o2_cm:.4e} O={o_cm:.4e} He={he_cm:.4e}")
    print(f"  H={h_cm:.4e} Ar={ar_cm:.4e} N={n_cm:.4e} anomO={anom_o_cm:.4e}")
    print()

# Solar moderate (what species test uses)
print("\n=== solar_moderate (F10.7=150, Ap=15) ===")
f107s_mod = [150.0]
f107as_mod = [150.0]
aps_mod = [[15.0] * 7]
for alt in [72.5, 90.0, 100.0, 110.0, 120.0, 400.0]:
    result = pymsis.calculate(
        dates, lons, lats, [alt], f107s_mod, f107as_mod, aps_mod, version=0
    )
    r = result.flatten()[:11]
    print(f"alt={alt:.1f}km: T={r[10]:.2f}K  N2={r[1]*1e-6:.4e} O={r[3]*1e-6:.4e}")

# High latitude to find worst temperature error
print("\n=== solar_min, lat=75, lon=180 (potential worst case) ===")
lats_hi = [75.0]
lons_hi = [180.0]
for alt in [72.5, 90.0, 100.0, 110.0, 120.0, 400.0, 1000.0]:
    result = pymsis.calculate(
        dates, lons_hi, lats_hi, [alt], f107s, f107as, aps, version=0
    )
    r = result.flatten()[:11]
    print(f"alt={alt:.1f}km: T={r[10]:.2f}K")

# Check various latitudes at 100km solar_min for temperature pattern
print("\n=== Temperature at 100km vs latitude (solar_min, lon=0) ===")
for lat_val in [0.0, 45.0, 75.0, -45.0]:
    result = pymsis.calculate(
        dates, [0.0], [lat_val], [100.0], f107s, f107as, aps, version=0
    )
    r = result.flatten()[:11]
    print(f"lat={lat_val:.0f}: T={r[10]:.2f}K")
