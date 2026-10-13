//! Oscillators: sine, triangle, saw and pulse waves, and the noises.
//!
//! The saw and pulse have their sharp edges rounded off (polyBLEP) and the
//! triangle its corners (polyBLAMP), so high notes do not fold back down into
//! harsh, out-of-tune tones.

use crate::math::{fast_sin, Rng};
use crate::params::{WAVE_PULSE, WAVE_SAW, WAVE_SINE, WAVE_TRIANGLE};

/// The correction to add around a step of +2 at phase 0.
/// `t` is the phase (0..1) and `dt` how far the phase moves each sample.
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// The correction around a corner, `d` samples away (0..1), for a slope that
/// changes by 1 per sample.
#[inline]
fn blamp(d: f32) -> f32 {
    if d < 1.0 {
        let x = 1.0 - d;
        x * x * x * (1.0 / 6.0)
    } else {
        0.0
    }
}

/// The plain wave with no rounding, used at the instant of a hard sync.
#[inline]
fn naive(wave: u8, t: f32, pw: f32) -> f32 {
    match wave {
        WAVE_SINE => fast_sin(t),
        WAVE_TRIANGLE => {
            let mut u = t + 0.25;
            if u >= 1.0 {
                u -= 1.0;
            }
            1.0 - 4.0 * (u - 0.5).abs()
        }
        WAVE_SAW => t + t - 1.0,
        _ => (if t < pw { 1.0 } else { -1.0 }) - (pw + pw - 1.0),
    }
}

/// One oscillator. `inc` is how far the phase moves each sample
/// (frequency / sample rate); `dinc` and `dpw` slide it and the pulse width
/// smoothly towards new values.
#[derive(Clone, Copy, Default)]
pub struct Osc {
    pub phase: f32,
    pub inc: f32,
    pub dinc: f32,
    pub pw: f32,
    pub dpw: f32,
    /// The last sample, held back one step while hard sync is on.
    z1: f32,
}

impl Osc {
    /// Start a note. A saw starts in the middle of its ramp, so that every
    /// wave starts at (or near) zero.
    pub fn reset(&mut self, wave: u8, offset: f32) {
        let mut p = offset + if wave == WAVE_SAW { 0.5 } else { 0.0 };
        if p >= 1.0 {
            p -= 1.0;
        }
        self.phase = p;
        self.z1 = 0.0;
    }

    /// For hard sync: where in each sample this oscillator starts a new cycle
    /// (how long ago, as a part of a sample), or -1 where it does not. Call
    /// before `render`; it does not move the oscillator.
    pub fn wrap_points(&self, out: &mut [f32]) {
        let mut ph = self.phase;
        let mut inc = self.inc;
        for o in out.iter_mut() {
            inc += self.dinc;
            ph += inc;
            if ph >= 1.0 {
                ph -= 1.0;
                *o = if inc > 0.0 { (ph / inc).min(1.0) } else { 0.0 };
            } else {
                *o = -1.0;
            }
        }
    }

    /// Write the next samples of the wave into `out`.
    pub fn render(&mut self, wave: u8, out: &mut [f32]) {
        let mut ph = self.phase;
        let mut inc = self.inc;
        let dinc = self.dinc;
        match wave {
            WAVE_SINE => {
                for o in out.iter_mut() {
                    inc += dinc;
                    ph += inc;
                    if ph >= 1.0 {
                        ph -= 1.0;
                    }
                    *o = fast_sin(ph);
                }
            }
            WAVE_TRIANGLE => {
                for o in out.iter_mut() {
                    inc += dinc;
                    ph += inc;
                    if ph >= 1.0 {
                        ph -= 1.0;
                    }
                    let mut u = ph + 0.25;
                    if u >= 1.0 {
                        u -= 1.0;
                    }
                    let mut y = 1.0 - 4.0 * (u - 0.5).abs();
                    if inc > 0.0 {
                        let inv = 1.0 / inc;
                        let trough = u.min(1.0 - u) * inv;
                        let peak = (u - 0.5).abs() * inv;
                        y += 8.0 * inc * (blamp(trough) - blamp(peak));
                    }
                    *o = y;
                }
            }
            WAVE_SAW => {
                for o in out.iter_mut() {
                    inc += dinc;
                    ph += inc;
                    if ph >= 1.0 {
                        ph -= 1.0;
                    }
                    *o = ph + ph - 1.0 - blep(ph, inc);
                }
            }
            _ => {
                let mut pw = self.pw;
                for o in out.iter_mut() {
                    inc += dinc;
                    pw += self.dpw;
                    ph += inc;
                    if ph >= 1.0 {
                        ph -= 1.0;
                    }
                    let mut y = if ph < pw { 1.0 } else { -1.0 };
                    y += blep(ph, inc);
                    let mut t2 = ph - pw;
                    if t2 < 0.0 {
                        t2 += 1.0;
                    }
                    y -= blep(t2, inc);
                    // Without its average, so that a pulse narrowed to
                    // nothing fades to silence instead of a thump.
                    *o = y - (pw + pw - 1.0);
                }
                self.pw = pw;
            }
        }
        self.phase = ph;
        self.inc = inc;
    }

    /// The same, hard synced: `sync` comes from the other oscillator's
    /// `wrap_points`. The output is one sample late, which lets the jump at
    /// each restart be rounded off like any other edge.
    pub fn render_synced(&mut self, wave: u8, out: &mut [f32], sync: &[f32]) {
        let mut ph = self.phase;
        let mut inc = self.inc;
        let mut pw = self.pw;
        let mut z1 = self.z1;
        for (o, &f) in out.iter_mut().zip(sync.iter()) {
            inc += self.dinc;
            pw += self.dpw;
            ph += inc;
            let y;
            let mut before = 0.0;
            if f >= 0.0 {
                // The other oscillator restarted `f` of a sample ago.
                let mut at = ph - f * inc;
                if at >= 1.0 {
                    at -= 1.0;
                }
                if at < 0.0 {
                    at = 0.0;
                }
                let step = naive(wave, 0.0, pw) - naive(wave, at, pw);
                ph = f * inc;
                let a = 1.0 - f;
                y = naive(wave, ph, pw) - step * 0.5 * a * a;
                before = step * 0.5 * f * f;
            } else {
                if ph >= 1.0 {
                    ph -= 1.0;
                }
                y = match wave {
                    WAVE_SAW => ph + ph - 1.0 - blep(ph, inc),
                    WAVE_PULSE => {
                        let mut t2 = ph - pw;
                        if t2 < 0.0 {
                            t2 += 1.0;
                        }
                        (if ph < pw { 1.0 } else { -1.0 }) + blep(ph, inc) - blep(t2, inc) - (pw + pw - 1.0)
                    }
                    _ => naive(wave, ph, pw),
                };
            }
            *o = z1 + before;
            z1 = y;
        }
        self.phase = ph;
        self.inc = inc;
        self.pw = pw;
        self.z1 = z1;
    }
}

pub const NOISE_WHITE: u8 = 0;
pub const NOISE_PINK: u8 = 1;
pub const NOISE_LFSR: u8 = 2;
pub const NOISE_LFSR_SHORT: u8 = 3;

/// The noise source of a voice.
#[derive(Clone, Copy)]
pub struct Noise {
    rng: Rng,
    pink: [f32; 3],
    /// The shift register of the chip-style noise.
    lfsr: u32,
    short: u32,
    clock: f32,
    /// How many times the shift register steps each sample.
    pub clock_inc: f32,
    held: f32,
}

impl Noise {
    pub fn new(seed: u32) -> Noise {
        Noise {
            rng: Rng::new(seed),
            pink: [0.0; 3],
            lfsr: 0x7F_FFF8,
            short: 1,
            clock: 0.0,
            clock_inc: 0.0,
            held: 0.0,
        }
    }

    /// Add the next samples of noise, times `level`, to `out`.
    pub fn render_add(&mut self, kind: u8, level: f32, out: &mut [f32]) {
        match kind {
            NOISE_WHITE => {
                for o in out.iter_mut() {
                    *o += self.rng.bipolar() * level;
                }
            }
            NOISE_PINK => {
                // White noise through three gentle low-pass filters, which
                // together tilt it to equal loudness in every octave.
                let [mut b0, mut b1, mut b2] = self.pink;
                for o in out.iter_mut() {
                    let w = self.rng.bipolar();
                    b0 = 0.99765 * b0 + w * 0.0990460;
                    b1 = 0.96300 * b1 + w * 0.2965164;
                    b2 = 0.57000 * b2 + w * 1.0526913;
                    *o += (b0 + b1 + b2 + w * 0.1848) * 0.25 * level;
                }
                self.pink = [b0, b1, b2];
            }
            NOISE_LFSR => {
                // A 23-bit shift register stepped at a rate that follows the
                // note, read eight bits at a time, as in an 8-bit sound chip.
                let inc = self.clock_inc.clamp(0.0, 4.0);
                for o in out.iter_mut() {
                    self.clock += inc;
                    let mut steps = 0;
                    while self.clock >= 1.0 && steps < 4 {
                        self.clock -= 1.0;
                        steps += 1;
                        let r = self.lfsr;
                        let bit = ((r >> 22) ^ (r >> 17)) & 1;
                        self.lfsr = ((r << 1) | bit) & 0x7F_FFFF;
                        let r = self.lfsr;
                        let byte = ((r >> 20) & 1) << 7
                            | ((r >> 18) & 1) << 6
                            | ((r >> 14) & 1) << 5
                            | ((r >> 11) & 1) << 4
                            | ((r >> 9) & 1) << 3
                            | ((r >> 5) & 1) << 2
                            | ((r >> 2) & 1) << 1
                            | (r & 1);
                        self.held = byte as f32 * (1.0 / 127.5) - 1.0;
                    }
                    *o += self.held * level;
                }
            }
            _ => {
                // A 15-bit shift register in its short, 93-step mode: a buzz.
                let inc = self.clock_inc.clamp(0.0, 4.0);
                for o in out.iter_mut() {
                    self.clock += inc;
                    let mut steps = 0;
                    while self.clock >= 1.0 && steps < 4 {
                        self.clock -= 1.0;
                        steps += 1;
                        let r = self.short;
                        let bit = (r ^ (r >> 6)) & 1;
                        self.short = (r >> 1) | (bit << 14);
                        self.held = if self.short & 1 == 1 { 1.0 } else { -1.0 };
                    }
                    *o += self.held * level;
                }
            }
        }
    }
}
