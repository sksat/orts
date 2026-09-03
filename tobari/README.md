# tobari

Earth environment models — atmospheric drag density, spherical-harmonic
geopotential, IGRF geomagnetic field, and space weather integration for
orbital mechanics simulation. The name
「帳」(tobari) means "veil" — Earth's atmosphere and magnetosphere form the
environmental veil a spacecraft flies through.

Provides:

- Atmospheric density models (`Exponential`, `HarrisPriester`, `NRLMSISE-00`)
  behind the `AtmosphereModel` trait, with typed `SimpleEci` position input.
- Spherical-harmonic geopotential (`gravity::SphericalHarmonicField`): ICGEM
  `.gfc` loader (static fields such as EGM96 / EGM2008 / EIGEN-6C4) and a
  Holmes–Featherstone evaluator for the non-central acceleration in the
  body-fixed frame, regular at the poles.
- IGRF-14 full geomagnetic field + tilted-dipole approximation via
  `MagneticFieldModel`.
- Space weather providers (CSSI, GFZ) feeding F10.7 / Ap / Kp indices into
  the atmosphere models.

