//! KiCad 6–8 `.kicad_pcb` reader (S-expression). Unknown tokens are ignored; malformed known
//! items give a `ParseError` with a line number. Never panics.
use crate::board::*;
use crate::sexpr::{self, err, Node, ParseError};
use std::collections::HashMap;

fn copper_rank(name: &str) -> Option<u32> {
    match name {
        "F.Cu" => Some(0),
        "B.Cu" => Some(1000),
        _ => name.strip_prefix("In").and_then(|r| r.strip_suffix(".Cu")).and_then(|k| k.parse().ok()),
    }
}

pub fn parse(src: &str) -> Result<Board, ParseError> {
    let root = sexpr::parse(src)?;
    if root.head() != Some("kicad_pcb") {
        return err(root.line(), "not a kicad_pcb file (missing '(kicad_pcb')");
    }
    let mut b = Board { nets: vec![String::new()], ..Board::default() };

    // Copper layers, ordered top → bottom.
    let mut names: Vec<String> = root
        .child("layers")
        .map(|l| l.items().iter().skip(1).filter_map(|e| e.arg_str(1)).filter(|n| copper_rank(n).is_some()).map(String::from).collect())
        .unwrap_or_default();
    if names.is_empty() {
        names = vec!["F.Cu".into(), "B.Cu".into()];
    }
    names.sort_by_key(|n| copper_rank(n).unwrap_or(999));
    names.dedup();
    let thickness = match root.child("general").and_then(|g| g.child("thickness")) {
        Some(t) => t.num(1)?,
        None => 1.6,
    };
    b.thickness = thickness;
    b.layers = even_layers(names.len(), thickness);
    for (l, n) in b.layers.iter_mut().zip(&names) {
        l.name = n.clone();
    }
    b.eps_r = vec![4.5; names.len().saturating_sub(1)];
    if let Some(st) = root.child("setup").and_then(|s| s.child("stackup")) {
        read_stackup(st, &mut b)?;
    }
    let layer_of: HashMap<String, usize> = b.layers.iter().enumerate().map(|(i, l)| (l.name.clone(), i)).collect();
    let all: Vec<usize> = (0..b.layers.len()).collect();
    let layer_list = |n: &Node| -> Vec<usize> {
        let mut v = Vec::new();
        for a in n.items().iter().skip(1).filter_map(Node::atom) {
            if a == "*.Cu" || a == "F&B.Cu" {
                return all.clone();
            }
            if let Some(&k) = layer_of.get(a) {
                v.push(k);
            }
        }
        v
    };

    // Nets: file id → index.
    let mut net_map: HashMap<i64, usize> = HashMap::new();
    for n in root.children("net") {
        let id = n.num(1)? as i64;
        let name = n.arg_str(2).unwrap_or("").to_string();
        if id == 0 {
            net_map.insert(0, 0);
            continue;
        }
        net_map.insert(id, b.nets.len());
        b.nets.push(name);
    }
    let net_of = |n: Option<&Node>, nets: &mut Vec<String>| -> usize {
        let Some(n) = n else { return 0 };
        match n.arg_str(1) {
            Some(a) => match a.parse::<i64>() {
                Ok(id) => *net_map.get(&id).unwrap_or(&0),
                Err(_) => match nets.iter().position(|x| x == a) {
                    Some(k) => k,
                    None => {
                        nets.push(a.to_string());
                        nets.len() - 1
                    }
                },
            },
            None => 0,
        }
    };

    for item in root.items().iter().skip(1) {
        match item.head() {
            Some("segment") | Some("arc") => {
                let get = |k: &str| item.child(k).ok_or(ParseError { line: item.line(), msg: format!("track without '{k}'") });
                let a = xy(get("start")?)?;
                let e = xy(get("end")?)?;
                let w = get("width")?.num(1)?;
                let Some(&layer) = item.child("layer").and_then(|l| l.arg_str(1)).and_then(|n| layer_of.get(n)) else { continue };
                let net = net_of(item.child("net"), &mut b.nets);
                let pts = if item.head() == Some("arc") { arc_points(a, xy(get("mid")?)?, e) } else { vec![a, e] };
                for p in pts.windows(2) {
                    b.segs.push(Seg { a: p[0], b: p[1], w, layer, net });
                }
            }
            Some("via") => {
                let at = xy(item.child("at").ok_or(ParseError { line: item.line(), msg: "via without 'at'".into() })?)?;
                let size = item.child("size").map(|s| s.num(1)).transpose()?.unwrap_or(0.6);
                let drill = item.child("drill").map(|s| s.num(1)).transpose()?.unwrap_or(0.3);
                let ls = item.child("layers").map(&layer_list).unwrap_or_default();
                let span = match (ls.iter().min(), ls.iter().max()) {
                    (Some(&a), Some(&z)) => (a, z),
                    _ => (0, b.layers.len().saturating_sub(1)),
                };
                let net = net_of(item.child("net"), &mut b.nets);
                b.vias.push(Via { at, size, drill, span, net });
            }
            Some("footprint") | Some("module") => {
                let fa = item.child("at").map(|a| a.nums()).transpose()?.unwrap_or_default();
                let at = [fa.first().copied().unwrap_or(0.0), fa.get(1).copied().unwrap_or(0.0)];
                let frot = fa.get(2).copied().unwrap_or(0.0);
                let reference = item
                    .children("property")
                    .find(|p| p.arg_str(1) == Some("Reference"))
                    .and_then(|p| p.arg_str(2))
                    .or_else(|| item.children("fp_text").find(|t| t.arg_str(1) == Some("reference")).and_then(|t| t.arg_str(2)))
                    .unwrap_or("")
                    .to_string();
                let bottom = item.child("layer").and_then(|l| l.arg_str(1)) == Some("B.Cu");
                let fi = b.footprints.len();
                b.footprints.push(Footprint { reference, name: item.arg_str(1).unwrap_or("").into(), at, rot: frot, bottom });
                for pad in item.children("pad") {
                    let pa = pad.child("at").map(|a| a.nums()).transpose()?.unwrap_or_default();
                    let local = [pa.first().copied().unwrap_or(0.0), pa.get(1).copied().unwrap_or(0.0)];
                    let rot_abs = pa.get(2).copied().unwrap_or(frot);
                    let sz = pad.child("size").map(|s| s.nums()).transpose()?.unwrap_or_default();
                    let size = [sz.first().copied().unwrap_or(0.5), sz.get(1).copied().unwrap_or(sz.first().copied().unwrap_or(0.5))];
                    let shape = match pad.arg_str(3) {
                        Some("circle") => PadShape::Circle,
                        Some("oval") => PadShape::Oval,
                        _ => PadShape::Rect,
                    };
                    let layers = pad.child("layers").map(&layer_list).unwrap_or_default();
                    let net = net_of(pad.child("net"), &mut b.nets);
                    b.pads.push(Pad {
                        at: add(at, rot(local, frot)),
                        size,
                        rot: rot_abs,
                        shape,
                        layers,
                        net,
                        footprint: fi,
                        number: pad.arg_str(1).unwrap_or("").into(),
                    });
                }
            }
            Some("zone") => {
                let net = net_of(item.child("net"), &mut b.nets);
                for fp in item.children("filled_polygon") {
                    let Some(&layer) = fp.child("layer").and_then(|l| l.arg_str(1)).and_then(|n| layer_of.get(n)) else { continue };
                    let Some(pts) = fp.child("pts") else { continue };
                    let poly: Vec<P2> = pts.children("xy").map(xy).collect::<Result<_, _>>()?;
                    if poly.len() < 3 {
                        continue;
                    }
                    match b.zones.iter_mut().find(|z| z.layer == layer && z.net == net) {
                        Some(z) => z.polys.push(poly),
                        None => b.zones.push(Zone { layer, net, polys: vec![poly], clear: false }),
                    }
                }
            }
            _ => {}
        }
    }
    Ok(b)
}

fn xy(n: &Node) -> Result<P2, ParseError> {
    Ok([n.num(1)?, n.num(2)?])
}

fn read_stackup(st: &Node, b: &mut Board) -> Result<(), ParseError> {
    // Walk the stack top → bottom; z of each copper centre, εr of the dielectric below it.
    let mut z = 0.0;
    let mut k = 0usize;
    let mut eps: Vec<(f64, f64)> = Vec::new(); // (thickness, εr) between copper k and k+1
    let mut seen_copper = false;
    for l in st.children("layer") {
        let name = l.arg_str(1).unwrap_or("");
        let t = l.child("thickness").map(|t| t.num(1)).transpose()?.unwrap_or(0.0);
        let kind = l.child("type").and_then(|t| t.arg_str(1)).unwrap_or("");
        if copper_rank(name).is_some() || kind == "copper" {
            if seen_copper {
                k += 1;
            }
            if let Some(cl) = b.layers.get_mut(k) {
                cl.z = z + t / 2.0;
                cl.thickness = t.max(0.005);
            }
            seen_copper = true;
            z += t;
        } else if seen_copper && (kind == "core" || kind == "prepreg" || name.starts_with("dielectric")) {
            let e = l.child("epsilon_r").map(|e| e.num(1)).transpose()?.unwrap_or(4.5);
            while eps.len() <= k {
                eps.push((0.0, 0.0));
            }
            eps[k].0 += t;
            eps[k].1 += t * e;
            z += t;
        }
    }
    for (i, (t, te)) in eps.into_iter().enumerate() {
        if let Some(slot) = b.eps_r.get_mut(i) {
            if t > 0.0 {
                *slot = te / t;
            }
        }
    }
    if z > 0.0 {
        b.thickness = z;
    }
    Ok(())
}
