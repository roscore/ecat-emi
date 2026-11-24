//! Aggressor: PWM half-bridge switching spectrum (trapezoid envelope) and single-edge densities.
use std::f64::consts::PI;

/// Inverter / half-bridge settings shared by all aggressor nets (see README, model section).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inverter {
    /// DC bus voltage (V), 12–100.
    pub v_bus: f64,
    /// PWM frequency (Hz), 4–100 kHz.
    pub f_pwm: f64,
    /// Edge rise time (s), 5–500 ns.
    pub t_r: f64,
    /// Phase current amplitude (A), 0–60.
    pub i_phase: f64,
    /// Duty cycle (0–1) used for the trapezoid envelope.
    pub duty: f64,
    /// Switch-node capacitance (Coss + layout), sets the displacement current C·dv/dt (F).
    pub c_sw: f64,
}

impl Default for Inverter {
    fn default() -> Self {
        Inverter { v_bus: 48.0, f_pwm: 20e3, t_r: 30e-9, i_phase: 20.0, duty: 0.5, c_sw: 1e-9 }
    }
}

impl Inverter {
    pub fn dv_dt(&self) -> f64 {
        self.v_bus / self.t_r
    }
    pub fn di_dt(&self) -> f64 {
        self.i_phase / self.t_r
    }
    /// Corner frequencies of the trapezoid envelope: 1/(π·t_on) and 1/(π·t_r).
    pub fn corners(&self) -> (f64, f64) {
        let t_on = self.duty / self.f_pwm;
        (1.0 / (PI * t_on), 1.0 / (PI * self.t_r))
    }
}

/// Harmonic amplitude envelope of a trapezoid of amplitude `a`:
/// 2·a·d · min(1, 1/(π f t_on)) · min(1, 1/(π f t_r)).
pub fn trapezoid_envelope(a: f64, inv: &Inverter, f: f64) -> f64 {
    let (f1, f2) = inv.corners();
    2.0 * a * inv.duty * (f1 / f).min(1.0) * (f2 / f).min(1.0)
}

/// Magnitude spectral density (per Hz, one-sided) of a single ramp step of height `step` with
/// rise time `t_r`: step/(2πf) · min(1, 1/(π f t_r)). The envelope replaces the sinc nulls.
pub fn edge_density(step: f64, t_r: f64, f: f64) -> f64 {
    step / (2.0 * PI * f) * (1.0 / (PI * f * t_r)).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_corners_are_where_expected() {
        let inv = Inverter::default();
        let (f1, f2) = inv.corners();
        // t_on = 25 µs → 12.7 kHz; t_r = 30 ns → 10.6 MHz.
        assert!((f1 - 1.0 / (PI * 25e-6)).abs() < 1e-6);
        assert!((f2 - 1.0 / (PI * 30e-9)).abs() < 1e-3);
        // Flat below f1, −20 dB/dec between, −40 dB/dec above f2.
        let e = |f: f64| trapezoid_envelope(48.0, &inv, f);
        assert!((e(1e3) - 48.0).abs() < 1e-9);
        let slope1 = (e(1e6) / e(1e5)).log10() * 20.0;
        let slope2 = (e(1e9) / e(1e8)).log10() * 20.0;
        assert!((slope1 + 20.0).abs() < 1e-9, "{slope1}");
        assert!((slope2 + 40.0).abs() < 1e-9, "{slope2}");
    }

    #[test]
    fn faster_edges_raise_high_frequency_content() {
        let si = Inverter { t_r: 100e-9, ..Inverter::default() };
        let gan = Inverter { t_r: 10e-9, ..Inverter::default() };
        assert!(trapezoid_envelope(48.0, &gan, 50e6) > trapezoid_envelope(48.0, &si, 50e6) * 5.0);
        assert!(edge_density(1.0, 10e-9, 50e6) > edge_density(1.0, 100e-9, 50e6));
    }
}
