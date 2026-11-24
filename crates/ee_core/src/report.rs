//! Local report export (Markdown and CSV). Contains the user's own net names and coordinates,
//! so it is only ever generated in the browser and handed to them as a download.
use crate::couple::{Analysis, Fix, FixKind, Path, Prepared, RunKind};
use crate::link::Verdict;
use crate::roles::Roles;
use std::fmt::Write;

pub fn verdict_str(v: Verdict) -> &'static str {
    match v {
        Verdict::Ok => "OK",
        Verdict::Marginal => "Marginal",
        Verdict::Fail => "Fail",
    }
}

pub fn path_str(p: Path) -> &'static str {
    match p {
        Path::Capacitive => "capacitive",
        Path::Inductive => "inductive",
        Path::ModeConversion => "mode conversion",
        Path::PlaneGap => "plane gap",
        Path::HotLoop => "hot loop",
        Path::Cable => "board-to-board cable",
    }
}

pub fn fix_str(f: &FixKind, pair: &str) -> String {
    match f {
        FixKind::MovePair { mm } => format!("move pair {pair} by {mm:.1} mm"),
        FixKind::FillGap { at } => format!("fill the plane gap at ({:.1}, {:.1}) mm", at[0], at[1]),
        FixKind::ShrinkHotLoop { mm2 } => format!("shrink the hot loop to {mm2:.1} mm²"),
        FixKind::StitchVias { at } => format!("add stitching vias around ({:.1}, {:.1}) mm", at[0], at[1]),
        FixKind::SlowEdges { t_r } => format!("slow the edges to {:.0} ns", t_r * 1e9),
        FixKind::InnerLayer => format!("route {pair} on an inner layer between planes"),
    }
}

fn aggr_name(prep: &Prepared, roles: &Roles, r: &crate::couple::Run) -> String {
    match r.kind {
        RunKind::Parallel => r.aggr.and_then(|a| roles.aggressors.get(a)).map(|a| prep.board.nets[a.net].clone()).unwrap_or_default(),
        RunKind::HotLoop => "hot loop".into(),
        RunKind::Cable => "phase cable".into(),
    }
}

pub fn csv(prep: &Prepared, roles: &Roles, an: &Analysis) -> String {
    let mut s = String::from("pair,aggressor,kind,length_mm,s_p_mm,s_n_mm,h_mm,gap_mm,x_mm,y_mm,peak_mV,share_pct,db\n");
    for r in &an.runs {
        let mid = [(r.band[0][0] + r.band[1][0]) / 2.0, (r.band[0][1] + r.band[1][1]) / 2.0];
        let _ = writeln!(
            s,
            "{},{},{:?},{:.2},{:.2},{:.2},{:.3},{:.2},{:.2},{:.2},{:.3},{:.1},{:.1}",
            roles.pairs[r.pair].name,
            aggr_name(prep, roles, r),
            r.kind,
            r.len,
            r.s_p,
            r.s_n,
            r.h,
            r.gap_len,
            mid[0],
            mid[1],
            r.peak * 1e3,
            r.share * 100.0,
            r.db
        );
    }
    s
}

pub fn markdown(prep: &Prepared, roles: &Roles, an: &Analysis, fixes: &[(usize, Vec<Fix>)]) -> String {
    let mut s = String::from("# ecat-emi report\n\nLumped, quasi-static coupling estimate — not a field solve and not a compliance test.\n\n");
    s.push_str("## Pairs\n\n| pair | verdict | BER | FER | lost frames/h | peak mV | margin mV | dominant |\n|---|---|---|---|---|---|---|---|\n");
    for p in &an.pairs {
        let l = &p.link;
        let _ = writeln!(
            s,
            "| {} | {} | {:.2e} | {:.2e} | {:.3e} | {:.1} | {:.1} | {} |",
            roles.pairs[p.pair].name,
            verdict_str(l.verdict()),
            l.ber,
            l.fer,
            l.lost_per_hour,
            l.noise_peak * 1e3,
            l.margin * 1e3,
            path_str(p.dominant)
        );
    }
    s.push_str("\n## Coupled runs\n\n| pair | aggressor | ℓ mm | s mm | h mm | gap mm | share | dB |\n|---|---|---|---|---|---|---|---|\n");
    for r in &an.runs {
        let _ = writeln!(
            s,
            "| {} | {} | {:.1} | {:.2} | {:.3} | {:.1} | {:.0}% | {:.1} |",
            roles.pairs[r.pair].name,
            aggr_name(prep, roles, r),
            r.len,
            r.s_p,
            r.h,
            r.gap_len,
            r.share * 100.0,
            r.db
        );
    }
    s.push_str("\n## Plane gaps\n\n");
    if an.gaps.is_empty() {
        s.push_str("None crossed.\n");
    }
    for g in &an.gaps {
        let layer = prep.planes.get(g.plane).map(|p| prep.board.layers[p.layer].name.as_str()).unwrap_or("?");
        let _ = writeln!(s, "- {} crosses a void on {} at ({:.1}, {:.1}) mm over {:.1} mm", roles.pairs[g.pair].name, layer, g.at[0], g.at[1], g.len);
    }
    s.push_str("\n## Fixes (each applied alone)\n\n");
    for (pair, list) in fixes {
        let name = &roles.pairs[*pair].name;
        let _ = writeln!(s, "### {name}\n");
        for f in list {
            let _ = writeln!(s, "1. {} → BER {:.2e} ({})", fix_str(&f.kind, name), f.ber, verdict_str(f.verdict));
        }
        s.push('\n');
    }
    s
}
