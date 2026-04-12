//! EOP data entry (one row of the table).

/// A single EOP data point at a specific MJD.
///
/// All values are in the units expected by arika's EOP traits:
/// - `xp`, `yp`: arcseconds
/// - `dut1`: seconds (UT1 - UTC)
/// - `lod`: seconds (excess length of day)
/// - `dx`, `dy`: milliarcseconds (nutation corrections wrt IAU 2000A)
#[derive(Debug, Clone, PartialEq)]
pub struct EopEntry {
    /// Modified Julian Date (UTC).
    pub mjd: f64,
    /// x component of polar motion [arcsec].
    pub xp: f64,
    /// y component of polar motion [arcsec].
    pub yp: f64,
    /// UT1 - UTC [seconds].
    pub dut1: f64,
    /// Length of Day excess [seconds]. `None` if not available.
    pub lod: Option<f64>,
    /// dX nutation correction [mas]. `None` if not available.
    pub dx: Option<f64>,
    /// dY nutation correction [mas]. `None` if not available.
    pub dy: Option<f64>,
}
