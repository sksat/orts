---
name: orekit-fixtures
description: Reference for generating Orekit cross-validation fixtures. Covers orekit-jpype setup, force model matching, CSSI space weather, and fixture generation patterns.
---

# Orekit Fixture Generation

Reference for creating Python scripts that generate Orekit reference data for cross-validation with our Rust orbit propagator.

## Setup

All generators are PEP 723 uv scripts. Run with `uv run tools/generate_*.py`.

```python
# /// script
# requires-python = ">=3.10"
# dependencies = ["orekit-jpype", "jdk4py"]
# ///

import orekit_jpype as orekit
orekit.initVM()

from orekit_jpype.pyhelpers import download_orekit_data_curdir, setup_orekit_curdir
from pathlib import Path

data_dir = Path("orekit-data")
if not data_dir.exists():
    download_orekit_data_curdir()
setup_orekit_curdir()
```

## Constants (must match Rust)

```python
MU_EARTH_KM3_S2 = 398600.4418       # WGS84
R_EARTH_KM = 6378.137               # WGS84 equatorial
J2_EARTH = 1.08263e-3               # WGS84/EGM96
J3_EARTH = -2.5356e-6
J4_EARTH = -1.6199e-6
MU_SUN_KM3_S2 = 132712440018.0
MU_MOON_KM3_S2 = 4902.800066
OMEGA_EARTH = 7.2921159e-5          # rad/s
SOLAR_RADIATION_PRESSURE = 4.5396e-6  # N/m² at 1 AU
DEFAULT_CR = 1.5
DEFAULT_AREA_TO_MASS = 0.02         # m²/kg
DEFAULT_BALLISTIC_COEFF = 0.01      # m²/kg
```

Verify at runtime:
```python
from org.orekit.utils import Constants
mu_orekit = Constants.WGS84_EARTH_MU / 1e9  # m³/s² → km³/s²
re_orekit = Constants.WGS84_EARTH_EQUATORIAL_RADIUS / 1e3
```

## NumericalPropagator Creation

```python
from org.hipparchus.ode.nonstiff import DormandPrince853Integrator
from org.orekit.frames import FramesFactory
from org.orekit.orbits import CartesianOrbit, OrbitType
from org.orekit.propagation import SpacecraftState
from org.orekit.propagation.numerical import NumericalPropagator
from org.orekit.utils import PVCoordinates
from org.hipparchus.geometry.euclidean.threed import Vector3D

eci = FramesFactory.getEME2000()
mu_si = MU_EARTH_KM3_S2 * 1e9

pos_m = Vector3D(pos_km[0]*1e3, pos_km[1]*1e3, pos_km[2]*1e3)
vel_m = Vector3D(vel_kms[0]*1e3, vel_kms[1]*1e3, vel_kms[2]*1e3)
pv = PVCoordinates(pos_m, vel_m)
orbit = CartesianOrbit(pv, eci, epoch_date, mu_si)

integrator = DormandPrince853Integrator(0.001, 300.0, 1e-14, 1e-12)
propagator = NumericalPropagator(integrator)
propagator.setOrbitType(OrbitType.CARTESIAN)
propagator.setInitialState(SpacecraftState(orbit, 1.0))  # mass=1 for unit A/m
```

## Force Models

### Gravity (zonal harmonics only)

**Important**: Use EME2000 as body frame so gravity pole = J2000 Z-axis (matches our Rust code which uses the J2000 Z-axis as Earth's pole).

```python
from org.orekit.forces.gravity import HolmesFeatherstoneAttractionModel
from org.orekit.forces.gravity.potential import GravityFieldFactory

provider = GravityFieldFactory.getNormalizedProvider(degree, 0)  # order=0 for zonal
hf = HolmesFeatherstoneAttractionModel(FramesFactory.getEME2000(), provider)
propagator.addForceModel(hf)
```

### Third-body (Sun/Moon)

```python
from org.orekit.bodies import CelestialBodyFactory
from org.orekit.forces.gravity import ThirdBodyAttraction

propagator.addForceModel(ThirdBodyAttraction(CelestialBodyFactory.getSun()))
propagator.addForceModel(ThirdBodyAttraction(CelestialBodyFactory.getMoon()))
```

### SRP (cannonball + cylindrical shadow)

```python
from org.orekit.forces.radiation import IsotropicRadiationSingleCoefficient, SolarRadiationPressure
from org.orekit.bodies import OneAxisEllipsoid
from org.orekit.utils import Constants, IERSConventions

sun = CelestialBodyFactory.getSun()
itrf = FramesFactory.getITRF(IERSConventions.IERS_2010, True)
earth = OneAxisEllipsoid(Constants.WGS84_EARTH_EQUATORIAL_RADIUS,
                         Constants.WGS84_EARTH_FLATTENING, itrf)
spacecraft = IsotropicRadiationSingleCoefficient(area_to_mass, cr)
srp = SolarRadiationPressure(sun, earth, spacecraft)  # earth → cylindrical shadow
propagator.addForceModel(srp)
```

### Atmospheric Drag

Our ballistic coefficient B = Cd*A/(2m). For unit mass (m=1): area = 2*B/Cd.

```python
from org.orekit.forces.drag import DragForce, IsotropicDrag

cd = 2.2
area = 2.0 * ballistic_coeff / cd  # m²
drag_spacecraft = IsotropicDrag(area, cd)
```

#### Harris-Priester

```python
from org.orekit.models.earth.atmosphere import HarrisPriester
atmosphere = HarrisPriester(sun, earth, n)  # n=2 (default exponent)
propagator.addForceModel(DragForce(atmosphere, drag_spacecraft))
```

#### NRLMSISE-00 with constant weather

```python
import jpype
from org.orekit.models.earth.atmosphere import NRLMSISE00, NRLMSISE00InputParameters
from org.orekit.time import AbsoluteDate

class ConstantSolarActivity:
    def getDailyFlux(self, date): return float(f107)
    def getAverageFlux(self, date): return float(f107)
    def getAp(self, date): return [float(ap)] * 7
    def getMinDate(self): return AbsoluteDate.PAST_INFINITY
    def getMaxDate(self): return AbsoluteDate.FUTURE_INFINITY

proxy = jpype.JProxy(NRLMSISE00InputParameters, inst=ConstantSolarActivity())
atmosphere = NRLMSISE00(proxy, sun, earth)
```

#### NRLMSISE-00 with CSSI real weather

**Important**: Orekit's `CssiSpaceWeatherData` has internal cache/generator requirements
that make it incompatible with trimmed CSSI files. Use the full `SpaceWeather-All-v1.2.txt`
from orekit-data instead (same CelesTrak source — F10.7/Ap are identical for overlapping dates).

```python
from org.orekit.models.earth.atmosphere import NRLMSISE00
from org.orekit.models.earth.atmosphere.data import CssiSpaceWeatherData

# Uses the full SpaceWeather-All file from orekit-data.zip
cssi = CssiSpaceWeatherData("SpaceWeather-All-v1.2.txt")
atmosphere = NRLMSISE00(cssi, sun, earth)
```

**Rust equivalent**:
```rust
let cssi_text = include_str!("fixtures/cssi_test_weather.txt");
let cssi_data = tobari::CssiData::parse(cssi_text).unwrap();
let weather = Box::new(tobari::CssiSpaceWeather::new(cssi_data));
let model = tobari::Nrlmsise00::new(weather);
```

## Epoch Parsing

```python
from org.orekit.time import AbsoluteDate, TimeScalesFactory
utc = TimeScalesFactory.getUTC()
date = AbsoluteDate("2024-03-20T12:00:00", utc)
```

## Density Computation (without propagation)

```python
from org.orekit.bodies import GeodeticPoint
lat_rad = math.radians(lat_deg)
lon_rad = math.radians(lon_deg)
geod = GeodeticPoint(lat_rad, lon_rad, alt_km * 1e3)
pos_ecef = earth.transform(geod)
density = msise_model.getDensity(epoch_date, pos_ecef, itrf)  # kg/m³
```

## Existing Generators

| Script | Output | Run command |
|---|---|---|
| `tools/generate_orekit_propagation_fixtures.py` | `orbits/tests/fixtures/orekit_propagation_reference.json` | `uv run tools/generate_orekit_propagation_fixtures.py` |
| `tools/generate_orekit_msise_density_fixtures.py` | `tobari/tests/fixtures/orekit_msise_density_reference.json` | `uv run tools/generate_orekit_msise_density_fixtures.py` |
| `tools/generate_hp_fixtures.py` | `tobari/tests/fixtures/hp_orekit_reference.json` | `uv run tools/generate_hp_fixtures.py` |
| `tools/generate_sgp4_fixtures.py` | `orbits/tests/fixtures/sgp4_reference.json` | `uv run tools/generate_sgp4_fixtures.py` |
| `tools/generate_nrlmsise00_fixtures.py` | `tobari/tests/fixtures/nrlmsise00_reference.json` | `uv run tools/generate_nrlmsise00_fixtures.py` |
| `tools/generate_iss_decay_fixtures.py` | `orbits/tests/fixtures/iss_decay_reference.json` | `uv run tools/generate_iss_decay_fixtures.py` |
| `tools/generate_cssi_test_fixture.py` | `tobari/tests/fixtures/cssi_test_weather.txt` | `uv run tools/generate_cssi_test_fixture.py` |

## Known Differences (Orekit vs Rust)

| Source | Magnitude | Affects |
|---|---|---|
| Sun position (DE405 vs Meeus) | ~0.35° | SRP, HP bulge, NRLMSISE-00 LST |
| Moon position (DE405 vs analytical) | ~10' | Third-body |
| LST (precise vs UT+lon/15) | ~16 min | NRLMSISE-00 density (1-5%) |
| Gravity (HolmesFeatherstone vs explicit) | J2 diff ~3e-9 | Sub-meter at 10 orbits |
| Geodetic altitude | Both WGS-84 | Matched after geo.rs fix |
