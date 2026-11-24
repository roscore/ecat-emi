//! Bit raster over the board bbox (default 0.1 mm) for planes, voids and Gerber polarity.
use crate::board::{point_seg, P2};

#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub x0: f64,
    pub y0: f64,
    pub res: f64,
    pub nx: usize,
    pub ny: usize,
    pub bits: Vec<u64>,
}

/// Largest raster we allocate (cells); coarser resolution beyond that.
const MAX_CELLS: f64 = 64e6;

impl Raster {
    pub fn new(bbox: [f64; 4], res: f64) -> Raster {
        let (w, h) = ((bbox[2] - bbox[0]).max(res), (bbox[3] - bbox[1]).max(res));
        let res = res.max((w * h / MAX_CELLS).sqrt());
        let nx = (w / res).ceil() as usize + 1;
        let ny = (h / res).ceil() as usize + 1;
        Raster { x0: bbox[0], y0: bbox[1], res, nx, ny, bits: vec![0; (nx * ny).div_ceil(64)] }
    }
    pub fn get(&self, ix: usize, iy: usize) -> bool {
        if ix >= self.nx || iy >= self.ny {
            return false;
        }
        let i = iy * self.nx + ix;
        self.bits.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
    }
    pub fn set(&mut self, ix: usize, iy: usize, v: bool) {
        if ix >= self.nx || iy >= self.ny {
            return;
        }
        let i = iy * self.nx + ix;
        if let Some(w) = self.bits.get_mut(i / 64) {
            if v {
                *w |= 1 << (i % 64);
            } else {
                *w &= !(1 << (i % 64));
            }
        }
    }
    /// Cell index containing point p.
    pub fn cell(&self, p: P2) -> Option<(usize, usize)> {
        let fx = (p[0] - self.x0) / self.res;
        let fy = (p[1] - self.y0) / self.res;
        (fx >= 0.0 && fy >= 0.0 && (fx as usize) < self.nx && (fy as usize) < self.ny).then(|| (fx as usize, fy as usize))
    }
    pub fn centre(&self, ix: usize, iy: usize) -> P2 {
        [self.x0 + (ix as f64 + 0.5) * self.res, self.y0 + (iy as f64 + 0.5) * self.res]
    }
    pub fn at(&self, p: P2) -> bool {
        self.cell(p).is_some_and(|(x, y)| self.get(x, y))
    }
    pub fn count(&self) -> usize {
        self.bits.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Even-odd scanline fill of one polygon (cell centres), writing `v`.
    pub fn fill_poly(&mut self, poly: &[P2], v: bool) {
        let n = poly.len();
        if n < 3 {
            return;
        }
        let (mut ymin, mut ymax) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in poly {
            ymin = ymin.min(p[1]);
            ymax = ymax.max(p[1]);
        }
        let iy0 = (((ymin - self.y0) / self.res - 0.5).floor().max(0.0)) as usize;
        let iy1 = ((((ymax - self.y0) / self.res - 0.5).ceil()).max(0.0) as usize).min(self.ny.saturating_sub(1));
        let mut xs: Vec<f64> = Vec::new();
        for iy in iy0..=iy1 {
            let y = self.y0 + (iy as f64 + 0.5) * self.res;
            xs.clear();
            for i in 0..n {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                if (a[1] <= y) != (b[1] <= y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
            xs.sort_by(f64::total_cmp);
            for pair in xs.chunks(2) {
                if let [xa, xb] = *pair {
                    let ia = ((xa - self.x0) / self.res - 0.5).ceil().max(0.0) as usize;
                    let ib = ((xb - self.x0) / self.res - 0.5).floor();
                    if ib < 0.0 {
                        continue;
                    }
                    for ix in ia..=(ib as usize).min(self.nx.saturating_sub(1)) {
                        self.set(ix, iy, v);
                    }
                }
            }
        }
    }

    /// Stroke a round-capped line of radius r.
    pub fn fill_capsule(&mut self, a: P2, b: P2, r: f64, v: bool) {
        let (lo, hi) = ([a[0].min(b[0]) - r, a[1].min(b[1]) - r], [a[0].max(b[0]) + r, a[1].max(b[1]) + r]);
        self.for_box(lo, hi, |s, ix, iy| {
            if point_seg(s.centre(ix, iy), a, b) <= r {
                s.set(ix, iy, v);
            }
        });
    }

    /// Axis-aligned rectangle (centre, half sizes) rotated by `deg`.
    pub fn fill_rect(&mut self, c: P2, half: P2, deg: f64, v: bool) {
        let corners = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
            .map(|[sx, sy]: [f64; 2]| crate::board::add(c, crate::board::rot([sx * half[0], sy * half[1]], deg)));
        self.fill_poly(&corners, v);
    }

    fn for_box(&mut self, lo: P2, hi: P2, mut f: impl FnMut(&mut Raster, usize, usize)) {
        let ix0 = ((lo[0] - self.x0) / self.res).floor().max(0.0) as usize;
        let iy0 = ((lo[1] - self.y0) / self.res).floor().max(0.0) as usize;
        let ix1 = (((hi[0] - self.x0) / self.res).ceil().max(0.0) as usize).min(self.nx.saturating_sub(1));
        let iy1 = (((hi[1] - self.y0) / self.res).ceil().max(0.0) as usize).min(self.ny.saturating_sub(1));
        for iy in iy0..=iy1 {
            for ix in ix0..=ix1 {
                f(self, ix, iy);
            }
        }
    }

    /// Set every cell in the bbox (x0, y0, x1, y1).
    pub fn fill_box(&mut self, bb: [f64; 4], v: bool) {
        self.for_box([bb[0], bb[1]], [bb[2], bb[3]], |s, x, y| s.set(x, y, v));
    }

    /// 4-connected flood of cells equal to `val` from (ix, iy), at most `cap` cells.
    /// Returns the cells and whether the cap was hit.
    pub fn flood(&self, ix: usize, iy: usize, val: bool, cap: usize) -> (Vec<(u32, u32)>, bool) {
        let mut seen = vec![0u64; self.bits.len()];
        let mut out = Vec::new();
        let mut stack = vec![(ix, iy)];
        let mark = |seen: &mut Vec<u64>, x: usize, y: usize| -> bool {
            let i = y * self.nx + x;
            let w = &mut seen[i / 64];
            let was = *w >> (i % 64) & 1 == 1;
            *w |= 1 << (i % 64);
            !was
        };
        if ix >= self.nx || iy >= self.ny || self.get(ix, iy) != val {
            return (out, false);
        }
        mark(&mut seen, ix, iy);
        while let Some((x, y)) = stack.pop() {
            out.push((x as u32, y as u32));
            if out.len() >= cap {
                return (out, true);
            }
            let nb = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
            for (nx, ny) in nb {
                if nx < self.nx && ny < self.ny && self.get(nx, ny) == val && mark(&mut seen, nx, ny) {
                    stack.push((nx, ny));
                }
            }
        }
        (out, false)
    }

    /// Horizontal runs of set cells per row: (iy, ix_start, ix_end_inclusive). For drawing.
    pub fn runs(&self) -> Vec<(u32, u32, u32)> {
        let mut v = Vec::new();
        for iy in 0..self.ny {
            let mut ix = 0;
            while ix < self.nx {
                if self.get(ix, iy) {
                    let s = ix;
                    while ix + 1 < self.nx && self.get(ix + 1, iy) {
                        ix += 1;
                    }
                    v.push((iy as u32, s as u32, ix as u32));
                }
                ix += 1;
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridged_hole_is_empty() {
        let mut r = Raster::new([0.0, 0.0, 10.0, 10.0], 0.1);
        let poly = [[4.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0], [4.0, 4.0]];
        r.fill_poly(&poly, true);
        assert!(r.at([1.0, 1.0]) && r.at([4.02, 2.0]) && r.at([3.98, 2.0]));
        assert!(!r.at([5.0, 5.0]));
        let cells = r.cell([5.0, 5.0]).unwrap();
        let (hole, capped) = r.flood(cells.0, cells.1, false, 10_000);
        assert!(!capped);
        assert!((hole.len() as f64 - 400.0).abs() < 50.0, "{}", hole.len());
    }
}
