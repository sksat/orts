/** Convert a Date to astronomical Julian Date (JD). */
export function dateToJd(date: Date): number {
  // Unix epoch (1970-01-01T00:00:00Z) = JD 2440587.5
  return date.getTime() / 86400000 + 2440587.5;
}

export type FieldComponent = "total" | "inclination" | "declination" | "north" | "east" | "down";
export type AtmoModel = "exponential" | "harris-priester" | "nrlmsise00";
export type MagModel = "igrf" | "dipole";

export interface ViewerParams {
  epochJd: number;
  altitudeKm: number;
  f107: number;
  ap: number;
  fieldComponent: FieldComponent;
  atmoModel: AtmoModel;
  magModel: MagModel;
  /** Grid resolution (number of latitude bins). */
  nLat: number;
}
