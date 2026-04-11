# utsuroi

Numerical ODE integrators for orbital mechanics and rigid-body simulation.
The name 「移ろい」(utsuroi) evokes the flowing passage of state over time.

Provides Runge-Kutta (RK4), Dormand-Prince (RK45), Störmer-Verlet, and
Yoshida symplectic integrators with a generic trait interface. Designed
for use as the stepping engine inside the
[orts](https://github.com/sksat/orts) simulation workspace but usable
standalone for any ODE system.

