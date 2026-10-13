//! Effects: the echo and chorus every part can send to, each part's own
//! distortion and bit crusher, and the limiter on the way out. The reverb
//! is in `crate::reverb`.

use crate::math::{fast_sin, fast_tanh, flush};

fn zeros(n: usize) -> Box<[f32]> {
    vec![0.0; n].into_boxed_slice()
}

/// A stereo echo with a low-pass filter in its feedback, so each repeat is
/// duller than the last.
pub struct Delay {
    left: Box<[f32]>,
    right: Box<[f32]>,
    mask: usize,
    write: usize,
    /// The delay in samples, sliding towards the time asked for.
    time: f32,
    slide: f32,
    lp: [f32; 2],
}

impl Delay {
    pub fn new(sample_rate: f32, max_seconds: f32) -> Delay {
        let len = ((sample_rate * max_seconds) as usize + 8).next_power_of_two();
        Delay {
            left: zeros(len),
            right: zeros(len),
            mask: len - 1,
            write: 0,
            time: -1.0,
            slide: 1.0 - (-1.0 / (0.05 * sample_rate)).exp(),
            lp: [0.0; 2],
        }
    }

    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.lp = [0.0; 2];
    }

    /// `time` in samples, `tone` the filter's coefficient (0..1). Adds the
    /// echoes to `out`.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        time: f32,
        feedback: f32,
        tone: f32,
        pingpong: bool,
    ) {
        let max = (self.mask - 4) as f32;
        let time = time.clamp(1.0, max);
        if self.time < 0.0 {
            self.time = time;
        }
        let mask = self.mask;
        for i in 0..in_l.len() {
            self.time += (time - self.time) * self.slide;
            let whole = self.time as usize;
            let f = self.time - whole as f32;
            let a = self.write.wrapping_sub(whole) & mask;
            let b = a.wrapping_sub(1) & mask;
            let dl = self.left[a] + (self.left[b] - self.left[a]) * f;
            let dr = self.right[a] + (self.right[b] - self.right[a]) * f;
            self.lp[0] = flush(self.lp[0] + (dl - self.lp[0]) * tone);
            self.lp[1] = flush(self.lp[1] + (dr - self.lp[1]) * tone);
            if pingpong {
                self.left[self.write] = 0.5 * (in_l[i] + in_r[i]) + self.lp[1] * feedback;
                self.right[self.write] = self.lp[0] * feedback;
            } else {
                self.left[self.write] = in_l[i] + self.lp[0] * feedback;
                self.right[self.write] = in_r[i] + self.lp[1] * feedback;
            }
            self.write = (self.write + 1) & mask;
            out_l[i] += self.lp[0];
            out_r[i] += self.lp[1];
        }
    }
}

/// A chorus: two copies of the sound, each a little late by an amount that
/// drifts, one left and one right.
pub struct Chorus {
    left: Box<[f32]>,
    right: Box<[f32]>,
    mask: usize,
    write: usize,
    phase: f32,
    sample_rate: f32,
}

impl Chorus {
    pub fn new(sample_rate: f32) -> Chorus {
        let len = ((sample_rate * 0.05) as usize + 8).next_power_of_two();
        Chorus { left: zeros(len), right: zeros(len), mask: len - 1, write: 0, phase: 0.0, sample_rate }
    }

    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
    }

    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        rate: f32,
        depth: f32,
    ) {
        let mask = self.mask;
        let base = 0.012 * self.sample_rate;
        let swing = 0.006 * self.sample_rate * depth.clamp(0.0, 1.0);
        let step = rate / self.sample_rate;
        for i in 0..in_l.len() {
            self.phase += step;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let mut quarter = self.phase + 0.25;
            if quarter >= 1.0 {
                quarter -= 1.0;
            }
            self.left[self.write] = in_l[i];
            self.right[self.write] = in_r[i];
            let tl = base + swing * fast_sin(self.phase);
            let tr = base + swing * fast_sin(quarter);
            let (wl, wr) = (tl as usize, tr as usize);
            let (fl, fr) = (tl - wl as f32, tr - wr as f32);
            let a = self.write.wrapping_sub(wl) & mask;
            let b = a.wrapping_sub(1) & mask;
            out_l[i] += self.left[a] + (self.left[b] - self.left[a]) * fl;
            let a = self.write.wrapping_sub(wr) & mask;
            let b = a.wrapping_sub(1) & mask;
            out_r[i] += self.right[a] + (self.right[b] - self.right[a]) * fr;
            self.write = (self.write + 1) & mask;
        }
    }
}

/// A part's distortion and bit crusher.
#[derive(Clone, Copy, Default)]
pub struct Crusher {
    held: [f32; 2],
    count: u32,
}

impl Crusher {
    /// `drive` 0..1, `bits` 1..16 (16 is off), `every` 1..64 (1 is off).
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], drive: f32, bits: f32, every: u32) {
        if drive > 0.0 {
            let gain = 1.0 + 30.0 * drive * drive;
            let trim = 1.0 / gain.sqrt().sqrt();
            for x in l.iter_mut().chain(r.iter_mut()) {
                *x = fast_tanh(*x * gain) * trim;
            }
        }
        if bits < 16.0 {
            let steps = (bits - 1.0).max(0.0).exp2();
            let inv = 1.0 / steps;
            for x in l.iter_mut().chain(r.iter_mut()) {
                *x = (*x * steps).round() * inv;
            }
        }
        if every > 1 {
            for i in 0..l.len() {
                if self.count == 0 {
                    self.held = [l[i], r[i]];
                }
                self.count += 1;
                if self.count >= every {
                    self.count = 0;
                }
                l[i] = self.held[0];
                r[i] = self.held[1];
            }
        } else {
            self.count = 0;
        }
    }
}

/// The limiter on the output. It turns the sound down the instant it would
/// pass the ceiling and lets it back up slowly, so nothing ever clips, and it
/// adds no delay.
pub struct Limiter {
    follow: f32,
    release: f32,
}

pub const CEILING: f32 = 0.98;

impl Limiter {
    pub fn new(sample_rate: f32) -> Limiter {
        Limiter { follow: 0.0, release: (-1.0 / (0.12 * sample_rate)).exp() }
    }

    pub fn clear(&mut self) {
        self.follow = 0.0;
    }

    /// Returns the sample pair, limited (if `on`) and never outside -1..1.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32, on: bool) -> (f32, f32) {
        // Anything that is not a number becomes silence.
        let l = if l.is_finite() { l } else { 0.0 };
        let r = if r.is_finite() { r } else { 0.0 };
        if on {
            let peak = l.abs().max(r.abs());
            self.follow = flush((self.follow * self.release).max(peak));
            if self.follow > CEILING {
                let g = CEILING / self.follow;
                return (l * g, r * g);
            }
            (l, r)
        } else {
            (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
        }
    }
}
