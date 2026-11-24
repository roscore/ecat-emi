//! Loaded board + roles + virtual edits + results (pure state, no web-sys).
use crate::i18n::tr;
use ee_core::board::{Board, P2};
use ee_core::couple::{self, Analysis, Edits, Fix, Params, PairResult, Prepared};
use ee_core::gerber::{self, LayerGuess};
use ee_core::link::Verdict;
use ee_core::roles::{self, AggrKind, AggrRole, PairRole, Roles, Side};

/// Assumed board thickness for Gerber sets (they carry no stackup).
pub const GERBER_THICKNESS: f64 = 1.6;

pub struct Doc {
    pub prep: Prepared,
    pub roles: Roles,
    pub edits: Edits,
    pub an: Analysis,
    /// Ranked fixes per non-OK pair (recomputed on demand: each costs dozens of analyses).
    pub fixes: Vec<(usize, Vec<Fix>)>,
    pub gerber: Option<gerber::Files>,
    /// Net tagged "pair P", waiting for its N.
    pub pending_p: Option<usize>,
}

/// Segments moved by one drag, with their offsets at drag start.
pub type DragStart = Vec<(usize, P2)>;

impl Doc {
    fn from_board(board: Board, gerber: Option<gerber::Files>, p: &Params) -> Doc {
        let roles = roles::suggest(&board, &p.inv);
        let prep = couple::prepare(board);
        let edits = Edits::default();
        let an = couple::analyze(&prep, &roles, p, &edits);
        Doc { prep, roles, edits, an, fixes: vec![], gerber, pending_p: None }
    }

    pub fn sample(p: &Params) -> Doc {
        let b = ee_core::kicad::parse(&ee_core::sample::kicad_text()).expect("bundled sample parses");
        Doc::from_board(b, None, p)
    }

    /// Open user files (KiCad, Gerber/Excellon, zip). Errors are localised and carry the line.
    pub fn open(files: Vec<(String, Vec<u8>)>, p: &Params, ko: bool) -> Result<Doc, String> {
        match ee_core::open(files) {
            Ok(ee_core::Input::Kicad(b)) => Ok(Doc::from_board(b, None, p)),
            Ok(ee_core::Input::Gerber(f)) => Ok(Doc::from_board(gerber::build(&f, GERBER_THICKNESS).0, Some(f), p)),
            Err(e) if e.line > 0 => Err(format!("{} {}: {}", tr(ko, "line"), e.line, e.msg)),
            Err(e) => Err(e.msg),
        }
    }

    pub fn result(&self, pair: usize) -> Option<&PairResult> {
        self.an.pairs.iter().find(|r| r.pair == pair)
    }

    pub fn analyze(&mut self, p: &Params) {
        self.an = couple::analyze(&self.prep, &self.roles, p, &self.edits);
    }

    /// Ranked fixes for every pair that is not OK.
    pub fn refix(&mut self, p: &Params) {
        // ponytail: capped at 6 pairs; lazy per-pair fixes if boards carry more failing links.
        let bad: Vec<usize> = self.an.pairs.iter().filter(|r| r.verdict() != Verdict::Ok).map(|r| r.pair).take(6).collect();
        self.fixes = bad.into_iter().map(|k| (k, couple::fixes(&self.prep, &self.roles, p, &self.edits, k))).collect();
    }

    pub fn reset_edits(&mut self, p: &Params) {
        self.edits = Edits::default();
        self.analyze(p);
    }

    /// Re-apply the inverter presets (bus V, rise time, current) to every aggressor.
    pub fn set_inverter(&mut self, p: &Params) {
        for a in &mut self.roles.aggressors {
            let en = a.enabled;
            *a = AggrRole::preset(a.net, a.kind, &p.inv);
            a.enabled = en;
        }
        self.analyze(p);
    }

    pub fn tag_aggr(&mut self, net: usize, p: &Params) {
        match self.roles.aggressors.iter_mut().find(|a| a.net == net) {
            Some(a) => a.enabled = true,
            None => {
                let name = roles::short(self.prep.board.nets.get(net).map(String::as_str).unwrap_or(""));
                let kind = roles::aggr_kind_by_name(&name).unwrap_or(AggrKind::SwitchNode);
                self.roles.aggressors.push(AggrRole::preset(net, kind, &p.inv));
            }
        }
        self.analyze(p);
    }

    /// Tag P (remembered) or N (completes a pair with the pending P). True when a pair was made.
    pub fn tag_pair(&mut self, net: usize, positive: bool, p: &Params) -> bool {
        if positive {
            self.pending_p = Some(net);
            return false;
        }
        let Some(pn) = self.pending_p.take().filter(|&x| x != net) else { return false };
        let nm = |k: usize| roles::short(self.prep.board.nets.get(k).map(String::as_str).unwrap_or("?"));
        let name = format!("{}/{}", nm(pn), nm(net));
        self.roles.pairs.push(PairRole { name, p: pn, n: net, side: Side::Phy, enabled: true });
        self.analyze(p);
        true
    }

    /// Override a Gerber layer guess and rebuild the islands (roles and edits start over).
    pub fn set_gerber_layer(&mut self, i: usize, g: LayerGuess, p: &Params) {
        let Some(mut f) = self.gerber.take() else { return };
        if let Some(l) = f.layers.get_mut(i) {
            l.1 = g;
        }
        let b = gerber::build(&f, GERBER_THICKNESS).0;
        *self = Doc::from_board(b, Some(f), p);
    }

    /// The dragged victim segment plus its partner-net neighbour, with current offsets.
    pub fn drag_start(&self, seg: usize) -> DragStart {
        let mut v = vec![seg];
        v.extend(couple::partner(&self.prep, &self.roles, seg));
        v.into_iter().map(|k| (k, self.edits.offsets.iter().find(|o| o.0 == k).map(|o| o.1).unwrap_or([0.0; 2]))).collect()
    }

    /// Move the drag set sideways by `delta` (board mm) and re-analyse.
    pub fn drag(&mut self, start: &DragStart, delta: P2, p: &Params) {
        for &(k, o) in start {
            let s = self.prep.board.segs[k];
            crate::scene::set_offset(&mut self.edits, k, crate::scene::drag_offset(&s, o, delta));
        }
        self.analyze(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ee_core::link::Verdict;

    const HEAD: &str = "%FSLAX36Y36*%\n%MOMM*%\n%ADD10C,0.200000*%\n%ADD11C,0.500000*%\n";

    fn rx(d: &Doc) -> usize {
        d.roles.pairs.iter().position(|p| p.name == "ECAT_RX").unwrap()
    }

    #[test]
    fn sample_fails_and_a_ranked_fix_repairs_it() {
        let p = Params::default();
        let mut d = Doc::sample(&p);
        let rx = rx(&d);
        assert_eq!(d.result(rx).unwrap().verdict(), Verdict::Fail);
        d.refix(&p);
        let list = &d.fixes.iter().find(|f| f.0 == rx).expect("fixes for the failing pair").1;
        assert!(!list.is_empty());
        d.edits = list[0].edits.clone();
        d.analyze(&p);
        assert_ne!(d.result(rx).unwrap().verdict(), Verdict::Fail);
        d.reset_edits(&p);
        assert_eq!(d.edits, Edits::default());
        assert_eq!(d.result(rx).unwrap().verdict(), Verdict::Fail);
    }

    #[test]
    fn parse_error_reports_the_line() {
        let e = Doc::open(vec![("x.kicad_pcb".into(), b"(kicad_pcb\n (net 1 \"A\")\n (segment (start 1".to_vec())], &Params::default(), true).err().unwrap();
        assert!(e.contains("줄 "), "{e}");
        let e = Doc::open(vec![("x.kicad_pcb".into(), b"(kicad_pcb\n (net 1 \"A\")\n (segment (start 1".to_vec())], &Params::default(), false).err().unwrap();
        assert!(e.starts_with("line "), "{e}");
        assert!(Doc::open(vec![], &Params::default(), false).is_err());
    }

    #[test]
    fn gerber_islands_can_be_tagged_and_layers_overridden() {
        let p = Params::default();
        // A 20 mm victim pair at y = 0 / 0.5 and an aggressor 1 mm away, top layer.
        let top = format!(
            "{HEAD}D10*\nX0Y0D02*\nX20000000Y0D01*\nX0Y500000D02*\nX20000000Y500000D01*\nD11*\nX0Y1700000D02*\nX20000000Y1700000D01*\nM02*\n"
        );
        let mut d = Doc::open(vec![("b.gtl".into(), top.into_bytes())], &p, true).unwrap();
        assert!(d.gerber.is_some());
        assert!(d.roles.pairs.is_empty());
        let net_at = |d: &Doc, y: f64| crate::scene::pick_net(&d.prep.board, [10.0, y], &[true; 4], 0.05).unwrap();
        let (a, b, c) = (net_at(&d, 0.0), net_at(&d, -0.5), net_at(&d, -1.7));
        assert!(a != b && b != c);
        assert!(!d.tag_pair(a, true, &p));
        assert!(d.tag_pair(b, false, &p), "P then N makes a pair");
        d.tag_aggr(c, &p);
        assert_eq!(d.roles.pairs.len(), 1);
        assert!(d.an.runs.iter().any(|r| r.kind == ee_core::couple::RunKind::Parallel), "run found between tagged islands");
        d.set_gerber_layer(0, LayerGuess::Copper(1000), &p);
        assert_eq!(d.gerber.as_ref().unwrap().layers[0].1, LayerGuess::Copper(1000));
        assert!(d.roles.pairs.is_empty(), "islands renumber: roles cleared");
    }

    #[test]
    fn inverter_change_re_presets_aggressors() {
        let mut p = Params::default();
        let mut d = Doc::sample(&p);
        let sw = d.roles.aggressors.iter().position(|a| a.kind == AggrKind::SwitchNode).unwrap();
        assert_eq!(d.roles.aggressors[sw].dv, 48.0);
        p.inv.v_bus = 80.0;
        d.set_inverter(&p);
        assert_eq!(d.roles.aggressors[sw].dv, 80.0);
    }

    #[test]
    fn dragging_the_rx_pair_away_lowers_its_noise() {
        let p = Params::default();
        let mut d = Doc::sample(&p);
        let rx = rx(&d);
        let before = d.result(rx).unwrap().link.noise_peak;
        let run = d.an.runs.iter().filter(|r| r.pair == rx && r.aggr.is_some()).max_by(|a, b| a.energy.total_cmp(&b.energy)).unwrap();
        let seg = run.victim_segs[0];
        let start = d.drag_start(seg);
        let s = d.prep.board.segs[seg];
        let away = ee_core::board::sub(run.band[0], run.band[3]);
        d.drag(&start, ee_core::board::scale(away, 4.0 / ee_core::board::len(away)), &p);
        assert!(d.edits.offsets.len() >= 2, "pair partner moves too: {:?}", d.edits.offsets);
        assert!(d.result(rx).unwrap().link.noise_peak < before);
        let _ = s;
    }
}
