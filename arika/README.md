# arika

Coordinate frames, reference transforms, epochs, and celestial body
ephemerides for orbital mechanics. The name 「在処」(arika) means "the
whereabouts of a thing" — coordinate transformations express the same
position in different reference frames, telling you _where_ something _is_
from different points of view.

Provides typed coordinate frames (ECI, ECEF, ITRS, TIRS, CIRS, GCRS,
topocentric), the IAU 2006/2000A CIO-based Earth rotation model, geodetic
conversions against WGS84, time-scale-tagged `Epoch<S>`, and Sun/Moon/planet
ephemerides. Designed for the [orts](https://github.com/sksat/orts)
simulation workspace but usable standalone for any astrodynamics project.

