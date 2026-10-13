//! The voice effects of a part: a pitch shifter, a ring modulator and a
//! comb. A part's sound goes through them, in that order, before its
//! distortion and bit crusher. They are what turns a microphone into a
//! chipmunk, a monster or a robot, and a synth part can use them too.
//!
//! The ring modulator remembers almost nothing, so every part has one. The
//! pitch shifter and the comb each remember a stretch of sound, so there is
//! a small pool of them (`FX_UNITS`), made when the engine is made; a part
//! takes one from the pool while its `fx.pitch` or `fx.comb` is on.

use crate::engine::NUM_PARTS;
use crate::math::{fast_sin, flush, semis_to_ratio, Rng, TAU};
use crate::params::*;

/// How many parts can use the pitch shifter or the comb at the same time.
pub const FX_UNITS: usize = 8;

/// The pitch shifter looks for a splice between this long ago...
pub const SPLICE_MIN: f32 = 0.0025;
/// ...and this long ago, in seconds: one cycle of a sound from 400 Hz down to 71 Hz.
pub const SPLICE_MAX: f32 = 0.014;
/// The longest crossfade at a splice, in seconds.
pub const SPLICE_FADE: f32 = 0.004;

const NO_UNIT: u8 = 0xFF;
/// Below this a part's effects count as silent.
const QUIET: f32 = 1.0e-6;
/// The most numbers the splice search lays out in a row.
const LIN: usize = 512;
const MAX_WIN: usize = 128;

fn zeros(n: usize) -> Box<[f32]> {
    vec![0.0; n].into_boxed_slice()
}

/// A smooth curve through four neighbouring samples; `f` is how far between `p1` and `p2`.
#[inline]
fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, f: f32) -> f32 {
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * f + c2) * f + c1) * f + p1
}

/// A pitch shifter that works on the sound as it arrives.
///
/// It keeps the last few hundredths of a second and reads them back faster
/// (higher) or slower (lower) than they came in. Reading faster catches up
/// with the present, and reading slower falls behind, so every so often the
/// reader has to jump: back a little, to play a stretch twice, or forward,
/// to skip one. A jump of just any length would click or warble. So the
/// shifter looks at what it is playing and what it could jump to, and picks
/// the jump where the two look most alike (for a voice or a note, one whole
/// cycle of the wave, or a few), then crossfades over to it.
pub struct Shifter {
    left: Box<[f32]>,
    right: Box<[f32]>,
    mask: usize,
    /// How many samples have been written (the next place to write).
    write: usize,
    /// A copy of the sound at about 12 kHz, left and right together, to search in.
    dec: Box<[f32]>,
    dmask: usize,
    dwrite: usize,
    acc: f32,
    accn: usize,
    /// How far behind the present the reader is, in samples. (In double
    /// precision: it moves by a small fraction every sample, and the
    /// rounding of a single would put the pitch a few hundredths of a cent out.)
    d: f64,
    /// The same for the reader being faded out after a jump.
    d_old: f64,
    fade: u32,
    fade_len: u32,
    /// How alike the two sides of the last jump were, 0..1.
    corr: f32,
    /// The speed of reading, sliding towards what was asked for.
    ratio: f32,
    // Fixed by the sample rate.
    /// One number of `dec` for this many samples.
    dn: usize,
    dec_gain: f32,
    lag_min: usize,
    lag_max: usize,
    win: usize,
    fine: usize,
    stride: usize,
    t_min: f32,
    t_max: f32,
    x_min: f32,
    x_max: f32,
    margin: f32,
    d_max: f32,
    glide: f32,
}

impl Shifter {
    pub fn new(sample_rate: f32) -> Shifter {
        let sr = sample_rate;
        let dn = ((sr / 12000.0).round() as usize).clamp(1, 16);
        let t_min = SPLICE_MIN * sr;
        let t_max = SPLICE_MAX * sr;
        let lag_min = (t_min / dn as f32).ceil() as usize;
        let lag_max = ((t_max / dn as f32) as usize).max(lag_min);
        let win = ((0.010 * sr / dn as f32) as usize).clamp(16, MAX_WIN);
        let stride = (dn / 2).max(1);
        let fine = ((0.004 * sr) as usize / stride).clamp(16, MAX_WIN);
        let margin = (4 + 2 * dn) as f32;
        let x_max = SPLICE_FADE * sr;
        let d_max = margin + x_max + t_max + 8.0;
        let len = ((d_max + t_max) as usize + dn + fine * stride + 16).next_power_of_two();
        let dlen = (d_max as usize / dn + lag_max + win + 8).next_power_of_two();
        Shifter {
            left: zeros(len),
            right: zeros(len),
            mask: len - 1,
            write: 0,
            dec: zeros(dlen),
            dmask: dlen - 1,
            dwrite: 0,
            acc: 0.0,
            accn: 0,
            d: margin as f64,
            d_old: margin as f64,
            fade: 0,
            fade_len: 1,
            corr: 0.0,
            ratio: 1.0,
            dn,
            dec_gain: 0.5 / dn as f32,
            lag_min,
            lag_max,
            win,
            fine,
            stride,
            t_min,
            t_max,
            x_min: 0.001 * sr,
            x_max,
            margin,
            d_max,
            glide: 1.0 - (-1.0 / (0.005 * sr)).exp(),
        }
    }

    /// Forget everything heard so far.
    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.dec.fill(0.0);
        self.acc = 0.0;
        self.accn = 0;
        self.d = self.margin as f64;
        self.d_old = self.margin as f64;
        self.fade = 0;
        self.ratio = 1.0;
    }

    /// The longest the sound can be held up, in samples, when shifting by
    /// `semitones`: up to that much later, and never earlier than 1 sample.
    pub fn max_delay(sample_rate: f32, semitones: f32) -> f32 {
        let dn = ((sample_rate / 12000.0).round() as usize).clamp(1, 16);
        let margin = (4 + 2 * dn) as f32;
        let ratio = semis_to_ratio(semitones);
        // Going up, the reader keeps this far back so that the one it
        // fades out can run on; going down, the one it fades out falls
        // this much further behind before it is gone.
        let fade = (ratio - 1.0).abs() * SPLICE_FADE * sample_rate;
        margin + fade + SPLICE_MAX * sample_rate + dn as f32 + 2.0
    }

    /// The sound `d` samples ago, between samples.
    #[inline]
    fn read(buf: &[f32], mask: usize, write: usize, d: f64) -> f32 {
        let whole = d as usize;
        let t = 1.0 - (d - whole as f64) as f32;
        let at = write.wrapping_sub(1 + whole);
        cubic(
            buf[at.wrapping_sub(2) & mask],
            buf[at.wrapping_sub(1) & mask],
            buf[at & mask],
            buf[at.wrapping_add(1) & mask],
            t,
        )
    }

    /// Find the jump, between `SPLICE_MIN` and `SPLICE_MAX` long, after
    /// which the sound carries on most like it was going: further back for
    /// `up`, nearer the present otherwise. Returns its length in samples
    /// and how alike the two places are, 0..1.
    fn search(&self, up: bool) -> (f32, f32) {
        let age = self.d as usize;
        let fallback = if up { 2.0 * self.t_min } else { (0.5 * self.t_max).min(self.d as f32 - 2.0).max(0.0) };
        // First at about 12 kHz, over every length of jump. `lin` holds the
        // slow copy laid out in a row, from the reader's place (or the
        // nearest place a jump could reach) back into the past.
        let j = age.saturating_sub(self.accn) / self.dn;
        let m = self.win;
        let lo = self.lag_min;
        let hi = if up { self.lag_max } else { self.lag_max.min(j) };
        if hi < lo || hi + m > LIN {
            return (fallback, 0.0);
        }
        let mut lin = [0.0f32; LIN];
        let base = if up { j } else { j - hi };
        for (i, v) in lin[..hi + m].iter_mut().enumerate() {
            *v = self.dec[self.dwrite.wrapping_sub(1 + base + i) & self.dmask];
        }
        let a0 = if up { 0 } else { hi };
        let ea: f32 = lin[a0..a0 + m].iter().map(|v| v * v).sum();
        if ea < 1.0e-10 * m as f32 {
            // Silence: any jump will do.
            return (fallback, 0.0);
        }
        let mut best = (f32::MIN, lo);
        let lags = (hi - lo + 1) as f32;
        for lag in lo..=hi {
            let b0 = if up { lag } else { hi - lag };
            let (mut dot, mut eb) = (0.0f32, 0.0f32);
            for (x, y) in lin[a0..a0 + m].iter().zip(lin[b0..b0 + m].iter()) {
                dot += x * y;
                eb += y * y;
            }
            // Of two jumps that fit as well, the shorter holds the sound up less.
            let score = dot / (ea * eb + 1.0e-20).sqrt() - 0.03 * (lag - lo) as f32 / lags;
            if score > best.0 {
                best = (score, lag);
            }
        }
        // Then at the full rate, a few samples either side of the best.
        let dn = self.dn as isize;
        let (s, n) = (self.stride, self.fine);
        let a_age = age + 1;
        let mono = |age: usize| -> f32 {
            let i = self.write.wrapping_sub(1 + age) & self.mask;
            self.left[i] + self.right[i]
        };
        let mut a = [0.0f32; MAX_WIN];
        for (i, v) in a[..n].iter_mut().enumerate() {
            *v = mono(a_age + i * s);
        }
        let ea: f32 = a[..n].iter().map(|v| v * v).sum();
        let centre = (best.1 * self.dn) as isize;
        let mut scores = [f32::MIN; 33];
        let mut alike = [0.0f32; 33];
        let mut top = usize::MAX;
        for k in 0..(2 * dn + 1) as usize {
            let t = centre + k as isize - dn;
            if t < 1 || (!up && t + 1 > a_age as isize) {
                continue;
            }
            let b_age = if up { a_age + t as usize } else { a_age - t as usize };
            let (mut dot, mut eb) = (0.0f32, 0.0f32);
            for (i, x) in a[..n].iter().enumerate() {
                let y = mono(b_age + i * s);
                dot += x * y;
                eb += y * y;
            }
            scores[k] = dot / (eb + 1.0e-20).sqrt();
            alike[k] = dot / (ea * eb + 1.0e-20).sqrt();
            if top == usize::MAX || scores[k] > scores[top] {
                top = k;
            }
        }
        if top == usize::MAX {
            return (fallback, 0.0);
        }
        // The best fit is usually between two samples: a curve through the
        // three scores around the top says where.
        let mut lag = (centre + top as isize - dn) as f32;
        if top > 0 && top < (2 * dn) as usize && scores[top - 1] > f32::MIN && scores[top + 1] > f32::MIN {
            let (l, c, r) = (scores[top - 1], scores[top], scores[top + 1]);
            let bend = l - 2.0 * c + r;
            if bend < -1.0e-12 {
                lag += (0.5 * (l - r) / bend).clamp(-0.5, 0.5);
            }
        }
        if !up {
            lag = lag.min(self.d as f32 - 2.0).max(0.0);
        }
        (lag, alike[top].clamp(0.0, 1.0))
    }

    /// Jump, and start the crossfade from where the reader was.
    fn splice(&mut self, up: bool) {
        let (lag, corr) = self.search(up);
        self.d_old = self.d;
        self.d = if up { self.d + lag as f64 } else { self.d - lag as f64 }.clamp(1.0, self.d_max as f64);
        self.corr = corr;
        // Half the time until the next jump, within limits.
        let speed = (1.0 - self.ratio).abs().max(1.0e-3);
        self.fade_len = (0.5 * lag / speed).clamp(self.x_min, self.x_max) as u32;
        self.fade_len = self.fade_len.max(1);
        self.fade = self.fade_len;
    }

    /// Shift `l` and `r` in place by `semitones`.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], semitones: f32) {
        let target = semis_to_ratio(semitones.clamp(-12.0, 12.0));
        let mask = self.mask;
        for i in 0..l.len() {
            let w = self.write & mask;
            self.left[w] = l[i];
            self.right[w] = r[i];
            self.write = self.write.wrapping_add(1);
            self.acc += l[i] + r[i];
            self.accn += 1;
            if self.accn >= self.dn {
                self.dec[self.dwrite & self.dmask] = self.acc * self.dec_gain;
                self.dwrite = self.dwrite.wrapping_add(1);
                self.acc = 0.0;
                self.accn = 0;
            }
            self.ratio += (target - self.ratio) * self.glide;
            if (target - self.ratio).abs() < 5.0e-5 {
                // So near that the slide would stall just short of it.
                self.ratio = target;
            }
            // How far the reader falls behind each sample (below 0: it catches up).
            let drift = 1.0 - self.ratio as f64;
            if self.fade == 0 {
                if drift < 0.0 {
                    // Leave room for the old reader to run on through the crossfade.
                    if self.d < self.margin as f64 - drift * self.x_max as f64 {
                        self.splice(true);
                    }
                } else if drift > 0.0 && self.d > (self.margin + self.t_max) as f64 {
                    self.splice(false);
                }
            }
            let mut yl = Shifter::read(&self.left, mask, self.write, self.d);
            let mut yr = Shifter::read(&self.right, mask, self.write, self.d);
            if self.fade > 0 {
                let t = (self.fade_len - self.fade) as f32 / self.fade_len as f32;
                // A smooth step from 0 to 1.
                let new = 0.5 - 0.5 * fast_sin(t * 0.5 + 0.25);
                let old = 1.0 - new;
                // Two sounds that are alike add up to more than two that are
                // not; this keeps the loudness level through the fade either way.
                let norm = 1.0 / (old * old + new * new + 2.0 * old * new * self.corr).sqrt();
                let ol = Shifter::read(&self.left, mask, self.write, self.d_old);
                let or = Shifter::read(&self.right, mask, self.write, self.d_old);
                yl = (yl * new + ol * old) * norm;
                yr = (yr * new + or * old) * norm;
                self.d_old = (self.d_old + drift).clamp(1.0, self.d_max as f64);
                self.fade -= 1;
            }
            self.d = (self.d + drift).clamp(1.0, self.d_max as f64);
            l[i] = yl;
            r[i] = yr;
        }
    }
}

/// A comb: the sound added to a copy of itself from a moment ago, round and
/// round, so that it rings at one note. Swept, it is a flanger.
pub struct Comb {
    left: Box<[f32]>,
    right: Box<[f32]>,
    mask: usize,
    write: usize,
    /// The delay in samples, sliding towards what was asked for.
    delay: f32,
    slide: f32,
    phase: f32,
    sample_rate: f32,
}

/// The lowest note the comb rings at, in Hz.
pub const COMB_LOWEST: f32 = 50.0;

impl Comb {
    pub fn new(sample_rate: f32) -> Comb {
        let len = ((sample_rate / COMB_LOWEST) as usize * 2 + 8).next_power_of_two();
        Comb {
            left: zeros(len),
            right: zeros(len),
            mask: len - 1,
            write: 0,
            delay: -1.0,
            slide: 1.0 - (-1.0 / (0.005 * sample_rate)).exp(),
            phase: 0.0,
            sample_rate,
        }
    }

    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.delay = -1.0;
        self.phase = 0.0;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], mix: f32, freq: f32, feedback: f32, rate: f32, depth: f32) {
        let mask = self.mask;
        let centre = self.sample_rate / freq.max(COMB_LOWEST);
        let swing = 0.9 * depth.clamp(0.0, 1.0);
        let step = rate / self.sample_rate;
        let feedback = feedback.clamp(-0.97, 0.97);
        // A long ring is loud at its own note; this keeps the whole about as loud as it went in.
        let level = (1.0 - feedback * feedback).sqrt();
        let longest = (mask - 2) as f32;
        if self.delay < 0.0 {
            self.delay = centre;
        }
        for i in 0..l.len() {
            let mut target = centre;
            if swing > 0.0 {
                self.phase += step;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
                target *= 1.0 + swing * fast_sin(self.phase);
            }
            self.delay += (target - self.delay) * self.slide;
            let d = self.delay.clamp(1.0, longest);
            let whole = d as usize;
            let f = d - whole as f32;
            let a = self.write.wrapping_sub(whole) & mask;
            let b = a.wrapping_sub(1) & mask;
            let yl = l[i] + feedback * (self.left[a] + (self.left[b] - self.left[a]) * f);
            let yr = r[i] + feedback * (self.right[a] + (self.right[b] - self.right[a]) * f);
            self.left[self.write] = flush(yl).clamp(-64.0, 64.0);
            self.right[self.write] = flush(yr).clamp(-64.0, 64.0);
            self.write = (self.write + 1) & mask;
            l[i] += (yl * level - l[i]) * mix;
            r[i] += (yr * level - r[i]) * mix;
        }
    }
}

/// A ring modulator: the sound multiplied by a sine wave, or by a rumble of noise.
#[derive(Clone, Copy)]
pub struct Ring {
    phase: f32,
    low: f32,
    rng: Rng,
}

impl Ring {
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32], mix: f32, freq: f32, noise: bool, sample_rate: f32) {
        if noise {
            // Noise with everything above `freq` taken off, brought back up to a steady wave's power.
            let a = (1.0 - (-TAU * freq / sample_rate).exp()).clamp(1.0e-5, 1.0);
            let level = (1.5 * (2.0 - a) / a).sqrt();
            for (x, y) in l.iter_mut().zip(r.iter_mut()) {
                self.low = flush(self.low + (self.rng.bipolar() - self.low) * a);
                let m = 1.0 + mix * (self.low * level - 1.0);
                *x *= m;
                *y *= m;
            }
        } else {
            let step = freq / sample_rate;
            for (x, y) in l.iter_mut().zip(r.iter_mut()) {
                self.phase += step;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
                let m = 1.0 + mix * (fast_sin(self.phase) - 1.0);
                *x *= m;
                *y *= m;
            }
        }
    }
}

struct Unit {
    used: bool,
    shifter: Shifter,
    comb: Comb,
}

/// What a part remembers of its voice effects.
#[derive(Clone, Copy)]
struct PartFx {
    unit: u8,
    /// The shifter and the comb were running a moment ago (if not, they start clean).
    pitch_on: bool,
    comb_on: bool,
    /// The effects still have something to say after the part's voices have stopped.
    live: bool,
    /// How many more frames to keep listening for it.
    tail: u32,
    ring: Ring,
}

/// The voice effects of every part.
pub struct VoiceFx {
    sample_rate: f32,
    parts: Box<[PartFx]>,
    units: Box<[Unit]>,
    /// How long after its last sound a part's effects are kept running, in frames.
    hold: u32,
    /// How many parts are `live`.
    ringing: u32,
}

impl VoiceFx {
    pub fn new(sample_rate: f32) -> VoiceFx {
        VoiceFx {
            sample_rate,
            parts: (0..NUM_PARTS)
                .map(|i| PartFx {
                    unit: NO_UNIT,
                    pitch_on: false,
                    comb_on: false,
                    live: false,
                    tail: 0,
                    ring: Ring { phase: 0.0, low: 0.0, rng: Rng::new(0x51CE_0001 + i as u32) },
                })
                .collect(),
            units: (0..FX_UNITS)
                .map(|_| Unit { used: false, shifter: Shifter::new(sample_rate), comb: Comb::new(sample_rate) })
                .collect(),
            hold: (0.06 * sample_rate) as u32,
            ringing: 0,
        }
    }

    /// Silence everything at once. Not for the middle of a render.
    pub fn clear(&mut self) {
        for (i, p) in self.parts.iter_mut().enumerate() {
            p.pitch_on = false;
            p.comb_on = false;
            p.live = false;
            p.tail = 0;
            p.ring = Ring { phase: 0.0, low: 0.0, rng: Rng::new(0x51CE_0001 + i as u32) };
        }
        for u in self.units.iter_mut() {
            u.shifter.clear();
            u.comb.clear();
        }
        self.ringing = 0;
    }

    /// Are a part's effects still sounding after its voices have stopped?
    /// They are let go at the start of a control period (`boundary`), so
    /// that the moment does not depend on how a host slices time.
    #[inline]
    pub fn live(&mut self, part: usize, boundary: bool) -> bool {
        if self.ringing == 0 {
            return false;
        }
        let s = &mut self.parts[part];
        if s.live && boundary && s.tail == 0 {
            s.live = false;
            self.ringing -= 1;
        }
        s.live
    }

    /// Put a part's sound through its voice effects. `driven` says the part
    /// has voices (or the live input) sounding in these frames.
    pub fn process(&mut self, part: usize, p: &[f32; PART_PARAMS], l: &mut [f32], r: &mut [f32], driven: bool) {
        let (pitch, ring, comb) = (p[FX_PITCH], p[FX_RING], p[FX_COMB]);
        let s = &mut self.parts[part];
        if pitch == 0.0 && comb <= 0.0 {
            // The part has no use for a shifter or a comb: back to the pool.
            if s.unit != NO_UNIT {
                self.units[s.unit as usize].used = false;
                s.unit = NO_UNIT;
            }
            s.pitch_on = false;
            s.comb_on = false;
            self.ringing -= s.live as u32;
            s.live = false;
            s.tail = 0;
            if ring > 0.0 {
                s.ring.process(l, r, ring, p[FX_RING_FREQ], p[FX_RING_WAVE] >= 0.5, self.sample_rate);
            }
            return;
        }
        if s.unit == NO_UNIT {
            if let Some(i) = self.units.iter().position(|u| !u.used) {
                self.units[i].used = true;
                s.unit = i as u8;
            }
        }
        let unit = self.units.get_mut(s.unit as usize);
        let has_unit = unit.is_some();
        let (mut shifter, mut combs) = match unit {
            Some(u) => (Some(&mut u.shifter), Some(&mut u.comb)),
            None => (None, None),
        };
        match shifter.as_mut() {
            Some(sh) if pitch != 0.0 => {
                if !s.pitch_on {
                    sh.clear();
                    s.pitch_on = true;
                }
                sh.process(l, r, pitch);
            }
            _ => s.pitch_on = false,
        }
        if ring > 0.0 {
            s.ring.process(l, r, ring, p[FX_RING_FREQ], p[FX_RING_WAVE] >= 0.5, self.sample_rate);
        }
        match combs.as_mut() {
            Some(c) if comb > 0.0 => {
                if !s.comb_on {
                    c.clear();
                    s.comb_on = true;
                }
                c.process(l, r, comb, p[FX_COMB_FREQ], p[FX_COMB_FEEDBACK], p[FX_COMB_RATE], p[FX_COMB_DEPTH]);
            }
            _ => s.comb_on = false,
        }
        if !has_unit {
            return;
        }
        // Count the frames since the effects last made a sound.
        if driven {
            s.tail = self.hold;
        } else {
            let n = l.len();
            match (0..n).rev().find(|&i| l[i].abs() + r[i].abs() >= QUIET) {
                Some(i) => s.tail = self.hold.saturating_sub((n - 1 - i) as u32),
                None => s.tail = s.tail.saturating_sub(n as u32),
            }
        }
        if s.tail > 0 && !s.live {
            s.live = true;
            self.ringing += 1;
        }
    }
}
