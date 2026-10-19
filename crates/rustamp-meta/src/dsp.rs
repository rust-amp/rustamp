//! A biquad filter.
//!
//! Both loudness measurements of this crate are chains of biquads: the
//! ReplayGain 1.0 filter (a Yule shelf and a Butterworth high-pass) and
//! the K-weighting of ITU-R BS.1770-4 (a high-shelf and the RLB
//! high-pass). The filter lives here so that both use the same one.

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
