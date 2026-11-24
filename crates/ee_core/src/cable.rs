//! Secondary input: board-to-board cable run next to the motor phase cable (see README, "board-to-board cable").
//! Two round wires at height h over a ground plane, separation d (multiconductor-line closed forms).
use std::f64::consts::PI;

pub const MU0: f64 = 4e-7 * PI;
pub const EPS0: f64 = 8.854_187_8e-12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shield {
    None,
    Foil,
    Braid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShieldGround {
    None,
    OneEnd,
    BothEnds,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CableParams {
    /// Parallel run length with the phase cable (m); 0 disables the cable path.
    pub len_m: f64,
    /// Centre separation between the cables (m).
    pub sep_m: f64,
    /// Height above the chassis / ground plane (m).
    pub height_m: f64,
    /// Equivalent conductor radius (m).
    pub wire_r: f64,
    /// Twists per metre of the victim pair.
    pub twist_per_m: f64,
    pub shield: Shield,
    pub ground: ShieldGround,
    /// Snap-on ferrite impedance at 100 MHz (Ω), 0 = none.
    pub ferrite_ohm: f64,
}

impl Default for CableParams {
    fn default() -> Self {
        CableParams {
            len_m: 0.0,
            sep_m: 0.01,
            height_m: 0.02,
            wire_r: 0.5e-3,
            twist_per_m: 40.0,
            shield: Shield::None,
            ground: ShieldGround::None,
            ferrite_ohm: 0.0,
        }
    }
}

/// Mutual inductance per metre: (μ0/4π)·ln(1 + (2h/d)²).
pub fn mutual_l(h: f64, d: f64) -> f64 {
    MU0 / (4.0 * PI) * (1.0 + (2.0 * h / d).powi(2)).ln()
}

/// Self inductance per metre of a wire of radius r at height h: (μ0/2π)·ln(2h/r).
pub fn self_l(h: f64, r: f64) -> f64 {
    MU0 / (2.0 * PI) * (2.0 * h / r).ln()
}

/// Mutual capacitance per metre (magnitude of the off-diagonal Maxwell coefficient) in a
/// homogeneous medium: C = μ0·ε·L⁻¹ for the 2×2 inductance matrix.
pub fn mutual_c(h: f64, d: f64, r: f64, eps_r: f64) -> f64 {
    let (ls, lm) = (self_l(h, r), mutual_l(h, d));
    MU0 * EPS0 * eps_r * lm / (ls * ls - lm * lm)
}

/// Twisted-pair cancellation: 1 / (1 + 2π·N·ℓ_eff), ℓ_eff = the run length.
pub fn twist_factor(n_per_m: f64, l_eff: f64) -> f64 {
    1.0 / (1.0 + 2.0 * PI * n_per_m * l_eff)
}

/// Transfer impedance per metre of the shield: (R_dc, L_t).
pub fn shield_zt(s: Shield) -> (f64, f64) {
    match s {
        Shield::None => (0.0, 0.0),
        Shield::Foil => (30e-3, 0.3e-9),
        Shield::Braid => (10e-3, 1.0e-9),
    }
}

/// Inductive-path multiplier. Grounded at both ends, the shield carries the return current above
/// its cutoff f_c = R/(2π·L_sh): factor = |R + jωL_t| / |R + jωL_sh|. One end or none: 1.
pub fn shield_inductive(p: &CableParams, f: f64) -> f64 {
    if p.shield == Shield::None || p.ground != ShieldGround::BothEnds {
        return 1.0;
    }
    let (r, lt) = shield_zt(p.shield);
    let lsh = self_l(p.height_m, p.wire_r * 3.0);
    let w = 2.0 * PI * f;
    (r * r + (w * lt).powi(2)).sqrt() / (r * r + (w * lsh).powi(2)).sqrt()
}

/// Capacitive-path multiplier: any grounded shield blocks the electric field.
pub fn shield_capacitive(p: &CableParams) -> f64 {
    match (p.shield, p.ground) {
        (Shield::None, _) | (_, ShieldGround::None) => 1.0,
        (Shield::Foil, _) => 0.003,
        (Shield::Braid, _) => 0.01,
    }
}

/// Ferrite magnitude |Z(f)|: rises ∝ f to the 100 MHz rating, flat above.
pub fn ferrite_z(ohm_100m: f64, f: f64) -> f64 {
    ohm_100m * (f / 100e6).min(1.0)
}

/// Common-mode voltage density coupled onto the victim cable (V/Hz) from aggressor edge
/// densities v (V/Hz) and i (A/Hz). Returns (capacitive, inductive).
pub fn cable_cm(p: &CableParams, f: f64, v: f64, i: f64) -> (f64, f64) {
    if p.len_m <= 0.0 {
        return (0.0, 0.0);
    }
    let w = 2.0 * PI * f;
    // Victim CM node impedance: 100 Ω pair seen as 25 Ω CM termination, plus the ferrite.
    let z_v = 25.0 + ferrite_z(p.ferrite_ohm, f);
    let cm = mutual_c(p.height_m, p.sep_m, p.wire_r, 2.3);
    let cap = (w * cm * p.len_m * z_v).min(1.0) * v * shield_capacitive(p);
    let ind = w * mutual_l(p.height_m, p.sep_m) * p.len_m * i * shield_inductive(p, f);
    (cap, ind)
}

/// Differential residue of the twisted pair for the inductive part (V/Hz).
pub fn cable_dm(p: &CableParams, ind_cm: f64) -> f64 {
    ind_cm * twist_factor(p.twist_per_m, p.len_m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutual_l_and_c_match_hand_values() {
        // h = 20 mm, d = 10 mm: (2h/d)² = 16 → M = 1e-7·ln 17 = 283.3 nH/m.
        let m = mutual_l(0.02, 0.01);
        assert!((m - 1e-7 * 17f64.ln()).abs() < 1e-12);
        assert!((m - 283.32e-9).abs() < 0.01e-9);
        // Ls = 2e-7·ln(80) = 876.4 nH/m; Cm = μ0ε0·Lm/(Ls² − Lm²) = 4.58 pF/m in air.
        let c = mutual_c(0.02, 0.01, 0.5e-3, 1.0);
        let ls = 2e-7 * 80f64.ln();
        let hand = 1.0 / (299_792_458f64.powi(2)) * m / (ls * ls - m * m);
        assert!((c - hand).abs() < 1e-18);
        assert!((c - 4.583e-12).abs() < 0.002e-12, "{c}");
    }

    #[test]
    fn doubling_separation_reduces_cable_coupling() {
        let a = CableParams { len_m: 1.0, ..CableParams::default() };
        let b = CableParams { sep_m: a.sep_m * 2.0, ..a };
        let (ca, la) = cable_cm(&a, 10e6, 1e-6, 1e-7);
        let (cb, lb) = cable_cm(&b, 10e6, 1e-6, 1e-7);
        assert!(cb < ca && lb < la);
    }

    #[test]
    fn twisting_reduces_inductive_path() {
        let p = CableParams { len_m: 1.0, ..CableParams::default() };
        let loose = CableParams { twist_per_m: 0.0, ..p };
        assert!(cable_dm(&p, 1.0) < cable_dm(&loose, 1.0) * 0.01);
        assert!(twist_factor(10.0, 0.5) < twist_factor(5.0, 0.5));
    }

    #[test]
    fn shield_both_ends_blocks_inductive_above_cutoff_one_end_does_not() {
        let both = CableParams { len_m: 1.0, shield: Shield::Braid, ground: ShieldGround::BothEnds, ..CableParams::default() };
        let one = CableParams { ground: ShieldGround::OneEnd, ..both };
        assert!(shield_inductive(&both, 10e6) < 0.01);
        assert!(shield_inductive(&both, 10.0) > 0.5, "below cutoff the shield does little");
        assert_eq!(shield_inductive(&one, 10e6), 1.0);
        assert!(shield_capacitive(&one) < 0.1);
    }
}
