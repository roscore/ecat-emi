//! Model, parser-robustness and scale tests on small synthetic boards.
use ee_core::board::*;
use ee_core::couple::{self, k_coef, mode_conversion, Edits, Params, RunKind};
use ee_core::num::Rng;
use ee_core::roles::{AggrKind, AggrRole, PairRole, Roles, Side};
use ee_core::{gerber, kicad, sample, Input};

fn board(segs: Vec<Seg>, nets: &[&str]) -> Board {
    let mut b = Board {
        nets: std::iter::once("").chain(nets.iter().copied()).map(String::from).collect(),
        layers: even_layers(2, 1.6),
        eps_r: vec![4.5],
        thickness: 1.6,
        segs,
        ..Board::default()
    };
    // Solid GND plane on the bottom layer.
    b.zones.push(Zone { layer: 1, net: 0, polys: vec![vec![[-20.0, -20.0], [120.0, -20.0], [120.0, 120.0], [-20.0, 120.0]]], clear: false });
    b
}

fn seg(a: P2, b: P2, net: usize) -> Seg {
    Seg { a, b, w: 0.2, layer: 0, net }
}

fn roles_for(p: &Params) -> Roles {
    Roles {
        pairs: vec![PairRole { name: "ETH_RX".into(), p: 1, n: 2, side: Side::Phy, enabled: true }],
        aggressors: vec![AggrRole::preset(3, AggrKind::SwitchNode, &p.inv)],
    }
}

fn pair_with_aggressor(gap: f64) -> Board {
    board(
        vec![
            seg([0.0, 0.0], [40.0, 0.0], 1),
            seg([0.0, 0.5], [40.0, 0.5], 2),
            Seg { a: [0.0, -gap - 0.2], b: [40.0, -gap - 0.2], w: 0.2, layer: 0, net: 3 },
        ],
        &["ETH_RX_P", "ETH_RX_N", "SW"],
    )
}

#[test]
fn doubling_spacing_lowers_k() {
    for h in [0.1, 0.235, 1.0] {
        for s in [0.2, 1.5, 4.0] {
            assert!(k_coef(2.0 * s, h) < k_coef(s, h));
        }
    }
    assert!((k_coef(1.0, 1.0) - 0.5).abs() < 1e-12);
    // And end to end: the run noise falls when the aggressor moves away.
    let p = Params::default();
    let peak = |g: f64| {
        let prep = couple::prepare(pair_with_aggressor(g));
        couple::analyze(&prep, &roles_for(&p), &p, &Edits::default()).pairs[0].link.noise_peak
    };
    assert!(peak(2.0) < peak(1.0) && peak(4.0) < peak(2.0));
}

#[test]
fn pair_imbalance_raises_mode_conversion() {
    let base = mode_conversion(-40.0, 0.0, 31.25e6);
    assert!((base - 0.01).abs() < 1e-12);
    assert!(mode_conversion(-40.0, 5e-12, 31.25e6) > base);
    assert!(mode_conversion(-40.0, 20e-12, 31.25e6) > mode_conversion(-40.0, 5e-12, 31.25e6));
    // P closer than N → differential pickup; a symmetric aggressor (equidistant) gives none.
    let p = Params::default();
    let prep = couple::prepare(pair_with_aggressor(1.0));
    let an = couple::analyze(&prep, &roles_for(&p), &p, &Edits::default());
    let run = an.runs.iter().find(|r| r.kind == RunKind::Parallel).unwrap();
    assert!(run.s_n > run.s_p);
    assert!(an.pairs[0].path_energy[0] > 0.0);
}

#[test]
fn no_false_run_between_perpendicular_traces() {
    let b = board(
        vec![seg([0.0, 0.0], [40.0, 0.0], 1), seg([0.0, 0.5], [40.0, 0.5], 2), seg([20.0, -10.0], [20.0, 10.0], 3)],
        &["ETH_RX_P", "ETH_RX_N", "SW"],
    );
    let p = Params::default();
    let an = couple::analyze(&couple::prepare(b), &roles_for(&p), &p, &Edits::default());
    assert!(an.runs.iter().all(|r| r.kind != RunKind::Parallel));
    assert!(an.gaps.is_empty(), "solid plane: no gap");
}

#[test]
fn hot_loop_area_of_known_triangle() {
    let mut b = board(vec![], &["VBUS", "GND", "SW"]);
    let pad = |at: P2, net: usize, fp: usize| Pad { at, size: [1.0, 1.0], rot: 0.0, shape: PadShape::Rect, layers: vec![0], net, footprint: fp, number: "1".into() };
    for (r, at) in [("Q1", [0.0, 0.0]), ("Q2", [10.0, 0.0]), ("C1", [0.0, 10.0])] {
        b.footprints.push(Footprint { reference: r.into(), name: String::new(), at, rot: 0.0, bottom: false });
    }
    b.pads = vec![pad([0.0, 0.0], 1, 0), pad([0.0, 3.0], 3, 0), pad([10.0, 0.0], 2, 1), pad([10.0, 3.0], 3, 1), pad([0.0, 10.0], 1, 2), pad([0.0, 10.0], 2, 2)];
    let hl = couple::find_hot_loop(&b).expect("hot loop");
    assert!((hl.area_mm2 - 50.0).abs() < 1e-9, "{}", hl.area_mm2);
}

#[test]
fn malformed_kicad_errors_never_panics() {
    let src = sample::kicad_text();
    assert!(kicad::parse("(kicad_pcb (segment (start 1 x) (end 2 3) (width 1) (layer \"F.Cu\")))").unwrap_err().line == 1);
    let e = kicad::parse("(kicad_pcb\n (net 1 \"A\")\n (segment (start 1 2) (end 2 3) (width w) (layer \"F.Cu\") (net 1)))").unwrap_err();
    assert_eq!(e.line, 3);
    assert!(kicad::parse("(not_a_board)").is_err());
    let step = (src.len() / 300).max(1);
    for cut in (0..src.len()).step_by(step) {
        let _ = kicad::parse(&src[..cut]);
    }
    let mut rng = Rng(42);
    let bytes = src.as_bytes();
    for _ in 0..200 {
        let mut m = bytes.to_vec();
        for _ in 0..8 {
            let i = (rng.f64() * m.len() as f64) as usize % m.len();
            m[i] = b"()\" x0.-\n"[(rng.f64() * 9.0) as usize % 9];
        }
        let _ = kicad::parse(&String::from_utf8_lossy(&m));
    }
}

const GTL: &str = "%FSLAX36Y36*%\n%MOMM*%\n%TF.FileFunction,Copper,L1,Top*%\n%ADD10C,0.200000*%\n%ADD11C,2.000000*%\nD10*\nX0Y0D02*\nX40000000Y0D01*\nX0Y-500000D02*\nX40000000Y-500000D01*\nD11*\nX0Y2200000D02*\nX40000000Y2200000D01*\nM02*\n";
const GBL: &str = "%FSLAX36Y36*%\n%MOMM*%\n%TF.FileFunction,Copper,L2,Bot*%\nG36*\nX-10000000Y10000000D02*\nG01X50000000Y10000000D01*\nX50000000Y-10000000D01*\nX-10000000Y-10000000D01*\nX-10000000Y10000000D01*\nG37*\nM02*\n";

#[test]
fn malformed_gerber_and_zip_never_panic() {
    for cut in 0..GTL.len() {
        let _ = gerber::parse(&GTL[..cut]);
    }
    assert!(gerber::parse("%FSLAX36Y36*%\nX12abcY0D01*\n").is_err());
    assert!(gerber::parse("%unterminated").is_err());
    assert!(ee_core::open(vec![("x.zip".into(), b"PK\x05\x06".to_vec())]).is_err());
}

#[test]
fn zip_of_gerbers_loads_and_couples() {
    let z = zip(&[("board-F_Cu.gtl", GTL.as_bytes()), ("board-B_Cu.gbl", GBL.as_bytes()), ("board.drl", b"M48\nMETRIC\nT1C0.3\n%\nT1\nX60.0Y0.0\nM30\n")]);
    let Ok(Input::Gerber(files)) = ee_core::open(vec![("gerbers.zip".into(), z)]) else { panic!("gerber set") };
    assert_eq!(files.layers.len(), 2);
    assert_eq!(files.drills.len(), 1);
    let (b, map) = gerber::build(&files, 1.6);
    assert_eq!(b.layers.len(), 2);
    // Tag islands like a user would: P, N, aggressor.
    let (p_net, n_net, a_net) = (map[0][0], map[0][1], map[0][2]);
    assert!(p_net != n_net && n_net != a_net);
    let prep = couple::prepare(b);
    assert_eq!(prep.planes.len(), 1, "bottom region is a plane");
    let p = Params::default();
    let roles = Roles {
        pairs: vec![PairRole { name: "TX".into(), p: p_net, n: n_net, side: Side::Phy, enabled: true }],
        aggressors: vec![AggrRole::preset(a_net, AggrKind::SwitchNode, &p.inv)],
    };
    let an = couple::analyze(&prep, &roles, &p, &Edits::default());
    let run = an.runs.iter().find(|r| r.kind == RunKind::Parallel).expect("run");
    assert!((run.len - 40.0).abs() < 0.5);
    assert!((run.s_p - 1.1).abs() < 0.05, "{}", run.s_p);
}

#[test]
fn twenty_thousand_segments_scale() {
    let mut rng = Rng(7);
    let mut segs = Vec::new();
    for _ in 0..20_000 {
        let a = [rng.range(0.0, 300.0), rng.range(0.0, 300.0)];
        let (l, t) = (rng.range(0.5, 6.0), (rng.f64() * 4.0).floor() * std::f64::consts::FRAC_PI_4);
        let net = 1 + (rng.f64() * 40.0) as usize % 40;
        segs.push(Seg { a, b: [a[0] + l * t.cos(), a[1] + l * t.sin()], w: 0.2, layer: (rng.f64() * 2.0) as usize % 2, net });
    }
    let names: Vec<String> = (1..=40).map(|k| if k <= 8 { format!("ETH{}_{}", (k + 1) / 2, if k % 2 == 1 { "P" } else { "N" }) } else { format!("SW{k}") }).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let b = board(segs, &refs);
    let t0 = std::time::Instant::now();
    let p = Params::default();
    let prep = couple::prepare(b);
    let roles = ee_core::roles::suggest(&prep.board, &p.inv);
    assert_eq!(roles.pairs.len(), 4);
    let an = couple::analyze(&prep, &roles, &p, &Edits::default());
    let dt = t0.elapsed().as_secs_f64();
    assert!(!an.runs.is_empty());
    eprintln!("20k segments: {dt:.3} s, {} runs", an.runs.len());
    if !cfg!(debug_assertions) {
        assert!(dt < 2.0, "{dt}");
    }
}

/// Small zip writer (deflate) for the test above.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, data) in files {
        let comp = miniz_oxide::deflate::compress_to_vec(data, 6);
        let off = out.len() as u32;
        for central in [false, true] {
            let buf = if central { &mut cd } else { &mut out };
            buf.extend_from_slice(if central { b"PK\x01\x02" } else { b"PK\x03\x04" });
            if central {
                buf.extend_from_slice(&20u16.to_le_bytes());
            }
            for v in [20u16, 0, 8, 0, 0] {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf.extend_from_slice(&0u32.to_le_bytes());
            buf.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes());
            if central {
                buf.extend_from_slice(&[0; 6]);
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf.extend_from_slice(&off.to_le_bytes());
            }
            buf.extend_from_slice(name.as_bytes());
            if !central {
                buf.extend_from_slice(&comp);
            }
        }
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
