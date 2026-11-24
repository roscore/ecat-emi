//! Small numeric helpers: erfc, log grid, integration, deterministic RNG.

/// Complementary error function. Chebyshev-fitted rational form (fractional error < 1.2e-7
/// everywhere), from the classic numerical-methods literature.
pub fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98 + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    let r = t * poly.exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// Gaussian tail: BER for a decision distance of `q` standard deviations.
pub fn q_to_ber(q: f64) -> f64 {
    (0.5 * erfc(q / std::f64::consts::SQRT_2)).min(0.5)
}

/// `n` log-spaced points from `f0` to `f1` inclusive.
pub fn log_grid(f0: f64, f1: f64, n: usize) -> Vec<f64> {
    let (l0, l1) = (f0.ln(), f1.ln());
    (0..n).map(|i| (l0 + (l1 - l0) * i as f64 / (n - 1).max(1) as f64).exp()).collect()
}

/// Trapezoid integral of samples `y` over abscissae `x`.
pub fn trapz(x: &[f64], y: &[f64]) -> f64 {
    x.windows(2).zip(y.windows(2)).map(|(a, b)| (a[1] - a[0]) * (b[0] + b[1]) * 0.5).sum()
}

pub fn db20(x: f64) -> f64 {
    20.0 * x.max(1e-300).log10()
}

pub fn from_db20(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

/// Smooth saturation x/√(1+x²): linear for small x, 1 for large.
pub fn sat(x: f64) -> f64 {
    x / (1.0 + x * x).sqrt()
}

/// xorshift64* — deterministic, seedable, no dependencies (tests and synthetic boards).
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erfc_matches_reference_values() {
        // Reference values of erfc to 16 digits.
        let refs = [
            (0.0, 1.0),
            (0.1, 0.887_537_083_981_715_1),
            (0.5, 0.479_500_122_186_953_5),
            (1.0, 0.157_299_207_050_285_13),
            (1.5, 0.033_894_853_524_689_27),
            (2.0, 0.004_677_734_981_047_266),
            (3.0, 2.209_049_699_858_544e-5),
            (-1.0, 1.842_700_792_949_714_8),
            (-2.5, 1.999_593_047_982_555),
        ];
        for (x, r) in refs {
            assert!((erfc(x) - r).abs() < 1e-6, "erfc({x}) = {} vs {r}", erfc(x));
        }
        // Deep tail: relative accuracy matters for BER.
        let r5 = 1.537_459_794_428_035e-12;
        assert!((erfc(5.0) / r5 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn log_grid_ends_and_trapz() {
        let g = log_grid(1e4, 1e9, 200);
        assert_eq!(g.len(), 200);
        assert!((g[0] - 1e4).abs() < 1e-6 && (g[199] / 1e9 - 1.0).abs() < 1e-12);
        let x = [0.0, 1.0, 2.0];
        assert!((trapz(&x, &[0.0, 1.0, 2.0]) - 2.0).abs() < 1e-12);
    }
}
