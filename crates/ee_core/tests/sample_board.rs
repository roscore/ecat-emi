//! End-to-end checks on the bundled synthetic sample board.
use ee_core::couple::{self, Edits, FixKind, Params, RunKind};
use ee_core::link::Verdict;
use ee_core::{kicad, roles, sample};

fn setup() -> (couple::Prepared, roles::Roles, Params) {
    let b = kicad::parse(&sample::kicad_text()).expect("sample parses");
    let p = Params::default();
    let r = roles::suggest(&b, &p.inv);
    (couple::prepare(b), r, p)
}

fn rx(r: &roles::Roles) -> usize {
    r.pairs.iter().position(|p| p.name == "ECAT_RX").expect("RX pair suggested")
}

#[test]
fn kicad_parser_reads_the_sample() {
    let b = kicad::parse(&sample::kicad_text()).unwrap();
    assert_eq!(b.nets.len(), sample::NETS.len() + 1, "net count incl. the empty net 0");
    assert_eq!(b.layers.len(), 4);
    assert!((b.thickness - 1.605).abs() < 1e-9, "stackup thickness {}", b.thickness);
    assert!((b.layers[1].z - 0.2525).abs() < 1e-9);
    assert_eq!(b.segs.len(), 39, "track segment count");
    assert_eq!(b.vias.len(), 14);
    assert_eq!(b.footprints.len(), 11);
    assert_eq!(b.zones.len(), 2);
    let q1 = b.footprints.iter().position(|f| f.reference == "Q1").unwrap();
    assert!(b.pads_of(q1).any(|p| p.at == [14.0, 16.0]));
}

#[test]
fn roles_are_suggested() {
    let (prep, r, _) = setup();
    let names: Vec<&str> = r.pairs.iter().map(|p| p.name.as_str()).collect();
    for n in ["ECAT_RX", "ECAT_TX", "MDI_RX", "MDI_TX"] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    let mdi = r.pairs.iter().find(|p| p.name == "MDI_TX").unwrap();
    assert_eq!(mdi.side, roles::Side::Line);
    assert_eq!(r.pairs.iter().find(|p| p.name == "ECAT_TX").unwrap().side, roles::Side::Phy);
    let ag: Vec<&str> = r.aggressors.iter().map(|a| prep.board.nets[a.net].as_str()).collect();
    for n in ["SW", "GATE_H", "GATE_L", "BOOT", "VBUS"] {
        assert!(ag.contains(&n), "{n} in {ag:?}");
    }
    assert!(!ag.contains(&"GND") && !ag.contains(&"+3V3"));
}

#[test]
fn planted_run_and_plane_gap_are_found() {
    let (prep, r, p) = setup();
    let an = couple::analyze(&prep, &r, &p, &Edits::default());
    let rxp = rx(&r);
    let sw = prep.board.net_id("SW").unwrap();
    let run = an
        .runs
        .iter()
        .filter(|x| x.pair == rxp && x.kind == RunKind::Parallel && r.aggressors[x.aggr.unwrap()].net == sw)
        .max_by(|a, b| a.len.total_cmp(&b.len))
        .expect("RX ↔ SW run");
    assert!((run.len - 23.0).abs() < 0.3, "run length {}", run.len);
    assert!((run.s_p - 1.5).abs() < 0.05, "edge spacing {}", run.s_p);
    assert!(run.s_n > run.s_p);
    assert!((run.h - 0.235).abs() < 1e-6, "height to In1 {}", run.h);
    assert!(run.gap_len > 10.0, "gap length {}", run.gap_len);
    // The TX pair runs 14 mm away: no run with the switch node.
    let txp = r.pairs.iter().position(|p| p.name == "ECAT_TX").unwrap();
    assert!(!an.runs.iter().any(|x| x.pair == txp && x.kind == RunKind::Parallel && r.aggressors[x.aggr.unwrap()].net == sw));
    let gap = an.gaps.iter().find(|g| g.pair == rxp).expect("slot crossing");
    let [x0, y0, x1, y1] = sample::SLOT;
    assert!(gap.bbox[0] <= x0 + 0.2 && gap.bbox[2] >= x1 - 0.2 && gap.bbox[1] <= y0 + 0.2 && gap.bbox[3] >= y1 - 0.2, "{:?}", gap.bbox);
    assert!(prep.hot_loop.is_some());
}

#[test]
fn sample_fails_before_and_passes_after_the_fixes() {
    let (prep, r, p) = setup();
    let rxp = rx(&r);
    let base = couple::analyze(&prep, &r, &p, &Edits::default());
    let res = base.pairs.iter().find(|x| x.pair == rxp).unwrap();
    assert_eq!(res.verdict(), Verdict::Fail, "{:?}", res.link);
    let tx = base.pairs.iter().find(|x| r.pairs[x.pair].name == "ECAT_TX").unwrap();
    assert_eq!(tx.verdict(), Verdict::Ok, "{:?}", tx.link);

    let fixes = couple::fixes(&prep, &r, &p, &Edits::default(), rxp);
    assert!(fixes.windows(2).all(|w| w[0].ber <= w[1].ber), "ranked fixes are sorted");
    let mv = fixes.iter().find(|f| matches!(f.kind, FixKind::MovePair { .. })).expect("move fix");
    assert!(mv.ber < res.link.ber, "moving the pair lowers BER");
    assert_eq!(mv.verdict, Verdict::Ok, "{:?}", mv.kind);
    let fill = fixes.iter().find(|f| matches!(f.kind, FixKind::FillGap { .. })).expect("fill fix");
    assert_eq!(fill.verdict, Verdict::Ok);
    // Re-running with the fix edits reproduces the fix result.
    let again = couple::analyze(&prep, &r, &p, &fill.edits);
    assert_eq!(again.pairs.iter().find(|x| x.pair == rxp).unwrap().link.ber, fill.ber);
}

#[test]
fn analysis_is_deterministic() {
    let (prep, r, p) = setup();
    let a = couple::analyze(&prep, &r, &p, &Edits::default());
    let b = couple::analyze(&prep, &r, &p, &Edits::default());
    assert_eq!(a.pairs, b.pairs);
    assert_eq!(a.runs, b.runs);
}
