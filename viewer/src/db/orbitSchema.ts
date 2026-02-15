import type { TableSchema } from "uneri";
import type { OrbitPoint } from "../orbit.js";

const MU_EARTH = 398600.4418;
const RADIUS_EARTH = 6378.137;

export function createOrbitSchema(
  mu: number = MU_EARTH,
  bodyRadius: number = RADIUS_EARTH,
): TableSchema<OrbitPoint> {
  return {
    tableName: "orbit_points",
    columns: [
      { name: "t", type: "DOUBLE" },
      { name: "x", type: "DOUBLE" },
      { name: "y", type: "DOUBLE" },
      { name: "z", type: "DOUBLE" },
      { name: "vx", type: "DOUBLE" },
      { name: "vy", type: "DOUBLE" },
      { name: "vz", type: "DOUBLE" },
      { name: "a", type: "DOUBLE" },
      { name: "e", type: "DOUBLE" },
      { name: "inc", type: "DOUBLE" },
      { name: "raan", type: "DOUBLE" },
      { name: "omega", type: "DOUBLE" },
      { name: "nu", type: "DOUBLE" },
    ],
    derived: [
      // Pass-through: expose base columns for charting
      // (buildDerivedQuery only SELECTs derived columns, not base columns)
      { name: "a", sql: "a", unit: "km" },
      { name: "e", sql: "e", unit: "-" },
      {
        name: "altitude",
        sql: `sqrt(x*x + y*y + z*z) - ${bodyRadius}`,
        unit: "km",
      },
      {
        name: "energy",
        sql: `(vx*vx + vy*vy + vz*vz)/2.0 - ${mu} / sqrt(x*x + y*y + z*z)`,
        unit: "km^2/s^2",
      },
      {
        name: "angular_momentum",
        sql: `sqrt(power(y*vz - z*vy, 2) + power(z*vx - x*vz, 2) + power(x*vy - y*vx, 2))`,
        unit: "km^2/s",
      },
      {
        name: "velocity",
        sql: `sqrt(vx*vx + vy*vy + vz*vz)`,
        unit: "km/s",
      },
      {
        name: "inc_deg",
        sql: `inc * 180.0 / 3.141592653589793`,
        unit: "deg",
      },
      {
        name: "raan_deg",
        sql: `raan * 180.0 / 3.141592653589793`,
        unit: "deg",
      },
      {
        name: "omega_deg",
        sql: `omega * 180.0 / 3.141592653589793`,
        unit: "deg",
      },
      {
        name: "nu_deg",
        sql: `nu * 180.0 / 3.141592653589793`,
        unit: "deg",
      },
    ],
    toRow: (p: OrbitPoint) => [
      p.t, p.x, p.y, p.z, p.vx, p.vy, p.vz,
      p.a, p.e, p.inc, p.raan, p.omega, p.nu,
    ],
  };
}
