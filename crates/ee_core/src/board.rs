//! Format-neutral board model (millimetres, KiCad orientation: x right, y down).

pub type P2 = [f64; 2];

#[derive(Clone, Debug, PartialEq)]
pub struct CopperLayer {
    pub name: String,
    /// Depth of the layer centre below the top surface (mm).
    pub z: f64,
    pub thickness: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub a: P2,
    pub b: P2,
    pub w: f64,
    /// Index into `Board::layers`.
    pub layer: usize,
    /// Index into `Board::nets` (0 = no net).
    pub net: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Via {
    pub at: P2,
    pub size: f64,
    pub drill: f64,
    /// First and last copper layer spanned.
    pub span: (usize, usize),
    pub net: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadShape {
    Rect,
    Circle,
    Oval,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    pub at: P2,
    pub size: P2,
    /// Absolute rotation (deg).
    pub rot: f64,
    pub shape: PadShape,
    pub layers: Vec<usize>,
    pub net: usize,
    pub footprint: usize,
    pub number: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    pub reference: String,
    pub name: String,
    pub at: P2,
    pub rot: f64,
    pub bottom: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Zone {
    pub layer: usize,
    pub net: usize,
    /// Filled polygons (even-odd; holes may be bridged into the outline).
    pub polys: Vec<Vec<P2>>,
    /// Clear polarity (Gerber LPC): erases copper drawn before it.
    pub clear: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Board {
    pub nets: Vec<String>,
    pub layers: Vec<CopperLayer>,
    /// Dielectric εr between copper layer k and k+1.
    pub eps_r: Vec<f64>,
    pub thickness: f64,
    pub segs: Vec<Seg>,
    pub vias: Vec<Via>,
    pub pads: Vec<Pad>,
    pub footprints: Vec<Footprint>,
    pub zones: Vec<Zone>,
    /// Layer guesses / source notes (Gerber), shown to the user.
    pub notes: Vec<String>,
}

impl Board {
    pub fn net_id(&self, name: &str) -> Option<usize> {
        self.nets.iter().position(|n| n == name)
    }
    /// Gap between the centres of copper layers a and b (mm).
    pub fn layer_gap(&self, a: usize, b: usize) -> f64 {
        match (self.layers.get(a), self.layers.get(b)) {
            (Some(x), Some(y)) => (x.z - y.z).abs(),
            _ => self.thickness,
        }
    }
    /// Effective εr of the dielectric next to layer `l`.
    pub fn eps_near(&self, l: usize) -> f64 {
        let k = l.min(self.eps_r.len().saturating_sub(1));
        self.eps_r.get(k).copied().unwrap_or(4.5)
    }
    pub fn is_outer(&self, l: usize) -> bool {
        l == 0 || l + 1 == self.layers.len()
    }
    /// Bounding box of all copper (min x, min y, max x, max y), padded by 1 mm.
    pub fn bbox(&self) -> [f64; 4] {
        let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut add = |p: P2, r: f64| {
            bb[0] = bb[0].min(p[0] - r);
            bb[1] = bb[1].min(p[1] - r);
            bb[2] = bb[2].max(p[0] + r);
            bb[3] = bb[3].max(p[1] + r);
        };
        for s in &self.segs {
            add(s.a, s.w);
            add(s.b, s.w);
        }
        for v in &self.vias {
            add(v.at, v.size);
        }
        for p in &self.pads {
            add(p.at, p.size[0].max(p.size[1]));
        }
        for z in &self.zones {
            for poly in &z.polys {
                for &q in poly {
                    add(q, 0.0);
                }
            }
        }
        if !bb[0].is_finite() {
            return [0.0, 0.0, 1.0, 1.0];
        }
        [bb[0] - 1.0, bb[1] - 1.0, bb[2] + 1.0, bb[3] + 1.0]
    }
    /// Pads of footprint `fp`.
    pub fn pads_of(&self, fp: usize) -> impl Iterator<Item = &Pad> {
        self.pads.iter().filter(move |p| p.footprint == fp)
    }
}

/// Default stackup: `n` copper layers spread evenly over `thickness`.
pub fn even_layers(n: usize, thickness: f64) -> Vec<CopperLayer> {
    let n = n.max(1);
    (0..n)
        .map(|k| CopperLayer {
            name: match k {
                0 => "F.Cu".into(),
                k if k + 1 == n => "B.Cu".into(),
                k => format!("In{k}.Cu"),
            },
            z: if n == 1 { 0.0 } else { thickness * k as f64 / (n - 1) as f64 },
            thickness: 0.035,
        })
        .collect()
}

// ---- 2D geometry ----

pub fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}
pub fn add(a: P2, b: P2) -> P2 {
    [a[0] + b[0], a[1] + b[1]]
}
pub fn scale(a: P2, s: f64) -> P2 {
    [a[0] * s, a[1] * s]
}
pub fn dot(a: P2, b: P2) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
pub fn cross(a: P2, b: P2) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
pub fn len(a: P2) -> f64 {
    dot(a, a).sqrt()
}
pub fn dist(a: P2, b: P2) -> f64 {
    len(sub(a, b))
}

/// Distance from p to segment ab.
pub fn point_seg(p: P2, a: P2, b: P2) -> f64 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    let t = if l2 > 0.0 { (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    dist(p, add(a, scale(ab, t)))
}

/// Minimum distance between segments ab and cd (0 if they cross).
pub fn seg_seg(a: P2, b: P2, c: P2, d: P2) -> f64 {
    let (r, s) = (sub(b, a), sub(d, c));
    let den = cross(r, s);
    if den.abs() > 1e-12 {
        let t = cross(sub(c, a), s) / den;
        let u = cross(sub(c, a), r) / den;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
            return 0.0;
        }
    }
    point_seg(a, c, d).min(point_seg(b, c, d)).min(point_seg(c, a, b)).min(point_seg(d, a, b))
}

/// Rotate a KiCad local offset by `deg` (counter-clockwise on screen, y down).
pub fn rot(p: P2, deg: f64) -> P2 {
    let (s, c) = deg.to_radians().sin_cos();
    [p[0] * c + p[1] * s, -p[0] * s + p[1] * c]
}

/// Shoelace area (absolute).
pub fn poly_area(p: &[P2]) -> f64 {
    let n = p.len();
    (0..n).map(|i| cross(p[i], p[(i + 1) % n])).sum::<f64>().abs() / 2.0
}

pub fn centroid(p: &[P2]) -> P2 {
    let n = p.len().max(1) as f64;
    let s = p.iter().fold([0.0, 0.0], |a, q| add(a, *q));
    scale(s, 1.0 / n)
}

/// Convex hull (monotone chain), counter-clockwise.
pub fn hull(pts: &[P2]) -> Vec<P2> {
    let mut p: Vec<P2> = pts.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let mut h: Vec<P2> = Vec::new();
    for pass in 0..2 {
        let start = h.len();
        let it: Box<dyn Iterator<Item = &P2>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= start + 2 && cross(sub(h[h.len() - 1], h[h.len() - 2]), sub(q, h[h.len() - 2])) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// Points on the circular arc through start, mid, end (chord error ≲ 0.02 mm).
pub fn arc_points(a: P2, m: P2, b: P2) -> Vec<P2> {
    let d = 2.0 * (a[0] * (m[1] - b[1]) + m[0] * (b[1] - a[1]) + b[0] * (a[1] - m[1]));
    if d.abs() < 1e-12 {
        return vec![a, b];
    }
    let sq = |p: P2| dot(p, p);
    let c = [
        (sq(a) * (m[1] - b[1]) + sq(m) * (b[1] - a[1]) + sq(b) * (a[1] - m[1])) / d,
        (sq(a) * (b[0] - m[0]) + sq(m) * (a[0] - b[0]) + sq(b) * (m[0] - a[0])) / d,
    ];
    let r = dist(a, c);
    let ang = |p: P2| (p[1] - c[1]).atan2(p[0] - c[0]);
    let (t0, tm, t1) = (ang(a), ang(m), ang(b));
    let tau = std::f64::consts::TAU;
    let norm = |x: f64| x.rem_euclid(tau);
    // Sweep from t0 through tm to t1 in whichever direction contains tm.
    let ccw = norm(tm - t0) < norm(t1 - t0);
    let sweep = if ccw { norm(t1 - t0) } else { -norm(t0 - t1) };
    arc_by_sweep(c, r, t0, sweep)
}

/// Points on an arc of radius r about c from angle t0 sweeping `sweep` rad.
pub fn arc_by_sweep(c: P2, r: f64, t0: f64, sweep: f64) -> Vec<P2> {
    let step = if r > 0.02 { 2.0 * (1.0 - 0.02 / r).clamp(-1.0, 1.0).acos() } else { 1.0 };
    let n = ((sweep.abs() / step.max(0.05)).ceil() as usize).clamp(1, 256);
    (0..=n)
        .map(|i| {
            let t = t0 + sweep * i as f64 / n as f64;
            [c[0] + r * t.cos(), c[1] + r * t.sin()]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance_and_hull_area() {
        assert!((seg_seg([0., 0.], [10., 0.], [0., 2.], [10., 2.]) - 2.0).abs() < 1e-12);
        assert_eq!(seg_seg([0., 0.], [10., 0.], [5., -1.], [5., 1.]), 0.0);
        let h = hull(&[[0., 0.], [4., 0.], [0., 3.], [1., 1.]]);
        assert_eq!(h.len(), 3);
        assert!((poly_area(&h) - 6.0).abs() < 1e-12);
        let arc = arc_points([1., 0.], [0., 1.], [-1., 0.]);
        assert!(arc.iter().all(|p| (len(*p) - 1.0).abs() < 1e-9));
        assert!(arc.iter().all(|p| p[1] >= -1e-9), "passes through the mid point side");
    }
}
