//! The LFO: a slow wave that moves other things (pitch for vibrato, loudness
//! for tremolo, the filter for a wah).

use crate::math::{fast_sin, Rng};

pub const LFO_SINE: u8 = 0;
pub const LFO_TRIANGLE: u8 = 1;
pub const LFO_SAW: u8 = 2;
pub const LFO_SQUARE: u8 = 3;

/// The value of an LFO wave at `phase` (0..1); `held` is the current random step.
#[inline]
pub fn lfo_wave(wave: u8, phase: f32, held: f32) -> f32 {
    match wave {
        LFO_SINE => fast_sin(phase),
        LFO_TRIANGLE => {
            if phase < 0.25 {
                4.0 * phase
            } else if phase < 0.75 {
                2.0 - 4.0 * phase
            } else {
                4.0 * phase - 4.0
            }
        }
        LFO_SAW => 1.0 - 2.0 * phase,
        LFO_SQUARE => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => held,
    }
}

/// The settings of one LFO, read from the patch.
#[derive(Clone, Copy)]
pub struct LfoShape {
    pub wave: u8,
    pub rate: f32,
    pub delay: f32,
    pub fade: f32,
    pub sync: bool,
}

/// A part's free-running LFO, which the voices share when sync is off.
#[derive(Clone, Copy, Default)]
pub struct FreeLfo {
    pub phase: f32,
    pub held: f32,
}

impl FreeLfo {
    #[inline]
    pub fn advance(&mut self, cycles: f32, rng: &mut Rng) {
        self.phase += cycles;
        if self.phase >= 1.0 {
            self.phase -= self.phase.floor();
            self.held = rng.bipolar();
        }
    }
}

/// A voice's own LFO.
#[derive(Clone, Copy, Default)]
pub struct Lfo {
    phase: f32,
    held: f32,
    /// Samples since the note started.
    age: u32,
}

impl Lfo {
    pub fn reset(&mut self, rng: &mut Rng) {
        self.phase = 0.0;
        self.held = rng.bipolar();
        self.age = 0;
    }

    /// Step on `n` samples and give the value, from -1 to 1.
    pub fn tick(&mut self, n: u32, s: &LfoShape, free: &FreeLfo, inv_sr: f32, rng: &mut Rng) -> f32 {
        self.age = self.age.saturating_add(n);
        let value = if s.sync {
            // The wave starts when the delay is over, not when the note does.
            let since = (self.age as f32 - s.delay / inv_sr).max(0.0);
            self.phase += since.min(n as f32) * s.rate * inv_sr;
            if self.phase >= 1.0 {
                self.phase -= self.phase.floor();
                self.held = rng.bipolar();
            }
            lfo_wave(s.wave, self.phase, self.held)
        } else {
            lfo_wave(s.wave, free.phase, free.held)
        };
        let t = self.age as f32 * inv_sr - s.delay;
        if t <= 0.0 && s.delay > 0.0 {
            0.0
        } else if s.fade > 0.0 && t < s.fade {
            value * (t.max(0.0) / s.fade)
        } else {
            value
        }
    }
}
