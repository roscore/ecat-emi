//! URL-hash settings: model parameters and view options only — never board data.
use crate::cable::{Shield, ShieldGround};
use crate::couple::Params;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub ko: bool,
    pub params: Params,
    /// Thickness exaggeration for the 3D view.
    pub exag: f64,
    /// View preset: 0 iso, 1 top, 2 bottom.
    pub view: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { ko: true, params: Params::default(), exag: 4.0, view: 0 }
    }
}

fn shield_code(s: Shield) -> u8 {
    match s {
        Shield::None => 0,
        Shield::Foil => 1,
        Shield::Braid => 2,
    }
}
fn ground_code(g: ShieldGround) -> u8 {
    match g {
        ShieldGround::None => 0,
        ShieldGround::OneEnd => 1,
        ShieldGround::BothEnds => 2,
    }
}

/// Fields: key, getter, setter. Values are stored in display units.
type Field = (&'static str, fn(&Settings) -> f64, fn(&mut Settings, f64));

const FIELDS: &[Field] = &[
    ("vbus", |s| s.params.inv.v_bus, |s, v| s.params.inv.v_bus = v.clamp(12.0, 100.0)),
    ("fpwm", |s| s.params.inv.f_pwm / 1e3, |s, v| s.params.inv.f_pwm = v.clamp(4.0, 100.0) * 1e3),
    ("tr", |s| s.params.inv.t_r * 1e9, |s, v| s.params.inv.t_r = v.clamp(5.0, 500.0) * 1e-9),
    ("iph", |s| s.params.inv.i_phase, |s, v| s.params.inv.i_phase = v.clamp(0.0, 60.0)),
    ("csw", |s| s.params.inv.c_sw * 1e12, |s, v| s.params.inv.c_sw = v.clamp(10.0, 20000.0) * 1e-12),
    ("lcl", |s| s.params.lcl_db, |s, v| s.params.lcl_db = v.clamp(-60.0, -25.0)),
    ("cmrr", |s| s.params.cmrr_db, |s, v| s.params.cmrr_db = v.clamp(0.0, 60.0)),
    ("len", |s| s.params.link.cable_m, |s, v| s.params.link.cable_m = v.clamp(0.0, 100.0)),
    ("frame", |s| s.params.link.frame_bytes, |s, v| s.params.link.frame_bytes = v.clamp(64.0, 1518.0)),
    ("cycle", |s| s.params.link.cycle_s * 1e6, |s, v| s.params.link.cycle_s = v.clamp(125.0, 4000.0) * 1e-6),
    ("floor", |s| s.params.link.floor_rms * 1e3, |s, v| s.params.link.floor_rms = v.clamp(1.0, 100.0) * 1e-3),
    ("b2b", |s| s.params.cable.len_m, |s, v| s.params.cable.len_m = v.clamp(0.0, 5.0)),
    ("sep", |s| s.params.cable.sep_m * 1e3, |s, v| s.params.cable.sep_m = v.clamp(1.0, 500.0) * 1e-3),
    ("shd", |s| shield_code(s.params.cable.shield) as f64, |s, v| {
        s.params.cable.shield = [Shield::None, Shield::Foil, Shield::Braid][(v as usize).min(2)]
    }),
    ("gnd", |s| ground_code(s.params.cable.ground) as f64, |s, v| {
        s.params.cable.ground = [ShieldGround::None, ShieldGround::OneEnd, ShieldGround::BothEnds][(v as usize).min(2)]
    }),
    ("fer", |s| s.params.cable.ferrite_ohm, |s, v| s.params.cable.ferrite_ohm = v.clamp(0.0, 1000.0)),
    ("exag", |s| s.exag, |s, v| s.exag = v.clamp(1.0, 20.0)),
    ("view", |s| s.view as f64, |s, v| s.view = (v as u8).min(2)),
];

fn fmt(v: f64) -> String {
    let s = format!("{:.4}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

pub fn encode(s: &Settings) -> String {
    let mut out = format!("lang={}", if s.ko { "ko" } else { "en" });
    let d = Settings::default();
    for (k, get, _) in FIELDS {
        if (get(s) - get(&d)).abs() > 1e-9 {
            out.push_str(&format!("&{k}={}", fmt(get(s))));
        }
    }
    out
}

/// Setting keys, in URL order.
pub fn keys() -> impl Iterator<Item = &'static str> {
    FIELDS.iter().map(|f| f.0)
}

/// One setting in display units (as stored in the hash), for filling the UI inputs.
pub fn value(s: &Settings, key: &str) -> Option<f64> {
    FIELDS.iter().find(|f| f.0 == key).map(|f| (f.1)(s))
}

/// Tolerant: unknown keys and bad values are ignored.
pub fn decode(h: &str) -> Settings {
    let mut s = Settings::default();
    for kv in h.trim_start_matches('#').split('&') {
        let Some((k, v)) = kv.split_once('=') else { continue };
        if k == "lang" {
            s.ko = v != "en";
            continue;
        }
        if let (Some((_, _, set)), Ok(x)) = (FIELDS.iter().find(|f| f.0 == k), v.parse::<f64>()) {
            if x.is_finite() {
                set(&mut s, x);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_and_tolerance() {
        let mut s = Settings { ko: false, ..Settings::default() };
        s.params.inv.v_bus = 60.0;
        s.params.inv.t_r = 10e-9;
        s.params.cable.shield = Shield::Braid;
        s.view = 1;
        let h = encode(&s);
        assert!(h.starts_with("lang=en") && h.contains("vbus=60") && h.contains("tr=10"));
        let back = decode(&h);
        assert_eq!(encode(&back), h);
        assert!((back.params.inv.t_r - 10e-9).abs() < 1e-15);
        let junk = decode("#vbus=abc&zzz=1&tr=1e9&lang=ko");
        assert!(junk.ko && junk.params.inv.v_bus == 48.0 && (junk.params.inv.t_r - 500e-9).abs() < 1e-15);
    }

    #[test]
    fn value_reads_display_units_for_every_key() {
        let d = Settings::default();
        assert_eq!(value(&d, "vbus"), Some(48.0));
        assert!((value(&d, "tr").unwrap() - 30.0).abs() < 1e-9, "ns");
        assert_eq!(value(&d, "nope"), None);
        for k in keys() {
            let v = value(&d, k).unwrap();
            assert_eq!(decode(&format!("{k}={v}")), d, "{k} round-trips at its default");
        }
    }
}
