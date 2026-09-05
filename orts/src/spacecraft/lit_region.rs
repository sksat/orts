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

/// A convex polygon of at most [`MAX_SHADOW_VERTICES`] vertices, on the stack.
///
/// The pairwise work — a caster clipped to the target's plane and then to the
/// target — runs for every panel pair at every integrator stage and is bounded,
/// so it does not go to the heap. Only a pair that really overlaps allocates,
/// and only for the subtraction, whose piece count is not bounded this way.
#[derive(Clone, Copy)]
struct StackPoly {
    pts: [Vector2<f64>; MAX_SHADOW_VERTICES],
    len: usize,
}

impl StackPoly {
    fn new() -> Self {
        Self {
            pts: [Vector2::zeros(); MAX_SHADOW_VERTICES],
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn as_slice(&self) -> &[Vector2<f64>] {
        &self.pts[..self.len]
    }
}

/// Where a clip writes its result.
///
/// The pairwise path wants the stack and the subtraction wants the heap; one
/// clip serves both rather than two that can drift apart.
trait Vertices {
    fn clear(&mut self);
    fn push(&mut self, p: Vector2<f64>);
}

impl Vertices for StackPoly {
    fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, p: Vector2<f64>) {
        // The bound is `MAX_SHADOW_VERTICES` by construction; a debug assert
        // rather than a release panic, and dropping the vertex is the same
        // failure the bound rules out.
        debug_assert!(self.len < MAX_SHADOW_VERTICES, "clip overran its bound");
        if self.len < MAX_SHADOW_VERTICES {
            self.pts[self.len] = p;
            self.len += 1;
        }
    }
}

impl Vertices for Vec<Vector2<f64>> {
    fn clear(&mut self) {
        Vec::clear(self);
    }

    fn push(&mut self, p: Vector2<f64>) {
        Vec::push(self, p);
    }
}

/// Roundoff allowance, as a multiple of [`f64::EPSILON`] times the magnitudes
/// that went into a subtraction.
///
/// A fixed length cannot serve: [`SurfacePanel::rectangle`] accepts
/// half-extents from `1e-150` to `1e150`, and a nanometre is either far below
/// the noise or wider than the whole panel. Everything below scales its
/// tolerance by the coordinates it is comparing.
const ROUNDOFF: f64 = 32.0;

/// The smallest `normal · upstream` a panel is given a force for.
///
/// This is a force cutoff, not just a guard against dividing by zero. Both
/// force laws carry the projected area `A cos θ`, so a panel this close to
/// edge-on contributes at most `1e-12` of its face-on load. It has to exist
/// because [`lit_region`] carries shadow vertices to the target's plane along
/// `1 / cos θ`, which sends them past the range of `f64` before `cos θ`
/// reaches zero.
pub(crate) const MIN_FORCE_COSINE: f64 = 1e-12;

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
/// length. Which way the panel faces does not matter: a shadow is measured
/// along the light, so a panel whose normal points away from the source is
/// answered for correctly too, and the force models decide separately whether
/// such a panel gets a force at all.
///
/// The magnitude of `normal · upstream` does matter. The projection divides by
/// it, so within [`MIN_FORCE_COSINE`] of edge-on the shadow vertices leave the
/// range of `f64`; the caller's force cutoff is what keeps the arithmetic in
/// range, and a vertex that overflows anyway drops its shadow.
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
    let Some((u, v)) = panel.in_plane_axes() else {
        return LitRegion::all_lit(panel);
    };
    let PanelOutline::Rectangle {
        half_extent: [hx, hy],
        ..
    } = panel
        .outline
        .expect("in_plane_axes answered, so there is an outline");
    // The plane coordinates are in units of the panel's own half-extents, so
    // the target is the unit square whatever the panel measures. Working in
    // metres instead puts the aspect ratio into every comparison: `rectangle`
    // judges the half-extents by their product, so a panel of a few square
    // metres can be `f64::MAX` along one axis and `1e-308` along the other,
    // and its first moment overflows before the shadow is even subtracted.
    let half = [hx, hy];

    let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
    let Some(corners) = panel.corners_into(&mut buf) else {
        return LitRegion::all_lit(panel);
    };
    let mut target = StackPoly::new();
    for c in corners {
        target.push(to_plane(c, &panel.cp_offset, &u, &v, half));
    }
    let target = target;
    let target_area = signed_area(target.as_slice()).abs();
    if !target_area.is_finite() || target_area <= 0.0 {
        return LitRegion::all_lit(panel);
    }

    // Every shadow, already trimmed to the target so that later arithmetic
    // stays inside the target's own scale.
    // Each shadow came out of the bounded pairwise path, so they share one
    // allocation instead of taking one apiece.
    let mut shadows: Vec<StackPoly> = Vec::new();
    for other in others {
        if std::ptr::eq(other, panel) {
            continue;
        }
        if let Some(s) = shadow_on_target(panel, other, upstream, &u, &v, half, target.as_slice()) {
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
    let mut pieces = vec![target.as_slice().to_vec()];
    let mut next: Vec<Vec<Vector2<f64>>> = Vec::new();
    for shadow in &shadows {
        next.clear();
        for piece in &pieces {
            subtract_into(piece, shadow.as_slice(), &mut next);
        }
        std::mem::swap(&mut pieces, &mut next);
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

    // Back from half-extents to metres.
    let centre = moment / lit_area;
    LitRegion {
        fraction: (lit_area / target_area).clamp(0.0, 1.0),
        centroid: panel.cp_offset + u * (centre.x * hx) + v * (centre.y * hy),
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
    half: [f64; 2],
    target: &[Vector2<f64>],
) -> Option<StackPoly> {
    let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
    let corners = other.corners_into(&mut buf)?;

    let denom = panel.normal.dot(upstream);
    // Distance from the target's plane, along the normal.
    let depth = |p: &Vector3<f64>| panel.normal.dot(&(p - panel.cp_offset));
    // Positive on the side the source is on. The sign is what makes the answer
    // right for a target whose normal points away from the source; the
    // magnitude stays a distance in metres, which is what the tolerance below
    // is scaled for. Dividing by `denom` here instead would multiply the
    // roundoff in a coplanar caster's depth by `1 / cos θ` and let it read as
    // standing in front at oblique incidence.
    let toward_source = |p: &Vector3<f64>| depth(p) * denom.signum();

    // Only the part of the caster on the source's side of the target can
    // shadow it. Clipping in 3D first, before projecting, is what makes a
    // caster that tilts through the target shadow only the part of it that is
    // really in front — projecting the whole caster would report more shadow
    // than there is.
    let mut front_buf = [Vector3::zeros(); MAX_PANEL_CORNERS + 1];
    let front = clip_3d(
        corners,
        &toward_source,
        &|p: &Vector3<f64>| plane_roundoff(&panel.normal, p, &panel.cp_offset),
        &mut front_buf,
    );
    if front.len() < 3 {
        return None;
    }

    // Carry each front vertex along the light to the target's plane.
    let mut poly = StackPoly::new();
    for q in front {
        let hit = q - upstream * (depth(q) / denom);
        let p = to_plane(&hit, &panel.cp_offset, u, v, half);
        // Grazing incidence can send a vertex past the range of f64 even though
        // every input was finite. A shadow that cannot be located is dropped,
        // which leaves the force in place rather than removing one that exists.
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        poly.push(p);
    }

    // Trim to the target, one target edge at a time, on the stack: this runs
    // for every panel pair at every integrator stage, and only a pair that
    // really does overlap goes on to allocate.
    let mut spare = StackPoly::new();
    for (a, b) in edges(target) {
        if poly.len() < 3 {
            return None;
        }
        clip_half_plane(poly.as_slice(), &a, &b, &mut spare);
        std::mem::swap(&mut poly, &mut spare);
    }

    let area = signed_area(poly.as_slice()).abs();
    let scale = ROUNDOFF * f64::EPSILON * signed_area(target).abs();
    (poly.len() >= 3 && area > scale).then_some(poly)
}

/// A point in the panel's plane, in units of the panel's half-extents.
///
/// The division comes after the dot product, not before: `rectangle` accepts a
/// half-extent of `1e-310` as long as the area is finite, and its reciprocal is
/// not representable, so scaling the axis first would send every coordinate to
/// infinity.
fn to_plane(
    p: &Vector3<f64>,
    origin: &Vector3<f64>,
    u: &Vector3<f64>,
    v: &Vector3<f64>,
    half: [f64; 2],
) -> Vector2<f64> {
    let d = p - origin;
    Vector2::new(d.dot(u) / half[0], d.dot(v) / half[1])
}

/// Keep the part of a 3D convex polygon where `depth` is above its own
/// roundoff, judged by `eps` at each vertex.
///
/// Writes into `out` and returns the filled prefix: a half-space clip of a
/// panel outline adds at most one vertex, so the caller can hold the result on
/// the stack.
fn clip_3d<'b>(
    poly: &[Vector3<f64>],
    depth: &impl Fn(&Vector3<f64>) -> f64,
    eps: &impl Fn(&Vector3<f64>) -> f64,
    out: &'b mut [Vector3<f64>; MAX_PANEL_CORNERS + 1],
) -> &'b [Vector3<f64>] {
    let cap = out.len();
    let mut n = 0;
    for i in 0..poly.len() {
        let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
        let (da, db) = (depth(a), depth(b));
        let mut write = |p: Vector3<f64>| {
            debug_assert!(n < cap, "the half-space clip overran its bound");
            if n < cap {
                out[n] = p;
                n += 1;
            }
        };
        if da > eps(a) {
            write(*a);
        }
        if (da > eps(a)) != (db > eps(b)) && da != db {
            write(crossing(*a, *b, da, db));
        }
    }
    &out[..n]
}

/// The roundoff `n · (p - origin)` can carry.
///
/// Two sources, and both have to be counted. The dot product's own error
/// follows the terms it adds up — not the length of either vector: for a panel
/// `1e308` m along one axis a tolerance scaled from that coordinate comes to
/// `1e293` and swallows every real distance, while the term along the normal,
/// the only one the dot product keeps, may be exactly zero.
///
/// The subtraction adds error of its own, proportional to the operands rather
/// than to the difference. A corner is built as `cp_offset + offset`, which
/// rounds at the scale of `cp_offset`, so recovering `offset` from it leaves a
/// residue of about `EPSILON * |cp_offset|` — 1e-14 m for a panel a hundred
/// metres from the CoM. Scaling only from the recovered difference put that
/// residue above the tolerance and let a coincident back face read as standing
/// in front of its own front face. Positions that far out are not known any
/// closer than that, so the tolerance has to say so.
fn plane_roundoff(n: &Vector3<f64>, p: &Vector3<f64>, origin: &Vector3<f64>) -> f64 {
    let d = p - origin;
    // The largest term rather than their sum: a coordinate can reach
    // `f64::MAX`, where a sum of terms that size overflows and leaves an
    // infinite tolerance — which would call every point coplanar and drop
    // every shadow. `ROUNDOFF` already covers the factor of three this gives
    // up against the sum. Nothing is taken from an axis the normal ignores
    // either: a coordinate whose own subtraction overflowed would otherwise
    // turn `0 * inf` into a NaN tolerance, which rejects every point.
    let term = |i: usize| {
        if n[i] == 0.0 {
            0.0
        } else {
            n[i].abs() * d[i].abs().max(p[i].abs()).max(origin[i].abs())
        }
    };
    ROUNDOFF * f64::EPSILON * term(0).max(term(1)).max(term(2))
}

/// Where a segment crosses a boundary, given the signed distances at its ends.
///
/// As a convex combination rather than `a + (b - a) * t`: an accepted panel can
/// put its corners at `±f64::MAX`, where the difference of two finite endpoints
/// overflows and the vertex comes out NaN, which drops a shadow that is really
/// there. The parameter is formed from distances scaled by the larger of the
/// two for the same reason.
fn crossing<T>(a: T, b: T, da: f64, db: f64) -> T
where
    T: std::ops::Mul<f64, Output = T> + std::ops::Add<T, Output = T>,
{
    let m = da.abs().max(db.abs());
    let (da, db) = (da / m, db / m);
    // Clamped to the segment. The side test allows each end its own roundoff
    // tolerance, so both ends can be on the same side of zero while landing on
    // opposite sides of the test — distances of `2eps` and `eps/2` give
    // `t = 4/3` — and an unclamped parameter would put the vertex a third of an
    // edge past the end, growing a nearly coplanar caster instead of clipping
    // it. Clamping keeps the boundary where the distances put it, which
    // shifting them by the tolerance instead would not.
    let t = (da / (da - db)).clamp(0.0, 1.0);
    a * (1.0 - t) + b * t
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
    // A piece can carry a vertex per edge of every shadow subtracted so far,
    // which is not bounded the way the pairwise work is, so these are on the
    // heap — and reused across the edges rather than allocated per edge.
    let mut part: Vec<Vector2<f64>> = Vec::new();
    let mut spare: Vec<Vector2<f64>> = Vec::new();
    // The shadow is one of the bounded polygons the pairwise work produced,
    // so its edges fit an array.
    let mut edge_buf = [(Vector2::zeros(), Vector2::zeros()); MAX_SHADOW_VERTICES];
    let mut n_edges = 0;
    // A zero-length edge is dropped rather than used as a half-plane. Clipping
    // by one keeps the whole polygon — it bounds nothing — and here that would
    // hand back the whole piece as the part "outside" it, on top of the parts
    // the real edges contribute, so the pieces would overlap and their areas
    // would add up to more than the piece they came from. That total then
    // reads as a fully lit panel.
    for e in edges(shadow).filter(|(a, b)| a != b) {
        debug_assert!(
            n_edges < MAX_SHADOW_VERTICES,
            "a shadow grew past its bound"
        );
        if n_edges < MAX_SHADOW_VERTICES {
            edge_buf[n_edges] = e;
            n_edges += 1;
        }
    }
    let shadow_edges = &edge_buf[..n_edges];
    for j in 0..shadow_edges.len() {
        let (p, q) = shadow_edges[j];
        clip_half_plane(piece, &q, &p, &mut part); // outside edge j
        for &(pe, qe) in &shadow_edges[..j] {
            if part.len() < 3 {
                break;
            }
            clip_half_plane(&part, &pe, &qe, &mut spare);
            std::mem::swap(&mut part, &mut spare);
        }
        if part.len() >= 3 && signed_area(&part).abs() > 0.0 {
            out.push(part.clone());
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

/// Keep the part of `poly` on the left of the line `p -> q`, in `out`.
fn clip_half_plane(
    poly: &[Vector2<f64>],
    p: &Vector2<f64>,
    q: &Vector2<f64>,
    out: &mut impl Vertices,
) {
    let dir = q - p;
    // Scaled by the largest component rather than by the length. `rectangle`
    // accepts half-extents from 1e-150 to 1e150 and judges only their product,
    // so a panel `f64::MAX` by `1e-308` is a valid 7.2 m² rectangle whose edge
    // lengths overflow one way and underflow the other. The largest component
    // is representable whenever the edge is, and dividing by it leaves a
    // direction whose components are at most one.
    let scale = dir.abs().max();
    if !scale.is_finite() || scale <= 0.0 {
        // The edge is a point, so it bounds nothing: pass the polygon through.
        out.clear();
        for x in poly {
            out.push(*x);
        }
        return;
    }
    let unit = dir / scale;
    // Not a true distance — `unit` is only unit-ish — but linear in the point
    // and in the same units as the coordinates, which is what the tolerance
    // below is scaled from.
    let normal = Vector2::new(-unit.y, unit.x);
    let dist = |x: &Vector2<f64>| normal.dot(&(x - p));
    // Per vertex, and from the terms this dot product adds up plus the
    // operands of its subtraction — the same bound the 3D side uses, for the
    // same two reasons: a panel with one huge axis and one tiny one needs a
    // different tolerance along each, and a difference of two large
    // coordinates is not known to the precision of the difference.
    let eps = |x: &Vector2<f64>| {
        let d = x - p;
        // Same shape as the 3D bound, and for the same reasons: the largest
        // term rather than the sum, because a plane coordinate can reach
        // `f64::MAX` where the sum overflows into an infinite tolerance; and
        // nothing from an axis the normal ignores, because a coordinate whose
        // own subtraction overflowed would otherwise turn `0 * inf` into NaN.
        let term = |n: f64, d: f64, a: f64, b: f64| {
            if n == 0.0 {
                0.0
            } else {
                n.abs() * d.abs().max(a.abs()).max(b.abs())
            }
        };
        ROUNDOFF * f64::EPSILON * term(normal.x, d.x, x.x, p.x).max(term(normal.y, d.y, x.y, p.y))
    };

    out.clear();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (dist(&a), dist(&b));
        if da >= -eps(&a) {
            out.push(a);
        }
        // A vertex exactly on the line is kept above, and the crossing on the
        // edge that leaves the line from it is that same vertex, so the result
        // can carry it twice. The area and the first moment are unaffected —
        // the repeated pair contributes a zero cross product — and the
        // zero-length edge it leaves is dropped where edges are used as
        // half-planes, which is the only place it would matter.
        if (da >= -eps(&a)) != (db >= -eps(&b)) && da != db {
            out.push(crossing(a, b, da, db));
        }
    }
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
        // To roundoff, not bitwise: the shadows are subtracted in the order
        // given, so the areas and moments are summed in a different order
        // either way, and floating-point addition is not associative.
        near(one.fraction, other.fraction, "lit fraction");
        assert!(
            (one.centroid - other.centroid).norm() < 1e-15,
            "{:?} vs {:?}",
            one.centroid,
            other.centroid
        );
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
    fn a_caster_reaching_the_ends_of_f64_keeps_the_answer_finite() {
        // In units of the target's half-extents, a caster's coordinates are
        // its size over the target's, and `rectangle` accepts both ends of
        // that ratio: a target 1e-150 m across with a caster 1e158 by 1e-150
        // (4e8 m², a finite area, which is all the constructor asks) is 1e308
        // by 1 in plane units, so two of its corners differ by more than f64
        // can hold.
        //
        // How much of the target that shadows is not answerable in f64: the
        // crossing parameter carries sixteen digits and the segment spans
        // 1e308, so the target's own edge sits far below the resolution of the
        // interpolation. What has to hold is that nothing NaN or infinite
        // escapes — a NaN fraction would poison the whole spacecraft's
        // acceleration, not just this panel's. That is what the convex
        // combination and the tolerances taken from the largest term buy: the
        // difference of the endpoints and the sum of the tolerance's terms
        // both overflow here.
        let normal = Vector3::z();
        let small = 1e-150;
        let target = SurfacePanel::rectangle([small, small], Vector3::x(), normal, 2.2, optics());
        let vast = SurfacePanel::rectangle([1e158, small], Vector3::x(), normal, 2.2, optics())
            .with_cp_offset(Vector3::z() * small);

        let lit = lit_region(&target, &[vast], &normal);
        assert!(
            (0.0..=1.0).contains(&lit.fraction),
            "the lit share has to stay a share, got {}",
            lit.fraction
        );
        assert!(
            lit.centroid.iter().all(|c| c.is_finite()),
            "{:?}",
            lit.centroid
        );
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
    fn a_back_face_built_from_a_barely_off_axis_shadows_nothing() {
        // `rectangle` accepts an in-plane axis up to 1e-9 off perpendicular. At
        // 5e-10 the corners it generates sit 5e-10 either side of the panel's
        // own plane — seventy thousand times the roundoff the shadow arithmetic
        // works at — so half of a coincident back face read as standing in
        // front of the front face and shadowed half of it. The axes are
        // projected back onto the plane, so both faces are exactly coplanar and
        // neither shadows the other.
        let normal = Vector3::z();
        let off_axis = Vector3::new(1.0, 0.0, 5e-10).normalize();
        assert!(
            normal.dot(&off_axis) > 1e-10,
            "the axis has to be off perpendicular for this to be the case"
        );
        let front = SurfacePanel::rectangle([1.0, 1.0], off_axis, normal, 2.2, optics());
        let back = front.back_face(optics());
        assert_eq!(
            lit_region(&front, &[back.clone()], &normal).fraction,
            1.0,
            "a coincident back face must not shadow the front"
        );
        assert_eq!(
            lit_region(&back, &[front], &-normal).fraction,
            1.0,
            "nor the other way round"
        );
    }

    #[test]
    fn a_panel_of_an_extreme_aspect_ratio_is_still_shadowed() {
        // `rectangle` judges the half-extents by their product, so both of
        // these are accepted rectangles of a few square metres. Each breaks a
        // different piece of arithmetic if the geometry is done in metres: the
        // first has edges that overflow a length one way and underflow it the
        // other, and a first moment that overflows; the second has a
        // half-extent that is subnormal, whose reciprocal overflows. A caster
        // over half of either has to take half of it.
        let normal = Vector3::z();
        for (long, short) in [(f64::MAX / 2.0, 1e-308), (1e300, 1e-310)] {
            let target =
                SurfacePanel::rectangle([long, short], Vector3::x(), normal, 2.2, optics());
            // Covers x in [0, long] of the target's x in [-long, long].
            let caster =
                SurfacePanel::rectangle([long / 2.0, short], Vector3::x(), normal, 2.2, optics())
                    .with_cp_offset(Vector3::new(long / 2.0, 0.0, 1.0));

            let lit = lit_region(&target, &[caster], &normal);
            near(lit.fraction, 0.5, "lit fraction");
            near(
                lit.centroid.x / long,
                -0.5,
                "centroid along u, in half-extents",
            );
        }
    }

    #[test]
    fn a_back_face_far_from_the_centre_of_mass_shadows_nothing() {
        // A corner is built as `cp_offset + offset`, which rounds at the scale
        // of `cp_offset`, so recovering the offset from it leaves a residue of
        // about `EPSILON * |cp_offset|` — 1e-14 m at a hundred metres, far
        // above the roundoff of the offset itself. A tolerance scaled only
        // from the recovered difference let that residue put part of a
        // coincident back face on the source's side. A boom-mounted array is
        // the shape this happens to, so the sweep runs out to 100 m.
        for arm in [1.0, 10.0, 100.0] {
            for i in 0..40 {
                let a = 0.19 * i as f64;
                let normal =
                    Vector3::new(a.cos(), (1.3 * a).sin(), (0.7 * a + 0.4).cos()).normalize();
                let u = normal.cross(&Vector3::new(0.2, 0.8, -0.1)).normalize();
                let at = Vector3::new((0.9 * a).cos(), (1.1 * a).sin(), (0.5 * a).cos()) * arm;
                let front = SurfacePanel::rectangle([1.0, 1.0], u, normal, 2.2, optics())
                    .with_cp_offset(at);
                let back = front.back_face(optics());
                assert_eq!(
                    lit_region(&front, &[back], &normal).fraction,
                    1.0,
                    "arm {arm} m, orientation {i}: a coincident back face must not \
                     shadow the front"
                );
            }
        }
    }

    #[test]
    fn a_coincident_back_face_shadows_nothing_at_grazing_incidence() {
        // The side test compares a distance from the target's plane against a
        // tolerance in metres. Dividing that distance by `cos θ` first — the
        // way the projection has to — turns the roundoff left in a coplanar
        // caster's corners into 1e-4 m at `cos θ = 1e-12`, which clears any
        // roundoff tolerance: measured that way, the back face takes half of
        // the front face's area.
        //
        // Whether that roundoff is there at all depends on the orientation:
        // the corner arithmetic cancels exactly for some and leaves up to
        // 2.2e-16 m for others, so one orientation cannot stand for the case.
        // The light arrives at the cutoff, the shallowest angle the force
        // models ask about.
        let mut worst_residue = 0.0f64;
        for i in 0..400 {
            let a = 0.017 * i as f64;
            // Every fourth orientation puts the normal on an axis, where the
            // corner arithmetic cancels exactly and the tolerance goes to zero
            // with it. Residue and tolerance both come from the normal's
            // components, so they scale together and neither family is the
            // dangerous one on its own.
            let normal = if i % 4 == 0 {
                Vector3::z()
            } else {
                Vector3::new(a.cos(), (1.3 * a).sin(), (0.7 * a + 0.4).cos()).normalize()
            };
            let u = normal.cross(&Vector3::new(0.2, 0.8, -0.1)).normalize();
            let front = SurfacePanel::rectangle([1.0, 1.0], u, normal, 2.2, optics());
            let back = front.back_face(optics());

            let mut buf = [Vector3::zeros(); MAX_PANEL_CORNERS];
            let residue = back
                .corners_into(&mut buf)
                .expect("a rectangle has corners")
                .iter()
                .map(|c| normal.dot(&(c - front.cp_offset)).abs())
                .fold(0.0, f64::max);
            worst_residue = worst_residue.max(residue);

            let grazing = (u * (1.0 - MIN_FORCE_COSINE * MIN_FORCE_COSINE).sqrt()
                + normal * MIN_FORCE_COSINE)
                .normalize();
            assert!(
                normal.dot(&grazing) >= MIN_FORCE_COSINE * 0.9,
                "orientation {i}: the light has to arrive at the cutoff, got n·s = {}",
                normal.dot(&grazing)
            );
            assert_eq!(
                lit_region(&front, &[back], &grazing).fraction,
                1.0,
                "orientation {i}: a coincident back face must not shadow the \
                 front, however shallow the light"
            );
        }
        assert!(
            worst_residue > 0.0,
            "no orientation left any roundoff in the corners, so the sweep \
             cannot see the case it is about"
        );
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

    #[test]
    fn a_shadow_with_a_repeated_vertex_does_not_double_count_the_lit_part() {
        // A clip keeps a vertex that lies exactly on the line and then finds
        // the crossing on the edge that leaves the line from it, which is that
        // same vertex — so a shadow polygon can arrive here with a zero-length
        // edge. Clipping by such an edge keeps the whole polygon, and using it
        // as one of the subtraction's half-planes would hand back the whole
        // piece as the part "outside" it, on top of the real parts: the pieces
        // overlap and their areas add up to more than the piece they came
        // from, which reads as a fully lit panel.
        let piece = vec![
            Vector2::new(-1.0, -1.0),
            Vector2::new(1.0, -1.0),
            Vector2::new(1.0, 1.0),
            Vector2::new(-1.0, 1.0),
        ];
        // The bottom half, with its second corner written twice.
        let shadow = vec![
            Vector2::new(-1.0, -1.0),
            Vector2::new(1.0, -1.0),
            Vector2::new(1.0, 0.0),
            Vector2::new(1.0, 0.0),
            Vector2::new(-1.0, 0.0),
        ];
        let mut pieces = Vec::new();
        subtract_into(&piece, &shadow, &mut pieces);
        let total: f64 = pieces.iter().map(|p| signed_area(p).abs()).sum();
        near(total, 2.0, "the lit area of a square minus its bottom half");
    }
}
