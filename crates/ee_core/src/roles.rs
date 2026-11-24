//! Net roles: auto-suggested EtherCAT victim pairs and switching aggressors (user-editable).
use crate::board::Board;
use crate::spectrum::Inverter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// PHY ↔ magnetics: not attenuated.
    Phy,
    /// Magnetics ↔ RJ45: attenuated by the magnetics CMRR.
    Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PairRole {
    pub name: String,
    pub p: usize,
    pub n: usize,
    pub side: Side,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggrKind {
    /// Switch / phase node: full bus swing, displacement current C_sw·dv/dt.
    SwitchNode,
    /// Gate / bootstrap drive: ~12 V swing, ~1 A peak.
    Gate,
    /// DC bus / hot-loop supply: small ripple, full commutated phase current.
    Bus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AggrRole {
    pub net: usize,
    pub kind: AggrKind,
    /// Voltage step per edge (V).
    pub dv: f64,
    /// Current step per edge (A).
    pub di: f64,
    /// Edge time (s).
    pub t_r: f64,
    pub enabled: bool,
}

impl AggrRole {
    pub fn preset(net: usize, kind: AggrKind, inv: &Inverter) -> AggrRole {
        let (dv, di, t_r) = match kind {
            AggrKind::SwitchNode => (inv.v_bus, inv.c_sw * inv.dv_dt(), inv.t_r),
            AggrKind::Gate => (12.0, 1.0, inv.t_r.min(20e-9)),
            AggrKind::Bus => (0.02 * inv.v_bus, inv.i_phase, inv.t_r),
        };
        AggrRole { net, kind, dv, di, t_r, enabled: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Roles {
    pub pairs: Vec<PairRole>,
    pub aggressors: Vec<AggrRole>,
}

/// Upper-case net name without hierarchical path ("/eth/TX+" → "TX+").
pub fn short(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_ascii_uppercase()
}

/// Split a differential name into (stem, positive?).
fn diff_stem(n: &str) -> Option<(String, bool)> {
    for (p, m) in [("_P", "_N"), ("+", "-"), ("_DP", "_DN")] {
        if let Some(s) = n.strip_suffix(p) {
            return Some((s.to_string(), true));
        }
        if let Some(s) = n.strip_suffix(m) {
            return Some((s.to_string(), false));
        }
    }
    // TD_P style is covered above; also TDP / TDN, RXP / RXN.
    for (p, m) in [("P", "N")] {
        if n.len() > 2 && ["TD", "RD", "TX", "RX"].iter().any(|k| n.contains(k)) {
            if let Some(s) = n.strip_suffix(p) {
                return Some((s.to_string(), true));
            }
            if let Some(s) = n.strip_suffix(m) {
                return Some((s.to_string(), false));
            }
        }
    }
    None
}

fn is_ethernet_stem(stem: &str) -> bool {
    ["TX", "RX", "TD", "RD", "ECAT", "ETH", "MDI", "TRX", "LAN"].iter().any(|k| stem.contains(k))
}

/// Aggressor kind from the name, if it matches the switching-net patterns.
pub fn aggr_kind_by_name(n: &str) -> Option<AggrKind> {
    if n == "GND" || n.starts_with("GND") || n.starts_with("PGND") {
        return None;
    }
    if n.starts_with("SW") || n.starts_with("PHASE") || n.ends_with("_U") || n.ends_with("_V") || n.ends_with("_W") {
        return Some(AggrKind::SwitchNode);
    }
    if n.starts_with("GATE") || n.starts_with("HS") || n.starts_with("LS") || n.starts_with("BOOT") {
        return Some(AggrKind::Gate);
    }
    if n.starts_with("VBUS") || n == "VDC" || n == "DC+" || n.starts_with("HV") {
        return Some(AggrKind::Bus);
    }
    None
}

fn is_bus(n: &str) -> bool {
    aggr_kind_by_name(n) == Some(AggrKind::Bus)
}

fn is_magnetics(ref_: &str, name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    ref_.starts_with('T') || u.contains("MAGNETIC") || u.contains("TRANSFORMER") || u.contains("LAN_")
}

fn is_rj45(ref_: &str, name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    u.contains("RJ45") || u.contains("8P8C") || (ref_.starts_with('J') && u.contains("ETH"))
}

/// Suggest roles from net names and footprints.
pub fn suggest(b: &Board, inv: &Inverter) -> Roles {
    let mut r = Roles::default();
    let names: Vec<String> = b.nets.iter().map(|n| short(n)).collect();
    // Footprints each net touches.
    let mut fps: Vec<Vec<usize>> = vec![Vec::new(); b.nets.len()];
    for p in &b.pads {
        if let Some(v) = fps.get_mut(p.net) {
            if !v.contains(&p.footprint) {
                v.push(p.footprint);
            }
        }
    }
    for (i, n) in names.iter().enumerate() {
        let Some((stem, true)) = diff_stem(n) else { continue };
        let Some(j) = names.iter().position(|m| diff_stem(m) == Some((stem.clone(), false))) else { continue };
        let line_side = |net: usize| {
            let f = &fps[net];
            let mag = f.iter().any(|&k| is_magnetics(&b.footprints[k].reference, &b.footprints[k].name));
            let rj = f.iter().any(|&k| is_rj45(&b.footprints[k].reference, &b.footprints[k].name));
            mag && rj || stem.contains("MDI") && !f.iter().any(|&k| b.footprints[k].reference.starts_with('U'))
        };
        let side = if line_side(i) { Side::Line } else { Side::Phy };
        let enabled = is_ethernet_stem(&stem);
        r.pairs.push(PairRole { name: stem, p: i, n: j, side, enabled });
    }
    // Footprints that touch the bus (Q*/U* power devices).
    let bus_fp: Vec<bool> = (0..b.footprints.len())
        .map(|k| b.pads_of(k).any(|p| names.get(p.net).is_some_and(|n| is_bus(n))))
        .collect();
    for (i, n) in names.iter().enumerate() {
        if i == 0 || n.is_empty() || r.pairs.iter().any(|p| p.p == i || p.n == i) {
            continue;
        }
        let kind = aggr_kind_by_name(n).or_else(|| {
            let hit = fps[i].iter().any(|&k| {
                let rf = &b.footprints[k].reference;
                (rf.starts_with('Q') || rf.starts_with('U')) && bus_fp[k]
            });
            (hit && !n.starts_with("GND") && !n.starts_with('+') && !n.contains("3V3") && !n.contains("5V")).then_some(AggrKind::SwitchNode)
        });
        if let Some(k) = kind {
            r.aggressors.push(AggrRole::preset(i, k, inv));
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_names_and_aggressor_patterns() {
        assert_eq!(diff_stem("TX+"), Some(("TX".into(), true)));
        assert_eq!(diff_stem("ECAT_RX_N"), Some(("ECAT_RX".into(), false)));
        assert_eq!(diff_stem("TD_P"), Some(("TD".into(), true)));
        assert_eq!(diff_stem("RXN"), Some(("RX".into(), false)));
        assert_eq!(diff_stem("GND"), None);
        assert_eq!(aggr_kind_by_name("SW1"), Some(AggrKind::SwitchNode));
        assert_eq!(aggr_kind_by_name("MOTOR_U"), Some(AggrKind::SwitchNode));
        assert_eq!(aggr_kind_by_name("GATE_HS"), Some(AggrKind::Gate));
        assert_eq!(aggr_kind_by_name("VBUS"), Some(AggrKind::Bus));
        assert_eq!(aggr_kind_by_name("GND"), None);
        assert_eq!(short("/eth/tx+"), "TX+");
    }
}
