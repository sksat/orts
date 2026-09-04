//! How much of a panel the light reaches, and where the resulting force acts.
//!
//! The panel models used to ask one question per panel — is it *entirely*
//! behind one other panel? — and a partly shadowed panel answered "no" and
//! contributed its whole area at its geometric centre of pressure. That put a
//! false torque on the spacecraft (issue #407).
//!
//! [`lit_region`] answers with the lit fraction of the area and the centre of
//! that lit part, so a force model can shrink the force and move where it acts.

use nalgebra::{Vector2, Vector3};

use super::surface::{MAX_PANEL_CORNERS, PanelOutline, SurfacePanel};

/// The most vertices a shadow polygon can have.
///
/// A shadow starts as a caster outline ([`MAX_PANEL_CORNERS`] vertices), gains
/// at most one from the half-space clip, and gains at most one per target edge
/// from the clip to the target.
const MAX_SHADOW_VERTICES: usize = 2 * MAX_PANEL_CORNERS + 1;

/// Roundoff allowance, as a multiple of [`f64::EPSILON`] times the magnitudes
/// that went into a subtraction.
///
/// A fixed length cannot serve: [`SurfacePanel::rectangle`] accepts
/// half-extents from `1e-150` to `1e150`, and a nanometre is either far below
/// the noise or wider than the whole panel. Everything below scales its
/// tolerance by the coordinates it is comparing.
const ROUNDOFF: f64 = 32.0;

/// The lit part of a panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LitRegion {
    /// Lit share of the panel's area, in `[0, 1]`.
    pub fraction: f64,
    /// Where the lit part's force acts [m, body frame].
    ///
    /// Equal to the panel's `cp_offset` when the panel is fully lit or fully
    /// dark: a full panel's centre of pressure is its own, and a dark one
    /// produces no force for the point to apply to.
    pub centroid: Vector3<f64>,
}

impl LitRegion {
    fn all_lit(panel: &SurfacePanel) -> Self {
        Self {
            fraction: 1.0,
            centroid: panel.cp_offset,
        }
    }

    fn all_dark(panel: &SurfacePanel) -> Self {
        Self {
            fraction: 0.0,
            centroid: panel.cp_offset,
        }
    }
}

/// The lit part of `panel`, given every panel that could shadow it.
///
/// `upstream` points from the spacecraft toward where the light or the flow
/// comes from — SRP passes `s_body`, drag passes `-v̂_body` — and must be unit
/// length. The caller must have established that the panel faces it
/// (`normal · upstream > 0`); at grazing incidence the projection below divides
/// by that dot product, so the caller's own force cutoff is what keeps the
/// arithmetic in range.
///
/// `others` may contain `panel` itself; it is excluded by identity.
///
/// A panel without an outline is reported fully lit, and one without an outline
/// casts no shadow, so a fleet of area-only panels behaves exactly as it did
/// before outlines existed.
pub(crate) fn lit_region(
    panel: &SurfacePanel,
    others: &[SurfacePanel],
    upstream: &Vector3<f64>,
) -> LitRegion {
    let Some(PanelOutline::Rectangle { in_plane_x, .. }) = panel.outline else {
        return LitRegion::all_lit(panel);
    };
    let u = in_plane_x;
    let v = panel.normal.cross(&u);

    let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
    let Some(corners) = panel.corners_into(&mut buf) else {
        return LitRegion::all_lit(panel);
    };
    let target: Vec<Vector2<f64>> = corners
        .iter()
        .map(|c| to_plane(c, &panel.cp_offset, &u, &v))
        .collect();
    let target_area = signed_area(&target).abs();
    if !target_area.is_finite() || target_area <= 0.0 {
        return LitRegion::all_lit(panel);
    }

    // Every shadow, already trimmed to the target so that later arithmetic
    // stays inside the target's own scale.
    let mut shadows: Vec<Vec<Vector2<f64>>> = Vec::new();
    for other in others {
        if std::ptr::eq(other, panel) {
            continue;
        }
        if let Some(s) = shadow_on_target(panel, other, upstream, &u, &v, &target) {
            shadows.push(s);
        }
    }
    if shadows.is_empty() {
        return LitRegion::all_lit(panel);
    }

    // Subtract the shadows one at a time, keeping the lit part as a set of
    // disjoint convex pieces. Adding and subtracting overlaps instead
    // (inclusion-exclusion) needs 2^k - 1 terms for k shadows and builds the
    // small lit area out of the cancellation of larger ones; disjoint pieces
    // are bounded by the arrangement of the shadow edges and only ever added.
    let mut pieces = vec![target.clone()];
    for shadow in &shadows {
        let mut next: Vec<Vec<Vector2<f64>>> = Vec::new();
        for piece in &pieces {
            subtract_into(piece, shadow, &mut next);
        }
        pieces = next;
        if pieces.is_empty() {
            return LitRegion::all_dark(panel);
        }
    }

    let mut lit_area = 0.0;
    let mut moment = Vector2::zeros();
    for piece in &pieces {
        let (a, m) = area_and_moment(piece);
        lit_area += a;
        moment += m;
    }

    // Snap the ends rather than widening any outline: a caster exactly the size
    // of the target leaves a lit area of a few epsilons, and reporting that as
    // a sliver of force at an arbitrary point is worse than reporting none.
    let snap = ROUNDOFF * f64::EPSILON * target_area;
    if lit_area <= snap {
        return LitRegion::all_dark(panel);
    }
    if lit_area >= target_area - snap {
        return LitRegion::all_lit(panel);
    }

    let centre = moment / lit_area;
    LitRegion {
        fraction: (lit_area / target_area).clamp(0.0, 1.0),
        centroid: panel.cp_offset + u * centre.x + v * centre.y,
    }
}

/// `other`'s shadow on `panel`'s plane, in the target's in-plane coordinates
/// and trimmed to `target`, or `None` when it casts nothing onto it.
fn shadow_on_target(
    panel: &SurfacePanel,
    other: &SurfacePanel,
    upstream: &Vector3<f64>,
    u: &Vector3<f64>,
    v: &Vector3<f64>,
    target: &[Vector2<f64>],
) -> Option<Vec<Vector2<f64>>> {
    let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
    let corners = other.corners_into(&mut buf)?;

    // Only the part of the caster on the lit side of the target can shadow it.
    // Clipping in 3D first, before projecting, is what makes a caster that
    // tilts through the target shadow only the part of it that is really in
    // front — projecting the whole caster would report the target fully dark.
    let depth = |p: &Vector3<f64>| panel.normal.dot(&(p - panel.cp_offset));
    let front: Vec<Vector3<f64>> = clip_3d(corners, &depth, &panel.cp_offset);
    if front.len() < 3 {
        return None;
    }

    // Carry each front vertex along the light to the target's plane.
    let denom = panel.normal.dot(upstream);
    let mut poly: Vec<Vector2<f64>> = Vec::with_capacity(MAX_SHADOW_VERTICES);
    for q in &front {
        let hit = q - upstream * (depth(q) / denom);
        let p = to_plane(&hit, &panel.cp_offset, u, v);
        // Grazing incidence can send a vertex past the range of f64 even though
        // every input was finite. A shadow that cannot be located is dropped,
        // which leaves the force in place rather than removing one that exists.
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        poly.push(p);
    }

    let trimmed = intersect_convex(&poly, target);
    let area = signed_area(&trimmed).abs();
    let scale = ROUNDOFF * f64::EPSILON * signed_area(target).abs();
    (trimmed.len() >= 3 && area > scale).then_some(trimmed)
}

/// `panel`'s corner offset written in the panel's own in-plane axes.
fn to_plane(
    p: &Vector3<f64>,
    origin: &Vector3<f64>,
    u: &Vector3<f64>,
    v: &Vector3<f64>,
) -> Vector2<f64> {
    let d = p - origin;
    Vector2::new(d.dot(u), d.dot(v))
}

/// Keep the part of a 3D convex polygon where `depth` is positive.
fn clip_3d(
    poly: &[Vector3<f64>],
    depth: &impl Fn(&Vector3<f64>) -> f64,
    origin: &Vector3<f64>,
) -> Vec<Vector3<f64>> {
    let mut out = Vec::with_capacity(MAX_PANEL_CORNERS + 1);
    for i in 0..poly.len() {
        let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
        // The tolerance follows the magnitudes the subtraction inside `depth`
        // works on, so a panel 1e-150 m across and one 1e150 m across are both
        // judged against their own roundoff.
        let eps =
            |p: &Vector3<f64>| ROUNDOFF * f64::EPSILON * p.abs().max().max(origin.abs().max());
        let (da, db) = (depth(a), depth(b));
        if da > eps(a) {
            out.push(*a);
        }
        if (da > eps(a)) != (db > eps(b)) && da != db {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// The signed area of a polygon, positive when its vertices run
/// counter-clockwise.
fn signed_area(poly: &[Vector2<f64>]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut acc = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        acc += a.x * b.y - b.x * a.y;
    }
    acc / 2.0
}

/// Area and first moment (area x centroid) of a polygon, both positive-signed.
fn area_and_moment(poly: &[Vector2<f64>]) -> (f64, Vector2<f64>) {
    if poly.len() < 3 {
        return (0.0, Vector2::zeros());
    }
    let mut a2 = 0.0;
    let mut m = Vector2::zeros();
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let cross = p.x * q.y - q.x * p.y;
        a2 += cross;
        m += Vector2::new(p.x + q.x, p.y + q.y) * cross;
    }
    let area = a2 / 2.0;
    // Both flip together with the winding, so the ratio is orientation-free.
    (area.abs(), m / 6.0 * area.signum())
}

/// The intersection of two convex polygons, by clipping `poly` against every
/// edge of `by`.
fn intersect_convex(poly: &[Vector2<f64>], by: &[Vector2<f64>]) -> Vec<Vector2<f64>> {
    let mut out = poly.to_vec();
    for (p, q) in edges(by) {
        if out.len() < 3 {
            return Vec::new();
        }
        out = clip_half_plane(&out, &p, &q);
    }
    out
}

/// The lit part of `piece` outside `shadow`, as disjoint convex polygons.
///
/// Edge `j` of the shadow contributes the part of `piece` outside `j` and
/// inside every earlier edge; those parts are disjoint by construction and
/// together make up `piece` minus the shadow.
fn subtract_into(
    piece: &[Vector2<f64>],
    shadow: &[Vector2<f64>],
    out: &mut Vec<Vec<Vector2<f64>>>,
) {
    let edges: Vec<_> = edges(shadow).collect();
    for j in 0..edges.len() {
        let (p, q) = edges[j];
        let mut part = clip_half_plane(piece, &q, &p); // outside edge j
        for &(pe, qe) in &edges[..j] {
            if part.len() < 3 {
                break;
            }
            part = clip_half_plane(&part, &pe, &qe);
        }
        if part.len() >= 3 && signed_area(&part).abs() > 0.0 {
            out.push(part);
        }
    }
}

/// The edges of a polygon, wound counter-clockwise whichever way it was given.
fn edges(poly: &[Vector2<f64>]) -> impl Iterator<Item = (Vector2<f64>, Vector2<f64>)> + '_ {
    let flip = signed_area(poly) < 0.0;
    (0..poly.len()).map(move |i| {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if flip { (b, a) } else { (a, b) }
    })
}

/// Keep the part of `poly` on the left of the line `p -> q`.
fn clip_half_plane(poly: &[Vector2<f64>], p: &Vector2<f64>, q: &Vector2<f64>) -> Vec<Vector2<f64>> {
    let dir = q - p;
    let len = dir.norm();
    if !len.is_finite() || len <= 0.0 {
        return poly.to_vec();
    }
    // A true distance, so the tolerance below is a length in the target's own
    // units rather than an area-like cross product.
    let normal = Vector2::new(-dir.y, dir.x) / len;
    let dist = |x: &Vector2<f64>| normal.dot(&(x - p));
    let eps = ROUNDOFF
        * f64::EPSILON
        * poly
            .iter()
            .map(|x| x.abs().max())
            .fold(p.abs().max(), f64::max);

    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (dist(&a), dist(&b));
        if da >= -eps {
            out.push(a);
        }
        if (da >= -eps) != (db >= -eps) && da != db {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spacecraft::surface::PanelOptics;
    use nalgebra::Vector3;

    fn optics() -> PanelOptics {
        PanelOptics::absorber()
    }

    /// Every quantity below is of order one, so an absolute tolerance says
    /// what a relative one would and reads as the number of digits kept.
    fn near(got: f64, want: f64, what: &str) {
        assert!((got - want).abs() < 1e-12, "{what}: got {got}, want {want}");
    }

    /// The bus -y face of the shape issue #407 measures on: 1 m cube, the face
    /// at y = -0.5, in-plane axes u = +x and v = n x u = +z.
    fn bus_minus_y() -> SurfacePanel {
        SurfacePanel::rectangle([0.5, 0.5], Vector3::x(), -Vector3::y(), 2.2, optics())
            .with_cp_offset(Vector3::new(0.0, -0.5, 0.0))
    }

    /// The -y SAP: 2 m x 1 m in the y-z plane at x = 0, centred at y = -1.6, so
    /// it spans |y| from 0.6 to 2.6.
    fn sap_minus_y() -> SurfacePanel {
        SurfacePanel::rectangle([1.0, 0.5], Vector3::y(), Vector3::x(), 2.2, optics())
            .with_cp_offset(Vector3::new(0.0, -1.6, 0.0))
    }

    /// Sun in the x-y plane, `theta` from +x, on the -y side.
    fn sun(theta_deg: f64) -> Vector3<f64> {
        let t = theta_deg.to_radians();
        Vector3::new(t.cos(), -t.sin(), 0.0)
    }

    /// A panel parallel to the x-y plane at `z`, spanning the given ranges.
    fn plate_at(z: f64, x: (f64, f64), y: (f64, f64)) -> SurfacePanel {
        let half = [(x.1 - x.0) / 2.0, (y.1 - y.0) / 2.0];
        SurfacePanel::rectangle(half, Vector3::x(), Vector3::z(), 2.2, optics())
            .with_cp_offset(Vector3::new((x.0 + x.1) / 2.0, (y.0 + y.1) / 2.0, z))
    }

    #[test]
    fn a_sap_shadow_across_a_bus_face_matches_the_closed_form() {
        // The shadow is a strip in x: a ray from (x, -0.5, z) toward the Sun
        // meets the SAP's plane x = 0 at y = -0.5 + x*tan(theta), so the
        // shadowed x are [-(2.6 - 0.5)/tan, -(0.6 - 0.5)/tan], clipped to the
        // face. Lit area and centroid follow in closed form.
        let target = bus_minus_y();
        let panels = vec![target.clone(), sap_minus_y()];
        for (theta, want_fraction, want_u) in [
            (80.0, 0.647_346_038_583_069_8, 0.105_663_192_481_098_5),
            (85.0, 0.825_022_672_948_152_1, 0.020_410_815_717_392_28),
            // At 66 deg the far edge of the strip runs off the face, so the
            // shadow is clipped and the lit part is a single rectangle.
            (66.0, 0.544_522_868_530_853_6, 0.227_738_565_734_573_2),
        ] {
            let lit = lit_region(&panels[0], &panels, &sun(theta));
            near(lit.fraction, want_fraction, "lit fraction");
            // u = +x, and the centroid stays on the face (y = -0.5, z = 0).
            near(lit.centroid.x, want_u, "centroid along u");
            near(lit.centroid.y, -0.5, "centroid off the face");
            assert!(lit.centroid.z.abs() < 1e-15, "{:?}", lit.centroid);
        }
        // Nothing to do with the target's own outline being present twice.
        assert_eq!(
            lit_region(&target, &[sap_minus_y()], &sun(80.0)),
            lit_region(&panels[0], &panels, &sun(80.0))
        );
    }

    #[test]
    fn two_overlapping_shadows_count_their_union_once() {
        // Target [-2, 2] x [-1, 1] with the Sun straight along +z, so the
        // casters project straight down. S1 = [-2, 1] x [-1, 0] and
        // S2 = [-1, 2] x [-1, 0] each cover 3 of the 8; their union is the
        // whole bottom half, 4 of 8. Summing the two areas would say 2 of 8.
        let target = plate_at(0.0, (-2.0, 2.0), (-1.0, 1.0));
        let panels = vec![
            target.clone(),
            plate_at(1.0, (-2.0, 1.0), (-1.0, 0.0)),
            plate_at(1.0, (-1.0, 2.0), (-1.0, 0.0)),
        ];
        let lit = lit_region(&panels[0], &panels, &Vector3::z());
        near(lit.fraction, 0.5, "lit fraction");
        // v = n x u = z x x = y, so the lit half's centre sits at y = +0.5.
        near(lit.centroid.y, 0.5, "centroid along v");
        assert!(lit.centroid.x.abs() < 1e-15, "{:?}", lit.centroid);
    }

    #[test]
    fn the_order_of_the_casters_does_not_change_the_answer() {
        let target = plate_at(0.0, (-2.0, 2.0), (-1.0, 1.0));
        let a = plate_at(1.0, (-2.0, 1.0), (-1.0, 0.0));
        let b = plate_at(1.0, (-1.0, 2.0), (-1.0, 0.0));
        let one = lit_region(&target, &[a.clone(), b.clone()], &Vector3::z());
        let other = lit_region(&target, &[b, a], &Vector3::z());
        assert_eq!(one, other);
    }

    #[test]
    fn a_caster_given_twice_shadows_no_more_than_once() {
        let target = bus_minus_y();
        let once = lit_region(&target, &[sap_minus_y()], &sun(80.0));
        let twice = lit_region(&target, &[sap_minus_y(), sap_minus_y()], &sun(80.0));
        assert_eq!(once, twice);
    }

    #[test]
    fn a_caster_covering_the_whole_face_leaves_nothing_lit() {
        let target = plate_at(0.0, (-1.0, 1.0), (-1.0, 1.0));
        let cover = plate_at(1.0, (-2.0, 2.0), (-2.0, 2.0));
        let lit = lit_region(&target, &[cover], &Vector3::z());
        assert_eq!(lit.fraction, 0.0);
        assert_eq!(lit.centroid, target.cp_offset);
    }

    #[test]
    fn a_caster_of_exactly_the_same_size_leaves_nothing_lit() {
        // The old boolean test needed a widened outline to answer this; here
        // the area itself lands within roundoff of the whole face and is
        // snapped, so no outline has to be enlarged.
        let target = plate_at(0.0, (-1.0, 1.0), (-1.0, 1.0));
        let same = plate_at(1.0, (-1.0, 1.0), (-1.0, 1.0));
        let lit = lit_region(&target, &[same], &Vector3::z());
        assert_eq!(lit.fraction, 0.0);
    }

    #[test]
    fn a_panel_with_no_caster_is_fully_lit_at_its_own_centre_of_pressure() {
        let target = bus_minus_y();
        let lit = lit_region(&target, &[target.clone()], &sun(80.0));
        assert_eq!(lit.fraction, 1.0);
        assert_eq!(lit.centroid, target.cp_offset);
    }

    #[test]
    fn a_panel_without_an_outline_is_fully_lit_and_casts_nothing() {
        let bare = SurfacePanel::at_com(2.0, Vector3::x(), 2.2, optics());
        assert_eq!(
            lit_region(&bare, &[sap_minus_y()], &sun(80.0)).fraction,
            1.0
        );
        // And a bare panel standing in front of a real one takes nothing away.
        let target = bus_minus_y();
        let blocker = SurfacePanel::at_com(100.0, -Vector3::y(), 2.2, optics())
            .with_cp_offset(Vector3::new(0.0, -1.0, 0.0));
        assert_eq!(lit_region(&target, &[blocker], &sun(80.0)).fraction, 1.0);
    }

    #[test]
    fn the_two_faces_of_a_plate_do_not_shadow_each_other() {
        let front = plate_at(0.0, (-1.0, 1.0), (-1.0, 1.0));
        let back = front.back_face(optics());
        assert_eq!(lit_region(&front, &[back], &Vector3::z()).fraction, 1.0);
    }

    #[test]
    fn a_caster_tilted_through_the_target_shadows_only_the_part_in_front() {
        // A 2 m square caster through the middle of the target, turned 45 deg
        // about x, so half of it lies behind the target's plane. Carried down
        // the +z light, its front half covers y in [-1/sqrt(2), 0]: an area of
        // sqrt(2) out of the target's 4. Projecting the whole caster instead
        // would shadow twice that, so the number pins the half-space clip.
        let target = plate_at(0.0, (-1.0, 1.0), (-1.0, 1.0));
        let tilted = SurfacePanel::rectangle(
            [1.0, 1.0],
            Vector3::x(),
            Vector3::new(0.0, 1.0, 1.0),
            2.2,
            optics(),
        );
        let lit = lit_region(&target, &[tilted], &Vector3::z());
        let root2 = 2.0_f64.sqrt();
        near(lit.fraction, (4.0 - root2) / 4.0, "lit fraction");
        // Lit is y in [-1, -1/sqrt(2)] plus y in [0, 1], whose first moment is
        // 0.5 m³ over an area of 4 - sqrt(2).
        near(lit.centroid.y, 0.5 / (4.0 - root2), "centroid along v");
    }

    #[test]
    fn the_lit_fraction_is_unchanged_by_rotating_the_whole_spacecraft() {
        use nalgebra::{Rotation3, Unit};
        let rot =
            Rotation3::from_axis_angle(&Unit::new_normalize(Vector3::new(0.3, -0.7, 0.6)), 0.9);
        let turn = |p: &SurfacePanel| {
            let mut q = p.clone();
            q.normal = rot * p.normal;
            q.cp_offset = rot * p.cp_offset;
            if let Some(PanelOutline::Rectangle {
                half_extent,
                in_plane_x,
            }) = p.outline
            {
                q.outline = Some(PanelOutline::Rectangle {
                    half_extent,
                    in_plane_x: rot * in_plane_x,
                });
            }
            q
        };
        let flat = lit_region(&bus_minus_y(), &[sap_minus_y()], &sun(80.0));
        let spun = lit_region(
            &turn(&bus_minus_y()),
            &[turn(&sap_minus_y())],
            &(rot * sun(80.0)),
        );
        near(spun.fraction, flat.fraction, "lit fraction under rotation");
        assert!(
            (spun.centroid - rot * flat.centroid).norm() < 1e-12,
            "centroid did not turn with the spacecraft: {:?} vs {:?}",
            spun.centroid,
            rot * flat.centroid
        );
    }

    #[test]
    fn moving_the_whole_spacecraft_moves_the_centroid_with_it() {
        let shift = Vector3::new(-4.0, 11.0, 2.5);
        let move_by = |p: &SurfacePanel| {
            let mut q = p.clone();
            q.cp_offset += shift;
            q
        };
        let here = lit_region(&bus_minus_y(), &[sap_minus_y()], &sun(80.0));
        let there = lit_region(
            &move_by(&bus_minus_y()),
            &[move_by(&sap_minus_y())],
            &sun(80.0),
        );
        near(
            there.fraction,
            here.fraction,
            "lit fraction under translation",
        );
        assert!(
            (there.centroid - (here.centroid + shift)).norm() < 1e-12,
            "centroid did not move with the spacecraft: {:?}",
            there.centroid
        );
    }
}
