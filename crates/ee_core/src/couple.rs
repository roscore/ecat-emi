//! Board-level coupling model: parallel-run detection, plane gaps, hot loop, mode conversion,
//! per-pair noise → link result, and ranked geometric fixes. Lumped and quasi-static.
use crate::board::*;
use crate::cable::{self, CableParams, MU0};
use crate::grid::{Grid, S};
use crate::link::{rx_filter, LinkParams, LinkResult, Verdict};
use crate::num::{from_db20, log_grid, sat, trapz};
use crate::raster::Raster;
use crate::roles::{Roles, Side};
use crate::spectrum::{edge_density, Inverter};
use std::collections::HashMap;
use std::f64::consts::PI;

const C0: f64 = 299_792_458.0;
/// Single-ended reference impedance of each trace of a 100 Ω pair.
const Z0: f64 = 50.0;
pub const N_FREQ: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub inv: Inverter,
    pub link: LinkParams,
    /// PHY / magnetics mode conversion (LCL), dB (−60 … −25).
    pub lcl_db: f64,
    /// Magnetics common-mode rejection for line-side coupling, dB.
    pub cmrr_db: f64,
    /// Motor-frame stray capacitance driving the phase-cable CM current (F).
    pub c_cm: f64,
    pub cable: CableParams,
    /// Parallel-run search distance (mm) and angle (deg).
    pub search_mm: f64,
    pub max_angle: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            inv: Inverter::default(),
            link: LinkParams::default(),
            lcl_db: -40.0,
            cmrr_db: 35.0,
            c_cm: 200e-12,
            cable: CableParams::default(),
            search_mm: 10.0,
            max_angle: 30.0,
        }
    }
}

/// Virtual, non-destructive edits ("try a fix"); the file is never changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Edits {
    /// Segment index → offset (mm).
    pub offsets: Vec<(usize, P2)>,
    /// Plane voids filled: (plane index, bbox).
    pub fills: Vec<(usize, [f64; 4])>,
    /// Stitching vias added around these gap boxes: (plane index, bbox).
    pub stitches: Vec<(usize, [f64; 4])>,
    /// Hot-loop area override (mm²).
    pub hot_loop_mm2: Option<f64>,
    /// Edge-time multiplier (slew-rate limiting).
    pub tr_scale: f64,
    /// Pairs rerouted onto an inner (stripline) layer.
    pub inner: Vec<usize>,
}

impl Default for Edits {
    fn default() -> Self {
        Edits { offsets: vec![], fills: vec![], stitches: vec![], hot_loop_mm2: None, tr_scale: 1.0, inner: vec![] }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plane {
    pub layer: usize,
    pub raster: Raster,
    /// Convex envelope of the plane's zones; voids inside it are gaps.
    pub hull: Vec<P2>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HotLoop {
    pub poly: Vec<P2>,
    pub area_mm2: f64,
    pub centroid: P2,
}

/// Board plus everything derived once per file (plane rasters, hot loop).
pub struct Prepared {
    pub board: Board,
    pub planes: Vec<Plane>,
    pub hot_loop: Option<HotLoop>,
    pub bbox: [f64; 4],
}

pub const RES: f64 = 0.1;

pub fn prepare(board: Board) -> Prepared {
    let bbox = board.bbox();
    let area = (bbox[2] - bbox[0]) * (bbox[3] - bbox[1]);
    let mut planes = Vec::new();
    for l in 0..board.layers.len() {
        let zones: Vec<&Zone> = board.zones.iter().filter(|z| z.layer == l).collect();
        if zones.is_empty() {
            continue;
        }
        let mut r = Raster::new(bbox, RES);
        let mut pts = Vec::new();
        for z in &zones {
            for poly in &z.polys {
                r.fill_poly(poly, !z.clear);
                if !z.clear {
                    pts.extend_from_slice(poly);
                }
            }
        }
        if r.count() as f64 * r.res * r.res >= 0.3 * area {
            planes.push(Plane { layer: l, raster: r, hull: hull(&pts) });
        }
    }
    let hot_loop = find_hot_loop(&board);
    Prepared { board, planes, hot_loop, bbox }
}

/// Switching loop: hull through the high-side drain, low-side source and input-cap pads.
pub fn find_hot_loop(b: &Board) -> Option<HotLoop> {
    let name = |n: usize| crate::roles::short(b.nets.get(n).map(String::as_str).unwrap_or(""));
    let is_bus = |n: usize| crate::roles::aggr_kind_by_name(&name(n)) == Some(crate::roles::AggrKind::Bus);
    let is_gnd = |n: usize| {
        let s = name(n);
        s.starts_with("GND") || s.starts_with("PGND")
    };
    let mut pts = Vec::new();
    for (k, f) in b.footprints.iter().enumerate() {
        let pads: Vec<&Pad> = b.pads_of(k).collect();
        let has_bus = pads.iter().any(|p| is_bus(p.net));
        let has_gnd = pads.iter().any(|p| is_gnd(p.net));
        if f.reference.starts_with('Q') {
            // High side: its bus pad (drain). Low side: its ground pad (source).
            pts.extend(pads.iter().filter(|p| is_bus(p.net) || (!has_bus && is_gnd(p.net))).map(|p| p.at));
        } else if f.reference.starts_with('C') && has_bus && has_gnd {
            pts.extend(pads.iter().map(|p| p.at));
        }
    }
    if pts.len() < 3 {
        return None;
    }
    let poly = hull(&pts);
    let area = poly_area(&poly);
    (area > 0.0).then(|| HotLoop { centroid: centroid(&poly), area_mm2: area, poly })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Path {
    Capacitive,
    Inductive,
    ModeConversion,
    PlaneGap,
    HotLoop,
    Cable,
}

pub const PATHS: [Path; 6] = [Path::Capacitive, Path::Inductive, Path::ModeConversion, Path::PlaneGap, Path::HotLoop, Path::Cable];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Parallel,
    HotLoop,
    Cable,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub kind: RunKind,
    pub pair: usize,
    /// Aggressor role index (Parallel only).
    pub aggr: Option<usize>,
    pub aggr_seg: Option<usize>,
    pub victim_segs: Vec<usize>,
    /// Coupled length (mm), edge spacing (mm) for P and N, nominal height to reference (mm).
    pub len: f64,
    pub s_p: f64,
    pub s_n: f64,
    pub h: f64,
    pub layer_v: usize,
    pub layer_a: usize,
    /// Length of the run over a plane void (mm).
    pub gap_len: f64,
    /// Quad between the victim run and the aggressor run (mm), for drawing and picking.
    pub band: [P2; 4],
    pub line_side: bool,
    /// Results.
    pub peak: f64,
    pub energy: f64,
    pub share: f64,
    pub db: f64,
    /// Noise density at the RX (V/Hz) per frequency.
    pub noise: Vec<f64>,
    /// Energy per path.
    pub path_energy: [f64; 6],
    // Internal coupling inputs.
    kl_p: f64,
    kl_n: f64,
    len_p: f64,
    len_n: f64,
    pitch: f64,
    r_mm: f64,
    eps_eff: f64,
    kl_gap_loop: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gap {
    pub plane: usize,
    pub pair: usize,
    pub at: P2,
    pub len: f64,
    pub bbox: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PairResult {
    pub pair: usize,
    pub link: LinkResult,
    pub dominant: Path,
    pub path_energy: [f64; 6],
    /// Per-path noise density at the RX (V/Hz).
    pub path_noise: [Vec<f64>; 6],
    /// Mode conversion |CM→DM| at 31.25 MHz.
    pub mode_conv: f64,
    /// P/N length mismatch (mm).
    pub skew_mm: f64,
}

impl PairResult {
    pub fn verdict(&self) -> Verdict {
        self.link.verdict()
    }
}

pub struct Analysis {
    pub freqs: Vec<f64>,
    pub runs: Vec<Run>,
    pub gaps: Vec<Gap>,
    pub pairs: Vec<PairResult>,
    pub hot_loop_mm2: Option<f64>,
    pub t_r: f64,
}

/// Coupling coefficient for edge spacing s over a reference at height h.
pub fn k_coef(s: f64, h: f64) -> f64 {
    1.0 / (1.0 + (s / h.max(1e-3)).powi(2))
}

/// CM→DM conversion: LCL floor plus pair skew 2|sin(π f Δt)|.
pub fn mode_conversion(lcl_db: f64, skew_s: f64, f: f64) -> f64 {
    from_db20(lcl_db) + 2.0 * (PI * f * skew_s).sin().abs()
}

fn seg_with(b: &Board, k: usize, off: &HashMap<usize, P2>) -> Seg {
    let mut s = b.segs[k];
    if let Some(o) = off.get(&k) {
        s.a = add(s.a, *o);
        s.b = add(s.b, *o);
    }
    s
}

struct Ctx<'a> {
    prep: &'a Prepared,
    fills: Vec<(usize, Raster)>,
    stitches: &'a [(usize, [f64; 4])],
}

impl Ctx<'_> {
    fn plane_solid(&self, pi: usize, p: P2) -> bool {
        match self.fills.iter().find(|(k, _)| *k == pi) {
            Some((_, r)) => r.at(p),
            None => self.prep.planes[pi].raster.at(p),
        }
    }
    fn in_envelope(&self, pi: usize, p: P2) -> bool {
        let h = &self.prep.planes[pi].hull;
        let n = h.len();
        n >= 3 && (0..n).all(|i| cross(sub(h[(i + 1) % n], h[i]), sub(p, h[i])) >= 0.0) && (0..n).all(|i| point_seg(p, h[i], h[(i + 1) % n]) > 1.0)
    }
    /// (height to the nearest solid reference, nominal height, void plane index if a gap).
    fn reference(&self, layer: usize, p: P2) -> (f64, f64, Option<usize>) {
        let b = &self.prep.board;
        let mut planes: Vec<(f64, usize)> = self
            .prep
            .planes
            .iter()
            .enumerate()
            .filter(|(_, pl)| pl.layer != layer)
            .map(|(i, pl)| (b.layer_gap(layer, pl.layer), i))
            .collect();
        planes.sort_by(|a, b| a.0.total_cmp(&b.0));
        let nominal = planes.first().map(|x| x.0).unwrap_or(b.thickness);
        let mut gap = None;
        for (h, pi) in &planes {
            if self.plane_solid(*pi, p) {
                return (*h, nominal, gap);
            }
            if gap.is_none() && self.in_envelope(*pi, p) {
                gap = Some(*pi);
            }
        }
        (b.thickness.max(nominal), nominal, gap)
    }
    fn stitched(&self, pi: usize, p: P2) -> bool {
        self.stitches.iter().any(|(k, bb)| *k == pi && p[0] >= bb[0] - 1.0 && p[0] <= bb[2] + 1.0 && p[1] >= bb[1] - 1.0 && p[1] <= bb[3] + 1.0)
    }
}

/// One victim-segment ↔ aggressor-segment overlap.
struct Hit {
    vseg: usize,
    p_side: bool,
    len: f64,
    kl: f64,
    kl_gap_loop: f64,
    s: f64,
    c_lat: f64,
    h: f64,
    gap_len: f64,
    gap_at: Option<(usize, P2)>,
    lo: P2,
    hi: P2,
    alo: P2,
    ahi: P2,
    eps_eff: f64,
}

fn overlap(ctx: &Ctx, v: &Seg, a: &Seg, inner: bool, params: &Params) -> Option<Hit> {
    let b = &ctx.prep.board;
    let dv = sub(v.b, v.a);
    let lv = len(dv);
    let da = sub(a.b, a.a);
    let la = len(da);
    if lv < 1e-6 || la < 1e-6 {
        return None;
    }
    let u = scale(dv, 1.0 / lv);
    if (dot(u, da) / la).abs() < params.max_angle.to_radians().cos() {
        return None;
    }
    let (ta, tb) = (dot(sub(a.a, v.a), u), dot(sub(a.b, v.a), u));
    let (t0, t1) = (ta.min(tb).max(0.0), ta.max(tb).min(lv));
    let ell = t1 - t0;
    if ell < 0.2 {
        return None;
    }
    let lo = add(v.a, scale(u, t0));
    let hi = add(v.a, scale(u, t1));
    let mid = scale(add(lo, hi), 0.5);
    let c_lat = point_seg(mid, a.a, a.b);
    let dz = if inner { b.thickness / 4.0 } else { b.layer_gap(v.layer, a.layer) };
    let s = ((c_lat - (v.w + a.w) / 2.0).max(0.0).powi(2) + dz * dz).sqrt().max(0.02);
    if s > params.search_mm {
        return None;
    }
    // Sample along the overlap at the raster resolution.
    let n = ((ell / RES).ceil() as usize).max(1);
    let step = ell / n as f64;
    let (mut kl, mut kl_gl, mut gap_len, mut h_nom) = (0.0, 0.0, 0.0, f64::INFINITY);
    let mut gap_at = None;
    for i in 0..n {
        let p = add(lo, scale(u, (i as f64 + 0.5) * step));
        let (mut h, nominal, gap) = if inner {
            let hn = b.layer_gap(0, 1).max(0.05);
            (hn, hn, None)
        } else {
            ctx.reference(v.layer, p)
        };
        h_nom = h_nom.min(nominal);
        if let Some(pi) = gap {
            let mut loop_w = 1.0;
            if ctx.stitched(pi, p) {
                h = nominal + (h - nominal) * 0.3;
                loop_w = 0.3;
            }
            gap_len += step;
            kl_gl += step * loop_w;
            gap_at.get_or_insert((pi, p));
        }
        kl += k_coef(s, h) * step;
    }
    let er = b.eps_near(v.layer);
    let eps_eff = if b.is_outer(v.layer) && !inner { (er + 1.0) / 2.0 } else { er };
    let alo = add(a.a, scale(scale(da, 1.0 / la), dot(sub(lo, a.a), scale(da, 1.0 / la)).clamp(0.0, la)));
    let ahi = add(a.a, scale(scale(da, 1.0 / la), dot(sub(hi, a.a), scale(da, 1.0 / la)).clamp(0.0, la)));
    Some(Hit { vseg: 0, p_side: true, len: ell, kl, kl_gap_loop: kl_gl, s, c_lat, h: h_nom, gap_len, gap_at, lo, hi, alo, ahi, eps_eff })
}

pub fn freqs() -> Vec<f64> {
    log_grid(10e3, 1e9, N_FREQ)
}

/// Run the whole board-level model.
pub fn analyze(prep: &Prepared, roles: &Roles, params: &Params, edits: &Edits) -> Analysis {
    let b = &prep.board;
    let fr = freqs();
    let off: HashMap<usize, P2> = edits.offsets.iter().copied().collect();
    let mut fills = Vec::new();
    for (pi, bb) in &edits.fills {
        if let Some(pl) = prep.planes.get(*pi) {
            let pos = fills.iter().position(|(k, _): &(usize, Raster)| k == pi);
            let idx = pos.unwrap_or_else(|| {
                fills.push((*pi, pl.raster.clone()));
                fills.len() - 1
            });
            fills[idx].1.fill_box(*bb, true);
        }
    }
    let ctx = Ctx { prep, fills, stitches: &edits.stitches };
    let tr_scale = edits.tr_scale.max(0.01);
    let t_r = params.inv.t_r * tr_scale;

    // Aggressor segments (tracks plus the outline of non-plane pours on aggressor nets).
    let mut a_segs: Vec<(Seg, usize, Option<usize>)> = Vec::new();
    for (ri, ar) in roles.aggressors.iter().enumerate().filter(|(_, a)| a.enabled) {
        for (k, s) in b.segs.iter().enumerate().filter(|(_, s)| s.net == ar.net) {
            a_segs.push((*s, ri, Some(k)));
        }
        for z in b.zones.iter().filter(|z| z.net == ar.net && !z.clear && !prep.planes.iter().any(|p| p.layer == z.layer)) {
            for poly in &z.polys {
                for i in 0..poly.len() {
                    let s = Seg { a: poly[i], b: poly[(i + 1) % poly.len()], w: 0.0, layer: z.layer, net: ar.net };
                    a_segs.push((s, ri, None));
                }
            }
        }
    }
    let grid = Grid::new(&a_segs.iter().map(|(s, _, _)| S { a: s.a, b: s.b, hw: s.w / 2.0 }).collect::<Vec<_>>(), params.search_mm);

    let mut runs: Vec<Run> = Vec::new();
    let mut gaps: Vec<Gap> = Vec::new();
    let mut tmp = Vec::new();
    for (pi, pr) in roles.pairs.iter().enumerate().filter(|(_, p)| p.enabled) {
        let inner = edits.inner.contains(&pi);
        // Hits grouped by aggressor segment index.
        let mut by_aggr: Vec<(usize, Vec<Hit>)> = Vec::new();
        let vsegs: Vec<(usize, bool)> = b
            .segs
            .iter()
            .enumerate()
            .filter_map(|(k, s)| if s.net == pr.p { Some((k, true)) } else if s.net == pr.n { Some((k, false)) } else { None })
            .collect();
        for &(k, p_side) in &vsegs {
            let v = seg_with(b, k, &off);
            grid.query(&S { a: v.a, b: v.b, hw: v.w / 2.0 }, params.search_mm, &mut tmp);
            for &ai in &tmp {
                let (a, _, _) = &a_segs[ai as usize];
                if inner && b.is_outer(a.layer) {
                    continue; // a plane now separates them
                }
                if !inner && (v.layer as i64 - a.layer as i64).abs() > 1 {
                    continue;
                }
                if let Some(mut h) = overlap(&ctx, &v, a, inner, params) {
                    h.vseg = k;
                    h.p_side = p_side;
                    match by_aggr.iter_mut().find(|(x, _)| *x == ai as usize) {
                        Some((_, v)) => v.push(h),
                        None => by_aggr.push((ai as usize, vec![h])),
                    }
                }
            }
        }
        for (ai, hits) in by_aggr {
            let (aseg, ri, src) = a_segs[ai];
            let side = |p: bool| hits.iter().filter(move |h| h.p_side == p);
            let (len_p, len_n) = (side(true).map(|h| h.len).sum::<f64>(), side(false).map(|h| h.len).sum::<f64>());
            let (kl_p, kl_n) = (side(true).map(|h| h.kl).sum::<f64>(), side(false).map(|h| h.kl).sum::<f64>());
            let s_of = |p: bool| side(p).map(|h| h.s).fold(f64::INFINITY, f64::min);
            let c_of = |p: bool| side(p).map(|h| h.c_lat).fold(f64::INFINITY, f64::min);
            let (mut s_p, mut s_n) = (s_of(true), s_of(false));
            let pitch = if c_of(true).is_finite() && c_of(false).is_finite() { (c_of(true) - c_of(false)).abs().max(0.1) } else { 0.5 };
            // A missing partner trace sits one pitch further away.
            if !s_n.is_finite() {
                s_n = s_p + pitch;
            }
            if !s_p.is_finite() {
                s_p = s_n + pitch;
            }
            let (kl_p, kl_n, len_p, len_n) = match (len_p > 0.0, len_n > 0.0) {
                (true, false) => (kl_p, kl_p * k_coef(s_n, hits[0].h) / k_coef(s_p, hits[0].h), len_p, len_p),
                (false, true) => (kl_n * k_coef(s_p, hits[0].h) / k_coef(s_n, hits[0].h), kl_n, len_n, len_n),
                _ => (kl_p, kl_n, len_p, len_n),
            };
            let gap_len = hits.iter().map(|h| h.gap_len).fold(0.0, f64::max);
            let kl_gap_loop = hits.iter().map(|h| h.kl_gap_loop).fold(0.0, f64::max);
            for h in hits.iter().filter(|h| h.gap_at.is_some()) {
                if let Some((pl, at)) = h.gap_at {
                    add_gap(&ctx, &mut gaps, pl, pi, at, h.gap_len);
                }
            }
            let best = hits.iter().max_by(|a, b| a.kl.total_cmp(&b.kl)).map(|h| h.len).unwrap_or(0.0);
            let rep = hits.iter().find(|h| h.len == best).unwrap_or(&hits[0]);
            let r_mm = hits.iter().map(|h| h.c_lat).fold(f64::INFINITY, f64::min).max(0.1);
            runs.push(Run {
                kind: RunKind::Parallel,
                pair: pi,
                aggr: Some(ri),
                aggr_seg: src,
                victim_segs: hits.iter().map(|h| h.vseg).collect(),
                len: len_p.max(len_n),
                s_p,
                s_n,
                h: rep.h,
                layer_v: b.segs[rep.vseg].layer,
                layer_a: aseg.layer,
                gap_len,
                band: [rep.lo, rep.hi, rep.ahi, rep.alo],
                line_side: pr.side == Side::Line,
                peak: 0.0,
                energy: 0.0,
                share: 0.0,
                db: 0.0,
                noise: vec![],
                path_energy: [0.0; 6],
                kl_p,
                kl_n,
                len_p,
                len_n,
                pitch,
                r_mm,
                eps_eff: rep.eps_eff,
                kl_gap_loop,
            });
        }
        // Hot loop pseudo-run.
        let area = edits.hot_loop_mm2.or(prep.hot_loop.as_ref().map(|h| h.area_mm2));
        if let (Some(hl), Some(area)) = (&prep.hot_loop, area) {
            let mut best = (f64::INFINITY, hl.centroid);
            let mut pitch = 0.5;
            for &(k, _) in &vsegs {
                let v = seg_with(b, k, &off);
                let d = point_seg(hl.centroid, v.a, v.b);
                if d < best.0 {
                    best = (d, nearest_on(hl.centroid, v.a, v.b));
                }
            }
            if let (Some(&(kp, _)), Some(&(kn, _))) = (vsegs.iter().find(|x| x.1), vsegs.iter().find(|x| !x.1)) {
                let (sp, sn) = (seg_with(b, kp, &off), seg_with(b, kn, &off));
                pitch = point_seg(sp.a, sn.a, sn.b).clamp(0.1, 2.0);
            }
            if best.0.is_finite() {
                let r = best.0.max(1.0);
                let c = hl.centroid;
                let dir = scale(sub(best.1, c), 1.0 / r);
                let nrm = [-dir[1], dir[0]];
                runs.push(Run {
                    kind: RunKind::HotLoop,
                    pair: pi,
                    aggr: None,
                    aggr_seg: None,
                    victim_segs: vec![],
                    len: (2.0 * r).min(vsegs.iter().map(|&(k, _)| len(sub(b.segs[k].b, b.segs[k].a))).sum::<f64>() / 2.0),
                    s_p: r,
                    s_n: r + pitch,
                    h: 0.0,
                    layer_v: 0,
                    layer_a: 0,
                    gap_len: 0.0,
                    band: [add(c, scale(nrm, 0.3)), add(best.1, scale(nrm, 0.3)), add(best.1, scale(nrm, -0.3)), add(c, scale(nrm, -0.3))],
                    line_side: pr.side == Side::Line,
                    peak: 0.0,
                    energy: 0.0,
                    share: 0.0,
                    db: 0.0,
                    noise: vec![],
                    path_energy: [0.0; 6],
                    kl_p: area,
                    kl_n: 0.0,
                    len_p: 0.0,
                    len_n: 0.0,
                    pitch,
                    r_mm: r,
                    eps_eff: 1.0,
                    kl_gap_loop: 0.0,
                });
            }
        }
        if params.cable.len_m > 0.0 {
            runs.push(Run {
                kind: RunKind::Cable,
                pair: pi,
                aggr: None,
                aggr_seg: None,
                victim_segs: vec![],
                len: params.cable.len_m * 1e3,
                s_p: params.cable.sep_m * 1e3,
                s_n: params.cable.sep_m * 1e3,
                h: params.cable.height_m * 1e3,
                layer_v: 0,
                layer_a: 0,
                gap_len: 0.0,
                band: [[0.0; 2]; 4],
                line_side: true,
                peak: 0.0,
                energy: 0.0,
                share: 0.0,
                db: 0.0,
                noise: vec![],
                path_energy: [0.0; 6],
                kl_p: 0.0,
                kl_n: 0.0,
                len_p: 0.0,
                len_n: 0.0,
                pitch: 0.0,
                r_mm: 0.0,
                eps_eff: 1.0,
                kl_gap_loop: 0.0,
            });
        }
    }

    // Noise per run.
    let lp = &params.link;
    let cmrr = from_db20(-params.cmrr_db);
    let mut pairs = Vec::new();
    for (pi, pr) in roles.pairs.iter().enumerate().filter(|(_, p)| p.enabled) {
        let net_len = |n: usize| b.segs.iter().filter(|s| s.net == n).map(|s| dist(s.a, s.b)).sum::<f64>();
        let skew_mm = (net_len(pr.p) - net_len(pr.n)).abs();
        let er = b.segs.iter().find(|s| s.net == pr.p).map(|s| b.eps_near(s.layer)).unwrap_or(4.5);
        let skew_s = skew_mm * 1e-3 * ((er + 1.0) / 2.0).sqrt() / C0;
        let mut path_noise: [Vec<f64>; 6] = Default::default();
        for v in path_noise.iter_mut() {
            *v = vec![0.0; N_FREQ];
        }
        let mut total2 = vec![0.0; N_FREQ];
        for run in runs.iter_mut().filter(|r| r.pair == pi) {
            let (dv, di, tr) = match run.kind {
                RunKind::Parallel => {
                    let a = &roles.aggressors[run.aggr.unwrap_or(0)];
                    let di = if a.kind == crate::roles::AggrKind::SwitchNode { a.di / tr_scale } else { a.di };
                    (a.dv, di, a.t_r * tr_scale)
                }
                RunKind::HotLoop => (0.0, params.inv.i_phase, t_r),
                RunKind::Cable => (params.inv.v_bus, params.c_cm * params.inv.v_bus / t_r, t_r),
            };
            let mut noise = vec![0.0; N_FREQ];
            let mut pe = [0.0; 6];
            let mut comp = [vec![0.0; N_FREQ], vec![0.0; N_FREQ], vec![0.0; N_FREQ], vec![0.0; N_FREQ], vec![0.0; N_FREQ], vec![0.0; N_FREQ]];
            for (i, &f) in fr.iter().enumerate() {
                let (vd, id) = (edge_density(dv, tr, f), edge_density(di, tr, f));
                let w = 2.0 * PI * f;
                let m = mode_conversion(params.lcl_db, skew_s, f);
                let mut c = [0.0; 6];
                match run.kind {
                    RunKind::Parallel => {
                        let v = C0 / run.eps_eff.sqrt();
                        let trace = |kl: f64, l: f64, cap: bool, ind: bool| {
                            let l = l.max(1e-6);
                            let g = sat(w * l * 1e-3 / v) * kl / l;
                            let cc = if cap { g * vd / 2.0 } else { 0.0 };
                            let ll = if ind { g * Z0 * id / 2.0 } else { 0.0 };
                            0.5 * ((cc + ll).powi(2) + (cc - ll).powi(2)).sqrt()
                        };
                        let up = |cap, ind| trace(run.kl_p, run.len_p, cap, ind);
                        let un = |cap, ind| trace(run.kl_n, run.len_n, cap, ind);
                        c[0] = (up(true, false) - un(true, false)).abs();
                        c[1] = (up(false, true) - un(false, true)).abs();
                        let both = (up(true, true) - un(true, true)).abs();
                        // Split the combined DM between the two paths by their size.
                        let sum = (c[0] + c[1]).max(1e-300);
                        c[0] = both * c[0] / sum;
                        c[1] = both * c[1] / sum;
                        c[2] = m * (up(true, true) + un(true, true)) / 2.0;
                        // Return-path loop over the void: area = gap length × pair pitch.
                        let area = run.kl_gap_loop * run.pitch * 1e-6;
                        c[3] = w * MU0 * id / (2.0 * PI * run.r_mm * 1e-3) * area;
                    }
                    RunKind::HotLoop => {
                        let r = run.r_mm * 1e-3;
                        let hfield = id * run.kl_p * 1e-6 / (4.0 * PI * r.powi(3));
                        c[4] = w * MU0 * hfield * run.pitch * 1e-3 * run.len * 1e-3;
                    }
                    RunKind::Cable => {
                        let (cap, ind) = cable::cable_cm(&params.cable, f, vd, id);
                        c[5] = cable::cable_dm(&params.cable, ind) + m * (cap + ind);
                    }
                }
                let att = if run.line_side || run.kind == RunKind::Cable { cmrr } else { 1.0 } * rx_filter(f, lp.rx_bw);
                let mut tot = 0.0;
                for k in 0..6 {
                    comp[k][i] = c[k] * att;
                    tot += c[k] * att;
                }
                noise[i] = tot;
                total2[i] += tot * tot;
            }
            for k in 0..6 {
                let e2: Vec<f64> = comp[k].iter().map(|x| x * x).collect();
                pe[k] = 2.0 * trapz(&fr, &e2);
                for i in 0..N_FREQ {
                    path_noise[k][i] = (path_noise[k][i].powi(2) + comp[k][i].powi(2)).sqrt();
                }
            }
            run.peak = 2.0 * trapz(&fr, &noise);
            run.energy = pe.iter().sum();
            run.path_energy = pe;
            run.noise = noise;
        }
        let tot: Vec<f64> = total2.iter().map(|x| x.sqrt()).collect();
        let peak = 2.0 * trapz(&fr, &tot);
        let energy = 2.0 * trapz(&fr, &total2);
        let burst = t_r + 3.0 * lp.ring_tau;
        let rms = (energy / burst).sqrt();
        let link = lp.evaluate(peak, rms, params.inv.f_pwm, t_r);
        let mut path_energy = [0.0; 6];
        for r in runs.iter().filter(|r| r.pair == pi) {
            for k in 0..6 {
                path_energy[k] += r.path_energy[k];
            }
        }
        let dom = (0..6).max_by(|&a, &b| path_energy[a].total_cmp(&path_energy[b])).unwrap_or(0);
        let total_e: f64 = runs.iter().filter(|r| r.pair == pi).map(|r| r.energy).sum();
        for r in runs.iter_mut().filter(|r| r.pair == pi) {
            r.share = if total_e > 0.0 { r.energy / total_e } else { 0.0 };
            r.db = 10.0 * (r.energy.max(1e-300) / total_e.max(1e-300)).log10();
        }
        pairs.push(PairResult {
            pair: pi,
            link,
            dominant: PATHS[dom],
            path_energy,
            path_noise,
            mode_conv: mode_conversion(params.lcl_db, skew_s, 31.25e6),
            skew_mm,
        });
    }
    Analysis {
        freqs: fr,
        runs,
        gaps,
        pairs,
        hot_loop_mm2: edits.hot_loop_mm2.or(prep.hot_loop.as_ref().map(|h| h.area_mm2)),
        t_r,
    }
}

fn nearest_on(p: P2, a: P2, b: P2) -> P2 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab).max(1e-12);
    add(a, scale(ab, (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)))
}

fn add_gap(ctx: &Ctx, gaps: &mut Vec<Gap>, plane: usize, pair: usize, at: P2, len: f64) {
    let r = match ctx.fills.iter().find(|(k, _)| *k == plane) {
        Some((_, r)) => r,
        None => &ctx.prep.planes[plane].raster,
    };
    let Some((ix, iy)) = r.cell(at) else { return };
    let (cells, capped) = r.flood(ix, iy, false, 400_000);
    let bbox = if capped || cells.is_empty() {
        [at[0] - 3.0, at[1] - 3.0, at[0] + 3.0, at[1] + 3.0]
    } else {
        let (mut lo, mut hi) = ([u32::MAX; 2], [0u32; 2]);
        for (x, y) in &cells {
            lo = [lo[0].min(*x), lo[1].min(*y)];
            hi = [hi[0].max(*x), hi[1].max(*y)];
        }
        let c0 = r.centre(lo[0] as usize, lo[1] as usize);
        let c1 = r.centre(hi[0] as usize, hi[1] as usize);
        [c0[0] - r.res, c0[1] - r.res, c1[0] + r.res, c1[1] + r.res]
    };
    if let Some(g) = gaps.iter_mut().find(|g| g.plane == plane && g.pair == pair && g.bbox == bbox) {
        g.len = g.len.max(len);
        return;
    }
    gaps.push(Gap { plane, pair, at, len, bbox });
}

// ---- Fixes ----

#[derive(Clone, Debug, PartialEq)]
pub enum FixKind {
    MovePair { mm: f64 },
    FillGap { at: P2 },
    ShrinkHotLoop { mm2: f64 },
    StitchVias { at: P2 },
    SlowEdges { t_r: f64 },
    InnerLayer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub kind: FixKind,
    pub edits: Edits,
    pub ber: f64,
    pub verdict: Verdict,
}

fn pair_ber(prep: &Prepared, roles: &Roles, params: &Params, e: &Edits, pair: usize) -> f64 {
    let a = analyze(prep, roles, params, e);
    a.pairs.iter().find(|p| p.pair == pair).map(|p| p.link.ber).unwrap_or(0.5)
}

/// Offsets that move every victim segment of `pair` coupled to its worst run by `mm` away from it.
pub fn move_pair_edits(an: &Analysis, prep: &Prepared, base: &Edits, pair: usize, mm: f64) -> Edits {
    let b = &prep.board;
    let mut e = base.clone();
    let Some(worst) = an.runs.iter().filter(|r| r.pair == pair && r.kind == RunKind::Parallel).max_by(|a, b| a.energy.total_cmp(&b.energy))
    else {
        return e;
    };
    let [vlo, vhi, ahi, alo] = worst.band;
    let dir = sub(scale(add(vlo, vhi), 0.5), scale(add(alo, ahi), 0.5));
    let d = len(dir);
    let u = sub(vhi, vlo);
    let nrm = if d > 1e-9 { scale(dir, 1.0 / d) } else { scale([-u[1], u[0]], 1.0 / len(u).max(1e-9)) };
    let mut segs: Vec<usize> = an.runs.iter().filter(|r| r.pair == pair && r.aggr == worst.aggr).flat_map(|r| r.victim_segs.iter().copied()).collect();
    segs.sort_unstable();
    segs.dedup();
    let _ = b;
    for k in segs {
        let prev = e.offsets.iter().position(|(s, _)| *s == k);
        let o = scale(nrm, mm);
        match prev {
            Some(i) => e.offsets[i].1 = add(e.offsets[i].1, o),
            None => e.offsets.push((k, o)),
        }
    }
    e
}

/// Candidate fixes for `pair`, each applied alone on top of `base`, sorted by resulting BER.
pub fn fixes(prep: &Prepared, roles: &Roles, params: &Params, base: &Edits, pair: usize) -> Vec<Fix> {
    let an = analyze(prep, roles, params, base);
    let mut out = Vec::new();
    let mut push = |kind: FixKind, e: Edits| {
        let ber = pair_ber(prep, roles, params, &e, pair);
        out.push(Fix { kind, edits: e, ber, verdict: crate::link::verdict(ber) });
    };
    // Move the pair: smallest step (0.5 mm grid) that reaches OK, else the 10 mm result.
    if an.runs.iter().any(|r| r.pair == pair && r.kind == RunKind::Parallel) {
        let mut chosen = None;
        let mut mm = 0.5;
        while mm <= params.search_mm + 1e-9 {
            let e = move_pair_edits(&an, prep, base, pair, mm);
            if crate::link::verdict(pair_ber(prep, roles, params, &e, pair)) == Verdict::Ok {
                chosen = Some((mm, e));
                break;
            }
            mm += 0.5;
        }
        let (mm, e) = chosen.unwrap_or_else(|| (params.search_mm, move_pair_edits(&an, prep, base, pair, params.search_mm)));
        push(FixKind::MovePair { mm }, e);
    }
    let gaps: Vec<&Gap> = an.gaps.iter().filter(|g| g.pair == pair).collect();
    if !gaps.is_empty() {
        let mut e = base.clone();
        e.fills.extend(gaps.iter().map(|g| (g.plane, g.bbox)));
        push(FixKind::FillGap { at: gaps[0].at }, e);
        let mut e = base.clone();
        e.stitches.extend(gaps.iter().map(|g| (g.plane, g.bbox)));
        push(FixKind::StitchVias { at: gaps[0].at }, e);
    }
    if let Some(area) = an.hot_loop_mm2 {
        // Bisection for the area that reaches OK, floored at 10 %.
        let ok = |a: f64| {
            let e = Edits { hot_loop_mm2: Some(a), ..base.clone() };
            crate::link::verdict(pair_ber(prep, roles, params, &e, pair)) == Verdict::Ok
        };
        let target = if ok(area) {
            area * 0.5
        } else if ok(area * 0.1) {
            let (mut lo, mut hi) = (area * 0.1, area);
            for _ in 0..12 {
                let m = (lo + hi) / 2.0;
                if ok(m) {
                    lo = m;
                } else {
                    hi = m;
                }
            }
            lo
        } else {
            area * 0.1
        };
        let target = (target * 10.0).floor() / 10.0;
        push(FixKind::ShrinkHotLoop { mm2: target }, Edits { hot_loop_mm2: Some(target), ..base.clone() });
    }
    let e = Edits { tr_scale: base.tr_scale * 2.0, ..base.clone() };
    push(FixKind::SlowEdges { t_r: params.inv.t_r * e.tr_scale }, e);
    if prep.board.layers.len() >= 4 && !base.inner.contains(&pair) {
        let mut e = base.clone();
        e.inner.push(pair);
        push(FixKind::InnerLayer, e);
    }
    out.sort_by(|a, b| a.ber.total_cmp(&b.ber));
    out
}

/// Nearest coupled run to board point p (mm) within tol, for hover / click.
pub fn pick_run(an: &Analysis, p: P2, tol: f64) -> Option<usize> {
    let mut best = (tol, None);
    for (i, r) in an.runs.iter().enumerate().filter(|(_, r)| r.kind != RunKind::Cable) {
        let q = r.band;
        let inside = (0..4).all(|k| cross(sub(q[(k + 1) % 4], q[k]), sub(p, q[k])) >= 0.0) || (0..4).all(|k| cross(sub(q[(k + 1) % 4], q[k]), sub(p, q[k])) <= 0.0);
        let d = if inside { 0.0 } else { (0..4).map(|k| point_seg(p, q[k], q[(k + 1) % 4])).fold(f64::INFINITY, f64::min) };
        if d <= best.0 {
            best = (d, Some(i));
        }
    }
    best.1
}

/// Nearest victim segment (with edits applied) to p within tol.
pub fn pick_victim(prep: &Prepared, roles: &Roles, edits: &Edits, p: P2, tol: f64) -> Option<usize> {
    let off: HashMap<usize, P2> = edits.offsets.iter().copied().collect();
    let nets: Vec<usize> = roles.pairs.iter().filter(|r| r.enabled).flat_map(|r| [r.p, r.n]).collect();
    let mut best = (tol, None);
    for k in (0..prep.board.segs.len()).filter(|&k| nets.contains(&prep.board.segs[k].net)) {
        let s = seg_with(&prep.board, k, &off);
        let d = point_seg(p, s.a, s.b) - s.w / 2.0;
        if d <= best.0 {
            best = (d, Some(k));
        }
    }
    best.1
}

/// The partner-net segment running alongside `seg` (so a drag moves the pair together).
pub fn partner(prep: &Prepared, roles: &Roles, seg: usize) -> Option<usize> {
    let b = &prep.board;
    let s = b.segs.get(seg)?;
    let pr = roles.pairs.iter().find(|r| r.p == s.net || r.n == s.net)?;
    let other = if pr.p == s.net { pr.n } else { pr.p };
    let u = sub(s.b, s.a);
    let lu = len(u).max(1e-9);
    b.segs
        .iter()
        .enumerate()
        .filter(|(_, t)| t.net == other && t.layer == s.layer)
        .filter(|(_, t)| (dot(u, sub(t.b, t.a)) / (lu * len(sub(t.b, t.a)).max(1e-9))).abs() > 0.95)
        .map(|(k, t)| (k, point_seg(scale(add(s.a, s.b), 0.5), t.a, t.b)))
        .filter(|(_, d)| *d < 2.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// Apply edits to the segment geometry (for drawing).
pub fn edited_seg(prep: &Prepared, edits: &Edits, k: usize) -> Seg {
    let off: HashMap<usize, P2> = edits.offsets.iter().copied().collect();
    seg_with(&prep.board, k, &off)
}
