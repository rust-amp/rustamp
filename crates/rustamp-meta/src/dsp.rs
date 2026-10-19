//! A biquad filter.
//!
//! Both loudness measurements of this crate are chains of biquads: the
//! ReplayGain filter (an equal-loudness section and a high-pass section)
//! and the K-weighting of ITU-R BS.1770-4 (a high-shelf and the RLB
//! high-pass). The filter lives here so that both use the same one.
//!
//! The module also holds the rate transformation the ReplayGain analysis
//! uses: a filter designed for one sample rate is taken to the analog
//! domain and realized again for another rate, so that the *same* filter
//! runs at every rate instead of a filter that was designed for another
//! one. See [`retune`].

/// A biquad filter in direct form I: `y = b0 x + b1 x1 + b2 x2 - a1 y1 - a2 y2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Biquad {
    pub(crate) b0: f64,
    pub(crate) b1: f64,
    pub(crate) b2: f64,
    pub(crate) a1: f64,
    pub(crate) a2: f64,
}

impl Biquad {
    /// A biquad from the numerator and the denominator coefficients.
    ///
    /// The denominator's `a0` is 1.0, as in the coefficient tables of the
    /// standards the two filters come from.
    pub(crate) fn new(b: [f64; 3], a: [f64; 3]) -> Self {
        Self {
            b0: b[0],
            b1: b[1],
            b2: b[2],
            a1: a[1],
            a2: a[2],
        }
    }

    /// Filters `input`, returning the filtered signal.
    pub(crate) fn apply(&self, input: &[f64]) -> Vec<f64> {
        let mut out = Vec::with_capacity(input.len());
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for &x in input {
            let y = self.b0 * x + self.b1 * x1 + self.b2 * x2 - self.a1 * y1 - self.a2 * y2;
            out.push(y);
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
        }
        out
    }
}

/// Retunes a biquad section from one sample rate to another.
///
/// The section is taken to the analog domain with the inverse bilinear
/// transform and realized again for `to_rate` with the bilinear
/// transform. The analog transfer function - the shape of the filter, its
/// cutoff frequencies, its gain - is the same in both cases, so the
/// retuned section does to a signal what the original one does, only at
/// the other sample rate. At `from_rate` the section is returned
/// unchanged.
///
/// `from_rate` and `to_rate` are in hertz. The numerator and the
/// denominator must have the same degree: a trailing zero coefficient is
/// not a degree, so a first-order section is `[b0, b1, 0]`.
pub(crate) fn retune(
    b: [f64; 3],
    a: [f64; 3],
    from_rate: u32,
    to_rate: u32,
) -> ([f64; 3], [f64; 3]) {
    if from_rate == to_rate {
        return (b, a);
    }

    let numerator = trim(&b);
    let denominator = trim(&a);
    debug_assert_eq!(
        numerator.len(),
        denominator.len(),
        "a biquad section needs a numerator and a denominator of the same degree"
    );

    // To the analog domain: z^-1 = (1 - s T / 2) / (1 + s T / 2).
    let half_sample = 0.5 / from_rate as f64;
    let analog_b = mobius(&numerator, 1.0, -half_sample, 1.0, half_sample);
    let analog_a = mobius(&denominator, 1.0, -half_sample, 1.0, half_sample);

    // And back to the digital domain: s = 2 F (1 - z^-1) / (1 + z^-1).
    let rate = 2.0 * to_rate as f64;
    let digital_b = mobius(&analog_b, rate, -rate, 1.0, 1.0);
    let digital_a = mobius(&analog_a, rate, -rate, 1.0, 1.0);

    // The denominator leads with its `a0`, which the biquad form assumes
    // to be 1.0. Both polynomials are scaled by the same factor, so the
    // ratio between them - the filter - stays the same.
    let lead = digital_a[0];
    let scaled_b: Vec<f64> = digital_b.iter().map(|value| value / lead).collect();
    let scaled_a: Vec<f64> = digital_a.iter().map(|value| value / lead).collect();
    (pad(&scaled_b), pad(&scaled_a))
}

/// Drops the trailing zero coefficients of a polynomial.
///
/// The degree of the polynomial is what the transformations work on: a
/// section whose numerator is `[b0, b1, 0]` is a first-order section, not
/// a second-order one with a zero term.
fn trim(p: &[f64; 3]) -> Vec<f64> {
    let mut out: Vec<f64> = p.to_vec();
    while out.len() > 1 && out[out.len() - 1] == 0.0 {
        out.pop();
    }
    out
}

/// Pads a polynomial to the three coefficients of a biquad.
fn pad(p: &[f64]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (slot, value) in out.iter_mut().zip(p) {
        *slot = *value;
    }
    out
}

/// The coefficients of `p` with `x = (alpha + beta y) / (gamma + delta y)`.
///
/// The result is `p(x)` multiplied by `(gamma + delta y)^n`, where `n` is
/// the degree of `p`. The factor is the same for a numerator and a
/// denominator of the same degree and cancels in their ratio.
fn mobius(p: &[f64], alpha: f64, beta: f64, gamma: f64, delta: f64) -> Vec<f64> {
    let degree = p.len() - 1;
    let mut out = vec![0.0; degree + 1];
    for (index, &coefficient) in p.iter().enumerate() {
        let left = poly_pow(&[alpha, beta], index);
        let right = poly_pow(&[gamma, delta], degree - index);
        for (slot, value) in out.iter_mut().zip(poly_mul(&left, &right)) {
            *slot += coefficient * value;
        }
    }
    out
}

/// The polynomial `base` raised to the power `exponent`, lowest power
/// first.
fn poly_pow(base: &[f64], exponent: usize) -> Vec<f64> {
    let mut result = vec![1.0];
    for _ in 0..exponent {
        result = poly_mul(&result, base);
    }
    result
}

/// The product of two polynomials, lowest power first.
fn poly_mul(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            out[i + j] += x * y;
        }
    }
    out
}

/// The magnitudes of the poles of a biquad, from its denominator.
///
/// The roots of `a0 z^2 + a1 z + a2`: two real ones or a conjugate pair.
#[cfg(test)]
pub(crate) fn pole_magnitudes(a: [f64; 3]) -> [f64; 2] {
    let discriminant = a[1] * a[1] - 4.0 * a[0] * a[2];
    if discriminant >= 0.0 {
        let root = discriminant.sqrt();
        [
            ((-a[1] + root) / (2.0 * a[0])).abs(),
            ((-a[1] - root) / (2.0 * a[0])).abs(),
        ]
    } else {
        // A conjugate pair: both roots have the magnitude of their
        // geometric mean.
        let magnitude = (a[2] / a[0]).sqrt();
        [magnitude, magnitude]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The magnitude of a polynomial with real coefficients at `e^{-jw}`.
    fn polynomial_magnitude(p: [f64; 3], w: f64) -> f64 {
        let (real, imaginary) = (
            p[0] + p[1] * w.cos() + p[2] * (2.0 * w).cos(),
            -(p[1] * w.sin() + p[2] * (2.0 * w).sin()),
        );
        (real * real + imaginary * imaginary).sqrt()
    }

    /// The magnitude of the transfer function at `frequency` hertz.
    fn magnitude(b: [f64; 3], a: [f64; 3], rate: u32, frequency: f64) -> f64 {
        let w = 2.0 * core::f64::consts::PI * frequency / rate as f64;
        polynomial_magnitude(b, w) / polynomial_magnitude(a, w)
    }

    #[test]
    fn a_section_retuned_to_its_own_rate_is_unchanged() {
        let b = [0.995527971309, -1.991055942619, 0.995527971310];
        let a = [1.0, -1.991119453430, 0.991172964241];
        let (retuned_b, retuned_a) = retune(b, a, 48_000, 48_000);
        assert_eq!(retuned_b, b);
        assert_eq!(retuned_a, a);
    }

    #[test]
    fn a_section_survives_the_round_trip_through_the_analog_domain() {
        let b = [0.995527971309, -1.991055942619, 0.995527971310];
        let a = [1.0, -1.991119453430, 0.991172964241];
        // 48 kHz -> the analog domain -> 48 kHz.
        let half_sample = 0.5 / 48_000.0;
        let analog_b = mobius(&b, 1.0, -half_sample, 1.0, half_sample);
        let analog_a = mobius(&a, 1.0, -half_sample, 1.0, half_sample);
        let rate = 2.0 * 48_000.0;
        let digital_b = mobius(&analog_b, rate, -rate, 1.0, 1.0);
        let digital_a = mobius(&analog_a, rate, -rate, 1.0, 1.0);
        let lead = digital_a[0];
        for (got, want) in digital_b.iter().zip(b.iter()) {
            assert!((got / lead - want).abs() < 1e-12, "{got} vs {want}");
        }
        for (got, want) in digital_a.iter().zip(a.iter()) {
            assert!((got / lead - want).abs() < 1e-12, "{got} vs {want}");
        }
    }

    #[test]
    fn the_retuned_section_keeps_its_response_at_the_same_frequency() {
        let b = [0.995527971309, -1.991055942619, 0.995527971310];
        let a = [1.0, -1.991119453430, 0.991172964241];
        for rate in [8_000u32, 22_050, 44_100, 96_000, 192_000] {
            let (retuned_b, retuned_a) = retune(b, a, 48_000, rate);
            for frequency in [50.0, 100.0, 1000.0, 10_000.0] {
                let reference = magnitude(b, a, 48_000, frequency);
                let retuned = magnitude(retuned_b, retuned_a, rate, frequency);
                assert!(
                    (20.0 * reference.log10() - 20.0 * retuned.log10()).abs() < 0.05,
                    "at {frequency} Hz and {rate} Hz: {reference} vs {retuned}"
                );
            }
        }
    }

    #[test]
    fn the_retuned_section_is_stable() {
        let sections = [
            (
                [0.995527971309, -1.991055942619, 0.995527971310],
                [1.0, -1.991119453430, 0.991172964241],
            ),
            (
                [0.997862928247, -0.997862928247, 0.0],
                [1.0, -0.995789367305, 0.0],
            ),
        ];
        for rate in [4_000u32, 8_000, 22_050, 44_100, 48_000, 96_000, 192_000] {
            for (b, a) in sections {
                let (retuned_b, retuned_a) = retune(b, a, 48_000, rate);
                for pole in pole_magnitudes(retuned_a) {
                    assert!(
                        pole < 1.0,
                        "a pole of {pole} is outside the unit circle at {rate} Hz"
                    );
                }
                assert_eq!(retuned_a[0], 1.0);
                assert!(retuned_b.iter().all(|value| value.is_finite()));
            }
        }
    }

    #[test]
    fn a_first_order_section_stays_first_order() {
        let b = [0.997862928247, -0.997862928247, 0.0];
        let a = [1.0, -0.995789367305, 0.0];
        let (retuned_b, retuned_a) = retune(b, a, 48_000, 44_100);
        assert_eq!(retuned_b[2], 0.0);
        assert_eq!(retuned_a[2], 0.0);
        assert_eq!(retuned_a[0], 1.0);
    }
}
