//! Gerber RS-274X + Excellon drill reader, layer guessing, and copper-island connectivity.
//! Supported: %FS/%MO, %AD (C/R/O/P), %LP, D01/D02/D03, G01/G02/G03, G74/G75, G36/G37, X2 TF.
//! Not supported: aperture macros (AM: flashes are skipped), step-and-repeat, block apertures.
use crate::board::*;
use crate::raster::Raster;
use crate::sexpr::{err, ParseError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aperture {
    Circle(f64),
    Rect(f64, f64),
    Obround(f64, f64),
    Polygon(f64, u32, f64),
}

impl Aperture {
    /// Stroke width when used for a draw.
    pub fn width(&self) -> f64 {
        match *self {
            Aperture::Circle(d) | Aperture::Polygon(d, _, _) => d,
            Aperture::Rect(w, h) | Aperture::Obround(w, h) => w.min(h),
        }
    }
    /// Outline polygon at `c`.
    pub fn outline(&self, c: P2) -> Vec<P2> {
        match *self {
            Aperture::Circle(d) => arc_by_sweep(c, d / 2.0, 0.0, std::f64::consts::TAU),
            Aperture::Rect(w, h) => vec![[c[0] - w / 2.0, c[1] - h / 2.0], [c[0] + w / 2.0, c[1] - h / 2.0], [c[0] + w / 2.0, c[1] + h / 2.0], [c[0] - w / 2.0, c[1] + h / 2.0]],
            Aperture::Obround(w, h) => {
                let r = w.min(h) / 2.0;
                let (dx, dy) = ((w / 2.0 - r).max(0.0), (h / 2.0 - r).max(0.0));
                let pi = std::f64::consts::PI;
                let mut v = Vec::new();
                if w >= h {
                    v.extend(arc_by_sweep([c[0] + dx, c[1]], r, -pi / 2.0, pi));
                    v.extend(arc_by_sweep([c[0] - dx, c[1]], r, pi / 2.0, pi));
                } else {
                    v.extend(arc_by_sweep([c[0], c[1] + dy], r, 0.0, pi));
                    v.extend(arc_by_sweep([c[0], c[1] - dy], r, pi, pi));
                }
                v
            }
            Aperture::Polygon(d, n, rot) => {
                let n = n.clamp(3, 12);
                (0..n)
                    .map(|k| {
                        let t = rot.to_radians() + k as f64 * std::f64::consts::TAU / n as f64;
                        [c[0] + d / 2.0 * t.cos(), c[1] + d / 2.0 * t.sin()]
                    })
                    .collect()
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Line { a: P2, b: P2, w: f64, dark: bool },
    Flash { at: P2, ap: Aperture, dark: bool },
    Region { poly: Vec<P2>, dark: bool },
}

impl Prim {
    pub fn dark(&self) -> bool {
        match self {
            Prim::Line { dark, .. } | Prim::Flash { dark, .. } | Prim::Region { dark, .. } => *dark,
        }
    }
    /// Polygon approximation (lines become stadiums).
    pub fn polygon(&self) -> Vec<P2> {
        match self {
            Prim::Line { a, b, w, .. } => {
                let d = sub(*b, *a);
                let t = d[1].atan2(d[0]);
                let pi = std::f64::consts::PI;
                let mut v = arc_by_sweep(*b, w / 2.0, t - pi / 2.0, pi);
                v.extend(arc_by_sweep(*a, w / 2.0, t + pi / 2.0, pi));
                v
            }
            Prim::Flash { at, ap, .. } => ap.outline(*at),
            Prim::Region { poly, .. } => poly.clone(),
        }
    }
    pub fn bbox(&self) -> [f64; 4] {
        let p = match self {
            Prim::Region { poly, .. } => poly.clone(),
            _ => self.polygon(),
        };
        let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for q in p {
            bb = [bb[0].min(q[0]), bb[1].min(q[1]), bb[2].max(q[0]), bb[3].max(q[1])];
        }
        bb
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GerberLayer {
    pub prims: Vec<Prim>,
    /// X2 `.FileFunction` value, if present.
    pub function: Option<String>,
}

struct St {
    dec: (u32, u32),
    unit: f64,
    pos: P2,
    interp: u8,
    multi: bool,
    region: bool,
    contour: Vec<P2>,
    ap: Option<Aperture>,
    aps: Vec<(u32, Aperture)>,
    dark: bool,
}

fn coord(s: &str, dec: u32, unit: f64, line: u32) -> Result<f64, ParseError> {
    if s.contains('.') {
        return s.parse::<f64>().map(|v| v * unit).map_err(|_| ParseError { line, msg: format!("bad coordinate '{s}'") });
    }
    let v: i64 = s.parse().map_err(|_| ParseError { line, msg: format!("bad coordinate '{s}'") })?;
    Ok(v as f64 / 10f64.powi(dec as i32) * unit)
}

/// Parse one RS-274X file. Coordinates come out in mm with y flipped (y down, like KiCad).
pub fn parse(src: &str) -> Result<GerberLayer, ParseError> {
    let mut out = GerberLayer::default();
    let mut st = St { dec: (6, 6), unit: 1.0, pos: [0.0; 2], interp: 1, multi: false, region: false, contour: vec![], ap: None, aps: vec![], dark: true };
    let b = src.as_bytes();
    let (mut i, mut line) = (0usize, 1u32);
    let mut saw_any = false;
    while i < b.len() {
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'\r' | b' ' | b'\t' => i += 1,
            b'%' => {
                let l0 = line;
                let st0 = i + 1;
                let Some(end) = b[st0..].iter().position(|&c| c == b'%') else { return err(l0, "unterminated '%' block") };
                let block = &src[st0..st0 + end];
                line += block.matches('\n').count() as u32;
                i = st0 + end + 1;
                for cmd in block.split('*').map(|c| c.trim()).filter(|c| !c.is_empty()) {
                    extended(cmd, &mut st, &mut out, l0)?;
                    saw_any = true;
                }
            }
            _ => {
                let l0 = line;
                let st0 = i;
                let Some(end) = b[st0..].iter().position(|&c| c == b'*') else {
                    if src[st0..].trim().is_empty() {
                        break;
                    }
                    return err(l0, "command without terminating '*'");
                };
                let cmd: String = src[st0..st0 + end].chars().filter(|c| !c.is_whitespace()).collect();
                line += src[st0..st0 + end].matches('\n').count() as u32;
                i = st0 + end + 1;
                if word(&cmd, &mut st, &mut out, l0)? {
                    break;
                }
                saw_any = true;
            }
        }
    }
    if !saw_any {
        return err(1, "no Gerber commands found");
    }
    Ok(out)
}

fn extended(cmd: &str, st: &mut St, out: &mut GerberLayer, line: u32) -> Result<(), ParseError> {
    if let Some(r) = cmd.strip_prefix("FS") {
        // FSLAX36Y36: decimals are the second digit of X.
        let x = r.find('X').ok_or(ParseError { line, msg: "bad %FS".into() })?;
        let d = r.get(x + 2..x + 3).and_then(|c| c.parse().ok()).ok_or(ParseError { line, msg: "bad %FS".into() })?;
        st.dec = (d, d);
    } else if cmd == "MOIN" {
        st.unit = 25.4;
    } else if cmd == "MOMM" {
        st.unit = 1.0;
    } else if cmd == "LPD" {
        st.dark = true;
    } else if cmd == "LPC" {
        st.dark = false;
    } else if let Some(r) = cmd.strip_prefix("ADD") {
        let split = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
        let code: u32 = r[..split].parse().map_err(|_| ParseError { line, msg: "bad %AD code".into() })?;
        let rest = &r[split..];
        let (kind, params) = rest.split_once(',').unwrap_or((rest, ""));
        let v: Vec<f64> = params.split('X').filter(|s| !s.is_empty()).map(|s| s.parse::<f64>().unwrap_or(0.0) * st.unit).collect();
        let g = |k: usize| v.get(k).copied().unwrap_or(0.0);
        let ap = match kind {
            "C" => Some(Aperture::Circle(g(0))),
            "R" => Some(Aperture::Rect(g(0), g(1))),
            "O" => Some(Aperture::Obround(g(0), g(1))),
            "P" => Some(Aperture::Polygon(g(0), (g(1) / st.unit) as u32, g(2) / st.unit)),
            _ => None, // aperture macro: unsupported
        };
        st.aps.retain(|(c, _)| *c != code);
        if let Some(ap) = ap {
            st.aps.push((code, ap));
        }
    } else if let Some(r) = cmd.strip_prefix("TF.FileFunction,") {
        out.function = Some(r.to_string());
    }
    Ok(())
}

/// Returns true at M02 (end of file).
fn word(cmd: &str, st: &mut St, out: &mut GerberLayer, line: u32) -> Result<bool, ParseError> {
    if cmd.starts_with("G04") || cmd.is_empty() {
        return Ok(false);
    }
    if cmd == "M02" || cmd == "M00" {
        return Ok(true);
    }
    // Split into (letter, number) words.
    let mut words: Vec<(char, String)> = Vec::new();
    for c in cmd.chars() {
        if c.is_ascii_alphabetic() {
            words.push((c, String::new()));
        } else if let Some(w) = words.last_mut() {
            w.1.push(c);
        } else {
            return err(line, format!("unexpected '{c}'"));
        }
    }
    let (mut x, mut y, mut ij, mut d) = (None, None, [0.0, 0.0], None);
    for (l, n) in &words {
        match l {
            'G' => match n.parse::<u32>().unwrap_or(999) {
                1 => st.interp = 1,
                2 => st.interp = 2,
                3 => st.interp = 3,
                36 => {
                    st.region = true;
                    st.contour.clear();
                }
                37 => {
                    flush_contour(st, out);
                    st.region = false;
                }
                74 => st.multi = false,
                75 => st.multi = true,
                _ => {}
            },
            'X' => x = Some(coord(n, st.dec.0, st.unit, line)?),
            'Y' => y = Some(-coord(n, st.dec.1, st.unit, line)?),
            'I' => ij[0] = coord(n, st.dec.0, st.unit, line)?,
            'J' => ij[1] = -coord(n, st.dec.1, st.unit, line)?,
            'D' => d = Some(n.parse::<u32>().map_err(|_| ParseError { line, msg: format!("bad D code '{n}'") })?),
            'M' | 'N' => {}
            other => return err(line, format!("unexpected word '{other}{n}'")),
        }
    }
    let target = [x.unwrap_or(st.pos[0]), y.unwrap_or(st.pos[1])];
    match d {
        Some(1) => {
            let pts = if st.interp == 1 { vec![st.pos, target] } else { arc(st, target, ij) };
            if st.region {
                if st.contour.is_empty() {
                    st.contour.push(st.pos);
                }
                st.contour.extend(pts.into_iter().skip(1));
            } else if let Some(ap) = st.ap {
                for p in pts.windows(2) {
                    out.prims.push(Prim::Line { a: p[0], b: p[1], w: ap.width(), dark: st.dark });
                }
            }
            st.pos = target;
        }
        Some(2) => {
            if st.region {
                flush_contour(st, out);
            }
            st.pos = target;
        }
        Some(3) => {
            st.pos = target;
            if let Some(ap) = st.ap {
                out.prims.push(Prim::Flash { at: target, ap, dark: st.dark });
            }
        }
        Some(code) if code >= 10 => st.ap = st.aps.iter().find(|(c, _)| *c == code).map(|(_, a)| *a),
        _ => {
            // Bare coordinates repeat the previous operation (deprecated); treat as a move.
            if x.is_some() || y.is_some() {
                st.pos = target;
            }
        }
    }
    Ok(false)
}

fn flush_contour(st: &mut St, out: &mut GerberLayer) {
    if st.contour.len() >= 3 {
        out.prims.push(Prim::Region { poly: std::mem::take(&mut st.contour), dark: st.dark });
    }
    st.contour.clear();
}

/// Arc points from st.pos to `to` with centre offset `ij` (y already flipped).
fn arc(st: &St, to: P2, ij: P2) -> Vec<P2> {
    let a = st.pos;
    // Y is flipped, so Gerber CW (G02) becomes counter-clockwise in our frame.
    let ccw = st.interp == 2;
    let centre = if st.multi {
        add(a, ij)
    } else {
        // Single quadrant: pick the sign combination giving the best radius match within 90°.
        let mut best = (f64::INFINITY, add(a, ij));
        for sx in [1.0, -1.0] {
            for sy in [1.0, -1.0] {
                let c = add(a, [ij[0] * sx, ij[1] * sy]);
                let sw = sweep(a, to, c, ccw);
                let e = (dist(a, c) - dist(to, c)).abs();
                if sw.abs() <= std::f64::consts::FRAC_PI_2 + 1e-6 && e < best.0 {
                    best = (e, c);
                }
            }
        }
        best.1
    };
    let r = dist(a, centre);
    let t0 = (a[1] - centre[1]).atan2(a[0] - centre[0]);
    let mut sw = sweep(a, to, centre, ccw);
    if st.multi && sw.abs() < 1e-9 && dist(a, to) < 1e-9 {
        sw = if ccw { std::f64::consts::TAU } else { -std::f64::consts::TAU };
    }
    let mut pts = arc_by_sweep(centre, r, t0, sw);
    if let Some(l) = pts.last_mut() {
        *l = to;
    }
    pts
}

fn sweep(a: P2, b: P2, c: P2, ccw: bool) -> f64 {
    let t0 = (a[1] - c[1]).atan2(a[0] - c[0]);
    let t1 = (b[1] - c[1]).atan2(b[0] - c[0]);
    let tau = std::f64::consts::TAU;
    if ccw {
        (t1 - t0).rem_euclid(tau)
    } else {
        -(t0 - t1).rem_euclid(tau)
    }
}

/// Rasterise a layer in command order (dark sets, clear erases).
pub fn rasterize(l: &GerberLayer, bbox: [f64; 4], res: f64) -> Raster {
    let mut r = Raster::new(bbox, res);
    for p in &l.prims {
        match p {
            Prim::Line { a, b, w, dark } => r.fill_capsule(*a, *b, w / 2.0, *dark),
            _ => r.fill_poly(&p.polygon(), p.dark()),
        }
    }
    r
}

// ---- Excellon ----

/// Drill hits (position mm, y down; diameter mm).
pub fn parse_drill(src: &str) -> Result<Vec<(P2, f64)>, ParseError> {
    let mut unit = 1.0;
    let mut int_dec = 3;
    let mut tools: Vec<(u32, f64)> = Vec::new();
    let mut cur = 0.0;
    let mut out = Vec::new();
    let mut pos = [0.0, 0.0];
    let mut header_seen = false;
    for (ln, raw) in src.lines().enumerate() {
        let line = ln as u32 + 1;
        let l = raw.trim();
        if l.is_empty() || l.starts_with(';') {
            continue;
        }
        if l == "M48" {
            header_seen = true;
            continue;
        }
        if l.starts_with("INCH") {
            unit = 25.4;
            int_dec = 4;
            continue;
        }
        if l.starts_with("METRIC") {
            unit = 1.0;
            int_dec = 3;
            continue;
        }
        if l == "M30" || l == "M00" {
            break;
        }
        if let Some(r) = l.strip_prefix('T') {
            let split = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
            let code: u32 = r[..split].parse().unwrap_or(0);
            if let Some(c) = r[split..].find('C') {
                let rest = &r[split + c + 1..];
                let end = rest.find(|ch: char| !(ch.is_ascii_digit() || ch == '.')).unwrap_or(rest.len());
                let d: f64 = rest[..end].parse().map_err(|_| ParseError { line, msg: "bad tool diameter".into() })?;
                tools.push((code, d * unit));
            } else {
                cur = tools.iter().find(|(k, _)| *k == code).map(|(_, d)| *d).unwrap_or(0.0);
            }
            continue;
        }
        if l.starts_with('X') || l.starts_with('Y') {
            let mut chars = l.char_indices().peekable();
            while let Some((i, c)) = chars.next() {
                if c == 'X' || c == 'Y' {
                    let st = i + 1;
                    let mut end = st;
                    while let Some(&(j, ch)) = chars.peek() {
                        if ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+' {
                            end = j + 1;
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let v = coord(&l[st..end], int_dec, unit, line)?;
                    if c == 'X' {
                        pos[0] = v;
                    } else {
                        pos[1] = -v;
                    }
                }
            }
            out.push((pos, cur));
        }
    }
    if !header_seen && out.is_empty() {
        return err(1, "not an Excellon drill file");
    }
    Ok(out)
}

// ---- Layer guessing ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayerGuess {
    /// Copper, ordered: 0 = top, 1..=n inner, 1000 = bottom.
    Copper(u32),
    Silk,
    Mask,
    Paste,
    Outline,
    Drill,
    Unknown,
}

/// From the X2 file function first, then the file name.
pub fn guess(name: &str, function: Option<&str>) -> LayerGuess {
    if let Some(f) = function {
        let parts: Vec<&str> = f.split(',').collect();
        match parts.first().copied() {
            Some("Copper") => {
                let side = parts.get(2).copied().unwrap_or("");
                let n: u32 = parts.get(1).and_then(|l| l.trim_start_matches('L').parse().ok()).unwrap_or(1);
                return match side {
                    "Top" => LayerGuess::Copper(0),
                    "Bot" => LayerGuess::Copper(1000),
                    _ => LayerGuess::Copper(n.saturating_sub(1).max(1)),
                };
            }
            Some("Legend") => return LayerGuess::Silk,
            Some("Soldermask") => return LayerGuess::Mask,
            Some("Paste") => return LayerGuess::Paste,
            Some("Profile") => return LayerGuess::Outline,
            _ => {}
        }
    }
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    let ext = base.rsplit('.').next().unwrap_or("");
    if base.contains("f_cu") || base.contains("f.cu") || ext == "gtl" || ext == "cmp" {
        return LayerGuess::Copper(0);
    }
    if base.contains("b_cu") || base.contains("b.cu") || ext == "gbl" || ext == "sol" {
        return LayerGuess::Copper(1000);
    }
    for k in 1..=30u32 {
        if base.contains(&format!("in{k}_cu")) || base.contains(&format!("in{k}.cu")) || ext == format!("g{}", k + 1) && k < 30 || ext == format!("gp{k}") || ext == format!("g{k}l")
        {
            return LayerGuess::Copper(k);
        }
    }
    match ext {
        "gto" | "gbo" => LayerGuess::Silk,
        "gts" | "gbs" => LayerGuess::Mask,
        "gtp" | "gbp" => LayerGuess::Paste,
        "gko" | "gm1" | "gml" => LayerGuess::Outline,
        "drl" | "xln" | "exc" | "drd" => LayerGuess::Drill,
        _ if base.contains("edge_cuts") => LayerGuess::Outline,
        _ if base.contains("silk") => LayerGuess::Silk,
        _ if base.contains("mask") => LayerGuess::Mask,
        _ if base.contains("paste") => LayerGuess::Paste,
        _ => LayerGuess::Unknown,
    }
}

// ---- Connectivity → Board ----

/// Parsed input files with (possibly user-overridden) layer guesses.
pub struct Files {
    pub layers: Vec<(String, LayerGuess, GerberLayer)>,
    pub drills: Vec<(P2, f64)>,
}

struct Dsu(Vec<u32>);
impl Dsu {
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let p = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = p;
            x = p;
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b) as usize] = a.min(b);
        }
    }
}

fn point_in_poly(p: P2, poly: &[P2]) -> bool {
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

fn polys_touch(a: &[P2], b: &[P2]) -> bool {
    if a.iter().any(|p| point_in_poly(*p, b)) || b.iter().any(|p| point_in_poly(*p, a)) {
        return true;
    }
    let (na, nb) = (a.len(), b.len());
    for i in 0..na {
        for j in 0..nb {
            if seg_seg(a[i], a[(i + 1) % na], b[j], b[(j + 1) % nb]) == 0.0 {
                return true;
            }
        }
    }
    false
}

fn touch(p: &Prim, q: &Prim, pp: &[P2], qp: &[P2]) -> bool {
    match (p, q) {
        (Prim::Line { a, b, w, .. }, Prim::Line { a: c, b: d, w: w2, .. }) => seg_seg(*a, *b, *c, *d) <= (w + w2) / 2.0,
        _ => polys_touch(pp, qp),
    }
}

/// Build a board whose nets are copper islands ("ISL<n>"). Returns the board plus, for every
/// (copper layer, primitive index) the net id, so a click can be mapped back to an island.
pub fn build(files: &Files, thickness: f64) -> (Board, Vec<Vec<usize>>) {
    let mut copper: Vec<&(String, LayerGuess, GerberLayer)> = files.layers.iter().filter(|l| matches!(l.1, LayerGuess::Copper(_))).collect();
    copper.sort_by_key(|l| l.1);
    let mut b = Board { nets: vec![String::new()], layers: even_layers(copper.len().max(1), thickness), thickness, ..Board::default() };
    b.eps_r = vec![4.5; b.layers.len().saturating_sub(1)];
    for (k, l) in copper.iter().enumerate() {
        b.notes.push(format!("{} → {}", l.0, b.layers[k].name));
    }
    // Global primitive ids over dark primitives of all copper layers.
    let mut ids: Vec<(usize, usize)> = Vec::new();
    let mut polys: Vec<Vec<P2>> = Vec::new();
    let mut bbs: Vec<[f64; 4]> = Vec::new();
    for (li, l) in copper.iter().enumerate() {
        for (pi, p) in l.2.prims.iter().enumerate() {
            if p.dark() {
                ids.push((li, pi));
                let poly = p.polygon();
                let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
                for q in &poly {
                    bb = [bb[0].min(q[0]), bb[1].min(q[1]), bb[2].max(q[0]), bb[3].max(q[1])];
                }
                polys.push(poly);
                bbs.push(bb);
            }
        }
    }
    let mut dsu = Dsu((0..ids.len() as u32).collect());
    // Same-layer overlaps via a grid on bbox centres.
    let segs: Vec<crate::grid::S> = bbs
        .iter()
        .map(|bb| crate::grid::S { a: [bb[0], bb[1]], b: [bb[2], bb[3]], hw: 0.0 })
        .collect();
    let grid = crate::grid::Grid::new(&segs, 2.0);
    let mut tmp = Vec::new();
    for i in 0..ids.len() {
        let bb = bbs[i];
        let q = crate::grid::S { a: [bb[0], bb[1]], b: [bb[2], bb[3]], hw: 0.0 };
        grid.candidates(&q, 0.0, &mut tmp);
        for &j in &tmp {
            let j = j as usize;
            if j <= i || ids[j].0 != ids[i].0 {
                continue;
            }
            let o = bbs[j];
            if o[0] > bb[2] || o[2] < bb[0] || o[1] > bb[3] || o[3] < bb[1] {
                continue;
            }
            let (pi, pj) = (&copper[ids[i].0].2.prims[ids[i].1], &copper[ids[j].0].2.prims[ids[j].1]);
            if touch(pi, pj, &polys[i], &polys[j]) {
                dsu.union(i as u32, j as u32);
            }
        }
    }
    // Drill holes join layers.
    for (at, d) in &files.drills {
        let mut first: Option<u32> = None;
        let probe = crate::grid::S { a: *at, b: *at, hw: d / 2.0 };
        grid.candidates(&probe, 0.0, &mut tmp);
        for &j in &tmp {
            if point_in_poly(*at, &polys[j as usize]) || polys[j as usize].iter().any(|p| dist(*p, *at) <= d / 2.0) {
                match first {
                    None => first = Some(j),
                    Some(f) => dsu.union(f, j),
                }
            }
        }
        if first.is_some() {
            b.vias.push(Via { at: *at, size: d * 1.6, drill: *d, span: (0, b.layers.len() - 1), net: 0 });
        }
    }
    // Island nets.
    let mut root_net: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut map: Vec<Vec<usize>> = copper.iter().map(|l| vec![0; l.2.prims.len()]).collect();
    for (k, &(li, pi)) in ids.iter().enumerate() {
        let r = dsu.find(k as u32);
        let n = *root_net.entry(r).or_insert_with(|| {
            b.nets.push(format!("ISL{}", b.nets.len()));
            b.nets.len() - 1
        });
        map[li][pi] = n;
        let p = &copper[li].2.prims[pi];
        match p {
            Prim::Line { a, b: e, w, .. } => b.segs.push(Seg { a: *a, b: *e, w: *w, layer: li, net: n }),
            Prim::Flash { at, ap, .. } => {
                let (size, shape) = match *ap {
                    Aperture::Circle(d) | Aperture::Polygon(d, _, _) => ([d, d], PadShape::Circle),
                    Aperture::Rect(w, h) => ([w, h], PadShape::Rect),
                    Aperture::Obround(w, h) => ([w, h], PadShape::Oval),
                };
                b.pads.push(Pad { at: *at, size, rot: 0.0, shape, layers: vec![li], net: n, footprint: usize::MAX, number: String::new() })
            }
            Prim::Region { poly, .. } => b.zones.push(Zone { layer: li, net: n, polys: vec![poly.clone()], clear: false }),
        }
    }
    for v in b.vias.iter_mut() {
        if let Some(p) = b.pads.iter().find(|p| dist(p.at, v.at) < v.drill) {
            v.net = p.net;
        }
    }
    // Clear-polarity primitives cut the planes.
    for (li, l) in copper.iter().enumerate() {
        for p in l.2.prims.iter().filter(|p| !p.dark()) {
            b.zones.push(Zone { layer: li, net: 0, polys: vec![p.polygon()], clear: true });
        }
    }
    (b, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "%FSLAX36Y36*%\n%MOMM*%\n%TF.FileFunction,Copper,L1,Top*%\n%ADD10C,0.500000*%\n%ADD11R,2.000000X1.000000*%\n";

    #[test]
    fn arc_in_g75_mode_is_a_half_circle() {
        // From (10,0) to (-10,0) counter-clockwise about the origin (Gerber frame).
        let src = format!("{HEAD}G75*\nD10*\nX10000000Y0D02*\nG03X-10000000Y0I-10000000J0D01*\nM02*\n");
        let l = parse(&src).unwrap();
        assert!(l.prims.len() > 8);
        for p in &l.prims {
            let Prim::Line { a, b, w, .. } = p else { panic!() };
            assert!((len(*a) - 10.0).abs() < 1e-6 && (len(*b) - 10.0).abs() < 1e-6);
            assert_eq!(*w, 0.5);
            // Gerber +y is up; with y flipped the CCW top half ends at negative y.
            assert!(a[1] <= 1e-9 && b[1] <= 1e-9);
        }
        assert_eq!(l.function.as_deref(), Some("Copper,L1,Top"));
        assert_eq!(guess("x.gbr", l.function.as_deref()), LayerGuess::Copper(0));
    }

    #[test]
    fn region_fill_and_lp_clear() {
        let src = format!(
            "{HEAD}G36*\nX0Y0D02*\nG01X10000000Y0D01*\nX10000000Y10000000D01*\nX0Y10000000D01*\nX0Y0D01*\nG37*\n%LPC*%\nD11*\nX5000000Y5000000D03*\n%LPD*%\nM02*\n"
        );
        let l = parse(&src).unwrap();
        assert!(matches!(l.prims[0], Prim::Region { dark: true, .. }));
        assert!(matches!(l.prims[1], Prim::Flash { dark: false, .. }));
        let r = rasterize(&l, [-1.0, -11.0, 11.0, 1.0], 0.1);
        assert!(r.at([2.0, -2.0]), "region filled");
        assert!(!r.at([5.0, -5.0]), "cleared by the LPC flash");
        assert!(r.at([5.0, -6.0]), "flash is 1 mm tall: 0.5 mm below the centre is copper");
        assert!(!r.at([12.0, -5.0]));
    }

    #[test]
    fn islands_join_by_overlap_and_drills() {
        let top = format!("{HEAD}D10*\nX0Y0D02*\nX5000000Y0D01*\nX5000000Y0D02*\nX5000000Y5000000D01*\nX20000000Y0D02*\nX25000000Y0D01*\nM02*\n");
        let bot = format!("%FSLAX36Y36*%\n%MOMM*%\n%ADD10C,0.500000*%\nD10*\nX25000000Y0D02*\nX30000000Y0D01*\nM02*\n");
        let files = Files {
            layers: vec![("t.gtl".into(), LayerGuess::Copper(0), parse(&top).unwrap()), ("b.gbl".into(), LayerGuess::Copper(1000), parse(&bot).unwrap())],
            drills: vec![([25.0, 0.0], 0.3)],
        };
        let (b, map) = build(&files, 1.6);
        assert_eq!(map[0][0], map[0][1], "touching lines share an island");
        assert_ne!(map[0][0], map[0][2], "separate line is another island");
        assert_eq!(map[0][2], map[1][0], "drill joins top and bottom");
        assert_eq!(b.layers.len(), 2);
        assert_eq!(b.segs.len(), 4);
    }

    #[test]
    fn excellon_and_names() {
        let d = parse_drill("M48\nMETRIC\nT1C0.300\n%\nT1\nX1.5Y-2.0\nX003000Y-004000\nM30\n").unwrap();
        assert_eq!(d, vec![([1.5, 2.0], 0.3), ([3.0, 4.0], 0.3)]);
        assert_eq!(guess("board-In2_Cu.gbr", None), LayerGuess::Copper(2));
        assert_eq!(guess("BOARD.GBL", None), LayerGuess::Copper(1000));
        assert_eq!(guess("board.drl", None), LayerGuess::Drill);
        assert_eq!(guess("board.gto", None), LayerGuess::Silk);
    }
}
