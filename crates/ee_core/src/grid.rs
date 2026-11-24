//! Uniform-grid neighbour search between two segment sets (no O(n²)).
use crate::board::{seg_seg, P2};

/// A segment as (a, b, half width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct S {
    pub a: P2,
    pub b: P2,
    pub hw: f64,
}

/// Bins segments of `b` on a grid with cell ≈ radius; built once, queried many times.
pub struct Grid {
    x0: f64,
    y0: f64,
    cell: f64,
    nx: usize,
    ny: usize,
    start: Vec<u32>,
    items: Vec<u32>,
    segs: Vec<S>,
}

impl Grid {
    pub fn new(segs: &[S], radius: f64) -> Grid {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for s in segs {
            for p in [s.a, s.b] {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        if segs.is_empty() {
            lo = [0.0; 2];
            hi = [1.0; 2];
        }
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-6);
        let cell = radius.max(span / 1024.0).max(1e-3);
        let nx = ((hi[0] - lo[0]) / cell) as usize + 1;
        let ny = ((hi[1] - lo[1]) / cell) as usize + 1;
        let mut g = Grid { x0: lo[0], y0: lo[1], cell, nx, ny, start: vec![0; nx * ny + 1], items: Vec::new(), segs: segs.to_vec() };
        // Counting pass, then fill (CSR layout).
        let mut counts = vec![0u32; nx * ny];
        for s in segs {
            g.each_cell(s, 0.0, |c| counts[c] += 1);
        }
        for c in 0..nx * ny {
            g.start[c + 1] = g.start[c] + counts[c];
        }
        g.items = vec![0; g.start[nx * ny] as usize];
        let mut fill = g.start.clone();
        for (k, s) in segs.iter().enumerate() {
            let (x0, y0, cell, nx, ny) = (g.x0, g.y0, g.cell, g.nx, g.ny);
            for_cells(x0, y0, cell, nx, ny, s, 0.0, |c| {
                g.items[fill[c] as usize] = k as u32;
                fill[c] += 1;
            });
        }
        g
    }

    fn each_cell(&self, s: &S, pad: f64, f: impl FnMut(usize)) {
        for_cells(self.x0, self.y0, self.cell, self.nx, self.ny, s, pad, f);
    }

    /// Indices of grid segments whose cells overlap q's bbox grown by r (no exact test), ascending.
    pub fn candidates(&self, q: &S, r: f64, out: &mut Vec<u32>) {
        out.clear();
        self.each_cell(q, r + q.hw, |c| out.extend_from_slice(&self.items[self.start[c] as usize..self.start[c + 1] as usize]));
        out.sort_unstable();
        out.dedup();
    }

    /// Indices of grid segments within `r` (edge to edge) of `q`, ascending.
    pub fn query(&self, q: &S, r: f64, out: &mut Vec<u32>) {
        out.clear();
        self.each_cell(q, r + q.hw, |c| out.extend_from_slice(&self.items[self.start[c] as usize..self.start[c + 1] as usize]));
        out.sort_unstable();
        out.dedup();
        out.retain(|&k| near(q, &self.segs[k as usize], r));
    }
}

#[allow(clippy::too_many_arguments)]
fn for_cells(x0: f64, y0: f64, cell: f64, nx: usize, ny: usize, s: &S, pad: f64, mut f: impl FnMut(usize)) {
    let p = pad + s.hw;
    let cx = |x: f64| (((x - x0) / cell).floor().max(0.0) as usize).min(nx - 1);
    let cy = |y: f64| (((y - y0) / cell).floor().max(0.0) as usize).min(ny - 1);
    let (ix0, ix1) = (cx(s.a[0].min(s.b[0]) - p), cx(s.a[0].max(s.b[0]) + p));
    let (iy0, iy1) = (cy(s.a[1].min(s.b[1]) - p), cy(s.a[1].max(s.b[1]) + p));
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            f(iy * nx + ix);
        }
    }
}

/// Edge-to-edge distance ≤ r.
pub fn near(a: &S, b: &S, r: f64) -> bool {
    seg_seg(a.a, a.b, b.a, b.b) - a.hw - b.hw <= r
}

/// All (i, j) with a[i] near b[j], via the grid.
pub fn pairs_grid(a: &[S], b: &[S], r: f64) -> Vec<(u32, u32)> {
    let g = Grid::new(b, r);
    let mut out = Vec::new();
    let mut tmp = Vec::new();
    for (i, s) in a.iter().enumerate() {
        g.query(s, r, &mut tmp);
        out.extend(tmp.iter().map(|&j| (i as u32, j)));
    }
    out
}

/// Reference brute force.
pub fn pairs_brute(a: &[S], b: &[S], r: f64) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for (i, s) in a.iter().enumerate() {
        for (j, t) in b.iter().enumerate() {
            if near(s, t, r) {
                out.push((i as u32, j as u32));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::num::Rng;

    fn random(rng: &mut Rng, n: usize, size: f64) -> Vec<S> {
        (0..n)
            .map(|_| {
                let a = [rng.range(0.0, size), rng.range(0.0, size)];
                let (l, t) = (rng.range(0.1, 15.0), rng.range(0.0, 6.3));
                S { a, b: [a[0] + l * t.cos(), a[1] + l * t.sin()], hw: rng.range(0.05, 1.0) }
            })
            .collect()
    }

    #[test]
    fn grid_equals_brute_force_on_random_boards() {
        for seed in 1..=6u64 {
            let mut rng = Rng(seed * 7919);
            let size = [20.0, 80.0, 300.0][seed as usize % 3];
            let a = random(&mut rng, 300, size);
            let b = random(&mut rng, 400, size);
            for r in [0.5, 10.0] {
                assert_eq!(pairs_grid(&a, &b, r), pairs_brute(&a, &b, r), "seed {seed} r {r}");
            }
        }
    }
}
