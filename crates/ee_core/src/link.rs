//! Victim: 100BASE-TX EtherCAT receiver, eye margin, BER / FER / lost frames, verdict.
use crate::num::{erfc, q_to_ber};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkParams {
    /// MLT-3 eye opening at the RX for a short cable (V p-p differential).
    pub eye_pp: f64,
    /// Cable length to the link partner (m), sets the insertion loss.
    pub cable_m: f64,
    /// Receiver bandwidth (Hz), 2nd-order low-pass.
    pub rx_bw: f64,
    /// Receiver input noise floor (V RMS) without any aggressor.
    pub floor_rms: f64,
    /// EtherCAT frame length (bytes).
    pub frame_bytes: f64,
    /// Cycle time (s), one frame per cycle.
    pub cycle_s: f64,
    /// Switch-node ringing decay constant τ (s); burst ≈ t_r + 3τ.
    pub ring_tau: f64,
}

impl Default for LinkParams {
    fn default() -> Self {
        LinkParams {
            eye_pp: 1.0,
            cable_m: 50.0,
            rx_bw: 80e6,
            floor_rms: 0.025,
            frame_bytes: 300.0,
            cycle_s: 1e-3,
            ring_tau: 20e-9,
        }
    }
}

/// 100BASE-TX (Cat5) insertion loss in dB for `len_m`, `f_mhz` in MHz:
/// IL = (1.967·√f + 0.023·f + 0.050/√f) · L/100.
pub fn insertion_loss_db(f_mhz: f64, len_m: f64) -> f64 {
    let s = f_mhz.max(1e-6).sqrt();
    (1.967 * s + 0.023 * f_mhz + 0.050 / s) * len_m / 100.0
}

/// 2nd-order Butterworth magnitude.
pub fn rx_filter(f: f64, bw: f64) -> f64 {
    1.0 / (1.0 + (f / bw).powi(4)).sqrt()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Marginal,
    Fail,
}

/// OK below 1e-12, Marginal below 1e-9, otherwise Fail.
pub fn verdict(ber: f64) -> Verdict {
    if ber < 1e-12 {
        Verdict::Ok
    } else if ber < 1e-9 {
        Verdict::Marginal
    } else {
        Verdict::Fail
    }
}

/// FER = 1 − (1 − BER)^bits, computed without cancellation.
pub fn fer(ber: f64, bits: f64) -> f64 {
    -(bits * (-ber.min(1.0 - 1e-16)).ln_1p()).exp_m1()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LinkResult {
    pub eye_rx: f64,
    pub noise_peak: f64,
    pub noise_rms: f64,
    pub margin: f64,
    pub q: f64,
    /// Fraction of bits inside a PWM-edge burst.
    pub exposure: f64,
    /// Probability that one frame overlaps at least one burst.
    pub frame_hit: f64,
    pub burst_s: f64,
    pub ber: f64,
    pub fer: f64,
    pub lost_per_hour: f64,
    pub wkc_per_cycle: f64,
}

impl LinkResult {
    pub fn verdict(&self) -> Verdict {
        verdict(self.ber)
    }
}

impl LinkParams {
    pub fn eye_rx(&self) -> f64 {
        self.eye_pp * 10f64.powf(-insertion_loss_db(31.25, self.cable_m) / 20.0)
    }
    pub fn frame_bits(&self) -> f64 {
        self.frame_bytes * 8.0
    }
    pub fn frame_s(&self) -> f64 {
        self.frame_bits() / 100e6
    }

    /// Evaluate the link for a burst of `peak` (V) and in-burst RMS `rms` (V) at each PWM edge.
    pub fn evaluate(&self, peak: f64, rms: f64, f_pwm: f64, t_r: f64) -> LinkResult {
        let eye_rx = self.eye_rx();
        let burst_s = t_r + 3.0 * self.ring_tau;
        let edges = 2.0 * f_pwm;
        let exposure = (edges * burst_s).min(1.0);
        let frame_hit = (edges * (self.frame_s() + burst_s)).min(1.0);
        let sigma = (self.floor_rms.powi(2) + rms * rms).sqrt();
        let margin = eye_rx / 2.0 - peak;
        let q = margin / sigma;
        let ber_floor = 0.5 * erfc(eye_rx / 2.0 / self.floor_rms / std::f64::consts::SQRT_2);
        let ber = (exposure * q_to_ber(q) + (1.0 - exposure) * ber_floor).min(0.5);
        let fer = fer(ber, self.frame_bits());
        LinkResult {
            eye_rx,
            noise_peak: peak,
            noise_rms: rms,
            margin,
            q,
            exposure,
            frame_hit,
            burst_s,
            ber,
            fer,
            lost_per_hour: fer * 3600.0 / self.cycle_s,
            // A frame crosses the link on the way out and back in a line topology.
            wkc_per_cycle: 1.0 - (1.0 - fer).powi(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ber_and_fer_monotonic_in_noise() {
        let p = LinkParams::default();
        let mut last = (0.0, 0.0);
        for k in 0..40 {
            let peak = k as f64 * 0.01;
            let r = p.evaluate(peak, peak / 3.0, 20e3, 30e-9);
            assert!(r.ber >= last.0 && r.fer >= last.1, "step {k}");
            last = (r.ber, r.fer);
        }
        assert!(last.0 > 1e-6);
    }

    #[test]
    fn verdict_thresholds_hold() {
        assert_eq!(verdict(9.9e-13), Verdict::Ok);
        assert_eq!(verdict(1e-12), Verdict::Marginal);
        assert_eq!(verdict(9.9e-10), Verdict::Marginal);
        assert_eq!(verdict(1e-9), Verdict::Fail);
        assert_eq!(verdict(0.5), Verdict::Fail);
    }

    #[test]
    fn fer_small_ber_is_bits_times_ber() {
        assert!((fer(1e-12, 2400.0) / 2.4e-9 - 1.0).abs() < 1e-6);
        assert!(insertion_loss_db(31.25, 100.0) > 11.0 && insertion_loss_db(31.25, 100.0) < 12.5);
        assert!(rx_filter(80e6, 80e6) > 0.70 && rx_filter(80e6, 80e6) < 0.71);
    }
}
