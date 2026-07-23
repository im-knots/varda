//! Curve authoring + flattening for 2D surfaces.
//!
//! A [`SurfacePath`] is an optional authoring layer on a `Surface`: an ordered
//! list of line / cubic-bezier segments. It is flattened to the polygon in
//! `Surface::vertices` (the single source of truth every downstream consumer
//! reads) whenever the path is edited — mirroring the `CircleHint` pattern.
//!
//! This is the one shared bezier flattener: SVG import and on-canvas bezier
//! editing both go through here, so there is a single sampling convention and
//! no parallel curve math elsewhere in the codebase.

/// Default subdivision counts for bezier sampling, matched to the historical
/// SVG import behavior so detection output stays stable.
pub const QUAD_STEPS: usize = 8;
pub const CUBIC_STEPS: usize = 12;

pub use crate::engine::value::surface::{CubicHandle, PathSegment, SurfacePath};

impl PathSegment {
    /// The endpoint this segment terminates at.
    pub fn end(&self) -> [f32; 2] {
        match self {
            PathSegment::Line { to } => *to,
            PathSegment::Cubic { to, .. } => *to,
        }
    }
}

impl SurfacePath {
    /// Flatten this path to a polygon vertex list for `Surface::vertices`.
    ///
    /// The `start` point is emitted first, followed by the sampled points of
    /// each segment. The closing point is not duplicated — surfaces close
    /// implicitly at render time.
    pub fn flatten(&self) -> Vec<[f32; 2]> {
        let mut out: Vec<[f32; 2]> = Vec::with_capacity(1 + self.segments.len());
        out.push(self.start);
        let mut last = self.start;
        for seg in &self.segments {
            match seg {
                PathSegment::Line { to } => out.push(*to),
                PathSegment::Cubic { c1, c2, to } => {
                    out.extend(flatten_cubic(last, *c1, *c2, *to, CUBIC_STEPS));
                }
            }
            last = seg.end();
        }
        // A closed path's final segment returns to `start`; drop the duplicated
        // closing point so the polygon closes implicitly at render time.
        if self.closed && out.len() > 1 {
            let l = *out.last().unwrap();
            if (l[0] - self.start[0]).abs() < 1e-6 && (l[1] - self.start[1]).abs() < 1e-6 {
                out.pop();
            }
        }
        out
    }

    /// Build a closed path of straight-line segments from a polygon's vertices.
    ///
    /// Every edge — including the closing edge back to the first vertex — becomes
    /// an explicit segment, so each edge is individually addressable for bezier
    /// editing. `flatten` drops the duplicated closing point.
    pub fn from_polygon(verts: &[[f32; 2]], closed: bool) -> Self {
        let start = verts.first().copied().unwrap_or([0.0, 0.0]);
        let mut segments = Vec::with_capacity(verts.len());
        for v in verts.iter().skip(1) {
            segments.push(PathSegment::Line { to: *v });
        }
        if closed && verts.len() > 1 {
            segments.push(PathSegment::Line { to: start });
        }
        SurfacePath {
            start,
            segments,
            closed,
        }
    }

    /// Returns `true` if any segment is a cubic bezier (rather than a straight
    /// line) — i.e. the path carries curvature worth preserving.
    pub fn has_cubic(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, PathSegment::Cubic { .. }))
    }

    /// Apply `f` to every point of the path — `start` plus each segment's control
    /// points and endpoint. Used for normalization and affine transforms.
    pub fn apply_map(&mut self, f: impl Fn([f32; 2]) -> [f32; 2]) {
        self.start = f(self.start);
        for seg in &mut self.segments {
            match seg {
                PathSegment::Line { to } => *to = f(*to),
                PathSegment::Cubic { c1, c2, to } => {
                    *c1 = f(*c1);
                    *c2 = f(*c2);
                    *to = f(*to);
                }
            }
        }
    }

    /// Number of addressable edges (one per segment).
    pub fn edge_count(&self) -> usize {
        self.segments.len()
    }

    /// Number of distinct on-curve anchor points. A closed path's final segment
    /// returns to anchor 0, so it has one anchor per segment; an open path has
    /// one more anchor than segments (the start plus each endpoint).
    pub fn anchor_count(&self) -> usize {
        if self.closed {
            self.segments.len()
        } else {
            self.segments.len() + 1
        }
    }

    /// Position of anchor `idx`. Anchor 0 is `start`; anchor `i` is the endpoint
    /// of segment `i - 1`.
    pub fn anchor_pos(&self, idx: usize) -> [f32; 2] {
        if idx == 0 {
            self.start
        } else {
            self.segments
                .get(idx - 1)
                .map(|s| s.end())
                .unwrap_or(self.start)
        }
    }

    /// Start point of segment `idx` (the previous segment's endpoint, or `start`).
    pub fn segment_start(&self, idx: usize) -> [f32; 2] {
        if idx == 0 {
            self.start
        } else {
            self.segments
                .get(idx - 1)
                .map(|s| s.end())
                .unwrap_or(self.start)
        }
    }

    /// Whether edge `idx` is a cubic bezier.
    pub fn is_edge_cubic(&self, idx: usize) -> bool {
        matches!(self.segments.get(idx), Some(PathSegment::Cubic { .. }))
    }

    /// Convert edge `idx` from a line to a cubic, seeding the control points at
    /// the 1/3 and 2/3 points so the initial curve is visually identical.
    pub fn convert_edge_to_cubic(&mut self, idx: usize) {
        if idx >= self.segments.len() {
            return;
        }
        if !matches!(self.segments[idx], PathSegment::Line { .. }) {
            return;
        }
        let s = self.segment_start(idx);
        let e = self.segments[idx].end();
        let c1 = [s[0] + (e[0] - s[0]) / 3.0, s[1] + (e[1] - s[1]) / 3.0];
        let c2 = [
            s[0] + 2.0 * (e[0] - s[0]) / 3.0,
            s[1] + 2.0 * (e[1] - s[1]) / 3.0,
        ];
        self.segments[idx] = PathSegment::Cubic { c1, c2, to: e };
    }

    /// Convert edge `idx` back to a straight line, discarding control points.
    pub fn convert_edge_to_line(&mut self, idx: usize) {
        if let Some(seg) = self.segments.get_mut(idx) {
            *seg = PathSegment::Line { to: seg.end() };
        }
    }

    /// Move anchor `idx` to `pos`, dragging the adjacent segments' control
    /// handles along by the same delta so the local curvature is preserved.
    pub fn move_anchor(&mut self, idx: usize, pos: [f32; 2]) {
        let n = self.segments.len();
        if n == 0 {
            self.start = pos;
            return;
        }
        let old = self.anchor_pos(idx);
        let d = [pos[0] - old[0], pos[1] - old[1]];
        // Incoming segment (ends at this anchor): move its endpoint and c2.
        let incoming = (idx + n - 1) % n;
        if let Some(seg) = self.segments.get_mut(incoming) {
            match seg {
                PathSegment::Line { to } => *to = pos,
                PathSegment::Cubic { c2, to, .. } => {
                    *to = pos;
                    c2[0] += d[0];
                    c2[1] += d[1];
                }
            }
        }
        // Outgoing segment (starts at this anchor): move its c1.
        if let Some(PathSegment::Cubic { c1, .. }) = self.segments.get_mut(idx % n) {
            c1[0] += d[0];
            c1[1] += d[1];
        }
        if idx == 0 {
            self.start = pos;
        }
    }

    /// Move a cubic control handle of segment `idx` to `pos`. No-op if the
    /// segment is not a cubic.
    pub fn move_handle(&mut self, idx: usize, handle: CubicHandle, pos: [f32; 2]) {
        if let Some(PathSegment::Cubic { c1, c2, .. }) = self.segments.get_mut(idx) {
            match handle {
                CubicHandle::C1 => *c1 = pos,
                CubicHandle::C2 => *c2 = pos,
            }
        }
    }

    /// Sample edge `idx` into points for hit-testing / drawing. Includes the
    /// segment's start point followed by `steps` sampled points; a line edge
    /// returns just its two endpoints.
    pub fn sample_edge(&self, idx: usize, steps: usize) -> Vec<[f32; 2]> {
        let s = self.segment_start(idx);
        match self.segments.get(idx) {
            Some(PathSegment::Line { to }) => vec![s, *to],
            Some(PathSegment::Cubic { c1, c2, to }) => {
                let mut pts = Vec::with_capacity(1 + steps);
                pts.push(s);
                pts.extend(flatten_cubic(s, *c1, *c2, *to, steps));
                pts
            }
            None => vec![s],
        }
    }
}

/// Sample a quadratic bezier, emitting `steps` points for t in (0, 1]. The
/// start point `p0` is not included (callers already hold it).
pub fn flatten_quad(p0: [f32; 2], ctrl: [f32; 2], p1: [f32; 2], steps: usize) -> Vec<[f32; 2]> {
    let steps = steps.max(1);
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let inv = 1.0 - t;
            [
                inv * inv * p0[0] + 2.0 * inv * t * ctrl[0] + t * t * p1[0],
                inv * inv * p0[1] + 2.0 * inv * t * ctrl[1] + t * t * p1[1],
            ]
        })
        .collect()
}

/// Exactly convert a quadratic bezier's control point into the two control
/// points of an equivalent cubic bezier. Lets quads captured from SVG import be
/// stored as [`PathSegment::Cubic`] (the only curved segment kind) losslessly.
pub fn quad_to_cubic(p0: [f32; 2], ctrl: [f32; 2], p1: [f32; 2]) -> ([f32; 2], [f32; 2]) {
    let c1 = [
        p0[0] + 2.0 / 3.0 * (ctrl[0] - p0[0]),
        p0[1] + 2.0 / 3.0 * (ctrl[1] - p0[1]),
    ];
    let c2 = [
        p1[0] + 2.0 / 3.0 * (ctrl[0] - p1[0]),
        p1[1] + 2.0 / 3.0 * (ctrl[1] - p1[1]),
    ];
    (c1, c2)
}

/// Sample a cubic bezier, emitting `steps` points for t in (0, 1]. The start
/// point `p0` is not included (callers already hold it).
pub fn flatten_cubic(
    p0: [f32; 2],
    c1: [f32; 2],
    c2: [f32; 2],
    p1: [f32; 2],
    steps: usize,
) -> Vec<[f32; 2]> {
    let steps = steps.max(1);
    (1..=steps)
        .map(|i| {
            let t = i as f32 / steps as f32;
            let inv = 1.0 - t;
            let a = inv * inv * inv;
            let b = 3.0 * inv * inv * t;
            let c = 3.0 * inv * t * t;
            let d = t * t * t;
            [
                a * p0[0] + b * c1[0] + c * c2[0] + d * p1[0],
                a * p0[1] + b * c1[1] + c * c2[1] + d * p1[1],
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    #[test]
    fn cubic_emits_steps_points_ending_at_p1() {
        let pts = flatten_cubic([0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], CUBIC_STEPS);
        assert_eq!(pts.len(), CUBIC_STEPS);
        assert!(approx(*pts.last().unwrap(), [3.0, 0.0]));
        // Colinear control points → straight line: all y ~ 0.
        assert!(pts.iter().all(|p| p[1].abs() < 1e-4));
    }

    #[test]
    fn quad_emits_steps_points_ending_at_p1() {
        let pts = flatten_quad([0.0, 0.0], [1.0, 1.0], [2.0, 0.0], QUAD_STEPS);
        assert_eq!(pts.len(), QUAD_STEPS);
        assert!(approx(*pts.last().unwrap(), [2.0, 0.0]));
    }

    #[test]
    fn min_one_step() {
        assert_eq!(
            flatten_cubic([0.0; 2], [0.0; 2], [0.0; 2], [1.0, 0.0], 0).len(),
            1
        );
        assert_eq!(flatten_quad([0.0; 2], [0.0; 2], [1.0, 0.0], 0).len(), 1);
    }

    #[test]
    fn path_of_lines_flattens_to_polygon() {
        let path = SurfacePath {
            start: [0.0, 0.0],
            segments: vec![
                PathSegment::Line { to: [1.0, 0.0] },
                PathSegment::Line { to: [1.0, 1.0] },
                PathSegment::Line { to: [0.0, 1.0] },
            ],
            closed: true,
        };
        // start + 3 line endpoints, no duplicated closing point.
        assert_eq!(
            path.flatten(),
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        );
    }

    #[test]
    fn path_with_cubic_expands_segment() {
        let path = SurfacePath {
            start: [0.0, 0.0],
            segments: vec![PathSegment::Cubic {
                c1: [1.0, 0.0],
                c2: [2.0, 0.0],
                to: [3.0, 0.0],
            }],
            closed: false,
        };
        let verts = path.flatten();
        // start + CUBIC_STEPS sampled points.
        assert_eq!(verts.len(), 1 + CUBIC_STEPS);
        assert!(approx(verts[0], [0.0, 0.0]));
        assert!(approx(*verts.last().unwrap(), [3.0, 0.0]));
    }

    #[test]
    fn closed_defaults_true_on_deserialize() {
        // Old-style payload without `closed` → defaults to true.
        let json = r#"{"start":[0.0,0.0],"segments":[]}"#;
        let path: SurfacePath = serde_json::from_str(json).unwrap();
        assert!(path.closed);
    }

    fn square() -> SurfacePath {
        SurfacePath::from_polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], true)
    }

    #[test]
    fn from_polygon_makes_a_segment_per_edge_including_closing() {
        let p = square();
        // 4 vertices → 4 edges (last returns to start).
        assert_eq!(p.edge_count(), 4);
        assert_eq!(p.anchor_count(), 4);
        assert_eq!(p.segments.last().unwrap().end(), [0.0, 0.0]);
    }

    #[test]
    fn closing_edge_dedups_in_flatten() {
        // start + 4 endpoints, but the closing point == start is dropped.
        assert_eq!(
            square().flatten(),
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
        );
    }

    #[test]
    fn convert_edge_roundtrips_line_cubic_line() {
        let mut p = square();
        assert!(!p.is_edge_cubic(0));
        p.convert_edge_to_cubic(0);
        assert!(p.is_edge_cubic(0));
        // Colinear seeded controls → edge 0 stays on the original line (y≈0).
        assert!(p.sample_edge(0, 12).iter().all(|v| v[1].abs() < 1e-4));
        p.convert_edge_to_line(0);
        assert!(!p.is_edge_cubic(0));
    }

    #[test]
    fn anchor_pos_indexes_start_then_endpoints() {
        let p = square();
        assert!(approx(p.anchor_pos(0), [0.0, 0.0]));
        assert!(approx(p.anchor_pos(1), [1.0, 0.0]));
        assert!(approx(p.anchor_pos(3), [0.0, 1.0]));
    }

    #[test]
    fn move_anchor_zero_updates_start_and_closing_segment() {
        let mut p = square();
        p.move_anchor(0, [0.1, 0.1]);
        assert!(approx(p.start, [0.1, 0.1]));
        // Closing (incoming) segment now ends at the moved anchor.
        assert!(approx(p.segments.last().unwrap().end(), [0.1, 0.1]));
        // Outgoing edge start follows via `start`.
        assert!(approx(p.segment_start(0), [0.1, 0.1]));
    }

    #[test]
    fn move_anchor_drags_adjacent_cubic_handles() {
        let mut p = square();
        p.convert_edge_to_cubic(0); // outgoing from anchor 0
        p.convert_edge_to_cubic(3); // incoming to anchor 0 (closing edge)
        let d = [0.2, 0.3];
        // Capture pre-move control points.
        let (c1_before, c2_before) = match (p.segments[0], p.segments[3]) {
            (PathSegment::Cubic { c1, .. }, PathSegment::Cubic { c2, .. }) => (c1, c2),
            _ => unreachable!(),
        };
        p.move_anchor(0, [d[0], d[1]]);
        match (p.segments[0], p.segments[3]) {
            (PathSegment::Cubic { c1, .. }, PathSegment::Cubic { c2, to, .. }) => {
                assert!(approx(c1, [c1_before[0] + d[0], c1_before[1] + d[1]]));
                assert!(approx(c2, [c2_before[0] + d[0], c2_before[1] + d[1]]));
                assert!(approx(to, d));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn move_handle_sets_control_point() {
        let mut p = square();
        p.convert_edge_to_cubic(0);
        p.move_handle(0, CubicHandle::C1, [0.5, 0.5]);
        p.move_handle(0, CubicHandle::C2, [0.6, 0.4]);
        match p.segments[0] {
            PathSegment::Cubic { c1, c2, .. } => {
                assert!(approx(c1, [0.5, 0.5]));
                assert!(approx(c2, [0.6, 0.4]));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn move_handle_noop_on_line() {
        let mut p = square();
        p.move_handle(0, CubicHandle::C1, [0.5, 0.5]);
        assert!(!p.is_edge_cubic(0));
    }

    #[test]
    fn sample_edge_line_is_two_points() {
        assert_eq!(square().sample_edge(0, 8).len(), 2);
    }

    #[test]
    fn sample_edge_cubic_includes_start_plus_steps() {
        let mut p = square();
        p.convert_edge_to_cubic(0);
        assert_eq!(p.sample_edge(0, 6).len(), 1 + 6);
    }

    #[test]
    fn has_cubic_reflects_segment_kinds() {
        let mut p = square();
        assert!(!p.has_cubic());
        p.convert_edge_to_cubic(0);
        assert!(p.has_cubic());
    }

    #[test]
    fn apply_map_transforms_all_points() {
        let mut p = square();
        p.convert_edge_to_cubic(0);
        p.apply_map(|[x, y]| [x + 1.0, y + 2.0]);
        assert!(approx(p.start, [1.0, 2.0]));
        if let PathSegment::Cubic { c1, c2, to } = p.segments[0] {
            // Control points and endpoint all shifted by the same offset.
            assert!(c1[1] >= 2.0 && c2[1] >= 2.0);
            assert!(approx(to, [2.0, 2.0]));
        } else {
            panic!("edge 0 should be cubic after conversion");
        }
    }

    #[test]
    fn quad_to_cubic_matches_quadratic_curve() {
        // Sampling the derived cubic must equal sampling the original quadratic.
        let (p0, ctrl, p1) = ([0.0, 0.0], [1.0, 2.0], [2.0, 0.0]);
        let (c1, c2) = quad_to_cubic(p0, ctrl, p1);
        let quad = flatten_quad(p0, ctrl, p1, 8);
        let cubic = flatten_cubic(p0, c1, c2, p1, 8);
        assert_eq!(quad.len(), cubic.len());
        for (q, c) in quad.iter().zip(cubic.iter()) {
            assert!(approx(*q, *c), "quad {:?} vs cubic {:?}", q, c);
        }
    }
}
