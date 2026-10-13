//! The filter: a state-variable filter in its trapezoidal ("zero delay
//! feedback") form, which stays stable however fast the cutoff is moved.

pub const FILTER_OFF: u8 = 0;
pub const FILTER_LOWPASS: u8 = 1;
pub const FILTER_HIGHPASS: u8 = 2;
pub const FILTER_BANDPASS: u8 = 3;
pub const FILTER_NOTCH: u8 = 4;

/// One 12 dB per octave stage.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    pub fn reset(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    /// Filter `buf` in place. `g` is tan(pi * cutoff / sample rate) at the
    /// first sample and moves by `dg` each sample; `k` is 1 / Q.
    /// Returns `g` as it stands after the last sample.
    pub fn process(&mut self, buf: &mut [f32], g: f32, dg: f32, k: f32, kind: u8) -> f32 {
        let mut ic1 = self.ic1;
        let mut ic2 = self.ic2;
        let mut g = g;
        macro_rules! run {
            (|$v0:ident, $v1:ident, $v2:ident| $out:expr) => {
                for x in buf.iter_mut() {
                    g += dg;
                    let a1 = 1.0 / (1.0 + g * (g + k));
                    let a2 = g * a1;
                    let a3 = g * a2;
                    let $v0 = *x;
                    let v3 = $v0 - ic2;
                    let $v1 = a1 * ic1 + a2 * v3;
                    let $v2 = ic2 + a2 * ic1 + a3 * v3;
                    ic1 = $v1 + $v1 - ic1;
                    ic2 = $v2 + $v2 - ic2;
                    *x = $out;
                }
            };
        }
        match kind {
            FILTER_LOWPASS => run!(|_v0, v1, v2| v2),
            FILTER_HIGHPASS => run!(|v0, v1, v2| v0 - k * v1 - v2),
            FILTER_BANDPASS => run!(|_v0, v1, v2| v1),
            _ => run!(|v0, v1, v2| v0 - k * v1),
        }
        // Anything wild (it should never happen) becomes silence.
        if ic1.abs() < 1.0e6 && ic2.abs() < 1.0e6 {
            self.ic1 = ic1;
            self.ic2 = ic2;
        } else {
            self.reset();
            buf.fill(0.0);
        }
        g
    }

    /// Turn numbers too small to matter into zero (they are slow to work with).
    pub fn tidy(&mut self) {
        if self.ic1.abs() < 1.0e-15 {
            self.ic1 = 0.0;
        }
        if self.ic2.abs() < 1.0e-15 {
            self.ic2 = 0.0;
        }
    }
}
