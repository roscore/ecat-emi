//! Board → WebGL geometry and screen ↔ board mapping (pure, host-testable).
//! Board coordinates are mm with y down; the scene is mm with y up: x → x, board y → z.
use crate::gl::Geo;
use crate::math::{self, M4, V};
use ee_core::board::{self, Board, PadShape, Seg, P2};
use ee_core::couple::{Analysis, Edits, Prepared, RunKind};
use ee_core::raster::Raster;
use ee_core::roles::Roles;
use std::collections::HashMap;

/// Height of the board's bottom face above the floor (mm).
pub const LIFT: f32 = 2.0;
pub const VICTIM: [f32; 3] = [0.2, 0.9, 0.35];
pub const AGGR: [f32; 3] = [1.0, 0.55, 0.1];
const LAYER_COL: [[f32; 3]; 4] = [[0.86, 0.5, 0.25], [0.8, 0.7, 0.35], [0.45, 0.65, 0.85], [0.55, 0.45, 0.9]];

/// Base copper (all other nets) and role copper (victim / aggressor nets) of one layer.
pub struct LayerGeo {
    pub base: Geo,
    pub hot: Geo,
}

pub fn layer_color(b: &Board, l: usize) -> V {
    if l + 1 == b.layers.len() && l > 0 {
        LAYER_COL[3]
    } else {
        LAYER_COL[l.min(2)]
    }
}

/// World height of copper layer `l` (top layer highest).
pub fn layer_y(b: &Board, l: usize, exag: f32) -> f32 {
    let z = b.layers.get(l).map(|x| x.z).unwrap_or(0.0);
    LIFT + (b.thickness - z) as f32 * exag
}

/// Board centre in world space.
pub fn centre(b: &Board, exag: f32) -> V {
    let bb = b.bbox();
    [((bb[0] + bb[2]) / 2.0) as f32, LIFT + b.thickness as f32 * exag / 2.0, ((bb[1] + bb[3]) / 2.0) as f32]
}

/// Board model matrix: identity, or turned over about the board's x axis for the bottom view
/// (the camera pitch is clamped above the floor, so the board flips instead of the camera).
pub fn model(b: &Board, exag: f32, bottom: bool) -> M4 {
    if !bottom {
        return math::IDENTITY;
    }
    let c = centre(b, exag);
    let flip = math::basis([1., 0., 0.], [0., -1., 0.], [0., 0., -1.], [1.; 3], [0.; 3]);
    math::mul(&math::translate(c), &math::mul(&flip, &math::translate(math::scale(c, -1.0))))
}

/// Board point (mm) under NDC (x, y) on the model-space plane at height `plane_y`.
pub fn board_point(inv_mvp: &M4, x: f32, y: f32, plane_y: f32) -> Option<P2> {
    let (o, d) = math::ndc_ray(inv_mvp, x, y);
    math::ray_plane(o, d, [0.0, plane_y, 0.0], [0.0, 1.0, 0.0]).map(|p| [p[0] as f64, p[2] as f64])
}

/// Offset after dragging by `delta` from `start`: only the part across the segment counts.
pub fn drag_offset(s: &Seg, start: P2, delta: P2) -> P2 {
    let u = board::sub(s.b, s.a);
    let l = board::len(u).max(1e-9);
    let n = [-u[1] / l, u[0] / l];
    board::add(start, board::scale(n, board::dot(delta, n)))
}

/// Set segment `k`'s offset in place (the edit list must not hold duplicates).
pub fn set_offset(e: &mut Edits, k: usize, o: P2) {
    match e.offsets.iter_mut().find(|(s, _)| *s == k) {
        Some(x) => x.1 = o,
        None => e.offsets.push((k, o)),
    }
}

fn in_poly(p: P2, poly: &[P2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            inside = !inside;
        }
    }
    inside
}

/// Net under board point p on the visible layers: nearest track, else a pad, else a pour.
pub fn pick_net(b: &Board, p: P2, visible: &[bool], tol: f64) -> Option<usize> {
    let vis = |l: usize| visible.get(l).copied().unwrap_or(false);
    let mut best = (tol, None);
    for s in b.segs.iter().filter(|s| vis(s.layer) && s.net != 0) {
        let d = board::point_seg(p, s.a, s.b) - s.w / 2.0;
        if d <= best.0 {
            best = (d, Some(s.net));
        }
    }
    if best.1.is_some() {
        return best.1;
    }
    for pad in b.pads.iter().filter(|x| x.net != 0 && x.layers.iter().any(|&l| vis(l))) {
        let q = board::rot(board::sub(p, pad.at), -pad.rot);
        if q[0].abs() <= pad.size[0] / 2.0 + tol && q[1].abs() <= pad.size[1] / 2.0 + tol {
            return Some(pad.net);
        }
    }
    b.zones.iter().rev().find(|z| vis(z.layer) && z.net != 0 && !z.clear && z.polys.iter().any(|q| in_poly(p, q))).map(|z| z.net)
}

/// Run colour by its share of the pair's noise: yellow (small) → red (dominant).
pub fn share_color(s: f64) -> V {
    let t = s.clamp(0.0, 1.0) as f32;
    [1.0, 0.9 - 0.8 * t, 0.2 - 0.15 * t]
}

fn tri(g: &mut Geo, p: [V; 3], c: V) {
    let n = math::cross(math::sub(p[1], p[0]), math::sub(p[2], p[0]));
    let n = if math::dot(n, n) > 0.0 { math::norm(n) } else { [0.0, 1.0, 0.0] };
    for q in p {
        g.pos.extend(q);
        g.nrm.extend(n);
        g.col.extend(c);
    }
}

fn quad(g: &mut Geo, q: [V; 4], c: V) {
    tri(g, [q[0], q[1], q[2]], c);
    tri(g, [q[0], q[2], q[3]], c);
}

fn w3(p: P2, y: f32) -> V {
    [p[0] as f32, y, p[1] as f32]
}

/// Flat polygon (convex fan) at height y.
fn flat_fan(g: &mut Geo, pts: &[P2], y: f32, c: V) {
    for i in 1..pts.len().saturating_sub(1) {
        tri(g, [w3(pts[0], y), w3(pts[i], y), w3(pts[i + 1], y)], c);
    }
}

fn seg_quad(g: &mut Geo, s: &Seg, y: f32, c: V) {
    let u = board::sub(s.b, s.a);
    let l = board::len(u);
    let hw = s.w.max(0.05) / 2.0;
    let (u, n) = if l > 1e-9 { (board::scale(u, hw / l), [-u[1] / l * hw, u[0] / l * hw]) } else { ([hw, 0.0], [0.0, hw]) };
    let (a, b) = (board::sub(s.a, u), board::add(s.b, u));
    flat_fan(g, &[board::add(a, n), board::add(b, n), board::sub(b, n), board::sub(a, n)], y, c);
}

fn pad_outline(p: &board::Pad) -> Vec<P2> {
    let (hx, hy) = (p.size[0] / 2.0, p.size[1] / 2.0);
    let local: Vec<P2> = match p.shape {
        PadShape::Rect => vec![[-hx, -hy], [hx, -hy], [hx, hy], [-hx, hy]],
        _ => (0..12).map(|k| {
            let t = k as f64 / 12.0 * std::f64::consts::TAU;
            [hx * t.cos(), hy * t.sin()]
        }).collect(),
    };
    local.into_iter().map(|q| board::add(p.at, board::rot(q, p.rot))).collect()
}

fn raster_quads(g: &mut Geo, r: &Raster, y: f32, c: V) {
    for (iy, x0, x1) in r.runs() {
        let (ya, yb) = (r.y0 + iy as f64 * r.res, r.y0 + (iy + 1) as f64 * r.res);
        let (xa, xb) = (r.x0 + x0 as f64 * r.res, r.x0 + (x1 + 1) as f64 * r.res);
        flat_fan(g, &[[xa, ya], [xb, ya], [xb, yb], [xa, yb]], y, c);
    }
}

/// Copper geometry per layer, split into base and role (victim / aggressor) meshes.
/// Victim segments are drawn with the virtual drag offsets applied.
pub fn copper(prep: &Prepared, roles: &Roles, edits: &Edits, exag: f32) -> Vec<LayerGeo> {
    let b = &prep.board;
    let mut role: HashMap<usize, V> = HashMap::new();
    for a in roles.aggressors.iter().filter(|a| a.enabled) {
        role.insert(a.net, AGGR);
    }
    for p in roles.pairs.iter().filter(|p| p.enabled) {
        role.insert(p.p, VICTIM);
        role.insert(p.n, VICTIM);
    }
    let off: HashMap<usize, P2> = edits.offsets.iter().copied().collect();
    let last = b.layers.len().saturating_sub(1);
    let mut out: Vec<LayerGeo> = (0..b.layers.len()).map(|_| LayerGeo { base: Geo::default(), hot: Geo::default() }).collect();
    // Outer-layer copper sits slightly outside the laminate, inner copper on the layer plane.
    let lift = |l: usize, k: f32| layer_y(b, l, exag) + if l == last && l > 0 { -k } else { k };
    for (l, lg) in out.iter_mut().enumerate() {
        let c = layer_color(b, l);
        let dim = math::scale(c, 0.6);
        let y = lift(l, 0.01);
        if let Some(pl) = prep.planes.iter().find(|p| p.layer == l) {
            raster_quads(&mut lg.base, &pl.raster, y, dim);
        } else {
            for z in b.zones.iter().filter(|z| z.layer == l && !z.clear) {
                let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
                for q in z.polys.iter().flatten() {
                    bb = [bb[0].min(q[0]), bb[1].min(q[1]), bb[2].max(q[0]), bb[3].max(q[1])];
                }
                if !bb[0].is_finite() {
                    continue;
                }
                // ponytail: 0.2 mm display raster; finer only if pours look jagged.
                let mut r = Raster::new(bb, 0.2);
                for poly in &z.polys {
                    r.fill_poly(poly, true);
                }
                match role.get(&z.net) {
                    Some(rc) => raster_quads(&mut lg.hot, &r, y, math::scale(*rc, 0.7)),
                    None => raster_quads(&mut lg.base, &r, y, dim),
                }
            }
        }
    }
    for (k, s) in b.segs.iter().enumerate() {
        let Some(lg) = out.get_mut(s.layer) else { continue };
        let mut s = *s;
        if let Some(o) = off.get(&k) {
            s.a = board::add(s.a, *o);
            s.b = board::add(s.b, *o);
        }
        let y = lift(s.layer, 0.02);
        match role.get(&s.net) {
            Some(c) => seg_quad(&mut lg.hot, &s, y, *c),
            None => seg_quad(&mut lg.base, &s, y, layer_color(b, s.layer)),
        }
    }
    for p in &b.pads {
        let outline = pad_outline(p);
        for &l in &p.layers {
            let Some(lg) = out.get_mut(l) else { continue };
            let y = lift(l, 0.03);
            match role.get(&p.net) {
                Some(c) => flat_fan(&mut lg.hot, &outline, y, *c),
                None => flat_fan(&mut lg.base, &outline, y, [0.85, 0.8, 0.6]),
            }
        }
    }
    out
}

/// Coupled-run bands (victim run ↔ aggressor run quads), coloured by noise share.
pub fn bands(b: &Board, an: &Analysis, exag: f32) -> Geo {
    let mut g = Geo::default();
    for r in an.runs.iter().filter(|r| r.kind != RunKind::Cable) {
        let y = layer_y(b, r.layer_v, exag) + 0.06;
        quad(&mut g, r.band.map(|p| w3(p, y)), share_color(r.share));
    }
    g
}

/// Outline of one run's band (hover / selection highlight).
pub fn band_outline(b: &Board, an: &Analysis, run: usize, exag: f32) -> Geo {
    let Some(r) = an.runs.get(run) else { return Geo::default() };
    let y = layer_y(b, r.layer_v, exag) + 0.08;
    crate::gl::line_loop(&r.band.map(|p| w3(p, y)))
}

/// Plane voids crossed by a pair, as red hatched boxes on the plane layer, plus the hot loop.
pub fn hatch(prep: &Prepared, an: &Analysis, exag: f32) -> Geo {
    let mut pts: Vec<V> = Vec::new();
    for g in &an.gaps {
        let l = prep.planes.get(g.plane).map(|p| p.layer).unwrap_or(0);
        let y = layer_y(&prep.board, l, exag) + 0.04;
        let [x0, y0, x1, y1] = g.bbox;
        let c = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        for i in 0..4 {
            pts.extend([w3(c[i], y), w3(c[(i + 1) % 4], y)]);
        }
        // Diagonals x − y = k, clipped to the box.
        let step = 0.6;
        let mut k = (x0 - y1 + step / 2.0).min(x1 - y0);
        while k < x1 - y0 {
            let (xa, xb) = ((y0 + k).max(x0), (y1 + k).min(x1));
            if xb > xa {
                pts.extend([w3([xa, xa - k], y), w3([xb, xb - k], y)]);
            }
            k += step;
        }
    }
    if let Some(h) = &prep.hot_loop {
        let y = layer_y(&prep.board, 0, exag) + 0.05;
        let n = h.poly.len();
        for i in 0..n {
            pts.extend([w3(h.poly[i], y), w3(h.poly[(i + 1) % n], y)]);
        }
    }
    crate::gl::lines(&pts)
}

/// Vias as 8-sided barrels with caps, batched.
pub fn vias(b: &Board, exag: f32) -> Geo {
    let mut g = Geo::default();
    let col = [0.8, 0.75, 0.6];
    for v in &b.vias {
        let (ya, yb) = (layer_y(b, v.span.0, exag) + 0.04, layer_y(b, v.span.1, exag) - 0.04);
        let r = v.size / 2.0;
        let ring: Vec<P2> = (0..8).map(|k| {
            let t = k as f64 / 8.0 * std::f64::consts::TAU;
            [v.at[0] + r * t.cos(), v.at[1] + r * t.sin()]
        }).collect();
        for k in 0..8 {
            let (p, q) = (ring[k], ring[(k + 1) % 8]);
            quad(&mut g, [w3(p, ya), w3(q, ya), w3(q, yb), w3(p, yb)], col);
        }
        flat_fan(&mut g, &ring, ya, col);
        flat_fan(&mut g, &ring, yb, col);
    }
    g
}

/// Dielectric slabs between consecutive copper layers (boxes over the board outline).
pub fn slabs(b: &Board, exag: f32) -> Geo {
    let mut g = Geo::default();
    let bb = b.bbox();
    let c = [[bb[0], bb[1]], [bb[2], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]]];
    for k in 0..b.layers.len().saturating_sub(1) {
        let (top, bot) = (layer_y(b, k, exag), layer_y(b, k + 1, exag));
        let col = if k % 2 == 0 { [0.16, 0.42, 0.24] } else { [0.22, 0.5, 0.3] };
        quad(&mut g, c.map(|p| w3(p, top)), col);
        quad(&mut g, c.map(|p| w3(p, bot)), col);
        for i in 0..4 {
            let (p, q) = (c[i], c[(i + 1) % 4]);
            quad(&mut g, [w3(p, bot), w3(q, bot), w3(q, top), w3(p, top)], col);
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use ee_core::couple::{self, Edits, Params};
    use ee_core::{kicad, roles, sample};

    fn setup() -> (couple::Prepared, roles::Roles) {
        let b = kicad::parse(&sample::kicad_text()).unwrap();
        let r = roles::suggest(&b, &Params::default().inv);
        (couple::prepare(b), r)
    }

    #[test]
    fn layer_heights_stack_top_first() {
        let (prep, _) = setup();
        let b = &prep.board;
        let top = layer_y(b, 0, 4.0);
        let bot = layer_y(b, b.layers.len() - 1, 4.0);
        let gap = b.layer_gap(0, b.layers.len() - 1) as f32 * 4.0;
        assert!((top - bot - gap).abs() < 1e-3, "{top} {bot}");
        assert!((LIFT..LIFT + 0.2).contains(&bot));
    }

    #[test]
    fn screen_ray_maps_back_to_the_board_point() {
        let (prep, _) = setup();
        let b = &prep.board;
        for bottom in [false, true] {
            let m = model(b, 4.0, bottom);
            let vp = math::mul(&math::perspective(0.8, 1.5, 1.0, 5000.0), &math::look_at([30.0, 150.0, 80.0], [40.0, 3.0, 30.0], [0.0, 1.0, 0.0]));
            let mvp = math::mul(&vp, &m);
            let l = if bottom { b.layers.len() - 1 } else { 0 };
            let y = layer_y(b, l, 4.0);
            let p = [37.0, 21.0];
            let ndc = math::transform(&mvp, [p[0] as f32, y, p[1] as f32]);
            let q = board_point(&math::invert(&mvp).unwrap(), ndc[0], ndc[1], y).unwrap();
            assert!((q[0] - p[0]).abs() < 0.01 && (q[1] - p[1]).abs() < 0.01, "{q:?} bottom={bottom}");
        }
    }

    #[test]
    fn drag_moves_only_sideways() {
        let s = Seg { a: [0.0, 0.0], b: [10.0, 0.0], w: 0.2, layer: 0, net: 1 };
        let o = drag_offset(&s, [0.0, 1.0], [5.0, 2.0]);
        assert!((o[0]).abs() < 1e-12 && (o[1] - 3.0).abs() < 1e-12, "{o:?}");
        let mut e = Edits::default();
        set_offset(&mut e, 3, [0.0, 1.0]);
        set_offset(&mut e, 3, [0.0, 2.0]);
        assert_eq!(e.offsets, vec![(3, [0.0, 2.0])], "updated in place, no duplicates");
    }

    #[test]
    fn net_pick_finds_tracks_and_respects_layer_toggle() {
        let (prep, _) = setup();
        let b = &prep.board;
        let s = b.segs.iter().find(|s| b.nets[s.net] == "ECAT_RX_P").unwrap();
        let mid = [(s.a[0] + s.b[0]) / 2.0, (s.a[1] + s.b[1]) / 2.0];
        let all = vec![true; b.layers.len()];
        assert_eq!(pick_net(b, mid, &all, 0.05), Some(s.net));
        let mut off = all.clone();
        off[s.layer] = false;
        assert_ne!(pick_net(b, mid, &off, 0.05), Some(s.net));
    }

    #[test]
    fn copper_meshes_split_role_nets() {
        let (prep, r) = setup();
        let e = Edits::default();
        let geo = copper(&prep, &r, &e, 4.0);
        assert_eq!(geo.len(), prep.board.layers.len());
        let hot: usize = geo.iter().map(|g| g.hot.pos.len()).sum();
        let base: usize = geo.iter().map(|g| g.base.pos.len()).sum();
        assert!(hot > 0 && base > 0);
        for g in &geo {
            for m in [&g.base, &g.hot] {
                assert_eq!(m.pos.len(), m.nrm.len());
                assert_eq!(m.pos.len(), m.col.len());
                assert_eq!(m.pos.len() % 9, 0, "whole triangles");
                assert!(m.idx.is_empty());
            }
        }
        // Victims green, aggressors orange.
        let greens = geo[0].hot.col.chunks(3).filter(|c| c == &VICTIM).count();
        let oranges = geo[0].hot.col.chunks(3).filter(|c| c == &AGGR).count();
        assert!(greens > 0 && oranges > 0);
    }

    #[test]
    fn overlays_cover_runs_and_gaps() {
        let (prep, r) = setup();
        let an = couple::analyze(&prep, &r, &Params::default(), &Edits::default());
        let g = bands(&prep.board, &an, 4.0);
        let drawn = an.runs.iter().filter(|x| x.kind != couple::RunKind::Cable).count();
        assert_eq!(g.pos.len(), drawn * 6 * 3);
        let h = hatch(&prep, &an, 4.0);
        assert!(h.lines && !h.pos.is_empty());
        assert!(share_color(1.0)[1] < share_color(0.0)[1], "hotter = redder");
    }

    #[test]
    fn vias_and_slabs_are_whole_triangles() {
        let (prep, _) = setup();
        let v = vias(&prep.board, 4.0);
        assert_eq!(v.pos.len() % 9, 0);
        assert!(!v.pos.is_empty());
        let s = slabs(&prep.board, 4.0);
        assert_eq!(s.pos.len(), (prep.board.layers.len() - 1) * 36 * 3);
    }
}
