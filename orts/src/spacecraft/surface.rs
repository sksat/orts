use crate::model::{HasAttitude, HasFrame, HasMass, HasOrbit, Model};
use crate::perturbations::OMEGA_EARTH;
use arika::body::KnownBody;
use arika::earth::R as R_EARTH;
use arika::earth::geodetic::Geodetic;
use arika::earth::{EarthFixedTransform, EarthOrientation};
use arika::epoch::Epoch;
use arika::frame;
use nalgebra::Vector3;
use tobari::{AtmosphereInput, AtmosphereModel, Exponential};

use super::ExternalLoads;

/// How a flat panel reflects sunlight, for radiation pressure.
///
/// Absorption, specular reflection and diffuse reflection sum to 1, so two of
/// them determine the third. Absorption is derived rather than stored, and the
/// two stored fractions are private, so the constraint holds for every value
/// that exists — unlike the sibling fields of [`SurfacePanel`], which are
/// independent scalars a struct literal can set freely. A `specular` above 1
/// would make the absorbed fraction negative and point the Sun-line component
/// of the force *toward* the Sun.
///
/// A single lumped `Cr` cannot stand in for these: it fixes the force magnitude
/// face-on but leaves the direction undetermined at oblique incidence, where
/// specular and diffuse reflection push along the panel normal rather than
/// along the Sun line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelOptics {
    specular: f64,
    diffuse: f64,
}

impl PanelOptics {
    /// Reflection properties from the specular and diffuse fractions; the rest
    /// of the incident light is absorbed.
    ///
    /// # Panics
    /// Panics unless both coefficients are finite, non-negative and sum to at
    /// most 1.
    pub fn new(specular: f64, diffuse: f64) -> Self {
        assert!(
            specular.is_finite() && diffuse.is_finite(),
            "Panel reflectivities must be finite, got specular={specular}, diffuse={diffuse}"
        );
        assert!(
            specular >= 0.0 && diffuse >= 0.0,
            "Panel reflectivities must be non-negative, got specular={specular}, diffuse={diffuse}"
        );
        assert!(
            specular + diffuse <= 1.0,
            "Panel reflectivities must sum to at most 1, got specular={specular} + diffuse={diffuse}"
        );
        Self { specular, diffuse }
    }

    /// A black panel: everything absorbed, nothing reflected (Cr = 1 face-on).
    pub fn absorber() -> Self {
        Self {
            specular: 0.0,
            diffuse: 0.0,
        }
    }

    /// Specular reflectivity ρ_s: the fraction reflected mirror-like.
    pub fn specular(self) -> f64 {
        self.specular
    }

    /// Diffuse reflectivity ρ_d: the fraction re-emitted Lambertian.
    pub fn diffuse(self) -> f64 {
        self.diffuse
    }

    /// Absorbed fraction α = 1 − (ρ_s + ρ_d).
    ///
    /// Never negative: [`Self::new`] has already checked that the sum is at
    /// most 1, and subtracting that one sum keeps the guarantee. Subtracting
    /// the two terms one after the other would not — `1.0 - 0.9 - 0.1` is
    /// -2.8e-17, because the second subtraction rounds again.
    pub fn absorptivity(self) -> f64 {
        1.0 - (self.specular + self.diffuse)
    }
}

/// A panel's extent within its own plane.
///
/// A panel needs none of this to produce a force: the flat-plate law uses the
/// projected area, the normal and the optics, and never the boundary. It is
/// here so that one panel can be found to cover another completely, seen from
/// the Sun or the flow, which needs the boundary and nothing else.
///
/// An enum because the shapes will not stay one: a mesh read from CAD gives
/// triangles. Three operations know the shapes — `SurfacePanel::corners_into`,
/// which lists the corners, `SurfacePanel::outline_contains`, which answers
/// whether a point is inside, and
/// `SpacecraftShape::assert_outlines_are_consistent`, which checks a shape's
/// own invariants — so a new shape means a new arm in each of those and
/// nothing else. Each of the three destructures this enum without a fallback
/// arm, so a new variant that misses one of them does not compile.
/// Containment cannot be derived from the corners: a triangle's three corners
/// span a parallelogram larger than the triangle.
///
/// `#[non_exhaustive]` so that adding a shape stays a minor change: without it
/// a downstream `match` could be exhaustive today and stop compiling the day a
/// triangle arrives, which would defeat the reason this is an enum.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum PanelOutline {
    /// A rectangle centred on the panel's `cp_offset`.
    ///
    /// For a plate with uniform properties, fully lit, the centre of pressure
    /// *is* the area centroid, so centring on it is exact. A shape whose
    /// centroid moves away from the centre of pressure would have to carry its
    /// own reference point.
    Rectangle {
        /// Half-extents [m]: along `in_plane_x`, then along
        /// `normal × in_plane_x`.
        half_extent: [f64; 2],
        /// In-plane reference axis (unit length, perpendicular to the normal).
        in_plane_x: Vector3<f64>,
    },
}

/// The most corners any [`PanelOutline`] shape has.
pub(crate) const MAX_PANEL_CORNERS: usize = 4;

/// A flat surface panel on a spacecraft body.
///
/// Represents one face of the spacecraft's outer surface for computing
/// aerodynamic and SRP forces.  Each panel has an outward-pointing normal in
/// the body frame, a drag coefficient, optical properties, and a
/// centre-of-pressure offset from the centre of mass.
///
/// For thin surfaces like solar panels where both sides are exposed to the
/// flow, model each side as a separate panel with opposite normals; the two
/// sides may then carry different optical properties. [`Self::back_face`]
/// builds the second one from the first.
///
/// Both force models assume `normal` is unit length. [`Self::at_com`],
/// [`Self::rectangle`] and [`SpacecraftShape::cube`] guarantee that; a struct
/// literal does not, and the SRP force is cubic in `|normal|` through its
/// specular term.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacePanel {
    /// Panel area [m²].
    pub area: f64,
    /// Outward-pointing unit normal in the body frame. Must be unit length.
    pub normal: Vector3<f64>,
    /// Drag coefficient (typically 2.0–2.2 for LEO free-molecular flow).
    pub cd: f64,
    /// Reflection properties, used by SRP panel models.
    pub optics: PanelOptics,
    /// Centre-of-pressure offset from the spacecraft CoM [m, body frame].
    pub cp_offset: Vector3<f64>,
    /// In-plane extent, when the panel has one.
    ///
    /// Only panels that carry it take part in occlusion — a panel without an
    /// outline neither casts a shadow on another panel nor receives one. The
    /// force a lit panel produces is the same either way.
    pub outline: Option<PanelOutline>,
}

impl SurfacePanel {
    /// Create a panel whose centre of pressure coincides with the CoM.
    ///
    /// The `normal` vector is normalised internally; it need not be unit length.
    ///
    /// `optics` is a required argument rather than a default. The lumped `Cr`
    /// this replaced has no unique specular/diffuse split, so any default would
    /// be an invented surface; and since the force direction now depends on the
    /// split, no default reproduces the old behaviour at oblique incidence.
    /// Making it explicit turns what would be a silent change in every existing
    /// caller's SRP force into a compile error. [`PanelOptics::absorber`] is the
    /// value to pass when the surface is genuinely unknown.
    ///
    /// # Panics
    /// Panics unless `normal` has a finite non-zero magnitude. Finite
    /// components are not enough on their own: `[1e300, 1e300, 0]` squares to a
    /// norm that overflows and `[1e-200; 3]` to one that underflows, and
    /// neither can be normalised.
    pub fn at_com(area: f64, normal: Vector3<f64>, cd: f64, optics: PanelOptics) -> Self {
        let n = unit_direction(normal, "normal");
        Self {
            area,
            normal: n,
            cd,
            optics,
            cp_offset: Vector3::zeros(),
            outline: None,
        }
    }

    /// Override the reflection properties (builder pattern).
    pub fn with_optics(mut self, optics: PanelOptics) -> Self {
        self.optics = optics;
        self
    }

    /// Move the centre of pressure off the CoM (builder pattern) [m, body frame].
    ///
    /// This is what turns a panel's force into an attitude disturbance: the
    /// torque is `cp_offset x F`.
    pub fn with_cp_offset(mut self, cp_offset: Vector3<f64>) -> Self {
        self.cp_offset = cp_offset;
        self
    }

    /// Create a rectangular panel with a known extent, centred on `cp_offset`.
    ///
    /// The area follows from the half-extents, so this is the constructor to
    /// reach for when the panel's boundary matters — a panel built with
    /// [`Self::at_com`] has an area and no boundary, and takes no part in
    /// occlusion.
    ///
    /// # Panics
    /// Panics unless both vectors have a finite non-zero magnitude, which
    /// finite components alone do not give: `[1e300, 1e300, 0]` squares to a
    /// norm that overflows and `[1e-200; 3]` to one that underflows, and
    /// neither can be normalised. Panics too if the half-extents are not
    /// positive and finite, if their product underflows to zero or overflows to
    /// infinity, or if `in_plane_x` is not perpendicular to `normal` (to within
    /// 1e-9 after normalisation). An axis off the plane
    /// describes a rectangle that is not on the panel; projecting it onto the
    /// plane would build a panel the caller did not ask for, so it is rejected
    /// instead.
    pub fn rectangle(
        half_extent: [f64; 2],
        in_plane_x: Vector3<f64>,
        normal: Vector3<f64>,
        cd: f64,
        optics: PanelOptics,
    ) -> Self {
        assert!(
            half_extent.iter().all(|h| h.is_finite() && *h > 0.0),
            "panel half-extents must be positive and finite, got {half_extent:?}"
        );
        // The product needs its own check: `[1e-300, 1e-300]` underflows to a
        // zero area and `[1e200, 1e200]` overflows to infinity, and each
        // half-extent is positive and finite in both.
        //
        // The two extents multiply first. `4.0 * h[0] * h[1]` would overflow on
        // `4.0 * 1e308` before ever seeing the second extent, so the same
        // geometry passed or failed depending on which order it was written in.
        let area = half_extent[0] * half_extent[1] * 4.0;
        assert!(
            area.is_finite() && area > 0.0,
            "panel half-extents {half_extent:?} give an area of {area}"
        );
        let n = unit_direction(normal, "normal");
        let x = unit_direction(in_plane_x, "in-plane axis");
        assert!(
            n.dot(&x).abs() < 1e-9,
            "panel in-plane axis must be perpendicular to the normal, got n·x = {}",
            n.dot(&x)
        );
        Self {
            area,
            normal: n,
            cd,
            optics,
            cp_offset: Vector3::zeros(),
            outline: Some(PanelOutline::Rectangle {
                half_extent,
                in_plane_x: x,
            }),
        }
    }

    /// Write the outline's corners in order into `buf`, or `None` without one.
    ///
    /// Corner-count varies by shape, so the filled prefix is returned rather
    /// than a fixed array. Takes a buffer because occlusion runs per panel pair
    /// per integrator stage, where an allocation would not pay for itself.
    pub(crate) fn corners_into<'b>(
        &self,
        buf: &'b mut [Vector3<f64>; MAX_PANEL_CORNERS],
    ) -> Option<&'b [Vector3<f64>]> {
        match self.outline? {
            PanelOutline::Rectangle {
                half_extent: [hx, hy],
                in_plane_x,
            } => {
                let y = self.normal.cross(&in_plane_x);
                buf[0] = self.cp_offset + in_plane_x * hx + y * hy;
                buf[1] = self.cp_offset + in_plane_x * hx - y * hy;
                buf[2] = self.cp_offset - in_plane_x * hx - y * hy;
                buf[3] = self.cp_offset - in_plane_x * hx + y * hy;
                Some(&buf[..4])
            }
        }
    }

    /// Whether `point` lies within the outline, projected onto the panel plane.
    ///
    /// `false` without an outline: a panel with no boundary contains nothing,
    /// which is what keeps it from casting shadows.
    ///
    /// Each shape answers for itself. A triangle's three corners span a
    /// parallelogram larger than the triangle, so a shared corner-based rule
    /// would quietly over-report once meshes land.
    pub(crate) fn outline_contains(&self, point: &Vector3<f64>) -> bool {
        match self.outline {
            None => false,
            Some(PanelOutline::Rectangle {
                half_extent: [hx, hy],
                in_plane_x,
            }) => {
                let d = point - self.cp_offset;
                let y = self.normal.cross(&in_plane_x);
                d.dot(&in_plane_x).abs() <= hx * (1.0 + OUTLINE_EDGE_TOLERANCE)
                    && d.dot(&y).abs() <= hy * (1.0 + OUTLINE_EDGE_TOLERANCE)
            }
        }
    }

    /// The other side of the same thin plate: the normal is negated, and the
    /// area, drag coefficient and centre of pressure carry over.
    ///
    /// A panel is one face. Both force models drop a panel whose normal points
    /// away from the flow, so a thin structure exposed on both sides — a solar
    /// array — needs both faces present or it produces nothing for half of the
    /// attitudes it sees. The two sides rarely share optics, which is why they
    /// are passed rather than copied: cells on one side, substrate on the other.
    ///
    /// Both faces sit at the same `cp_offset`, since it is one plate. Give the
    /// front its offset first — this copies the value, so a later
    /// [`Self::with_cp_offset`] on the front leaves the back where it was:
    ///
    /// ```
    /// use nalgebra::Vector3;
    /// use orts::spacecraft::{PanelOptics, SurfacePanel};
    ///
    /// let cells = PanelOptics::new(0.1, 0.2);
    /// let substrate = PanelOptics::new(0.05, 0.4);
    /// let front = SurfacePanel::at_com(4.0, Vector3::x(), 2.2, cells)
    ///     .with_cp_offset(Vector3::new(0.0, 1.5, 0.0));
    /// let back = front.back_face(substrate);
    ///
    /// assert_eq!(back.normal, -front.normal);
    /// assert_eq!(back.cp_offset, front.cp_offset);
    /// ```
    ///
    /// A different `cd` per side needs two panels written out, since this shares
    /// it. Calling `back_face` on a back face gives a third panel facing the
    /// front's way again, which is not a plate with three sides — it is two
    /// coincident faces.
    pub fn back_face(&self, optics: PanelOptics) -> Self {
        Self {
            area: self.area,
            normal: -self.normal,
            cd: self.cd,
            optics,
            cp_offset: self.cp_offset,
            outline: self.outline,
        }
    }
}

/// Normalise a direction the caller supplied, rejecting one whose magnitude
/// cannot be computed.
///
/// Checking the normalised result is not enough. A vector whose squared norm
/// underflows normalises to `[inf, inf, inf]`, and an infinite magnitude
/// passes any lower bound: the `|n| > 0.5` this replaced read it as a valid
/// direction, and every force built from that normal is NaN. One whose squared
/// norm overflows normalises to `[0, 0, 0]` and is rejected, since dividing by
/// an infinite norm gives zero. Both start from finite components, so it is the
/// input that has to be measured, before it is divided.
fn unit_direction(v: Vector3<f64>, what: &str) -> Vector3<f64> {
    let mag = v.magnitude();
    assert!(
        mag.is_finite() && mag > 0.0,
        "panel {what} needs a finite non-zero magnitude, got {:?} with |v| = {mag}",
        v.as_slice()
    );
    v / mag
}

/// Spacecraft shape model for aerodynamic force computation.
///
/// Provides a gradation from the simplest attitude-independent model
/// (`Sphere`) to fully attitude-dependent panel models (`Panels`).
#[derive(Debug, Clone)]
pub enum SpacecraftShape {
    /// Attitude-independent model. Carries effective cross-section area and
    /// surface coefficients. Not limited to geometric spheres — represents
    /// any isotropic surface model.
    Sphere {
        /// Effective cross-sectional area [m²]
        area: f64,
        /// Drag coefficient (typically 2.0–2.2)
        cd: f64,
        /// Radiation pressure coefficient (1.0 absorber, 2.0 reflector).
        ///
        /// A lumped coefficient is the whole model here, not the shorthand it
        /// would be for a flat panel: an isotropic surface presents the same
        /// cross-section whatever the Sun direction, so there is no incidence
        /// angle for the specular and diffuse terms of [`PanelOptics`] to
        /// depend on.
        cr: f64,
    },
    /// Flat-panel model: attitude-dependent.
    Panels(Vec<SurfacePanel>),
}

impl SpacecraftShape {
    /// Create a sphere (attitude-independent) shape with the given parameters.
    ///
    /// # Panics
    /// Panics if `area` is not positive or if `cd`/`cr` are negative.
    pub fn sphere(area: f64, cd: f64, cr: f64) -> Self {
        assert!(area > 0.0, "area must be positive");
        assert!(cd >= 0.0, "cd must be non-negative");
        assert!(cr >= 0.0, "cr must be non-negative");
        Self::Sphere { area, cd, cr }
    }

    /// Create a panel model from an arbitrary set of panels.
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. `SurfacePanel`'s constructors establish
    /// both, but its fields are public, so a struct literal can reach here
    /// without them.
    pub fn panels(panels: Vec<SurfacePanel>) -> Self {
        let shape = Self::Panels(panels);
        shape.assert_normals_are_unit();
        shape.assert_outlines_are_consistent();
        shape
    }

    /// Check the unit-normal invariant the panel force models rely on.
    ///
    /// Both force models project onto the panel normal, and panel SRP is cubic
    /// in its length through the specular term, so a non-unit normal inflates
    /// the force silently. The check belongs at model construction rather than
    /// inside the force law: a model owns its shape and cannot be mutated
    /// afterwards, so once past here the invariant holds for the model's life —
    /// and the force law runs at every integrator stage, where a square root
    /// per panel would be pure overhead.
    ///
    /// # Panics
    /// Panics if any panel normal is not unit length. `Sphere` never panics.
    pub(crate) fn assert_normals_are_unit(&self) {
        let Self::Panels(panels) = self else {
            return;
        };
        for (i, panel) in panels.iter().enumerate() {
            let len = panel.normal.magnitude();
            assert!(
                (len - 1.0).abs() < 1e-9,
                "Panel {i} normal must be unit length, got |n|={len}. \
                 `SurfacePanel::at_com` normalises; a struct literal does not."
            );
        }
    }

    /// Check what [`SurfacePanel::rectangle`] asserts, for panels that did not
    /// come through it.
    ///
    /// `outline` is a public field and `PanelOutline::Rectangle`'s fields are
    /// public with it, so a struct literal can set an axis parallel to the
    /// normal, a non-positive extent, or an area that does not match the
    /// outline. A parallel axis makes both in-plane directions degenerate and
    /// the containment test answers for a line rather than a rectangle, which
    /// reports occlusion where there is none.
    ///
    /// # Panics
    /// Panics with the offending panel's index.
    pub(crate) fn assert_outlines_are_consistent(&self) {
        let Self::Panels(panels) = self else {
            return;
        };
        for (i, panel) in panels.iter().enumerate() {
            // The two cases are separate on purpose. No outline is nothing to
            // check — a panel without a boundary takes no part in occlusion.
            // A shape this does not recognise is a gap, so the enum is
            // destructured irrefutably: adding a variant stops this compiling
            // rather than skipping the panel.
            let Some(outline) = panel.outline else {
                continue;
            };
            let PanelOutline::Rectangle {
                half_extent,
                in_plane_x,
            } = outline;
            assert!(
                half_extent.iter().all(|h| h.is_finite() && *h > 0.0),
                "panel {i}: outline half-extents must be positive and finite, got {half_extent:?}"
            );
            let area = half_extent[0] * half_extent[1] * 4.0;
            assert!(
                area.is_finite() && area > 0.0,
                "panel {i}: outline half-extents {half_extent:?} give an area of {area}"
            );
            // Finite first: with `panel.area` infinite both sides of the
            // relative comparison are infinite, and `inf <= inf` holds.
            assert!(
                panel.area.is_finite() && panel.area > 0.0,
                "panel {i}: area must be positive and finite, got {}",
                panel.area
            );
            assert!(
                (area - panel.area).abs() <= 1e-9 * area.max(panel.area),
                "panel {i}: area {} does not match the outline's {area}",
                panel.area
            );
            let len = in_plane_x.magnitude();
            assert!(
                len.is_finite() && (len - 1.0).abs() < 1e-9,
                "panel {i}: outline in_plane_x must be unit length, got {len}"
            );
            let cos = panel.normal.dot(&in_plane_x);
            assert!(
                cos.abs() < 1e-9,
                "panel {i}: outline in_plane_x must be perpendicular to the normal, got n·x = {cos}"
            );
        }
    }

    /// Create a cube with the given half-size, drag coefficient, and optical
    /// properties, shared by all six faces.
    ///
    /// Generates 6 panels (±x, ±y, ±z), each `2 * half_size` on a side, with the
    /// centre of pressure at the face centre (`half_size` m from CoM along the
    /// face normal). The faces carry their outline, so a panel added beside the
    /// cube — a solar array, say — can be found completely covered by one.
    ///
    /// # Panics
    /// Panics unless `half_size` is positive and finite, and unless the area it
    /// implies is too — it builds the faces through [`SurfacePanel::rectangle`],
    /// so `half_size` of `1e200` overflows and `1e-300` underflows.
    pub fn cube(half_size: f64, cd: f64, optics: PanelOptics) -> Self {
        let face = |normal: Vector3<f64>, in_plane_x: Vector3<f64>| {
            SurfacePanel::rectangle([half_size, half_size], in_plane_x, normal, cd, optics)
                .with_cp_offset(normal * half_size)
        };
        let (x, y, z) = (Vector3::x(), Vector3::y(), Vector3::z());
        let panels = vec![
            face(x, y),
            face(-x, y),
            face(y, z),
            face(-y, z),
            face(z, x),
            face(-z, x),
        ];
        Self::Panels(panels)
    }
}

/// How far in front a shadow caster has to stand to count [m].
///
/// Absolute, and compared against a distance along the incoming direction, so
/// for a caster nearly edge-on to it the same value admits a much smaller gap
/// between the planes (`gap = t · cos θ`). Coplanar plates built through
/// [`SurfacePanel::rectangle`] land within 2e-14 m of each other even at 100 m
/// from the CoM, so this leaves several decades of margin — and the force it
/// could wrongly drop scales with `cos θ`, which is what makes the asymmetry
/// harmless. It is what stops the two faces of a thin plate, which share a
/// plane, from shadowing each other.
const OCCLUSION_DEPTH_EPS: f64 = 1e-9;

/// How far outside an outline a corner may fall and still count as inside, as
/// a fraction of the half-extent it is compared against.
///
/// One part in a billion, which is a nanometre on a one-metre panel. A corner
/// grazing an edge needs it: with an in-plane axis whose components are inexact,
/// a third of the corners of an exactly-covering caster land outside by up to
/// 4.8e-16 of the half-extent (measured over 3600 axis angles), so an exact
/// comparison would report a shadow or no shadow depending on the angle.
///
/// A fraction rather than a length because `rectangle` accepts half-extents
/// down to `1e-150`, and an absolute nanometre would describe such a panel as
/// 141 orders of magnitude wider than it is, letting it swallow targets it
/// could never cover. Scaling down instead makes a panel that small fail
/// containment, which reports no shadow — the direction that keeps a real force
/// rather than removing one.
const OUTLINE_EDGE_TOLERANCE: f64 = 1e-9;

/// Whether every corner of `panel` lies behind one other panel, seen from
/// `upstream`.
///
/// `upstream` points from the spacecraft toward where the light or the flow
/// comes from: SRP passes `s_body`, the direction of the Sun, and drag passes
/// `v̂_body`, the direction the spacecraft is heading through the atmosphere,
/// which is where the gas arrives from. It must be unit length.
///
/// `others` may contain `panel` itself, and no index is needed to exclude it: a
/// corner sitting on its own panel's plane gives `t = 0`, which the depth test
/// rejects. Taking an index instead would put an obligation on the caller that
/// nothing can check — a wrong one silently drops a real shadow.
///
/// A panel without an outline neither casts a shadow nor receives one, so a
/// fleet of area-only panels behaves exactly as it did before outlines existed.
///
/// Only single-panel occlusion is detected. A panel covered by two others
/// between them still counts as lit — the case that needs it is a segmented
/// structure standing in front of one face, which the panel list can describe
/// but this test cannot see.
pub(crate) fn is_fully_occluded(
    panel: &SurfacePanel,
    others: &[SurfacePanel],
    upstream: &Vector3<f64>,
) -> bool {
    let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
    let Some(corners) = panel.corners_into(&mut buf) else {
        return false;
    };
    others
        .iter()
        .any(|other| blocks_all(corners, other, upstream))
}

/// Whether `caster` stands in front of every one of `corners`.
///
/// Each corner sends a ray toward `upstream`; the corner is covered when the ray
/// crosses `caster`'s plane in front of it and lands within `caster`'s outline.
/// Comparing silhouettes on a plane instead would call a caster that tilts
/// through the panel a full shadow: its centre can sit in front while one edge
/// is behind, and a projection cannot tell.
fn blocks_all(corners: &[Vector3<f64>], caster: &SurfacePanel, upstream: &Vector3<f64>) -> bool {
    // Two cases need no guard of their own, measured by removing them:
    //
    // A caster with no outline: `outline_contains` answers `false`.
    //
    // A caster edge-on to the incoming direction: `denom` is zero or nearly so,
    // `t` runs to infinity, and the hit lands outside the outline. Special-casing
    // it would only avoid the arithmetic, not change the answer.
    let denom = caster.normal.dot(upstream);

    corners.iter().all(|corner| {
        // Where the ray from `corner` toward `upstream` meets the caster plane.
        let t = caster.normal.dot(&(caster.cp_offset - corner)) / denom;
        if t <= OCCLUSION_DEPTH_EPS {
            return false; // level with the corner, or behind it
        }
        caster.outline_contains(&(corner + upstream * t))
    })
}

/// Attitude-dependent drag model using flat surface panels.
///
/// Implements [`Model`] to produce both translational acceleration and
/// aerodynamic torque from per-panel drag forces.  For the [`SpacecraftShape::Sphere`]
/// variant, behaves identically to the scalar `AtmosphericDrag`.
///
/// The frame parameter `F` selects — exactly as for
/// [`AtmosphericDrag`](crate::perturbations::AtmosphericDrag) — how positions
/// are converted to geodetic coordinates for the density lookup and which spin
/// axis the atmosphere co-rotates about: `SimpleEci` uses the ERA-only ECEF
/// rotation and a `+Z` spin axis, `Gcrs` the full IAU 2006 CIO chain and the
/// true CIP spin axis.
pub struct PanelDrag<F: EarthFixedTransform = frame::SimpleEci> {
    shape: SpacecraftShape,
    atmosphere: Box<dyn AtmosphereModel>,
    body: Option<KnownBody>,
    body_radius: f64,
    omega_body: f64,
    /// EOP storage for the frame adapter. `()` for `SimpleEci`.
    eop: F::EopStorage,
}

impl PanelDrag<frame::SimpleEci> {
    /// Create a panel drag model for Earth orbit in the default `SimpleEci` frame.
    ///
    /// Uses piecewise exponential atmosphere and WGS-84 geodetic altitude by default.
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn for_earth(shape: SpacecraftShape) -> Self {
        Self::for_earth_in_frame(shape, ())
    }
}

impl<F: EarthFixedTransform> PanelDrag<F> {
    /// Create a panel drag model for Earth orbit in an arbitrary inertial frame
    /// `F`, with that frame's EOP storage (`()` for `SimpleEci`).
    ///
    /// # Panics
    /// Panics unless every panel normal is unit length and every outline is
    /// consistent with its panel — positive finite extents whose product is a
    /// finite non-zero area matching `area`, and a unit in-plane axis
    /// perpendicular to the normal. Both are re-checked here because
    /// `SurfacePanel`'s fields are public, so a struct literal can bypass the
    /// constructors that establish them.
    pub fn for_earth_in_frame(shape: SpacecraftShape, eop: F::EopStorage) -> Self {
        shape.assert_normals_are_unit();
        shape.assert_outlines_are_consistent();
        Self {
            shape,
            atmosphere: Box::new(Exponential),
            body: Some(KnownBody::Earth),
            body_radius: R_EARTH,
            omega_body: OMEGA_EARTH,
            eop,
        }
    }

    /// Replace the atmospheric density model (builder pattern).
    pub fn with_atmosphere(mut self, model: Box<dyn AtmosphereModel>) -> Self {
        self.atmosphere = model;
        self
    }
}

impl<F: EarthFixedTransform> PanelDrag<F> {
    /// Check if the position is inside the central body.
    /// Is the state below the surface (drag is meaningless there)?
    ///
    /// For Earth this asks the frame for the geodetic height rather than
    /// applying the WGS-84 semi-axes to the frame's own axes: the ellipsoid is
    /// axisymmetric about Earth's *polar* axis, which `SimpleEci`'s `+Z` is by
    /// definition but `Gcrs`'s is not (they differ by precession/nutation), so
    /// the raw-axis test misplaces the boundary by tens of metres there. Other
    /// bodies keep the spherical test.
    fn is_below_surface(&self, geodetic: Option<&Geodetic>, position: &Vector3<f64>) -> bool {
        match (self.body, geodetic) {
            (Some(KnownBody::Earth), Some(geodetic)) => geodetic.altitude < 0.0,
            _ => position.magnitude() < self.body_radius,
        }
    }

    /// Compute relative velocity accounting for atmosphere co-rotation [km/s].
    ///
    /// Ω is along the central body's spin axis: for Earth the axis
    /// [`EarthRotationPole`](arika::earth::EarthRotationPole) expresses in the
    /// integration frame `F` (`+Z` for
    /// `SimpleEci`, the true CIP for `Gcrs`), otherwise the frame Z axis with
    /// that body's rate — the same treatment as
    /// [`AtmosphericDrag`](crate::perturbations::AtmosphericDrag).
    fn relative_velocity_from_orbit(
        &self,
        orbit: &crate::OrbitalState<F>,
        utc: &Epoch<arika::epoch::Utc>,
    ) -> Vector3<f64> {
        let omega = match self.body {
            Some(KnownBody::Earth) => F::earth_pole(utc).into_inner() * self.omega_body,
            _ => Vector3::new(0.0, 0.0, self.omega_body),
        };
        *orbit.velocity() - omega.cross(orbit.position())
    }

    /// Compute loads from full state (using capability trait methods).
    pub(crate) fn loads_from_state(
        &self,
        orbit: &crate::OrbitalState<F>,
        body_to_inertial: arika::frame::Rotation<arika::frame::Body, F>,
        mass: f64,
        epoch: Option<&Epoch<arika::epoch::Utc>>,
    ) -> ExternalLoads<F> {
        // TODO: OrbitalSystem::epoch_0 を required にすれば dummy は不要
        let pos_vec = orbit.position_vec();
        let dummy_epoch = arika::epoch::Epoch::from_jd(2451545.0);
        let utc = epoch.unwrap_or(&dummy_epoch);
        // `EarthFixedTransform` supplies the ECI→ECEF conversion for the frame:
        // the ERA-only rotation for `SimpleEci` (identical to the legacy
        // `Epoch::gmst`, which is the same ERA formula) and the full IAU 2006
        // chain for `Gcrs`. Computed before the surface check so that check can
        // use the geodetic height instead of the frame's raw axes.
        let geodetic = F::to_geodetic(&pos_vec, &EarthOrientation::new(*utc, &self.eop));

        // Below the surface → zero
        if self.is_below_surface(Some(&geodetic), orbit.position()) {
            return ExternalLoads::zeros();
        }
        let rho = self.atmosphere.density(&AtmosphereInput { geodetic, utc });
        if rho == 0.0 {
            return ExternalLoads::zeros();
        }

        // Relative velocity (inertial frame, km/s)
        let v_rel = self.relative_velocity_from_orbit(orbit, utc);
        let v_rel_mag_km = v_rel.magnitude();
        if v_rel_mag_km < 1e-10 {
            return ExternalLoads::zeros();
        }

        match &self.shape {
            SpacecraftShape::Sphere { area, cd, .. } => {
                // F = -½ ρ Cd A |v_rel| v_rel  [N], where v_rel is in m/s
                // a = F/m [m/s²]
                let v_rel_m = v_rel * 1000.0; // km/s → m/s
                let v_rel_mag_m = v_rel_m.magnitude();
                let a_drag_m = -0.5 * rho * cd * area / mass * v_rel_mag_m * v_rel_m;
                ExternalLoads {
                    acceleration_inertial: arika::frame::Vec3::from_raw(a_drag_m / 1000.0), // m/s² → km/s²
                    torque_body: arika::frame::Vec3::zeros(),
                    mass_rate: 0.0,
                }
            }
            SpacecraftShape::Panels(panels) => {
                // Transform flow direction to body frame
                let v_body = body_to_inertial
                    .inverse()
                    .transform(&arika::frame::Vec3::<F>::from_raw(v_rel))
                    .into_inner(); // km/s in body frame
                let v_body_m = v_body * 1000.0; // m/s
                let v_body_mag_m = v_body_m.magnitude();
                let v_hat_body = v_body_m / v_body_mag_m;

                let mut total_force_body = Vector3::zeros(); // N
                let mut total_torque_body = Vector3::zeros(); // N·m

                // The upwind side, which is both the side a lit panel faces
                // and the side an occluding panel has to stand on. `v_rel` is
                // the spacecraft's velocity *through* the atmosphere, so in the
                // body frame the gas arrives from `+v̂` and leaves along `-v̂`:
                // the face turned into it is the one whose normal has a
                // positive component along `v̂`. Orekit's paneled model selects
                // the same face (#416).
                let upstream = v_hat_body;
                for panel in panels {
                    // cos(θ) = n̂ · v̂: the panel has to face into the flow
                    let cos_theta = panel.normal.dot(&upstream).max(0.0);
                    if cos_theta <= 0.0 {
                        continue;
                    }
                    // A panel upwind of this one shields it. Panels without an
                    // outline never do, so an area-only fleet is unaffected.
                    if is_fully_occluded(panel, panels, &upstream) {
                        continue;
                    }

                    let a_proj = panel.area * cos_theta; // m²

                    // F = -½ ρ Cd A_proj |v|² v̂  [N]
                    let force =
                        -0.5 * rho * panel.cd * a_proj * v_body_mag_m * v_body_mag_m * v_hat_body;

                    total_force_body += force;
                    total_torque_body += panel.cp_offset.cross(&force);
                }

                // a_body [m/s²] → a_inertial [km/s²]
                let a_body = arika::frame::Vec3::from_raw(total_force_body / mass);
                let a_inertial = body_to_inertial.transform(&a_body) / 1000.0;

                ExternalLoads {
                    acceleration_inertial: a_inertial,
                    torque_body: arika::frame::Vec3::from_raw(total_torque_body),
                    mass_rate: 0.0,
                }
            }
        }
    }
}

// Frame-generic panel drag: the geodetic lookup goes through
// `EarthFixedTransform` and the co-rotation about `EarthRotationPole`, so the
// model is valid in any inertial frame that provides them (a frame without the
// impl is a compile error). See #151.
impl<F: EarthFixedTransform, S: HasFrame<Frame = F> + HasAttitude + HasOrbit + HasMass> Model<S>
    for PanelDrag<F>
{
    fn name(&self) -> &str {
        "panel_drag"
    }

    fn eval(&self, _t: f64, state: &S, epoch: Option<&Epoch>) -> ExternalLoads<F> {
        self.loads_from_state(
            state.orbit(),
            state.attitude_to_inertial(),
            state.mass(),
            epoch,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpacecraftState;

    // SurfacePanel

    #[test]
    fn at_com_zero_cp_offset() {
        let p = SurfacePanel::at_com(
            2.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        assert_eq!(p.cp_offset, Vector3::zeros());
    }

    #[test]
    fn at_com_normalises_normal() {
        let p = SurfacePanel::at_com(
            1.0,
            Vector3::new(3.0, 4.0, 0.0),
            2.0,
            PanelOptics::absorber(),
        );
        let expected = Vector3::new(0.6, 0.8, 0.0);
        assert!(
            (p.normal - expected).magnitude() < 1e-15,
            "Normal should be normalised, got {:?}",
            p.normal
        );
    }

    #[test]
    fn at_com_already_unit() {
        let n = Vector3::new(0.0, 0.0, 1.0);
        let p = SurfacePanel::at_com(5.0, n, 2.2, PanelOptics::absorber());
        assert!((p.normal - n).magnitude() < 1e-15);
    }

    #[test]
    #[should_panic]
    fn at_com_zero_normal_panics() {
        SurfacePanel::at_com(1.0, Vector3::zeros(), 2.0, PanelOptics::absorber());
    }

    #[test]
    fn at_com_preserves_area_and_cd() {
        let p = SurfacePanel::at_com(
            3.5,
            Vector3::new(0.0, 1.0, 0.0),
            2.1,
            PanelOptics::absorber(),
        );
        assert!((p.area - 3.5).abs() < 1e-15);
        assert!((p.cd - 2.1).abs() < 1e-15);
    }

    #[test]
    fn at_com_carries_the_optics_it_is_given() {
        let optics = PanelOptics::new(0.3, 0.2);
        let p = SurfacePanel::at_com(3.5, Vector3::new(0.0, 1.0, 0.0), 2.1, optics);
        assert_eq!(p.optics, optics);
    }

    #[test]
    fn with_optics_replaces_the_panel_optics() {
        // The builder is how a face of a `cube` gets its own surface.
        let replacement = PanelOptics::new(0.6, 0.1);
        let p = SurfacePanel::at_com(
            3.5,
            Vector3::new(0.0, 1.0, 0.0),
            2.1,
            PanelOptics::absorber(),
        )
        .with_optics(replacement);
        assert_eq!(p.optics, replacement);
    }

    // PanelOptics

    #[test]
    #[should_panic(expected = "normal must be unit length")]
    fn panels_rejects_a_non_unit_normal() {
        // A struct literal skips `at_com`'s normalisation, and the SRP force is
        // cubic in |n| through its specular term, so the shape constructor is
        // where that has to be caught.
        SpacecraftShape::panels(vec![SurfacePanel {
            area: 10.0,
            normal: Vector3::new(1.0, 0.0, 1.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: None,
        }]);
    }

    /// `SpacecraftShape::Panels` is a public variant, so a shape can reach the
    /// drag model without passing through `SpacecraftShape::panels`. The drag
    /// projection relies on the unit normal too, so its constructor has to
    /// check as well.
    #[test]
    #[should_panic(expected = "normal must be unit length")]
    fn panel_drag_rejects_a_non_unit_normal() {
        let shape = SpacecraftShape::Panels(vec![SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 0.0, 3.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::zeros(),
            outline: None,
        }]);
        PanelDrag::for_earth(shape);
    }

    #[test]
    fn panels_accepts_normals_at_com_produced() {
        let shape = SpacecraftShape::panels(vec![SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 1.0),
            2.2,
            PanelOptics::absorber(),
        )]);
        let SpacecraftShape::Panels(panels) = shape else {
            panic!("expected a panel shape");
        };
        assert!((panels[0].normal.magnitude() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn panel_optics_absorptivity_is_the_remainder() {
        let o = PanelOptics::new(0.25, 0.15);
        assert!((o.absorptivity() - 0.6).abs() < 1e-15);
        assert!((o.absorptivity() + o.specular() + o.diffuse() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn absorptivity_is_never_negative() {
        // Subtracting the two reflectivities one at a time rounds twice, and
        // for 2077 of these 10001 pairs the result lands just below zero
        // (`1.0 - 0.9 - 0.1` is -2.8e-17). Subtracting their sum inherits
        // `new`'s check that the sum is at most 1, so it cannot.
        for i in 0..=10_000u32 {
            let specular = f64::from(i) / 10_000.0;
            let diffuse = f64::from(10_000 - i) / 10_000.0;
            let optics = PanelOptics::new(specular, diffuse);
            assert!(
                optics.absorptivity() >= 0.0,
                "α should not go negative: specular={specular}, diffuse={diffuse}, α={:e}",
                optics.absorptivity()
            );
        }
    }

    #[test]
    #[should_panic(expected = "non-negative")]
    fn panel_optics_rejects_negative_reflectivity() {
        PanelOptics::new(-0.1, 0.2);
    }

    #[test]
    #[should_panic(expected = "sum to at most 1")]
    fn panel_optics_rejects_sum_above_one() {
        PanelOptics::new(0.7, 0.5);
    }

    #[test]
    #[should_panic(expected = "finite")]
    fn panel_optics_rejects_non_finite() {
        PanelOptics::new(f64::NAN, 0.2);
    }

    // SpacecraftShape::sphere

    #[test]
    fn sphere_variant() {
        let shape = SpacecraftShape::sphere(10.0, 2.2, 1.5);
        match shape {
            SpacecraftShape::Sphere { area, cd, cr } => {
                assert!((area - 10.0).abs() < 1e-15);
                assert!((cd - 2.2).abs() < 1e-15);
                assert!((cr - 1.5).abs() < 1e-15);
            }
            _ => panic!("Expected Sphere variant"),
        }
    }

    #[test]
    #[should_panic(expected = "area must be positive")]
    fn sphere_zero_area_panics() {
        SpacecraftShape::sphere(0.0, 2.2, 1.5);
    }

    #[test]
    #[should_panic(expected = "cd must be non-negative")]
    fn sphere_negative_cd_panics() {
        SpacecraftShape::sphere(10.0, -1.0, 1.5);
    }

    #[test]
    #[should_panic(expected = "cr must be non-negative")]
    fn sphere_negative_cr_panics() {
        SpacecraftShape::sphere(10.0, 2.2, -0.1);
    }

    // SpacecraftShape::panels

    #[test]
    fn panels_stores_panels() {
        let panels = vec![
            SurfacePanel::at_com(
                1.0,
                Vector3::new(1.0, 0.0, 0.0),
                2.0,
                PanelOptics::absorber(),
            ),
            SurfacePanel::at_com(
                2.0,
                Vector3::new(0.0, 1.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            ),
        ];
        let shape = SpacecraftShape::panels(panels.clone());
        match shape {
            SpacecraftShape::Panels(p) => {
                assert_eq!(p.len(), 2);
                assert!((p[0].area - 1.0).abs() < 1e-15);
                assert!((p[1].area - 2.0).abs() < 1e-15);
            }
            _ => panic!("Expected Panels variant"),
        }
    }

    // SpacecraftShape::cube

    #[test]
    fn cube_has_six_panels() {
        let shape = SpacecraftShape::cube(0.5, 2.2, PanelOptics::absorber());
        match &shape {
            SpacecraftShape::Panels(panels) => {
                assert_eq!(panels.len(), 6, "Cube should have 6 faces");
            }
            _ => panic!("Expected Panels variant"),
        }
    }

    #[test]
    fn cube_face_area() {
        let half = 0.5;
        let expected_area = (2.0 * half) * (2.0 * half); // 1.0 m²
        let shape = SpacecraftShape::cube(half, 2.2, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.area - expected_area).abs() < 1e-15,
                    "Panel {i} area: expected {expected_area}, got {}",
                    p.area
                );
            }
        }
    }

    #[test]
    fn cube_normals_are_unit() {
        let shape = SpacecraftShape::cube(1.0, 2.0, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.normal.magnitude() - 1.0).abs() < 1e-15,
                    "Panel {i} normal not unit: magnitude = {}",
                    p.normal.magnitude()
                );
            }
        }
    }

    #[test]
    fn cube_normals_are_axis_aligned() {
        let shape = SpacecraftShape::cube(1.0, 2.0, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            let normals: Vec<_> = panels.iter().map(|p| p.normal).collect();
            let expected = [
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(-1.0, 0.0, 0.0),
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(0.0, -1.0, 0.0),
                Vector3::new(0.0, 0.0, 1.0),
                Vector3::new(0.0, 0.0, -1.0),
            ];
            for (i, (n, e)) in normals.iter().zip(expected.iter()).enumerate() {
                assert!(
                    (n - e).magnitude() < 1e-15,
                    "Panel {i}: expected normal {e:?}, got {n:?}"
                );
            }
        }
    }

    #[test]
    fn cube_cp_at_face_centre() {
        let half = 0.75;
        let shape = SpacecraftShape::cube(half, 2.0, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                // CP should be at half_size along the normal direction
                let expected_cp = p.normal * half;
                assert!(
                    (p.cp_offset - expected_cp).magnitude() < 1e-15,
                    "Panel {i}: expected CP {expected_cp:?}, got {:?}",
                    p.cp_offset
                );
            }
        }
    }

    #[test]
    fn cube_all_same_cd() {
        let cd = 2.2;
        let shape = SpacecraftShape::cube(0.5, cd, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            for (i, p) in panels.iter().enumerate() {
                assert!(
                    (p.cd - cd).abs() < 1e-15,
                    "Panel {i} cd: expected {cd}, got {}",
                    p.cd
                );
            }
        }
    }

    #[test]
    fn cube_opposite_normals_cancel() {
        let shape = SpacecraftShape::cube(1.0, 2.0, PanelOptics::absorber());
        if let SpacecraftShape::Panels(panels) = &shape {
            let normal_sum: Vector3<f64> = panels.iter().map(|p| p.normal).sum();
            assert!(
                normal_sum.magnitude() < 1e-14,
                "Opposite normals should cancel: sum = {normal_sum:?}"
            );
        }
    }

    // PanelDrag

    #[test]
    fn panel_drag_name() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(10.0, 2.2, 1.5));
        assert_eq!(Model::<SpacecraftState>::name(&drag), "panel_drag");
    }

    #[test]
    fn panel_drag_for_earth_defaults() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(10.0, 2.2, 1.5));
        assert_eq!(drag.body, Some(KnownBody::Earth));
        assert!((drag.body_radius - R_EARTH).abs() < 1e-10);
        assert!((drag.omega_body - OMEGA_EARTH).abs() < 1e-15);
    }

    #[test]
    fn panel_drag_with_atmosphere_builder() {
        use tobari::HarrisPriester;

        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(10.0, 2.2, 1.5))
            .with_atmosphere(Box::new(HarrisPriester::new()));
        // Should compile and not panic — atmosphere model replaced
        assert_eq!(Model::<SpacecraftState>::name(&drag), "panel_drag");
    }

    // PanelDrag loads() — shared helpers

    use crate::OrbitalState;
    use crate::attitude::AttitudeState;
    use nalgebra::Vector4;

    fn iss_state() -> SpacecraftState {
        let r = R_EARTH + 400.0;
        let v = (arika::earth::MU / r).sqrt();
        SpacecraftState {
            orbit: OrbitalState::new(Vector3::new(r, 0.0, 0.0), Vector3::new(0.0, v, 0.0)),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        }
    }

    // Frame-generalization characterization (#151)
    //
    // Pinned `SimpleEci` numbers at a fully 3D state (off-axis position,
    // velocity and attitude) so that opening `PanelDrag` to a generic inertial
    // frame `F` cannot change them. In particular the geodetic conversion moves
    // from the legacy `Epoch::gmst` to `EarthFixedTransform::to_geodetic`, which
    // is the same ERA formula, and the co-rotation axis moves from a literal
    // `+Z` to `EarthRotationPole::earth_pole` (`+Z` for `SimpleEci`).

    fn snapshot_state() -> SpacecraftState {
        SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(4000.0, -5000.0, 2500.0),
                Vector3::new(1.0, 2.0, 7.0),
            ),
            attitude: AttitudeState::new(
                nalgebra::UnitQuaternion::from_axis_angle(
                    &nalgebra::Unit::new_normalize(Vector3::new(0.3, -0.5, 0.8)),
                    0.7,
                ),
                Vector3::new(0.01, -0.02, 0.03),
            ),
            mass: 50.0,
        }
    }

    fn snapshot_epoch() -> Epoch {
        Epoch::from_gregorian(2024, 3, 20, 12, 0, 0.0)
    }

    /// Characterization: pinned `SimpleEci` panel-drag loads for a cube.
    #[test]
    fn panels_simple_eci_loads_snapshot() {
        let drag = PanelDrag::for_earth(SpacecraftShape::cube(0.5, 2.2, PanelOptics::absorber()));
        let loads = drag.eval(0.0, &snapshot_state(), Some(&snapshot_epoch()));
        let expected_a = Vector3::new(
            -1.112965959433058e-10,
            -2.992310652421664e-10,
            -1.2261304328438772e-9,
        );
        let a = loads.acceleration_inertial.into_inner();
        let tau = loads.torque_body.into_inner();
        assert!(
            (a - expected_a).magnitude() <= 1e-12 * expected_a.magnitude().max(1.0),
            "SimpleEci panel drag acceleration changed: {a:?}"
        );
        // A symmetric cube's opposite faces cancel, so the torque is zero up to
        // rounding (~1e-21 here). Assert that invariant rather than pinning the
        // residue: a snapshot of 1e-21 with an absolute floor would pass for
        // zero and for a value nine orders too large alike. The torque path
        // itself is covered by `asymmetric_panels_torque_*` below.
        assert!(
            tau.magnitude() < 1e-18 * a.magnitude().max(1.0),
            "a symmetric cube must produce no net torque, got {tau:?}"
        );
    }

    /// Characterization: pinned `SimpleEci` panel-drag loads for the
    /// attitude-independent sphere branch.
    #[test]
    fn sphere_simple_eci_loads_snapshot() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(1.0, 2.2, 1.5));
        let loads = drag.eval(0.0, &snapshot_state(), Some(&snapshot_epoch()));
        let expected_a = Vector3::new(
            -6.98845822315769e-11,
            -1.8789108335182917e-10,
            -7.699032691383112e-10,
        );
        let a = loads.acceleration_inertial.into_inner();
        assert!(
            (a - expected_a).magnitude() <= 1e-12 * expected_a.magnitude().max(1.0),
            "SimpleEci sphere drag acceleration changed: {a:?}"
        );
    }

    /// An asymmetric panel fixture: one panel whose centre of pressure is offset
    /// perpendicular to its own normal, so `cp × F` cannot cancel and the body
    /// torque is materially non-zero. A symmetric cube is useless for this — its
    /// faces cancel to ~1e-21, and any tolerance loose enough to admit that
    /// would also admit zero.
    fn asymmetric_panels() -> SpacecraftShape {
        // All four of ±x, ±y, so whichever way the flow points in the body frame
        // at least one panel faces it; the offsets are perpendicular to each
        // normal and deliberately unequal between opposite faces, so `cp × F`
        // does not cancel the way a symmetric cube's does.
        SpacecraftShape::Panels(vec![
            SurfacePanel {
                area: 2.0,
                normal: Vector3::x(),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(0.0, 1.5, 0.0),
                outline: None,
            },
            SurfacePanel {
                area: 2.0,
                normal: -Vector3::x(),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(0.0, 0.4, 0.0),
                outline: None,
            },
            SurfacePanel {
                area: 0.5,
                normal: Vector3::y(),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(0.0, 0.0, -0.8),
                outline: None,
            },
            SurfacePanel {
                area: 0.5,
                normal: -Vector3::y(),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(0.9, 0.0, 0.0),
                outline: None,
            },
        ])
    }

    /// The sphere branch never touches the body frame, so it cannot cover the
    /// two rotations this change made frame-generic (the `body_to_inertial`
    /// rotation the model is handed: inverted on the way in, applied as-is on
    /// the way out). Exercise the panel branch in
    /// `Gcrs` at a non-identity attitude with an asymmetric fixture, and
    /// reconstruct both outputs: the inertial acceleration, which round-trips
    /// through the body frame, and the body torque, which stays there.
    #[test]
    fn gcrs_asymmetric_panels_torque_and_acceleration() {
        use crate::test_support::zero_eop;
        use arika::earth::EarthRotationPole;
        use arika::frame::Vec3;

        let epoch = snapshot_epoch();
        let simple = snapshot_state();
        let pos = *simple.orbit.position();
        let vel = *simple.orbit.velocity();
        let attitude = simple.attitude.clone();
        let state = SpacecraftState::<frame::Gcrs> {
            orbit: OrbitalState::<frame::Gcrs>::new_in_frame(pos, vel),
            attitude: attitude.clone(),
            mass: simple.mass,
        };

        let drag = PanelDrag::<frame::Gcrs>::for_earth_in_frame(asymmetric_panels(), zero_eop());
        let loads = drag.eval(0.0, &state, Some(&epoch));
        let got_a = loads.acceleration_inertial.into_inner();
        let got_tau = loads.torque_body.into_inner();

        // Reconstruct: CIP co-rotation, Gcrs geodetic density, then the same
        // panel sum done in the body frame.
        let pole = <frame::Gcrs as EarthRotationPole>::earth_pole(&epoch).into_inner();
        let v_rel = vel - (pole * OMEGA_EARTH).cross(&pos);
        let geodetic = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &Vec3::from_raw(pos),
            &EarthOrientation::new(epoch, &zero_eop()),
        );
        let rho = Exponential.density(&AtmosphereInput {
            geodetic,
            utc: &epoch,
        });
        assert!(rho > 0.0, "expected non-zero density");

        let v_body_m = attitude
            .rotation_tagged_as::<frame::Gcrs>()
            .inverse()
            .transform(&Vec3::<frame::Gcrs>::from_raw(v_rel))
            .into_inner()
            * 1000.0;
        let v_mag = v_body_m.magnitude();
        let v_hat = v_body_m / v_mag;
        let mut force_body = Vector3::zeros();
        let mut torque_body = Vector3::zeros();
        for panel in match &asymmetric_panels() {
            SpacecraftShape::Panels(panels) => panels.clone(),
            _ => unreachable!("asymmetric_panels is a Panels shape"),
        } {
            let cos_theta = panel.normal.dot(&v_hat).max(0.0);
            if cos_theta <= 0.0 {
                continue;
            }
            let force = -0.5 * rho * panel.cd * (panel.area * cos_theta) * v_mag * v_mag * v_hat;
            force_body += force;
            torque_body += panel.cp_offset.cross(&force);
        }
        let expected_a = attitude
            .rotation_tagged_as::<frame::Gcrs>()
            .transform(&Vec3::from_raw(force_body / simple.mass))
            .into_inner()
            / 1000.0;

        // The fixture must actually produce a torque, or the comparison below
        // would hold vacuously.
        assert!(
            torque_body.magnitude() > 1e-9,
            "the asymmetric fixture must produce a materially non-zero torque, got {torque_body:?}"
        );
        assert!(
            (got_a - expected_a).magnitude() <= 1e-12 * expected_a.magnitude(),
            "Gcrs panel acceleration must round-trip through the body frame: \
             {got_a:?} vs {expected_a:?}"
        );
        // Tolerance scaled to the torque itself, not to 1.0.
        assert!(
            (got_tau - torque_body).magnitude() <= 1e-12 * torque_body.magnitude(),
            "Gcrs panel torque must be the body-frame sum: {got_tau:?} vs {torque_body:?}"
        );
    }

    /// **Discriminating test (#151)**: in `Gcrs` both frame-dependent steps
    /// change — the geodetic lookup uses the full IAU 2006 chain and the
    /// atmosphere co-rotates about the true CIP instead of `+Z`. Pin that the
    /// acceleration matches a reconstruction using those, and differs from the
    /// `SimpleEci` result at the same raw state.
    #[test]
    fn gcrs_panel_drag_uses_the_iau2006_chain_and_cip() {
        use crate::test_support::zero_eop;
        use arika::earth::EarthRotationPole;
        use arika::frame::Vec3;

        let epoch = snapshot_epoch();
        let simple = snapshot_state();
        let pos = *simple.orbit.position();
        let vel = *simple.orbit.velocity();
        let state = SpacecraftState::<frame::Gcrs> {
            orbit: OrbitalState::<frame::Gcrs>::new_in_frame(pos, vel),
            attitude: simple.attitude.clone(),
            mass: simple.mass,
        };

        let drag = PanelDrag::<frame::Gcrs>::for_earth_in_frame(
            SpacecraftShape::sphere(1.0, 2.2, 1.5),
            zero_eop(),
        );
        let got = drag
            .eval(0.0, &state, Some(&epoch))
            .acceleration_inertial
            .into_inner();

        // Reconstruct: CIP co-rotation + Gcrs geodetic density.
        let pole = <frame::Gcrs as EarthRotationPole>::earth_pole(&epoch).into_inner();
        let v_rel = vel - (pole * OMEGA_EARTH).cross(&pos);
        let geodetic = <frame::Gcrs as EarthFixedTransform>::to_geodetic(
            &Vec3::from_raw(pos),
            &EarthOrientation::new(epoch, &zero_eop()),
        );
        let rho = Exponential.density(&AtmosphereInput {
            geodetic,
            utc: &epoch,
        });
        assert!(rho > 0.0, "expected non-zero density");
        let v_rel_m = v_rel * 1000.0;
        let expected =
            (-0.5 * rho * 2.2 * 1.0 / simple.mass * v_rel_m.magnitude() * v_rel_m) / 1000.0;
        assert!(
            (got - expected).magnitude() <= 1e-12 * expected.magnitude().max(1.0),
            "Gcrs panel drag must use the CIP + Gcrs geodetic: {got:?} vs {expected:?}"
        );

        let simple_eci = Vector3::new(
            -6.98845822315769e-11,
            -1.8789108335182917e-10,
            -7.699032691383112e-10,
        );
        assert!(
            (got - simple_eci).magnitude() > simple_eci.magnitude() * 1e-6,
            "Gcrs panel drag should differ from the SimpleEci result"
        );
    }

    // Sphere branch

    #[test]
    fn sphere_nonzero_drag_at_iss() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(5.0, 2.0, 1.5));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Sphere should produce non-zero drag at ISS altitude"
        );
    }

    #[test]
    fn sphere_zero_torque() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(5.0, 2.0, 1.5));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert_eq!(
            loads.torque_body.into_inner(),
            Vector3::zeros(),
            "Sphere should produce zero torque"
        );
    }

    #[test]
    fn sphere_attitude_independent() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(5.0, 2.0, 1.5));
        let s1 = iss_state();
        let mut s2 = iss_state();
        // Rotate 90° about z-axis: q = (cos45, 0, 0, sin45)
        let c = std::f64::consts::FRAC_PI_4.cos();
        let s = std::f64::consts::FRAC_PI_4.sin();
        s2.attitude.quaternion = Vector4::new(c, 0.0, 0.0, s);

        let l1 = drag.eval(0.0, &s1, None);
        let l2 = drag.eval(0.0, &s2, None);
        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "Sphere drag should not depend on attitude"
        );
    }

    #[test]
    fn sphere_opposes_velocity() {
        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(5.0, 2.0, 1.5));
        let loads = drag.eval(0.0, &iss_state(), None);
        // v_rel is mostly in +y → drag should be in -y
        assert!(loads.acceleration_inertial.y() < 0.0);
    }

    // Panels branch — acceleration

    #[test]
    fn panels_facing_flow_nonzero_drag() {
        // Single panel facing +y: the side the spacecraft is heading toward,
        // which is where the gas comes from
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert!(
            loads.acceleration_inertial.magnitude() > 0.0,
            "Panel facing flow should produce drag"
        );
    }

    #[test]
    fn panels_backface_zero_drag() {
        // Single panel facing -y: sheltered behind the body from the +y flow
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, -1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert_eq!(
            loads.acceleration_inertial.into_inner(),
            Vector3::zeros(),
            "Panel facing away from flow should produce zero drag"
        );
    }

    /// **#416 reproduction**: the gas hits the face whose normal points along
    /// `+v_rel`, and that is the face the drag has to load.
    ///
    /// `relative_velocity_from_orbit` returns the spacecraft's velocity through
    /// the atmosphere, so in the body frame the gas arrives from the `+v_rel`
    /// side and leaves along `-v_rel`. The face turned into it is the one whose
    /// normal has a positive component along `v_rel`; the opposite face sits
    /// behind the body and is swept by nothing.
    ///
    /// The two single-sided panels are asked separately because a
    /// front-back-symmetric body cannot show this: whichever face of an
    /// opposite pair is picked, the projected areas sum to the same value and
    /// the total force is identical. `cube_projected_area_analytic` and
    /// `two_sided_panel_no_dead_zone` are both blind to it for that reason.
    #[test]
    fn drag_loads_the_windward_face_not_the_sheltered_one() {
        let state = iss_state();
        let drag_magnitude = |normal: Vector3<f64>| {
            let panel = SurfacePanel::at_com(10.0, normal, 2.2, PanelOptics::absorber());
            PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]))
                .eval(0.0, &state, None)
                .acceleration_inertial
                .magnitude()
        };

        // Which side the gas comes from, taken from the model's own relative
        // velocity rather than assumed. `eval` with no epoch falls back to
        // J2000, and `SimpleEci` co-rotates about `+Z` at every epoch anyway.
        let probe = PanelDrag::for_earth(SpacecraftShape::panels(vec![SurfacePanel::at_com(
            1.0,
            Vector3::x(),
            2.2,
            PanelOptics::absorber(),
        )]));
        let v_rel = probe.relative_velocity_from_orbit(&state.orbit, &Epoch::from_jd(2451545.0));
        assert!(
            v_rel.dot(state.orbit.velocity()) > 0.0,
            "the spacecraft moves along its own velocity through the atmosphere, \
             so the gas arrives from that side: v_rel = {v_rel:?}"
        );
        let windward = v_rel.normalize();

        let hit = drag_magnitude(windward);
        let sheltered = drag_magnitude(-windward);
        assert!(
            hit > 0.0,
            "the face turned into the flow has to drag, got |a| = {hit:.4e} \
             while the sheltered face got {sheltered:.4e}"
        );
        assert_eq!(
            sheltered, 0.0,
            "the face behind the body is swept by nothing, got |a| = {sheltered:.4e} \
             while the windward face got {hit:.4e}"
        );
    }

    /// **#416 reproduction**: the shadow falls on the side the gas comes from.
    ///
    /// Occlusion follows this model's own idea of which side is upwind (#424),
    /// so it moves with the facing test above. A caster standing between the
    /// flow and a windward panel takes that panel's force away; the same caster
    /// on the sheltered side is downstream of the panel and takes nothing.
    ///
    /// Both directions are asserted: a shadow that falls on the wrong side
    /// passes a one-sided test by removing the force of whichever panel it
    /// happens to be behind.
    ///
    /// Which side, and only that. What the occlusion does once it has the side
    /// is what #424 left: a panel one other panel covers completely is dropped
    /// whole, and a partly covered one still produces its full force, which is
    /// an asymmetry of its own (#407). Both casters here cover their target
    /// completely, so this test says nothing about that case either way.
    #[test]
    fn only_a_caster_between_the_flow_and_the_panel_shades_it() {
        let state = iss_state();
        // `iss_state` at identity attitude travels along `+y` through the
        // atmosphere (see `drag_loads_the_windward_face_not_the_sheltered_one`),
        // so `+y` is the windward normal and the gas comes from `+y`.
        let windward = Vector3::y();
        let plate = |half: f64, cp: Vector3<f64>| {
            SurfacePanel::rectangle(
                [half, half],
                Vector3::x(),
                windward,
                2.2,
                PanelOptics::absorber(),
            )
            .with_cp_offset(cp)
        };
        let target = plate(1.0, Vector3::zeros());
        let upwind_caster = plate(2.0, windward * 2.0);
        let downwind_caster = plate(2.0, -windward * 2.0);

        let magnitude = |panels: Vec<SurfacePanel>| {
            PanelDrag::for_earth(SpacecraftShape::panels(panels))
                .eval(0.0, &state, None)
                .acceleration_inertial
                .magnitude()
        };

        let target_alone = magnitude(vec![target.clone()]);
        let caster_alone = magnitude(vec![upwind_caster.clone()]);
        assert!(
            target_alone > 0.0 && caster_alone > 0.0,
            "both plates face the flow on their own: {target_alone:.4e}, {caster_alone:.4e}"
        );

        let shaded = magnitude(vec![target.clone(), upwind_caster]);
        assert!(
            (shaded - caster_alone).abs() / caster_alone < 1e-12,
            "a caster upwind of the target must leave only its own force: \
             {shaded:.4e} vs {caster_alone:.4e}"
        );

        let unshaded = magnitude(vec![target, downwind_caster]);
        let sum = target_alone + caster_alone;
        assert!(
            (unshaded - sum).abs() / sum < 1e-12,
            "a caster downwind of the target shades nothing, so both forces stand: \
             {unshaded:.4e} vs {sum:.4e}"
        );
    }

    #[test]
    fn panels_drag_opposes_velocity() {
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);
        // Drag should oppose velocity (predominantly -y)
        assert!(
            loads.acceleration_inertial.y() < 0.0,
            "Panel drag should oppose velocity"
        );
    }

    #[test]
    fn panels_different_attitude_different_drag() {
        // This is the core coupling test: rotating the spacecraft changes the drag
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );

        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        // Identity attitude: panel normal +y faces into the +y flow → full drag
        let s1 = iss_state();
        let l1 = drag.eval(0.0, &s1, None);

        // Rotate 90° about z: panel normal rotates from +y to -x in inertial
        // → panel no longer faces the +y flow → different drag
        let mut s2 = iss_state();
        let c = std::f64::consts::FRAC_PI_4.cos();
        let s = std::f64::consts::FRAC_PI_4.sin();
        s2.attitude.quaternion = Vector4::new(c, 0.0, 0.0, s);

        let l2 = drag.eval(0.0, &s2, None);

        let diff = (l1.acceleration_inertial - l2.acceleration_inertial).magnitude();
        assert!(
            diff > 1e-15,
            "Different attitudes should produce different drag: diff = {diff:.3e}"
        );
    }

    #[test]
    fn panels_rotated_to_backface_zero() {
        // Panel faces +y in body frame, into the flow. Rotate 180° about z →
        // panel faces -y → backface
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let mut state = iss_state();
        // 180° rotation about z: q = (0, 0, 0, 1)
        state.attitude.quaternion = Vector4::new(0.0, 0.0, 0.0, 1.0);
        let loads = drag.eval(0.0, &state, None);

        assert!(
            loads.acceleration_inertial.magnitude() < 1e-15,
            "Panel rotated to backface should produce zero drag, got {:?}",
            loads.acceleration_inertial
        );
    }

    #[test]
    fn panels_above_atmosphere_zero() {
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::new(R_EARTH + 3000.0, 0.0, 0.0),
                Vector3::new(0.0, 5.0, 0.0),
            ),
            attitude: AttitudeState::identity(),
            mass: 500.0,
        };
        let loads = drag.eval(0.0, &state, None);
        assert_eq!(loads.acceleration_inertial.into_inner(), Vector3::zeros());
    }

    // Panels branch — torque

    #[test]
    fn panels_at_com_zero_torque() {
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert_eq!(
            loads.torque_body.into_inner(),
            Vector3::zeros(),
            "Panel at CoM should produce zero torque"
        );
    }

    #[test]
    fn panels_cp_offset_produces_torque() {
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 1.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(1.0, 0.0, 0.0), // 1 m offset in +x
            outline: None,
        };
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);

        assert!(
            loads.torque_body.magnitude() > 0.0,
            "Offset CP should produce non-zero torque"
        );
        // Force is in -y body frame (opposing +y flow), CP offset in +x
        // τ = r × F = (1,0,0) × (0,F_y,0) = (0,0,F_y) → z-component
        assert!(
            loads.torque_body.z().abs() > loads.torque_body.x().abs(),
            "Torque should be primarily about z-axis"
        );
    }

    #[test]
    fn panels_double_offset_double_torque() {
        let make_panel = |offset: f64| SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 1.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(offset, 0.0, 0.0),
            outline: None,
        };

        let drag1 = PanelDrag::for_earth(SpacecraftShape::panels(vec![make_panel(1.0)]));
        let drag2 = PanelDrag::for_earth(SpacecraftShape::panels(vec![make_panel(2.0)]));
        let state = iss_state();

        let t1 = drag1.eval(0.0, &state, None).torque_body;
        let t2 = drag2.eval(0.0, &state, None).torque_body;

        // τ = r × F, so doubling r doubles τ (force is the same)
        let ratio = t2.magnitude() / t1.magnitude();
        assert!(
            (ratio - 2.0).abs() < 1e-10,
            "Double offset should give double torque, got ratio {ratio}"
        );
    }

    // Equivalence: Sphere ↔ AtmosphericDrag

    #[test]
    fn sphere_matches_atmospheric_drag() {
        use crate::perturbations::AtmosphericDrag;
        // AtmosphericDrag uses ballistic_coeff = Cd*A/(2m)
        // Sphere with area=5.0, cd=2.0, mass=500: b = 2.0*5.0/(2*500) = 0.01
        let b = 0.01;
        let panel_drag = PanelDrag::for_earth(SpacecraftShape::sphere(5.0, 2.0, 1.5));
        let atmo_drag = AtmosphericDrag::for_earth(Some(b));

        let state = iss_state();
        let panel_loads = panel_drag.eval(0.0, &state, None);
        let atmo_accel = atmo_drag.acceleration(&state.orbit, None);

        let diff = (panel_loads.acceleration_inertial.into_inner() - atmo_accel).magnitude();
        assert!(
            diff < 1e-15,
            "Sphere PanelDrag should match AtmosphericDrag: diff = {diff:.3e}"
        );
    }

    // Equivalence: single panel at CoM (cos θ = 1) ↔ Sphere

    #[test]
    fn single_panel_facing_flow_matches_sphere() {
        // Single panel at CoM: A=10 m², Cd=2.2, normal facing flow
        // For sphere: b_eff = Cd * A / (2 * m) = 2.2 * 10 / (2 * 500) = 0.022
        // Panel force:  F = -½ ρ Cd A |v|² v̂    → a = F/m = -½ ρ Cd A/m |v|² v̂
        // Sphere:       a = -½ ρ Cd A/m |v|² v̂
        // These should be identical when cos θ = 1
        let area = 10.0;
        let cd = 2.2;

        // Panel facing +y, into the +y flow at identity attitude
        let panel = SurfacePanel::at_com(
            area,
            Vector3::new(0.0, 1.0, 0.0),
            cd,
            PanelOptics::absorber(),
        );
        let panel_drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let sphere_drag = PanelDrag::for_earth(SpacecraftShape::sphere(area, cd, 1.5));

        let state = iss_state();
        let panel_loads = panel_drag.eval(0.0, &state, None);
        let sphere_loads = sphere_drag.eval(0.0, &state, None);

        // The accelerations should be very close but not exactly identical because:
        // - Sphere uses v_rel in inertial frame directly
        // - Panels transform to body frame then back
        // With identity attitude, these should be numerically identical
        let diff =
            (panel_loads.acceleration_inertial - sphere_loads.acceleration_inertial).magnitude();
        let rel = diff / sphere_loads.acceleration_inertial.magnitude();
        assert!(
            rel < 1e-10,
            "Single panel (cos θ=1) should match sphere: relative diff = {rel:.3e}"
        );
    }

    // Torque tests

    #[test]
    fn cube_symmetric_zero_net_torque() {
        // Symmetric cube at CoM has CP offsets, but opposite faces cancel
        // For flow in +y (identity attitude), the +y face is the one turned into
        // it and the -y face is sheltered
        // But the other 4 faces (±x, ±z) have cos(θ)=0 for exact +y flow
        // So only the +y face contributes, with CP at (0, half, 0)
        // Force is in -ŷ body: τ = (0,h,0) × (0,F,0) = 0 (parallel!)
        let drag = PanelDrag::for_earth(SpacecraftShape::cube(0.5, 2.2, PanelOptics::absorber()));
        let loads = drag.eval(0.0, &iss_state(), None);
        assert!(
            loads.torque_body.magnitude() < 1e-20,
            "Cube with flow along axis should have zero torque (CP parallel to force)"
        );
    }

    // Quantitative attitude coupling

    /// Helper: make a quaternion for rotation by `angle` about the given axis.
    fn quat_from_axis_angle(axis: Vector3<f64>, angle: f64) -> Vector4<f64> {
        let half = angle / 2.0;
        let (s, c) = half.sin_cos();
        let a = axis.normalize();
        Vector4::new(c, a.x * s, a.y * s, a.z * s)
    }

    #[test]
    fn cos_theta_scaling_45_degrees() {
        // Rotate 45° about x: cos θ = cos(45°) = √2/2
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let s0 = iss_state(); // identity attitude
        let mut s45 = iss_state();
        s45.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_4);

        let a0 = drag.eval(0.0, &s0, None).acceleration_inertial.magnitude();
        let a45 = drag.eval(0.0, &s45, None).acceleration_inertial.magnitude();

        let ratio = a45 / a0;
        let expected = std::f64::consts::FRAC_PI_4.cos(); // cos(45°) = √2/2
        assert!(
            (ratio - expected).abs() < 1e-10,
            "45° rotation: expected ratio {expected:.6}, got {ratio:.6}"
        );
    }

    #[test]
    fn cos_theta_scaling_60_degrees() {
        // Rotate 60° about x: cos θ = cos(60°) = 0.5
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let s0 = iss_state();
        let mut s60 = iss_state();
        s60.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_3);

        let a0 = drag.eval(0.0, &s0, None).acceleration_inertial.magnitude();
        let a60 = drag.eval(0.0, &s60, None).acceleration_inertial.magnitude();

        let ratio = a60 / a0;
        assert!(
            (ratio - 0.5).abs() < 1e-10,
            "60° rotation: expected ratio 0.5, got {ratio:.6}"
        );
    }

    #[test]
    fn cos_theta_scaling_90_degrees_zero() {
        // Rotate 90° about x: cos θ = 0 → zero drag
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let mut s90 = iss_state();
        s90.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2);

        let a = drag.eval(0.0, &s90, None).acceleration_inertial.magnitude();
        assert!(a < 1e-20, "90° rotation: expected zero drag, got {a:.3e}");
    }

    #[test]
    fn force_direction_always_anti_velocity() {
        // Pure drag invariant: a_inertial ∥ -v_rel for any attitude with nonzero drag.
        // Proof: F_body ∝ -v̂_body → a_inertial = R_ib*(-K·v̂_body) = -K·v̂_inertial
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let angles = [0.0, 0.3, 0.7, 1.0, -0.5];
        let axes = [
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(1.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 1.0),
        ];

        let base = iss_state();
        let v_rel = *base.orbit.velocity()
            - Vector3::new(0.0, 0.0, OMEGA_EARTH).cross(base.orbit.position());

        for (axis, angle) in axes.iter().zip(angles.iter()) {
            let mut state = iss_state();
            state.attitude.quaternion = quat_from_axis_angle(*axis, *angle);

            let loads = drag.eval(0.0, &state, None);
            let a = loads.acceleration_inertial.into_inner();

            if a.magnitude() < 1e-20 {
                continue; // backface, direction undefined
            }

            // Check: a × v_rel ≈ 0 (parallel)
            let cross = a.cross(&v_rel);
            let cross_rel = cross.magnitude() / (a.magnitude() * v_rel.magnitude());
            assert!(
                cross_rel < 1e-10,
                "axis={axis:?}, angle={angle}: force not parallel to -v_rel, |a×v|/|a||v| = {cross_rel:.3e}"
            );

            // Check: a · v_rel < 0 (opposing)
            assert!(
                a.dot(&v_rel) < 0.0,
                "axis={axis:?}, angle={angle}: force not opposing velocity"
            );
        }
    }

    #[test]
    fn energy_dissipation_always_negative() {
        // F · v_rel ≤ 0 for drag at any attitude (energy is always removed)
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        for i in 0..20 {
            let angle = (i as f64) * std::f64::consts::PI / 10.0; // 0 to 2π
            let mut state = iss_state();
            state.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 1.0, 1.0), angle);

            let loads = drag.eval(0.0, &state, None);
            let a = loads.acceleration_inertial.into_inner();
            let v_rel = *state.orbit.velocity()
                - Vector3::new(0.0, 0.0, OMEGA_EARTH).cross(state.orbit.position());

            let power = a.dot(&v_rel); // F·v / m, proportional to power
            assert!(
                power <= 0.0,
                "Drag should always dissipate energy: angle={angle:.2}, F·v = {power:.3e}"
            );
        }
    }

    /// The drag pair built the way a caller builds it, and with a torque.
    ///
    /// `two_sided_panel_no_dead_zone` covers the |cos θ| law for a hand-written
    /// pair at the centre of mass. This one goes through `back_face` and gives
    /// the plate an off-centre pressure point, so it also pins what the pair
    /// does to the torque: reversing the flow reverses it, since the two faces
    /// are pushed opposite ways through the same point.
    #[test]
    fn back_face_reverses_the_drag_torque() {
        let front = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        )
        .with_cp_offset(Vector3::new(0.0, 0.0, 1.5));
        let back = front.back_face(PanelOptics::absorber());
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![front, back]));

        let at_identity = drag.eval(0.0, &iss_state(), None);
        let mut flipped = iss_state();
        flipped.attitude.quaternion = Vector4::new(0.0, 0.0, 0.0, 1.0); // 180 deg about z
        let at_flipped = drag.eval(0.0, &flipped, None);

        let a0 = at_identity.acceleration_inertial.magnitude();
        let a180 = at_flipped.acceleration_inertial.magnitude();
        assert!(a0 > 0.0, "the front faces the flow at identity");
        assert!(
            (a0 - a180).abs() / a0 < 1e-10,
            "the same plate presents the same area either way round: {a0:.3e} vs {a180:.3e}"
        );

        let t0 = at_identity.torque_body.into_inner();
        let t180 = at_flipped.torque_body.into_inner();
        assert!(t0.magnitude() > 0.0, "an off-centre plate torques");
        assert!(
            (t0 + t180).magnitude() / t0.magnitude() < 1e-10,
            "reversing the flow reverses the torque: {t0:?} vs {t180:?}"
        );
    }

    /// Half-extents that are each positive and finite can still give an area
    /// that is neither.
    #[test]
    fn rectangle_rejects_an_area_that_underflows_or_overflows() {
        for half_extent in [[1e-300, 1e-300], [1e200, 1e200]] {
            let caught = std::panic::catch_unwind(|| {
                SurfacePanel::rectangle(
                    half_extent,
                    Vector3::new(0.0, 1.0, 0.0),
                    Vector3::new(1.0, 0.0, 0.0),
                    2.2,
                    PanelOptics::absorber(),
                )
            });
            assert!(
                caught.is_err(),
                "{half_extent:?} gives an area of {} and has to be rejected",
                4.0 * half_extent[0] * half_extent[1]
            );
        }

        // And the smallest extent whose area survives is still accepted, so the
        // check is on the product rather than on the magnitude of a side.
        let ok = SurfacePanel::rectangle(
            [1e-150, 1e-150],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        assert!(ok.area > 0.0 && ok.area.is_finite());
    }

    /// The same geometry is accepted whichever order the extents are written in.
    ///
    /// `4.0 * h[0] * h[1]` overflowed on `4.0 * 1e308` before it ever saw the
    /// second extent, so `[1e308, 1e-308]` was rejected and `[1e-308, 1e308]`
    /// was not.
    #[test]
    fn the_area_check_does_not_depend_on_the_extent_order() {
        let build = |half_extent: [f64; 2]| {
            SurfacePanel::rectangle(
                half_extent,
                Vector3::new(0.0, 1.0, 0.0),
                Vector3::new(1.0, 0.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            )
        };
        let a = build([1e308, 1e-308]);
        let b = build([1e-308, 1e308]);
        assert_eq!(a.area, b.area, "the product is commutative");
        assert!(a.area.is_finite() && a.area > 0.0, "got {}", a.area);
    }

    /// A direction vector is rejected unless its own magnitude is finite and
    /// non-zero.
    ///
    /// `normalize()` and then `magnitude() > 0.5` is not that check.
    /// `[1e-200; 3]` squares to a norm that underflows to zero, so normalising
    /// divides by zero and gives `[inf, inf, inf]` — an infinite magnitude,
    /// which passes `> 0.5`. The panel then carries an infinite normal, and
    /// cos θ and every force built from it are NaN.
    #[test]
    fn a_direction_vector_needs_a_magnitude_that_can_be_computed() {
        let rejected = [
            ("zero", Vector3::zeros()),
            ("NaN", Vector3::new(f64::NAN, 0.0, 0.0)),
            ("infinite", Vector3::new(f64::INFINITY, 0.0, 0.0)),
            // Every component finite, but the squared norm overflows, and
            // normalising then divides by infinity and gives zero.
            ("overflowing", Vector3::new(1e300, 1e300, 0.0)),
            // The same, underflowing: normalising divides by zero.
            ("underflowing", Vector3::new(1e-200, 1e-200, 1e-200)),
        ];

        for (name, v) in rejected {
            let as_normal = std::panic::catch_unwind(|| {
                SurfacePanel::at_com(4.0, v, 2.2, PanelOptics::absorber())
            });
            if let Ok(panel) = as_normal {
                panic!(
                    "at_com took a {name} normal and built one pointing {:?}",
                    panel.normal.as_slice()
                );
            }
            let as_rectangle_normal = std::panic::catch_unwind(|| {
                SurfacePanel::rectangle(
                    [1.0, 1.0],
                    Vector3::new(0.0, 1.0, 0.0),
                    v,
                    2.2,
                    PanelOptics::absorber(),
                )
            });
            if let Ok(panel) = as_rectangle_normal {
                panic!(
                    "rectangle took a {name} normal and built one pointing {:?}",
                    panel.normal.as_slice()
                );
            }
            let as_axis = std::panic::catch_unwind(|| {
                SurfacePanel::rectangle(
                    [1.0, 1.0],
                    v,
                    Vector3::new(1.0, 0.0, 0.0),
                    2.2,
                    PanelOptics::absorber(),
                )
            });
            if let Ok(panel) = as_axis {
                panic!(
                    "rectangle took a {name} in-plane axis and kept {:?}",
                    panel.outline
                );
            }
        }

        // Being small is not the problem: this one's magnitude is 1.7e-150,
        // which divides cleanly.
        let small = Vector3::new(1e-150, 1e-150, 1e-150);
        let panel = SurfacePanel::at_com(4.0, small, 2.2, PanelOptics::absorber());
        assert!(
            (panel.normal.magnitude() - 1.0).abs() < 1e-15,
            "a computable magnitude has to be accepted, got |n| = {}",
            panel.normal.magnitude()
        );
    }

    /// An offset that overflows is outside the outline, even one vast enough
    /// that the tolerance overflows too.
    ///
    /// `[f64::MAX, 1e-308]` is an accepted rectangle: both extents are finite
    /// and positive and the area comes to 7.2. Scaling that half-extent by the
    /// tolerance gives infinity, so the comparison along the long axis reads
    /// `inf <= inf` and passes — and the answer is still right, because the
    /// short axis sees `inf * 0`, which is NaN and fails. Nothing rests on the
    /// overflowed bound either way: no finite offset can exceed a half-extent
    /// of `f64::MAX`, so wherever a point can actually be, that bound and the
    /// true one agree.
    #[test]
    fn an_offset_that_overflows_is_outside_a_vast_outline() {
        let normal = Vector3::new(0.0, 0.0, 1.0);
        let axis = Vector3::new(1.0, 0.0, 0.0);
        let panel = SurfacePanel::rectangle(
            [f64::MAX, 1e-308],
            axis,
            normal,
            2.2,
            PanelOptics::absorber(),
        )
        .with_cp_offset(axis * -1e308);

        // 1e308 - (-1e308) overflows.
        let beyond = axis * 1e308;
        assert!(
            !panel.outline_contains(&beyond),
            "an offset that overflows is within no outline"
        );
    }

    /// An exactly-sized caster covers the panel even where the arithmetic
    /// lands the corners a rounding outside its outline.
    ///
    /// This is what the edge tolerance is for, and the other cases here do not
    /// need it: their in-plane axes are axis-aligned, so the dot products are
    /// exact and the corners land exactly on the boundary. With the axis turned
    /// to 108.2° the components are inexact, and a sweep of 3600 angles puts
    /// 4780 of 14400 corners outside by up to 4.8e-16 of the half-extent —
    /// which is a third of them, so without the tolerance an exactly-covering
    /// caster would report a shadow or no shadow depending on its angle.
    #[test]
    fn an_exactly_sized_caster_covers_the_panel_at_an_inexact_axis() {
        let normal = Vector3::new(1.0, 0.0, 0.0);
        let axis = nalgebra::Rotation3::from_axis_angle(
            &nalgebra::Unit::new_normalize(normal),
            108.2_f64.to_radians(),
        ) * Vector3::new(0.0, 1.0, 0.0);
        let extent = [0.7, 1.3];

        let target = SurfacePanel::rectangle(extent, axis, normal, 2.2, PanelOptics::absorber());
        // The same plate, exactly, two metres upstream.
        let caster = SurfacePanel::rectangle(extent, axis, normal, 2.2, PanelOptics::absorber())
            .with_cp_offset(normal * 2.0);

        let panels = vec![target, caster];
        assert!(
            is_fully_occluded(&panels[0], &panels, &normal),
            "a caster the same size and directly upstream covers the panel"
        );
    }

    /// A caster far smaller than the edge tolerance covers nothing.
    ///
    /// The tolerance absorbs floating-point error where a corner grazes an
    /// edge. As an absolute length it was one nanometre whatever the panel, and
    /// `rectangle` accepts half-extents down to `1e-150`, so a caster that size
    /// counted as roughly two nanometres across and swallowed every target
    /// below that — 141 orders of magnitude larger than the panel it described.
    #[test]
    fn a_caster_far_below_the_edge_tolerance_covers_nothing() {
        let axis = Vector3::new(0.0, 1.0, 0.0);
        let upstream = Vector3::new(1.0, 0.0, 0.0);
        let target =
            SurfacePanel::rectangle([5e-10, 5e-10], axis, upstream, 2.2, PanelOptics::absorber());
        let caster = SurfacePanel::rectangle(
            [1e-150, 1e-150],
            axis,
            upstream,
            2.2,
            PanelOptics::absorber(),
        )
        .with_cp_offset(upstream);

        let panels = vec![target, caster];
        assert!(
            !is_fully_occluded(&panels[0], &panels, &upstream),
            "a caster 1e-150 m across cannot cover a target 1e-9 m across"
        );
    }

    /// A caster tilted through the shaded panel's plane covers only part of it.
    ///
    /// Every other occlusion case here has the two plates parallel or
    /// coplanar, so all four corners of the target sit on one side of the
    /// caster's plane and one depth comparison could answer for the whole
    /// panel. Here the caster's centre is upstream of the target while one of
    /// its edges is behind it, and the corners on that side reach its plane
    /// going backwards even though their rays land inside its outline.
    ///
    /// The tilt runs through all four in-plane directions because each one
    /// leaves a different pair of corners behind, and between them every
    /// corner takes a turn: a depth test that looked at one fixed corner and
    /// projected the rest would answer this correctly for some tilts and
    /// wrongly for others.
    #[test]
    fn a_caster_tilted_through_the_panel_covers_only_part_of_it() {
        let upstream = Vector3::new(1.0, 0.0, 0.0);
        let shaded = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(0.0, 1.0, 0.0),
            upstream,
            2.2,
            PanelOptics::absorber(),
        );
        let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
        let corners = shaded
            .corners_into(&mut buf)
            .expect("a rectangle has corners");

        // Where a ray from `from` toward `upstream` meets a caster's plane.
        let depth = |from: &Vector3<f64>, c: &SurfacePanel| {
            c.normal.dot(&(c.cp_offset - from)) / c.normal.dot(&upstream)
        };

        let mut ever_behind = [false; MAX_PANEL_CORNERS];
        for tilt in [
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, -1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
            Vector3::new(0.0, 0.0, -1.0),
        ] {
            // Tilted 45° so its plane cuts the target's, and wide enough that
            // every corner's ray meets that plane inside the outline.
            let caster = |x: f64| {
                SurfacePanel::rectangle(
                    [1.6, 1.2],
                    (upstream - tilt).normalize(),
                    (upstream + tilt).normalize(),
                    2.2,
                    PanelOptics::absorber(),
                )
                .with_cp_offset(Vector3::new(x, 0.0, 0.0))
            };

            // The geometry the test is about, stated rather than assumed.
            let cutting = caster(0.5);
            assert!(
                depth(&shaded.cp_offset, &cutting) > 0.0,
                "tilt {tilt:?}: the caster's centre has to be upstream of the target's"
            );
            let mut behind = 0;
            for (i, corner) in corners.iter().enumerate() {
                let t = depth(corner, &cutting);
                if t <= 0.0 {
                    behind += 1;
                    ever_behind[i] = true;
                }
                let hit = corner + upstream * t;
                assert!(
                    cutting.outline_contains(&hit),
                    "tilt {tilt:?}: corner {corner:?} reaches the caster's outline at \
                     {hit:?}, so the depth is the only thing that can keep it lit"
                );
            }
            assert_eq!(
                behind, 2,
                "tilt {tilt:?}: the caster's plane has to split the target's corners"
            );

            let panels = vec![shaded.clone(), cutting];
            assert!(
                !is_fully_occluded(&panels[0], &panels, &upstream),
                "tilt {tilt:?}: a caster cutting through the panel leaves part of it lit"
            );

            // Slid upstream until it clears the target, the same plate does
            // cover it — so the answer above comes from the tilt, not a miss.
            let panels = vec![shaded.clone(), caster(2.0)];
            assert!(
                is_fully_occluded(&panels[0], &panels, &upstream),
                "tilt {tilt:?}: clear of the panel, the same tilted plate covers it"
            );
        }

        assert!(
            ever_behind.iter().all(|behind| *behind),
            "every corner has to be the one behind for some tilt, or a fixed \
             corner would do: {ever_behind:?}"
        );
    }

    /// The cube's faces carry outlines, so a panel added behind one is found.
    ///
    /// The doc says so and nothing tested it: the existing cube tests look at
    /// areas, normals and pressure centres, and the SRP smoke test only asks
    /// for a nonzero result.
    #[test]
    fn a_panel_behind_a_cube_face_is_shaded_by_it() {
        let optics = PanelOptics::absorber();
        let SpacecraftShape::Panels(faces) = SpacecraftShape::cube(0.5, 2.2, optics) else {
            panic!("cube is panelled");
        };

        // Smaller than a face and tucked just behind it, for each of the six.
        for face in &faces {
            let hidden = SurfacePanel::rectangle(
                [0.2, 0.2],
                // Any in-plane axis of this face works; take one from the face.
                match face.outline.expect("cube faces carry outlines") {
                    PanelOutline::Rectangle { in_plane_x, .. } => in_plane_x,
                },
                face.normal,
                2.2,
                optics,
            )
            .with_cp_offset(face.cp_offset - face.normal * 0.1);

            let mut panels = faces.clone();
            panels.push(hidden);

            // Face-on to that face, and obliquely across it.
            let oblique = (face.normal
                + match face.outline.expect("outline") {
                    PanelOutline::Rectangle { in_plane_x, .. } => in_plane_x * 0.6,
                })
            .normalize();
            for upstream in [face.normal, oblique] {
                assert!(
                    is_fully_occluded(&panels[6], &panels, &upstream),
                    "the panel behind face {:?} has to be shaded from {upstream:?}",
                    face.normal
                );
                // The faces turned toward the incoming direction stay lit:
                // the panel tucked behind one of them must not shade it back.
                // The far faces of a box are genuinely behind the near ones, so
                // they are occluded as well as back-facing — either way they
                // contribute nothing.
                for (i, f) in faces.iter().enumerate() {
                    if f.normal.dot(&upstream) <= 0.0 {
                        continue;
                    }
                    assert!(
                        !is_fully_occluded(f, &panels, &upstream),
                        "cube face {i} faces the source and must stay lit"
                    );
                }
            }
        }
    }

    /// A caster edge-on to the incoming direction covers nothing.
    ///
    /// It has no projected area to cover anything with. This falls out of the
    /// ray arithmetic rather than needing a case of its own — `t` runs to
    /// infinity and the hit lands outside the outline — so the test pins the
    /// property, not the way it is reached.
    #[test]
    fn an_edge_on_caster_covers_nothing() {
        let shaded = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        // Big enough to cover it if it were facing the right way, and turned
        // edge-on to the incoming direction.
        let caster = SurfacePanel::rectangle(
            [4.0, 4.0],
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        )
        .with_cp_offset(Vector3::new(2.0, 0.0, 0.0));

        let upstream = Vector3::new(1.0, 0.0, 0.0);
        assert_eq!(
            caster.normal.dot(&upstream),
            0.0,
            "the caster has to be exactly edge-on for this to be the guard's case"
        );
        let panels = vec![shaded, caster];
        assert!(
            !is_fully_occluded(&panels[0], &panels, &upstream),
            "an edge-on caster has no projected area to cover anything with"
        );
    }

    /// Drag skips a shaded panel too, and on the side its own facing test uses.
    ///
    /// The occlusion tests otherwise all sit in `panel_srp`, which leaves the
    /// sign of this model's `upstream` resting on a cube test that cannot see
    /// it: a cube's faces shade nothing either way round.
    ///
    /// `only_a_caster_between_the_flow_and_the_panel_shades_it` asserts the same
    /// removal and additionally that a caster on the sheltered side removes
    /// nothing. Both catch an occlusion test run from `-upstream`; only that one
    /// catches a shadow that falls on *both* sides, measured by making
    /// `is_fully_occluded` accept either direction — this test still passes
    /// there, since its one caster is upwind either way.
    #[test]
    fn drag_skips_a_panel_another_stands_in_front_of() {
        // `iss_state` at identity has velocity along +y, so the gas arrives from
        // +y and a panel facing +y is the one this model considers lit.
        let lit_normal = Vector3::new(0.0, 1.0, 0.0);
        let shaded = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::new(1.0, 0.0, 0.0),
            lit_normal,
            2.2,
            PanelOptics::absorber(),
        );
        let caster = SurfacePanel::rectangle(
            [2.0, 2.0],
            Vector3::new(1.0, 0.0, 0.0),
            lit_normal,
            2.2,
            PanelOptics::absorber(),
        )
        .with_cp_offset(lit_normal * 2.0);

        let alone = PanelDrag::for_earth(SpacecraftShape::panels(vec![shaded.clone()]));
        let behind = PanelDrag::for_earth(SpacecraftShape::panels(vec![shaded, caster.clone()]));
        let just_caster = PanelDrag::for_earth(SpacecraftShape::panels(vec![caster]));

        let alone = alone
            .eval(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();
        assert!(alone > 0.0, "the small panel is lit on its own");
        let behind = behind
            .eval(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();
        let just_caster = just_caster
            .eval(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();
        assert!(
            (behind - just_caster).abs() / just_caster < 1e-12,
            "the shaded panel must contribute nothing: {behind:e} vs {just_caster:e}"
        );
    }

    #[test]
    fn two_sided_panel_no_dead_zone() {
        // Two panels with opposite normals (±y): at least one faces the flow at any attitude
        let panels = vec![
            SurfacePanel::at_com(
                10.0,
                Vector3::new(0.0, -1.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            ),
            SurfacePanel::at_com(
                10.0,
                Vector3::new(0.0, 1.0, 0.0),
                2.2,
                PanelOptics::absorber(),
            ),
        ];
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(panels));

        // At identity: +y panel faces into the +y flow → drag
        let a0 = drag
            .eval(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();
        assert!(a0 > 0.0);

        // At 180° about z: -y panel is turned into the +y flow → same magnitude
        let mut s180 = iss_state();
        s180.attitude.quaternion = Vector4::new(0.0, 0.0, 0.0, 1.0);
        let a180 = drag
            .eval(0.0, &s180, None)
            .acceleration_inertial
            .magnitude();
        assert!(
            (a0 - a180).abs() / a0 < 1e-10,
            "Two-sided panel should have same drag at 0° and 180°: a0={a0:.3e}, a180={a180:.3e}"
        );

        // At 45° about x: only the +y panel contributes (cos θ = cos45),
        // the -y panel has cos θ = -cos45 → clamped to 0.
        // Opposite normals never both face the flow simultaneously:
        //   max(cosθ, 0) + max(-cosθ, 0) = |cosθ|
        let mut s45 = iss_state();
        s45.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_4);
        let a45 = drag.eval(0.0, &s45, None).acceleration_inertial.magnitude();
        let ratio = a45 / a0;
        let expected = std::f64::consts::FRAC_PI_4.cos(); // cos(45°) = √2/2
        assert!(
            (ratio - expected).abs() < 1e-10,
            "Two-sided at 45°: expected ratio {expected:.6}, got {ratio:.6}"
        );

        // At 90° about x: flow perpendicular to both normals → zero drag
        let mut s90 = iss_state();
        s90.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(1.0, 0.0, 0.0), std::f64::consts::FRAC_PI_2);
        let a90 = drag.eval(0.0, &s90, None).acceleration_inertial.magnitude();
        assert!(
            a90 < 1e-20,
            "Two-sided at 90° about x: both panels perpendicular → zero, got {a90:.3e}"
        );
    }

    #[test]
    fn cube_projected_area_analytic() {
        // For a cube (6 faces ±x,±y,±z), the total projected area in direction v̂ is:
        // A_proj = A * (|v̂_x| + |v̂_y| + |v̂_z|) in body frame
        // At identity: v̂_body = (0,1,0) → A_proj = A * 1 = A
        // At 45° about z: v̂_body = (sin45, cos45, 0) → A_proj = A * (sin45 + cos45) = A * √2
        let half = 0.5;
        let cd = 2.2;
        let drag = PanelDrag::for_earth(SpacecraftShape::cube(half, cd, PanelOptics::absorber()));

        let a0 = drag
            .eval(0.0, &iss_state(), None)
            .acceleration_inertial
            .magnitude();

        // 45° about z: v̂_body has components in both x and y
        let mut s45z = iss_state();
        s45z.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_4);
        let a45z = drag
            .eval(0.0, &s45z, None)
            .acceleration_inertial
            .magnitude();

        let ratio = a45z / a0;
        let expected = std::f64::consts::SQRT_2; // (sin45 + cos45) / 1
        assert!(
            (ratio - expected).abs() < 1e-10,
            "Cube at 45° about z: expected ratio {expected:.6}, got {ratio:.6}"
        );
    }

    #[test]
    fn torque_exact_cross_product() {
        // Panel with known offset: verify τ = r × F exactly
        // Setup: offset (1,0,0) m, flow in +y → force in -y body
        // τ = (1,0,0) × (0,F_y,0) = (0*0-0*F_y, 0*0-1*0, 1*F_y-0*0) = (0,0,F_y)
        let panel = SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 1.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(1.0, 0.0, 0.0),
            outline: None,
        };
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));
        let loads = drag.eval(0.0, &iss_state(), None);

        // Reconstruct F_body from acceleration: F = a_body * mass
        // a_inertial = R_ib * (F_body / mass) / 1000
        // At identity: R_ib = I, so a_inertial = F_body / (mass * 1000)
        let f_body_y = loads.acceleration_inertial.y() * iss_state().mass * 1000.0; // N

        // Expected torque: τ = r × F = (1,0,0) × (0,F_y,0) = (0, 0, F_y)
        assert!(
            loads.torque_body.x().abs() < 1e-20,
            "τ_x should be 0, got {:.3e}",
            loads.torque_body.x()
        );
        assert!(
            loads.torque_body.y().abs() < 1e-20,
            "τ_y should be 0, got {:.3e}",
            loads.torque_body.y()
        );
        let rel_err = (loads.torque_body.z() - f_body_y).abs() / f_body_y.abs();
        assert!(
            rel_err < 1e-10,
            "τ_z should equal F_y ({f_body_y:.6e}), got {:.6e}, rel_err={rel_err:.3e}",
            loads.torque_body.z()
        );
    }

    // Mock atmosphere for isolated frame-transform tests

    /// Constant density regardless of altitude/position/epoch.
    struct ConstantDensity(f64);

    impl AtmosphereModel for ConstantDensity {
        fn density(&self, _input: &AtmosphereInput<'_>) -> f64 {
            self.0
        }
    }

    /// Compose two quaternions: result represents R(q_second) * R(q_first).
    ///
    /// Delegates to nalgebra `UnitQuaternion` multiplication to avoid
    /// hand-coding Hamilton product with nalgebra's confusing Vector4 accessors
    /// (`.x`→[0], `.w`→[3] do NOT match quaternion component names).
    fn quat_compose(q_second: &Vector4<f64>, q_first: &Vector4<f64>) -> Vector4<f64> {
        use nalgebra::{Quaternion, UnitQuaternion};
        let uq_second = UnitQuaternion::from_quaternion(Quaternion::new(
            q_second[0],
            q_second[1],
            q_second[2],
            q_second[3],
        ));
        let uq_first = UnitQuaternion::from_quaternion(Quaternion::new(
            q_first[0], q_first[1], q_first[2], q_first[3],
        ));
        let result = uq_second * uq_first;
        Vector4::new(result.w, result.i, result.j, result.k)
    }

    /// Build a PanelDrag with constant density, no co-rotation, spherical body.
    /// Isolates pure frame-transformation physics from atmosphere position dependence.
    fn mock_drag(shape: SpacecraftShape, rho: f64) -> PanelDrag {
        PanelDrag {
            shape,
            atmosphere: Box::new(ConstantDensity(rho)),
            body: None,
            body_radius: 100.0, // well inside any test orbit
            omega_body: 0.0,    // no co-rotation
            eop: (),
        }
    }

    #[test]
    fn equivariance_acceleration_under_inertial_rotation() {
        // With constant density and no co-rotation, rotating the entire scenario
        // (position, velocity, attitude) by R in inertial frame should rotate
        // the acceleration by R: a' = R · a.
        //
        // Proof: v_body' = R_bi' · v_rel' = (R·R_ib)^T · R·v = R_bi · v = v_body
        // So per-panel forces in body frame are identical ⟹ a_body' = a_body
        // ⟹ a_inertial' = R_ib' · a_body = R · R_ib · a_body = R · a_inertial  ∎
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, 1.0, 0.0),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
                outline: None,
            },
            SurfacePanel::at_com(
                5.0,
                Vector3::new(1.0, 0.0, 0.0),
                2.0,
                PanelOptics::absorber(),
            ),
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        // Original state with non-trivial attitude
        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 1.0, 0.0), 0.5);
        let l1 = drag.eval(0.0, &s1, None);

        // Apply arbitrary rotation R (37° about (1,2,3))
        let q_r = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 37.0_f64.to_radians());
        let att_tmp = AttitudeState {
            quaternion: q_r,
            angular_velocity: Vector3::zeros(),
        };
        let r_mat = *att_tmp.orientation().to_rotation_matrix().matrix();

        let s2 = SpacecraftState {
            orbit: OrbitalState::new(r_mat * *s1.orbit.position(), r_mat * *s1.orbit.velocity()),
            attitude: AttitudeState {
                quaternion: quat_compose(&q_r, &s1.attitude.quaternion),
                angular_velocity: s1.attitude.angular_velocity,
            },
            mass: s1.mass,
        };
        let l2 = drag.eval(0.0, &s2, None);

        // a' should equal R · a
        let a1_rotated = r_mat * l1.acceleration_inertial.into_inner();
        let a_rel = (l2.acceleration_inertial.into_inner() - a1_rotated).magnitude()
            / l1.acceleration_inertial.magnitude();
        assert!(
            a_rel < 1e-10,
            "Acceleration should transform as R·a: relative error = {a_rel:.3e}"
        );
    }

    #[test]
    fn equivariance_torque_under_inertial_rotation() {
        // Body-frame torque should be invariant under inertial rotation,
        // since v_body is unchanged and all panel calculations happen in body frame.
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, 1.0, 0.0),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
                outline: None,
            },
            SurfacePanel {
                area: 8.0,
                normal: Vector3::new(1.0, 0.0, 0.0),
                cd: 2.0,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(0.0, 0.0, 0.5),
                outline: None,
            },
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(0.0, 1.0, 0.0), 0.8);
        let l1 = drag.eval(0.0, &s1, None);

        // Multiple arbitrary rotations
        let rotations = [
            (Vector3::new(1.0, 0.0, 0.0), 45.0_f64),
            (Vector3::new(0.0, 1.0, 0.0), 120.0),
            (Vector3::new(1.0, 2.0, 3.0), 37.0),
            (Vector3::new(-1.0, 0.5, 0.3), 200.0),
        ];

        for (axis, angle_deg) in &rotations {
            let q_r = quat_from_axis_angle(*axis, angle_deg.to_radians());
            let att_tmp = AttitudeState {
                quaternion: q_r,
                angular_velocity: Vector3::zeros(),
            };
            let r_mat = *att_tmp.orientation().to_rotation_matrix().matrix();

            let s2 = SpacecraftState {
                orbit: OrbitalState::new(
                    r_mat * *s1.orbit.position(),
                    r_mat * *s1.orbit.velocity(),
                ),
                attitude: AttitudeState {
                    quaternion: quat_compose(&q_r, &s1.attitude.quaternion),
                    angular_velocity: s1.attitude.angular_velocity,
                },
                mass: s1.mass,
            };
            let l2 = drag.eval(0.0, &s2, None);

            let tau_rel =
                (l2.torque_body - l1.torque_body).magnitude() / l1.torque_body.magnitude();
            assert!(
                tau_rel < 1e-10,
                "Body-frame torque should be invariant under {angle_deg}° about {axis:?}: \
                 relative error = {tau_rel:.3e}"
            );
        }
    }

    #[test]
    fn convention_anchor_yaw_positive_full_drag() {
        // Convention anchor: distinguishes R_bi from R_ib (would fail under transpose).
        //
        // Panel normal n_b = (1,0,0). Flow +y inertial.
        // +90° yaw about z: R_ib maps body_x → inertial_y.
        //   → Correct R_bi: v_body = R_bi * (0,v,0) = (v,0,0)
        //     cos θ = n_b · v̂_body = (1,0,0)·(1,0,0) = +1 → FULL drag
        //   → Wrong (R_ib): v_body = R_ib * (0,v,0) = (-v,0,0)
        //     cos θ = (1,0,0)·(-1,0,0) = -1 → backface → ZERO
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let mut state = iss_state();
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), std::f64::consts::FRAC_PI_2);

        let loads = drag.eval(0.0, &state, None);
        assert!(
            loads.acceleration_inertial.magnitude() > 1e-20,
            "Convention anchor: +90° yaw with n_b=(1,0,0) should be full drag. \
             Zero here indicates an R_ib/R_bi swap."
        );

        // The magnitude has to match the identity case for a +y normal panel,
        // which is the face turned into the +y flow there. Both are face-on, so
        // both see the full area.
        let panel_y = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag_y = mock_drag(SpacecraftShape::panels(vec![panel_y]), 1e-12);
        let ref_loads = drag_y.eval(0.0, &iss_state(), None);

        let rel = (loads.acceleration_inertial.magnitude()
            - ref_loads.acceleration_inertial.magnitude())
        .abs()
            / ref_loads.acceleration_inertial.magnitude();
        assert!(
            rel < 1e-10,
            "Full-drag magnitudes should match: relative diff = {rel:.3e}"
        );
    }

    #[test]
    fn convention_anchor_yaw_negative_backface() {
        // Complement of the above: -90° yaw turns the panel away from the flow.
        //   R_ib maps body_x → inertial -y.
        //   Correct R_bi: v_body = R_bi * (0,v,0) = (-v,0,0)
        //     cos θ = n_b · v̂_body = (1,0,0)·(-1,0,0) = -1 → backface → ZERO
        //   Wrong (R_ib): v_body = (v,0,0) → cos θ = +1 → full drag
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let mut state = iss_state();
        state.attitude.quaternion =
            quat_from_axis_angle(Vector3::new(0.0, 0.0, 1.0), -std::f64::consts::FRAC_PI_2);

        let loads = drag.eval(0.0, &state, None);
        assert!(
            loads.acceleration_inertial.magnitude() < 1e-20,
            "Convention anchor: -90° yaw with n_b=(1,0,0) should be backface (zero drag), \
             got {:.3e}. This indicates R_ib/R_bi swap.",
            loads.acceleration_inertial.magnitude()
        );
    }

    #[test]
    fn quaternion_sign_invariance() {
        // q and -q represent the same rotation.
        // PanelDrag should produce identical forces and torques.
        let panels = vec![
            SurfacePanel {
                area: 10.0,
                normal: Vector3::new(0.0, 1.0, 0.0),
                cd: 2.2,
                optics: PanelOptics::absorber(),
                cp_offset: Vector3::new(1.0, 0.0, 0.0),
                outline: None,
            },
            SurfacePanel::at_com(
                5.0,
                Vector3::new(1.0, 0.0, 0.0),
                2.0,
                PanelOptics::absorber(),
            ),
        ];
        let drag = mock_drag(SpacecraftShape::panels(panels), 1e-12);

        let mut s1 = iss_state();
        s1.attitude.quaternion = quat_from_axis_angle(Vector3::new(1.0, 2.0, 3.0), 0.7);

        let mut s2 = s1.clone();
        s2.attitude.quaternion = -s1.attitude.quaternion; // -q

        let l1 = drag.eval(0.0, &s1, None);
        let l2 = drag.eval(0.0, &s2, None);

        assert!(
            (l1.acceleration_inertial - l2.acceleration_inertial).magnitude() < 1e-15,
            "q and -q should give identical acceleration"
        );
        assert!(
            (l1.torque_body - l2.torque_body).magnitude() < 1e-15,
            "q and -q should give identical torque"
        );
    }

    #[test]
    fn density_linearity() {
        // a ∝ ρ: doubling density doubles acceleration and torque
        let panels = vec![SurfacePanel {
            area: 10.0,
            normal: Vector3::new(0.0, 1.0, 0.0),
            cd: 2.2,
            optics: PanelOptics::absorber(),
            cp_offset: Vector3::new(1.0, 0.0, 0.0),
            outline: None,
        }];

        let drag1 = mock_drag(SpacecraftShape::panels(panels.clone()), 1e-12);
        let drag2 = mock_drag(SpacecraftShape::panels(panels), 2e-12);
        let state = iss_state();

        let l1 = drag1.eval(0.0, &state, None);
        let l2 = drag2.eval(0.0, &state, None);

        let a_ratio = l2.acceleration_inertial.magnitude() / l1.acceleration_inertial.magnitude();
        assert!(
            (a_ratio - 2.0).abs() < 1e-10,
            "Acceleration should scale linearly with density: ratio = {a_ratio:.6}"
        );

        let tau_ratio = l2.torque_body.magnitude() / l1.torque_body.magnitude();
        assert!(
            (tau_ratio - 2.0).abs() < 1e-10,
            "Torque should scale linearly with density: ratio = {tau_ratio:.6}"
        );
    }

    #[test]
    fn velocity_squared_scaling() {
        // a ∝ |v|² (at constant density, same direction)
        // Use mock to eliminate altitude-dependent density changes
        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), 1e-12);

        let s1 = iss_state();
        let mut s2 = iss_state();
        // Scale velocity by 2x (keep position same → same density with mock)
        *s2.orbit.velocity_mut() = *s1.orbit.velocity() * 2.0;

        let a1 = drag.eval(0.0, &s1, None).acceleration_inertial.magnitude();
        let a2 = drag.eval(0.0, &s2, None).acceleration_inertial.magnitude();

        // a ∝ |v|² → ratio should be 4
        let ratio = a2 / a1;
        assert!(
            (ratio - 4.0).abs() < 1e-10,
            "Acceleration should scale as |v|²: ratio = {ratio:.6} (expected 4.0)"
        );
    }

    #[test]
    fn absolute_magnitude_analytic() {
        // For single panel at CoM with cos θ = 1 and constant density:
        //   |a| = ½ ρ Cd A |v|² / m   [m/s²]
        //   |a_km| = |a| / 1000       [km/s²]
        let area = 10.0; // m²
        let cd = 2.2;
        let mass = 500.0; // kg
        let rho = 1e-12; // kg/m³

        let panel = SurfacePanel::at_com(
            area,
            Vector3::new(0.0, 1.0, 0.0),
            cd,
            PanelOptics::absorber(),
        );
        let drag = mock_drag(SpacecraftShape::panels(vec![panel]), rho);

        let state = iss_state();
        let loads = drag.eval(0.0, &state, None);

        // With mock (no co-rotation), v_rel = v
        let v_ms = state.orbit.velocity().magnitude() * 1000.0; // m/s
        let expected_a_ms2 = 0.5 * rho * cd * area * v_ms * v_ms / mass;
        let expected_a_kms2 = expected_a_ms2 / 1000.0;

        let actual = loads.acceleration_inertial.magnitude();
        let rel_err = (actual - expected_a_kms2).abs() / expected_a_kms2;
        assert!(
            rel_err < 1e-10,
            "Absolute acceleration: expected {expected_a_kms2:.6e}, got {actual:.6e}, \
             rel_err = {rel_err:.3e}"
        );
    }

    /// Cross-validation against Orekit's paneled drag model.
    ///
    /// The fixture's job is the face selection: the `sheltered_*` cases are
    /// exactly zero in Orekit, and no fore-aft symmetric shape can show that —
    /// the projected areas of an opposite pair sum to the same value whichever
    /// face is picked, which is how #416 survived a cube test. The windward
    /// sweep pins the cos θ law over the same geometry, and `edge_on` grazes the
    /// boundary between them (`cos(π/2)` is 6.1e-17, not 0, so both sides
    /// produce a force 16 orders down rather than nothing).
    ///
    /// Compared on the force, because Orekit's paneled model returns only an
    /// acceleration; the torque this model builds from it is pinned by the exact
    /// cross-product tests. The mock atmosphere and its zero co-rotation are
    /// what keep the comparison about the panel law: the density and `v_rel` are
    /// then the fixture's own numbers rather than this crate's atmosphere and
    /// Earth rotation.
    ///
    /// Regenerate with `uv run tools/generate_orekit_panel_drag_fixtures.py`.
    #[test]
    fn orekit_panel_drag_force_reference() {
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            panel_normal_body: [f64; 3],
            force_body_n: [f64; 3],
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            density_kg_m3: f64,
            area_m2: f64,
            cd: f64,
            mass_kg: f64,
            position_inertial_m: [f64; 3],
            velocity_inertial_m_s: [f64; 3],
            cases: Vec<Case>,
        }

        let raw = include_str!("../../tests/fixtures/orekit_panel_drag_reference.json");
        let fx: Fixture = serde_json::from_str(raw).expect("fixture parses");
        assert!(fx.cases.len() >= 10, "expected the full case set");

        // The fixture is in SI; this crate works in km and km/s.
        let state = SpacecraftState {
            orbit: OrbitalState::new(
                Vector3::from_row_slice(&fx.position_inertial_m) / 1000.0,
                Vector3::from_row_slice(&fx.velocity_inertial_m_s) / 1000.0,
            ),
            attitude: AttitudeState::identity(),
            mass: fx.mass_kg,
        };

        let mut sheltered = 0;
        for case in &fx.cases {
            let panel = SurfacePanel::at_com(
                fx.area_m2,
                Vector3::from_row_slice(&case.panel_normal_body),
                fx.cd,
                PanelOptics::absorber(),
            );
            let drag = mock_drag(SpacecraftShape::panels(vec![panel]), fx.density_kg_m3);
            // The attitude is identity, so the body frame is the inertial one
            // and `F = m · a` needs only the km/s² → m/s² factor.
            let ours = drag
                .eval(0.0, &state, None)
                .acceleration_inertial
                .into_inner()
                * fx.mass_kg
                * 1000.0;
            let theirs = Vector3::from_row_slice(&case.force_body_n);

            if theirs == Vector3::zeros() {
                sheltered += 1;
                assert_eq!(
                    ours,
                    Vector3::zeros(),
                    "{}: Orekit loads nothing on a face turned away from the flow, \
                     and we produced {ours:?}",
                    case.name
                );
                continue;
            }
            let err = (ours - theirs).magnitude() / theirs.magnitude();
            assert!(
                err < 1e-12,
                "{}: orekit {theirs:?}, ours {ours:?}, rel_err={err:.3e}",
                case.name
            );
        }
        assert!(
            sheltered >= 3,
            "the sheltered faces are what this fixture is for, found {sheltered}"
        );
    }

    // SpacecraftDynamics integration

    #[test]
    fn panels_integrable_with_rk4() {
        use super::super::SpacecraftDynamics;
        use crate::orbital::gravity::PointMass;
        use arika::earth::MU as MU_EARTH;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, OdeState, Rk4};

        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia).with_model(drag);

        let result = Rk4.integrate(&dyn_sc, iss_state().into(), 0.0, 60.0, 1.0, |_, _| {});
        assert!(
            result.is_finite(),
            "State should remain finite after 60s integration"
        );
        assert!(result.plant.orbit.position().magnitude() > 0.0);
    }

    #[test]
    fn panels_drag_reduces_orbital_energy() {
        use super::super::SpacecraftDynamics;
        use crate::orbital::gravity::PointMass;
        use arika::earth::MU as MU_EARTH;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, Rk4};

        let panel = SurfacePanel::at_com(
            10.0,
            Vector3::new(0.0, 1.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia).with_model(drag);

        let s0 = iss_state();
        let e0 = 0.5 * s0.orbit.velocity().magnitude_squared()
            - MU_EARTH / s0.orbit.position().magnitude();

        let s1 = Rk4.integrate(&dyn_sc, s0.into(), 0.0, 300.0, 1.0, |_, _| {});
        let e1 = 0.5 * s1.plant.orbit.velocity().magnitude_squared()
            - MU_EARTH / s1.plant.orbit.position().magnitude();

        assert!(
            e1 < e0,
            "Drag should reduce orbital energy: e0={e0:.6e}, e1={e1:.6e}"
        );
    }

    #[test]
    fn tumbling_asymmetric_panels_varying_drag() {
        use super::super::SpacecraftDynamics;
        use crate::orbital::gravity::PointMass;
        use arika::earth::MU as MU_EARTH;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, Rk4};

        // Asymmetric panel: only one face, so drag depends on orientation
        let panel = SurfacePanel::at_com(
            20.0,
            Vector3::new(1.0, 0.0, 0.0),
            2.2,
            PanelOptics::absorber(),
        );
        let drag = PanelDrag::for_earth(SpacecraftShape::panels(vec![panel]));

        let inertia = Matrix3::from_diagonal(&Vector3::new(100.0, 200.0, 300.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia).with_model(drag);

        // Give it a tumble
        let mut state = iss_state();
        state.attitude.angular_velocity = Vector3::new(0.0, 0.0, 0.05); // slow tumble about z

        // Collect drag magnitude at several steps to verify it varies
        let mut magnitudes = Vec::new();
        let _ = Rk4.integrate(&dyn_sc, state.into(), 0.0, 60.0, 1.0, |_t, s| {
            let loads = dyn_sc.model_breakdown(0.0, &s.plant);
            if let Some((_, el)) = loads.first() {
                magnitudes.push(el.acceleration_inertial.magnitude());
            }
        });

        // Should have varying magnitudes (not all the same)
        let min = magnitudes.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = magnitudes.iter().cloned().fold(0.0_f64, f64::max);
        assert!(
            max > min * 1.01 || min == 0.0,
            "Tumbling should cause varying drag: min={min:.3e}, max={max:.3e}"
        );
    }

    #[test]
    fn sphere_integrable_with_spacecraft_dynamics() {
        use super::super::SpacecraftDynamics;
        use crate::orbital::gravity::PointMass;
        use arika::earth::MU as MU_EARTH;
        use nalgebra::Matrix3;
        use utsuroi::{Integrator, OdeState, Rk4};

        let drag = PanelDrag::for_earth(SpacecraftShape::sphere(10.0, 2.2, 1.5));
        let inertia = Matrix3::from_diagonal(&Vector3::new(10.0, 10.0, 10.0));
        let dyn_sc = SpacecraftDynamics::new(MU_EARTH, PointMass, inertia).with_model(drag);

        let result = Rk4.integrate(&dyn_sc, iss_state().into(), 0.0, 60.0, 1.0, |_, _| {});
        assert!(result.is_finite());
    }
}
