//! The reverb every part can send to.
//!
//! Four of its five types are one structure with different numbers: a
//! feedback delay network (Jot and Chaigne, 1991). Eight delay lines of
//! lengths that share no simple ratio feed each other through a Hadamard
//! matrix, which loses no energy and sends every line into every other, so
//! the echoes multiply until they are a smooth wash. On the way in the sound
//! passes a low cut, a pre-delay and four all-pass filters a side (Schroeder,
//! 1962) that smear a click into a burst; a few early reflections are tapped
//! from the pre-delay. In the loop each line has a filter that makes treble
//! (and, by a different amount, bass) die away at its own rate and an
//! all-pass filter that thickens the echoes, and every other line's length
//! drifts slowly by a few hundredths of a millisecond, which keeps any one
//! pitch from ringing on without being heard as a chorus.
//!
//! The fifth type is a spring: a long chain of all-pass filters in a loop
//! with a delay, as in a spring tank, where the high part of a sound travels
//! slower than the low part and every echo is a little "boing".
//!
//! Nothing here allocates after `Reverb::new`. What it puts out depends only
//! on what it was given, never on how the frames were sliced into calls.

use crate::math::{fast_sin, flush, TAU};

/// How many delay lines the network has.
pub const LINES: usize = 8;
const DIFFUSERS: usize = 4;
const TAPS: usize = 6;
/// The lengths below are in frames at this sample rate.
const REF_RATE: f32 = 48000.0;
/// How often, in frames, the slow things are looked at again.
const TICK: usize = 32;

/// `reverb.size` 0 to 1 makes every length this many times its own, 1 at 0.5.
pub const SCALE_SMALLEST: f32 = 0.6;
pub const SCALE_BIGGEST: f32 = 1.4;
/// With `reverb.decay` at 0 the tail lasts the type's own time at size 0.5,
/// and this many times shorter at size 0 and longer at size 1.
pub const SIZE_DECAY: f32 = 3.0;
/// The shortest and longest tail `reverb.decay` can ask for, in seconds.
pub const DECAY_MIN: f32 = 0.1;
pub const DECAY_MAX: f32 = 30.0;
/// The longest `reverb.predelay`, in seconds.
pub const PREDELAY_MAX: f32 = 0.2;
/// Once nothing is coming in and the reverb has been quieter than this for
/// as long as its longest echo, it empties itself and stops working.
pub const QUIET: f32 = 1.0e-6;
/// A number far too small to hear that is added to what comes in while the
/// reverb is working: no number inside it can then creep down among the
/// very small ones (denormals) that make some processors crawl. When the
/// reverb empties itself and stops, nothing is added and nothing comes out.
const AWAKE: f32 = 1.0e-18;
/// A change of type fades the old one out and the new one in, each over this long.
pub const TYPE_FADE: f32 = 0.02;
/// Where the bass part of the damping turns over, in Hz.
const BASS_HZ: f32 = 180.0;

/// The spring: how many all-pass filters each of its two springs has, their
/// coefficients, and the frequency its chirps rise to.
const SPRINGS: usize = 2;
pub const SPRING_STAGES: usize = 30;
const SPRING_COEF: [f32; SPRINGS] = [0.70, 0.74];
pub const SPRING_HZ: f32 = 4200.0;

/// One type of reverb: the numbers that make the structure a room or a hall.
pub struct Kind {
    pub name: &'static str,
    /// How long the tail takes to fall by 60 dB at size 0.5, in seconds.
    pub decay: f32,
    /// The delay lines' lengths (the spring uses the first two).
    lines: [u16; LINES],
    /// The all-pass filter in each line: lengths, and how strong.
    thicken: [u16; LINES],
    thickness: f32,
    /// The all-pass filters on the way in, left and right, and how strong.
    diffusers: [[u16; DIFFUSERS]; 2],
    diffusion: f32,
    /// The gap before the tail starts, in seconds.
    pub predelay: f32,
    /// When the early reflections arrive, left and right, in milliseconds, and how loud they are.
    early: [[f32; TAPS]; 2],
    early_level: f32,
    /// How much of the smeared sound goes straight out, ahead of the tail.
    bloom: f32,
    /// Above this the tail dies away faster, by as much as `reverb.damping` says.
    pub treble_hz: f32,
    /// How many times longer than the middle the bass lasts.
    pub bass: f32,
    /// How far each line's length drifts, in seconds, and about how fast, in Hz.
    drift: f32,
    drift_hz: f32,
    gain: f32,
}

pub const ROOM: usize = 0;
pub const HALL: usize = 1;
pub const PLATE: usize = 2;
pub const CATHEDRAL: usize = 3;
pub const SPRING: usize = 4;

const EARLY_GAINS: [[f32; TAPS]; 2] = [[0.9, -0.75, 0.65, 0.5, -0.42, 0.33], [-0.85, 0.7, 0.6, -0.5, 0.4, -0.3]];
/// How much of each line left and right hear. The two have nothing in
/// common, so left and right are unlike each other and their sum loses
/// nothing; and they are chosen so that what is heard now has as little as
/// can be in common with what any one line brings back a moment later
/// (a tenth, where the plain sum of the lines has a third).
const OUT: [[f32; LINES]; 2] = [
    [0.632, -1.265, -1.265, -0.632, -1.265, -0.632, -0.632, 1.265],
    [0.632, -1.265, 1.265, 0.632, 1.265, 0.632, -0.632, 1.265],
];
/// Each line drifts at its own speed: the type's, times this.
const DRIFT_RATES: [f32; LINES] = [1.0, 1.31, 0.77, 1.53, 0.61, 1.19, 0.89, 1.71];
const DRIFT_PHASES: [f32; LINES] = [0.0, 0.37, 0.71, 0.13, 0.59, 0.23, 0.83, 0.47];

/// The types, in the order of the choices of `reverb.type`. Every length is
/// a prime number of frames at 48 kHz, chosen so that no two are near a
/// simple ratio and no echo lands on another.
pub const KINDS: [Kind; 5] = [
    Kind {
        name: "room",
        decay: 1.0,
        lines: [839, 997, 1277, 1483, 1933, 2153, 2819, 3271],
        thicken: [283, 347, 383, 467, 523, 587, 709, 787],
        thickness: 0.5,
        diffusers: [[67, 103, 157, 239], [73, 127, 181, 293]],
        diffusion: 0.65,
        predelay: 0.004,
        early: [[2.9, 7.1, 11.3, 16.7, 22.9, 29.3], [4.1, 8.3, 13.1, 18.9, 25.1, 32.3]],
        early_level: 0.4,
        bloom: 0.45,
        treble_hz: 5000.0,
        bass: 1.0,
        drift: 0.00006,
        drift_hz: 1.0,
        gain: 0.397,
    },
    Kind {
        name: "hall",
        decay: 2.6,
        lines: [1847, 2083, 2347, 2647, 3041, 3617, 4177, 4793],
        thicken: [431, 509, 557, 631, 773, 853, 1039, 1129],
        thickness: 0.55,
        diffusers: [[109, 173, 293, 487], [137, 233, 359, 571]],
        diffusion: 0.7,
        predelay: 0.018,
        early: [[8.3, 14.9, 22.7, 31.9, 43.1, 55.7], [10.1, 17.3, 25.9, 36.1, 47.9, 59.3]],
        early_level: 0.3,
        bloom: 0.45,
        treble_hz: 4000.0,
        bass: 1.15,
        drift: 0.00008,
        drift_hz: 0.5,
        gain: 0.592,
    },
    Kind {
        name: "plate",
        decay: 1.8,
        lines: [457, 557, 773, 1039, 1409, 1789, 2267, 2927],
        thicken: [211, 251, 283, 311, 367, 409, 487, 541],
        thickness: 0.6,
        diffusers: [[53, 89, 149, 251], [67, 113, 211, 349]],
        diffusion: 0.75,
        predelay: 0.0,
        early: [[0.0; TAPS]; 2],
        early_level: 0.0,
        bloom: 0.6,
        treble_hz: 9000.0,
        bass: 0.9,
        drift: 0.00006,
        drift_hz: 0.9,
        gain: 0.399,
    },
    Kind {
        name: "cathedral",
        decay: 6.5,
        lines: [3049, 3251, 3469, 3877, 4217, 4591, 5039, 5297],
        thicken: [431, 509, 557, 631, 773, 853, 1039, 1129],
        thickness: 0.65,
        diffusers: [[157, 241, 373, 557], [173, 293, 491, 719]],
        diffusion: 0.75,
        predelay: 0.03,
        early: [[11.3, 19.7, 29.9, 37.1, 47.3, 58.9], [13.7, 23.3, 27.7, 41.9, 51.1, 59.9]],
        early_level: 0.22,
        bloom: 0.45,
        treble_hz: 3000.0,
        bass: 1.25,
        drift: 0.00004,
        drift_hz: 0.35,
        gain: 0.795,
    },
    Kind {
        name: "spring",
        decay: 2.0,
        lines: [2099, 2707, 0, 0, 0, 0, 0, 0],
        thicken: [0; LINES],
        thickness: 0.0,
        diffusers: [[0; DIFFUSERS]; 2],
        diffusion: 0.0,
        predelay: 0.0,
        early: [[0.0; TAPS]; 2],
        early_level: 0.0,
        bloom: 0.0,
        treble_hz: 3000.0,
        bass: 1.0,
        drift: 0.00002,
        drift_hz: 0.8,
        gain: 0.9,
    },
];

/// What the engine's parameters ask of the reverb.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// `reverb.type`: an index into `KINDS`.
    pub kind: usize,
    /// `reverb.size`, 0 to 1.
    pub size: f32,
    /// `reverb.damping`, 0 to 1.
    pub damping: f32,
    /// `reverb.decay`: seconds, or 0 for the type's own.
    pub decay: f32,
    /// `reverb.predelay`, in seconds.
    pub predelay: f32,
    /// `reverb.low_cut`, in Hz.
    pub low_cut: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { kind: ROOM, size: 0.5, damping: 0.4, decay: 0.0, predelay: 0.0, low_cut: 120.0 }
    }
}

/// How many times its own length `reverb.size` makes everything.
pub fn scale_of(size: f32) -> f32 {
    SCALE_SMALLEST + (SCALE_BIGGEST - SCALE_SMALLEST) * size.clamp(0.0, 1.0)
}

/// How long the tail takes to fall by 60 dB, in seconds: `decay` if it is
/// not 0, and otherwise the type's own time, three times shorter at size 0
/// and three times longer at size 1.
pub fn rt60(kind: usize, size: f32, decay: f32) -> f32 {
    if decay > 0.0 {
        decay.clamp(DECAY_MIN, DECAY_MAX)
    } else {
        KINDS[kind.min(KINDS.len() - 1)].decay * SIZE_DECAY.powf(2.0 * size.clamp(0.0, 1.0) - 1.0)
    }
}

/// How many times shorter than the middle the treble lasts, for `reverb.damping`.
pub fn treble_ratio(damping: f32) -> f32 {
    let d = damping.clamp(0.0, 1.0);
    1.0 / (1.0 + 7.5 * d * d)
}

/// A delay line: a run of the reverb's memory, a power of two long.
#[derive(Clone, Copy, Default)]
struct Span {
    at: usize,
    mask: usize,
}

impl Span {
    /// What went in `delay` frames before frame `w`, which is not yet
    /// written. Between frames it is worked out from the four nearest, by
    /// the cubic that passes through them (Lagrange): that takes far less
    /// treble off than a straight line does, and never adds any.
    #[inline(always)]
    fn read(self, mem: &[f32], w: usize, delay: f32) -> f32 {
        let whole = delay as usize;
        let f = delay - whole as f32;
        let at = w.wrapping_sub(whole);
        let x0 = mem[self.at + (at.wrapping_add(1) & self.mask)];
        let x1 = mem[self.at + (at & self.mask)];
        let x2 = mem[self.at + (at.wrapping_sub(1) & self.mask)];
        let x3 = mem[self.at + (at.wrapping_sub(2) & self.mask)];
        let (a, b, c) = (f + 1.0, f - 1.0, f - 2.0);
        let (fb, ac) = (f * b, a * c);
        (1.0 / 6.0) * (a * fb * x3 - fb * c * x0) + 0.5 * (ac * b * x1 - ac * f * x2)
    }

    /// The same for a whole number of frames.
    #[inline(always)]
    fn whole(self, mem: &[f32], w: usize, delay: usize) -> f32 {
        mem[self.at + (w.wrapping_sub(delay) & self.mask)]
    }

    /// The same between the two nearest frames only: `whole` frames back
    /// and `f` of the way to the one before.
    #[inline(always)]
    fn tap(self, mem: &[f32], w: usize, whole: usize, f: f32) -> f32 {
        let at = w.wrapping_sub(whole);
        let a = mem[self.at + (at & self.mask)];
        let b = mem[self.at + (at.wrapping_sub(1) & self.mask)];
        a + (b - a) * f
    }

    #[inline(always)]
    fn write(self, mem: &mut [f32], w: usize, x: f32) {
        mem[self.at + (w & self.mask)] = x;
    }

    /// An all-pass filter `len` frames long: every frequency comes out as
    /// loud as it went in, but spread out in time (Schroeder).
    #[inline(always)]
    fn allpass(self, mem: &mut [f32], w: usize, len: usize, gain: f32, x: f32) -> f32 {
        let b = mem[self.at + (w.wrapping_sub(len) & self.mask)];
        let v = x - gain * b;
        mem[self.at + (w & self.mask)] = v;
        b + gain * v
    }
}

/// A delay as whole frames and the fraction left over.
#[inline(always)]
fn split(delay: f32) -> (usize, f32) {
    let whole = delay as usize;
    (whole, delay - whole as f32)
}

/// Mix eight signals so that each goes equally into all eight and no energy
/// is gained or lost (a Hadamard matrix, as three rounds of sums and differences).
#[inline(always)]
fn hadamard(x: &mut [f32; LINES]) {
    for step in [1usize, 2, 4] {
        let mut i = 0;
        while i < LINES {
            for j in i..i + step {
                let (a, b) = (x[j], x[j + step]);
                x[j] = a + b;
                x[j + step] = a - b;
            }
            i += 2 * step;
        }
    }
    for v in x.iter_mut() {
        *v *= 0.353_553_4;
    }
}

/// Everything the reverb keeps from one frame to the next, apart from the
/// sound in its delay lines.
struct Core {
    w: usize,
    low: [f32; 2],
    low_k: f32,
    pre: [Span; 2],
    /// `reverb.predelay` in frames, and the size, each sliding to what is asked.
    wait: f32,
    wait_target: f32,
    scale: f32,
    scale_target: f32,
    scale_step: f32,
    early: bool,
    early_at: [[f32; TAPS]; 2],
    early_gain: [[f32; TAPS]; 2],
    /// Where the taps are while nothing slides, worked out once.
    settled: bool,
    taps: [[(usize, f32); TAPS + 1]; 2],
    tail_at: f32,
    diffusers: [[Span; DIFFUSERS]; 2],
    diffuser_len: [[usize; DIFFUSERS]; 2],
    diffusion: f32,
    bloom: f32,

    lines: [Span; LINES],
    /// Each line's length in frames, sliding to what the size asks for,
    /// and how far it may move in a frame.
    len: [f32; LINES],
    len_target: [f32; LINES],
    len_step: [f32; LINES],
    drift: [f32; LINES],
    drift_inc: [f32; LINES],
    thicken: [Span; LINES],
    thicken_len: [usize; LINES],
    thickness: f32,
    treble: [f32; LINES],
    bass: [f32; LINES],
    treble_k: f32,
    bass_k: f32,
    /// Each line's gain, how much of it the treble keeps, and how much more the bass has.
    g: [f32; LINES],
    keep: [f32; LINES],
    more: [f32; LINES],

    /// Where the springs' chains are in the memory, and how far apart in
    /// frames the two ends of each of their filters are.
    spring: usize,
    stretch: usize,
    spring_at: usize,
    spring_in: [[f32; 2]; SPRINGS],
    spring_k: f32,

    /// How loud it all is, easing to what is asked.
    out: f32,
    out_target: f32,
    out_ease: f32,
    /// The fade of a change of type: 1 is fully there.
    fade: f32,
    fade_step: f32,
    leaving: bool,
    /// Frames in a row with nothing coming in and next to nothing going out.
    quiet: u32,
    quiet_after: u32,
    /// The last frame's output, to notice anything going wrong.
    last: f32,
}

/// Why a run of frames ended.
#[derive(PartialEq)]
enum Stop {
    /// It was all done.
    Done,
    /// The old type has faded right out: time for the new one.
    Faded,
    /// The tail is over: time to empty everything and stop.
    Over,
}

impl Core {
    /// Whether the size or the pre-delay is on its way somewhere.
    fn sliding(&self) -> bool {
        self.scale != self.scale_target || self.wait != self.wait_target || self.len != self.len_target
    }

    /// Where the pre-delay is tapped, for the size and pre-delay as they are.
    #[inline(always)]
    fn place_taps(&mut self) {
        for side in 0..2 {
            for t in 0..TAPS {
                self.taps[side][t] = split(self.wait + self.early_at[side][t] * self.scale);
            }
            self.taps[side][TAPS] = split(self.wait + self.tail_at * self.scale);
        }
    }

    /// The way in, for up to `TICK` frames: the low cut, the pre-delay and
    /// the early reflections. `pl` and `pr` get what goes on to the tail,
    /// `el` and `er` the early reflections, and `fades` how far a change of
    /// type has faded at each frame. It stops after the frame on which the
    /// old type has faded right out, and says how many frames it did.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn way_in(&mut self, mem: &mut [f32], sliding: bool, in_l: &[f32], in_r: &[f32], pl: &mut [f32], pr: &mut [f32], el: &mut [f32], er: &mut [f32], fades: &mut [f32]) -> usize {
        if sliding {
            self.settled = false;
        } else if !self.settled {
            self.settled = true;
            self.place_taps();
        }
        for i in 0..in_l.len() {
            let w = self.w.wrapping_add(i);
            let (xl, xr) = (in_l[i], in_r[i]);
            self.low[0] = flush(self.low[0] + self.low_k * (xl - self.low[0]));
            self.low[1] = flush(self.low[1] + self.low_k * (xr - self.low[1]));
            self.pre[0].write(mem, w, xl - self.low[0] + AWAKE);
            self.pre[1].write(mem, w, xr - self.low[1] + AWAKE);
            if sliding {
                // The size and the pre-delay slide to where they are wanted.
                self.scale += (self.scale_target - self.scale).clamp(-self.scale_step, self.scale_step);
                self.wait += (self.wait_target - self.wait).clamp(-0.3, 0.3);
                self.place_taps();
            }
            // A change of type: the old one fades out, and the new one
            // starts empty and has the sound faded into it as well as out
            // of it, so that nothing starts or stops with a click.
            if self.leaving {
                self.fade = (self.fade - self.fade_step).max(0.0);
            } else if self.fade < 1.0 {
                self.fade = (self.fade + self.fade_step).min(1.0);
            }
            fades[i] = self.fade;
            let (tl, tr) = (self.taps[0][TAPS], self.taps[1][TAPS]);
            pl[i] = self.fade * self.pre[0].tap(mem, w, tl.0, tl.1);
            pr[i] = self.fade * self.pre[1].tap(mem, w, tr.0, tr.1);
            let (mut l, mut r) = (0.0, 0.0);
            if self.early {
                for t in 0..TAPS {
                    let (a, b) = (self.taps[0][t], self.taps[1][t]);
                    l += self.early_gain[0][t] * self.pre[0].tap(mem, w, a.0, a.1);
                    r += self.early_gain[1][t] * self.pre[1].tap(mem, w, b.0, b.1);
                }
            }
            el[i] = l;
            er[i] = r;
            if self.leaving && self.fade <= 0.0 {
                return i + 1;
            }
        }
        in_l.len()
    }

    /// The network: `pl` and `pr` go in, and the tail is added to `el` and `er`.
    #[inline(never)]
    fn network(&mut self, mem: &mut [f32], sliding: bool, pl: &[f32], pr: &[f32], el: &mut [f32], er: &mut [f32]) {
        let (g, keep, more, drift_inc) = (self.g, self.keep, self.more, self.drift_inc);
        let (mut treble, mut bass, mut drift, mut len) = (self.treble, self.bass, self.drift, self.len);
        let (treble_k, bass_k) = (self.treble_k, self.bass_k);
        let mut whole: [usize; LINES] = core::array::from_fn(|j| len[j] as usize);
        for i in 0..pl.len() {
            let w = self.w.wrapping_add(i);
            let (mut dl, mut dr) = (pl[i], pr[i]);
            for d in 0..DIFFUSERS {
                dl = self.diffusers[0][d].allpass(mem, w, self.diffuser_len[0][d], self.diffusion, dl);
                dr = self.diffusers[1][d].allpass(mem, w, self.diffuser_len[1][d], self.diffusion, dr);
            }
            // Every other line's length drifts, and is read between frames.
            // The rest are a whole number of frames long, except while the
            // size is changing.
            let mut x = [0.0f32; LINES];
            if sliding {
                for j in 0..LINES {
                    len[j] += (self.len_target[j] - len[j]).clamp(-self.len_step[j], self.len_step[j]);
                    whole[j] = len[j] as usize;
                }
                for j in (0..LINES).step_by(2) {
                    x[j] = self.lines[j].tap(mem, w, whole[j], len[j] - whole[j] as f32);
                }
            } else {
                for j in (0..LINES).step_by(2) {
                    x[j] = self.lines[j].whole(mem, w, whole[j]);
                }
            }
            for j in (1..LINES).step_by(2) {
                x[j] = self.lines[j].read(mem, w, len[j] + drift[j]);
                drift[j] += drift_inc[j];
            }
            let (mut l, mut r) = (self.bloom * dl, self.bloom * dr);
            for j in 0..LINES {
                l += OUT[0][j] * x[j];
                r += OUT[1][j] * x[j];
            }
            el[i] += l;
            er[i] += r;
            for j in 0..LINES {
                treble[j] += treble_k * (x[j] - treble[j]);
                let v = treble[j] + keep[j] * (x[j] - treble[j]);
                bass[j] += bass_k * (v - bass[j]);
                x[j] = (v + more[j] * bass[j]) * g[j];
            }
            for j in 0..LINES {
                x[j] = self.thicken[j].allpass(mem, w, self.thicken_len[j], self.thickness, x[j]);
            }
            hadamard(&mut x);
            for j in 0..LINES {
                let come_in = if j % 2 == 0 { dl } else { dr };
                self.lines[j].write(mem, w, x[j] + 0.5 * come_in);
            }
        }
        (self.treble, self.bass, self.drift, self.len) = (treble, bass, drift, len);
    }

    /// The two springs: `pl` and `pr` go in, and what comes out is put in
    /// `el` and `er`.
    #[inline(never)]
    fn springs(&mut self, mem: &mut [f32], pl: &[f32], pr: &[f32], el: &mut [f32], er: &mut [f32]) {
        let k = self.stretch;
        let mut done = 0;
        while done < pl.len() {
            // Each filter of a chain holds its sound for `stretch` frames,
            // so that many frames in a row do not depend on each other and
            // go through the chain side by side, both springs at once.
            let n = (k - self.spring_at).min(pl.len() - done).min(TICK);
            let mut v = [0.0f32; 2 * TICK];
            for i in 0..n {
                let at = done + i;
                let w = self.w.wrapping_add(at);
                for s in 0..SPRINGS {
                    self.len[s] += (self.len_target[s] - self.len[s]).clamp(-self.len_step[s], self.len_step[s]);
                    let x = self.lines[s].read(mem, w, self.len[s] + self.drift[s]);
                    self.drift[s] += self.drift_inc[s];
                    self.treble[s] += self.treble_k * (x - self.treble[s]);
                    let kept = self.treble[s] + self.keep[s] * (x - self.treble[s]);
                    self.bass[s] += self.bass_k * (kept - self.bass[s]);
                    let back = (kept + self.more[s] * self.bass[s]) * self.g[s];
                    // A spring carries nothing above the top of its chirp.
                    let come_in = if s == 0 { pl[at] + 0.3 * pr[at] } else { pr[at] + 0.3 * pl[at] };
                    let a = self.spring_in[s][0] + self.spring_k * (come_in - self.spring_in[s][0]);
                    let b = self.spring_in[s][1] + self.spring_k * (a - self.spring_in[s][1]);
                    self.spring_in[s] = [a, b];
                    v[2 * i + s] = b + back;
                }
            }
            // The chain: each filter holds the high part of the sound back
            // a little longer than the low part, and there are dozens.
            let v = &mut v[..2 * n];
            let chain = &mut mem[self.spring..self.spring + SPRING_STAGES * 2 * k];
            for stage in chain.chunks_exact_mut(2 * k) {
                let held = &mut stage[2 * self.spring_at..2 * (self.spring_at + n)];
                for (v, held) in v.chunks_exact_mut(2).zip(held.chunks_exact_mut(2)) {
                    let (a, b) = (SPRING_COEF[0] * v[0] + held[0], SPRING_COEF[1] * v[1] + held[1]);
                    held[0] = v[0] - SPRING_COEF[0] * a;
                    held[1] = v[1] - SPRING_COEF[1] * b;
                    v[0] = a;
                    v[1] = b;
                }
            }
            for i in 0..n {
                let w = self.w.wrapping_add(done + i);
                let (a, b) = (v[2 * i], v[2 * i + 1]);
                self.lines[0].write(mem, w, a);
                self.lines[1].write(mem, w, b);
                el[done + i] = a - 0.2 * b;
                er[done + i] = b + 0.2 * a;
            }
            self.spring_at += n;
            if self.spring_at >= k {
                self.spring_at = 0;
            }
            done += n;
        }
    }

    /// The way out: `wl` and `wr` are added to `out_l` and `out_r`. Says how
    /// many frames were let out and why it stopped there.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn way_out(&mut self, in_l: &[f32], in_r: &[f32], wl: &[f32], wr: &[f32], fades: &[f32], out_l: &mut [f32], out_r: &mut [f32]) -> (usize, Stop) {
        for i in 0..in_l.len() {
            self.out += (self.out_target - self.out) * self.out_ease;
            let (l, r) = (wl[i] * self.out, wr[i] * self.out);
            self.last = l + r;
            if self.leaving && fades[i] <= 0.0 {
                return (i + 1, Stop::Faded);
            }
            out_l[i] += l * fades[i];
            out_r[i] += r * fades[i];
            // Nothing in and next to nothing out for as long as the longest
            // echo: it is over.
            if in_l[i] == 0.0 && in_r[i] == 0.0 && l.abs() + r.abs() < QUIET {
                self.quiet += 1;
                if self.quiet >= self.quiet_after {
                    return (i + 1, Stop::Over);
                }
            } else {
                self.quiet = 0;
            }
        }
        (in_l.len(), Stop::Done)
    }
}

/// The reverb.
pub struct Reverb {
    sample_rate: f32,
    kind: usize,
    /// Empty and doing nothing, until sound comes in.
    idle: bool,
    /// The coefficients must be worked out again at the next look.
    fresh: bool,
    tick: usize,
    core: Core,
    /// Every delay line, one after another: the pre-delay's two, which
    /// end at `tail`, and then the rest.
    mem: Box<[f32]>,
    tail: usize,
    low_hz: f32,
    /// Each line's length at size 0.5, and what its loop has besides the
    /// line, in frames.
    base: [f32; LINES],
    extra: [f32; LINES],
    drift_phase: [f32; LINES],
    drift_step: [f32; LINES],
    drift_depth: f32,
    /// The decay and the treble's share of it, each easing to what is asked.
    t60: f32,
    treble_ratio: f32,
    ease: f32,
}

impl Reverb {
    /// Make a reverb. All its memory is made here, enough for the biggest
    /// size of every type.
    pub fn new(sample_rate: f32) -> Reverb {
        let rs = sample_rate / REF_RATE;
        let most = |f: &dyn Fn(&Kind) -> f32| KINDS.iter().map(f).fold(0.0f32, f32::max);
        let mut frames = 0;
        let mut span = |longest: f32| {
            let len = (longest as usize + 8).next_power_of_two();
            frames += len;
            Span { at: frames - len, mask: len - 1 }
        };
        let drift = most(&|k| k.drift) * sample_rate;
        let early = most(&|k| k.early.iter().flatten().fold(0.0f32, |m, &t| m.max(t)) * 0.001);
        let pre = (PREDELAY_MAX + most(&|k| k.predelay).max(early) * SCALE_BIGGEST) * sample_rate;
        let pre = [span(pre), span(pre)];
        let tail = pre[1].at + pre[1].mask + 1;
        let lines: [Span; LINES] = core::array::from_fn(|i| span(most(&|k| k.lines[i] as f32) * rs * SCALE_BIGGEST + drift));
        let thicken = core::array::from_fn(|i| span(most(&|k| k.thicken[i] as f32) * rs));
        let diffusers = core::array::from_fn(|side| core::array::from_fn(|i| span(most(&|k| k.diffusers[side][i] as f32) * rs)));
        let quiet_after = pre[0].mask + lines.iter().map(|l| l.mask).max().unwrap_or(0) + (0.1 * sample_rate) as usize;
        let stretch = ((sample_rate / (2.0 * SPRING_HZ)).round() as usize).max(1);
        let spring = frames;
        frames += SPRINGS * SPRING_STAGES * stretch;
        let mut r = Reverb {
            sample_rate,
            kind: ROOM,
            idle: true,
            fresh: true,
            tick: 0,
            core: Core {
                w: 0,
                low: [0.0; 2],
                low_k: 0.0,
                pre,
                wait: 0.0,
                wait_target: 0.0,
                scale: 1.0,
                scale_target: 1.0,
                scale_step: 0.0,
                early: false,
                early_at: [[0.0; TAPS]; 2],
                early_gain: [[0.0; TAPS]; 2],
                settled: false,
                taps: [[(0, 0.0); TAPS + 1]; 2],
                tail_at: 0.0,
                diffusers,
                diffuser_len: [[1; DIFFUSERS]; 2],
                diffusion: 0.0,
                bloom: 0.0,
                lines,
                len: [0.0; LINES],
                len_target: [0.0; LINES],
                len_step: [0.0; LINES],
                drift: [0.0; LINES],
                drift_inc: [0.0; LINES],
                thicken,
                thicken_len: [1; LINES],
                thickness: 0.0,
                treble: [0.0; LINES],
                bass: [0.0; LINES],
                treble_k: 0.0,
                bass_k: 1.0 - (-TAU * BASS_HZ / sample_rate).exp(),
                g: [0.0; LINES],
                keep: [1.0; LINES],
                more: [0.0; LINES],
                spring,
                stretch,
                spring_at: 0,
                spring_in: [[0.0; 2]; SPRINGS],
                spring_k: 1.0 - (-TAU * 0.8 * (0.5 * sample_rate / stretch as f32).min(0.45 * sample_rate) / sample_rate).exp(),
                out: 0.0,
                out_target: 0.0,
                out_ease: 1.0 - (-1.0 / (0.005 * sample_rate)).exp(),
                fade: 1.0,
                fade_step: 1.0 / (TYPE_FADE * sample_rate),
                leaving: false,
                quiet: 0,
                quiet_after: quiet_after as u32,
                last: 0.0,
            },
            mem: vec![0.0; frames].into_boxed_slice(),
            tail,
            low_hz: -1.0,
            base: [0.0; LINES],
            extra: [0.0; LINES],
            drift_phase: DRIFT_PHASES,
            drift_step: [0.0; LINES],
            drift_depth: 0.0,
            t60: 1.0,
            treble_ratio: 1.0,
            ease: 1.0 - (-(TICK as f32) / (0.08 * sample_rate)).exp(),
        };
        r.become_kind(ROOM);
        r
    }

    /// How many bytes its memory takes.
    pub fn bytes(&self) -> usize {
        4 * self.mem.len()
    }

    /// Whether it is empty and doing nothing.
    pub fn idle(&self) -> bool {
        self.idle
    }

    /// Empty it at once.
    pub fn clear(&mut self) {
        self.mem[..self.tail].fill(0.0);
        self.core.low = [0.0; 2];
        self.clear_tail();
        self.idle = true;
    }

    /// Empty everything after the pre-delay.
    fn clear_tail(&mut self) {
        self.mem[self.tail..].fill(0.0);
        let c = &mut self.core;
        c.spring_in = [[0.0; 2]; SPRINGS];
        c.treble = [0.0; LINES];
        c.bass = [0.0; LINES];
        c.last = 0.0;
        c.quiet = 0;
    }

    /// Take a type's lengths. Only for when the memory is empty.
    fn become_kind(&mut self, kind: usize) {
        let kind = kind.min(KINDS.len() - 1);
        let k = &KINDS[kind];
        let sr = self.sample_rate;
        let rs = sr / REF_RATE;
        let c = &mut self.core;
        let whole = |len: f32, span: Span| (len.round() as usize).clamp(1, span.mask);
        self.kind = kind;
        let mut longest = 1.0f32;
        for i in 0..LINES {
            self.base[i] = k.lines[i] as f32 * rs;
            longest = longest.max(self.base[i]);
            c.thicken_len[i] = whole(k.thicken[i] as f32 * rs, c.thicken[i]);
            self.extra[i] = if k.thicken[i] > 0 { c.thicken_len[i] as f32 } else { 0.0 };
            self.drift_step[i] = TICK as f32 * k.drift_hz * DRIFT_RATES[i] / sr;
            self.drift_phase[i] = DRIFT_PHASES[i];
            c.drift[i] = 0.0;
            c.drift_inc[i] = 0.0;
        }
        c.thickness = k.thickness;
        c.diffusion = k.diffusion;
        c.bloom = k.bloom;
        for side in 0..2 {
            for i in 0..DIFFUSERS {
                c.diffuser_len[side][i] = whole(k.diffusers[side][i] as f32 * rs, c.diffusers[side][i]);
            }
            for i in 0..TAPS {
                c.early_at[side][i] = k.early[side][i] * 0.001 * sr;
                c.early_gain[side][i] = EARLY_GAINS[side][i] * k.early_level;
            }
        }
        c.early = k.early_level != 0.0;
        if kind == SPRING {
            // What the chain of all-pass filters adds to the loop, at 800 Hz.
            let turn = 0.5 * sr / c.stretch as f32;
            let cos = (crate::math::PI * 800.0 / turn).cos();
            for s in 0..SPRINGS {
                let a = SPRING_COEF[s];
                let each = c.stretch as f32 * (1.0 - a * a) / (1.0 + 2.0 * a * cos + a * a);
                self.extra[s] = SPRING_STAGES as f32 * each;
            }
        }
        c.tail_at = k.predelay * sr;
        self.drift_depth = k.drift * sr;
        c.treble_k = 1.0 - (-TAU * k.treble_hz.min(0.45 * sr) / sr).exp();
        // A change of size may bend the pitch of the longest line by this much at most.
        c.scale_step = 0.15 / longest;
        for i in 0..LINES {
            c.len_step[i] = self.base[i] * c.scale_step;
        }
    }

    /// How long each line is to be at a size: the lines that do not drift
    /// are a whole number of frames.
    fn lengths(&self, scale: f32) -> [f32; LINES] {
        core::array::from_fn(|i| {
            let len = self.base[i] * scale;
            if self.kind != SPRING && i % 2 == 0 {
                len.round()
            } else {
                len
            }
        })
    }

    /// Start afresh with what is asked: for when the memory is empty.
    fn start(&mut self, p: &Settings) {
        self.become_kind(p.kind);
        let lengths = self.lengths(scale_of(p.size));
        let c = &mut self.core;
        c.scale_target = scale_of(p.size);
        c.scale = c.scale_target;
        c.len_target = lengths;
        c.len = lengths;
        c.wait_target = p.predelay.clamp(0.0, PREDELAY_MAX) * self.sample_rate;
        c.wait = c.wait_target;
        c.settled = false;
        c.spring_at = 0;
        c.quiet = 0;
        c.leaving = false;
        self.t60 = rt60(self.kind, p.size, p.decay);
        self.treble_ratio = treble_ratio(p.damping);
        self.fresh = true;
        self.tick = 0;
    }

    /// The slow things, every `TICK` frames.
    fn control(&mut self, p: &Settings) {
        let sr = self.sample_rate;
        let lengths = self.lengths(scale_of(p.size));
        let c = &mut self.core;
        c.scale_target = scale_of(p.size);
        c.len_target = lengths;
        c.wait_target = p.predelay.clamp(0.0, PREDELAY_MAX) * sr;
        let low = p.low_cut.clamp(10.0, 2000.0);
        if low != self.low_hz {
            self.low_hz = low;
            c.low_k = 1.0 - (-TAU * low / sr).exp();
        }
        let fresh = self.fresh;
        let mut changed = fresh || c.sliding();
        self.fresh = false;
        for (now, to) in [(&mut self.t60, rt60(self.kind, p.size, p.decay)), (&mut self.treble_ratio, treble_ratio(p.damping))] {
            if *now != to {
                *now += (to - *now) * self.ease;
                if (to - *now).abs() < 1.0e-4 * to {
                    *now = to;
                }
                changed = true;
            }
        }
        if changed {
            let k = &KINDS[self.kind];
            // A line that takes `t` seconds to go round must keep
            // 10^(-3 t / T) of the sound each time for it to have fallen by
            // 60 dB after T seconds (Jot): so every line, long or short,
            // dies away at the same rate, and no one echo outlasts the rest.
            let per = -6.907_755 / sr;
            let (mid, treble, bass) = (1.0 / self.t60, 1.0 / (self.t60 * self.treble_ratio), 1.0 / (self.t60 * k.bass));
            for i in 0..LINES {
                let t = (c.len[i] + self.extra[i]) * per;
                c.g[i] = (t * mid).exp();
                c.keep[i] = (t * (treble - mid)).exp();
                c.more[i] = (t * (bass - mid)).exp() - 1.0;
            }
            // A longer tail is turned down a little, and a bigger room up, so
            // that the reverb of a held chord grows only gently with its length.
            c.out_target = k.gain * c.scale.sqrt() / self.t60.sqrt().sqrt();
            if fresh {
                c.out = c.out_target;
            }
        }
        for i in 0..LINES {
            let mut phase = self.drift_phase[i] + self.drift_step[i];
            if phase >= 1.0 {
                phase -= 1.0;
            }
            self.drift_phase[i] = phase;
            c.drift_inc[i] = (self.drift_depth * fast_sin(phase) - c.drift[i]) * (1.0 / TICK as f32);
        }
        // Should anything ever go wrong inside, start again from silence.
        if !c.last.is_finite() || c.last.abs() > 1.0e6 {
            self.clear();
            self.idle = false;
        }
    }

    /// Add the reverb of `in_l` and `in_r` to `out_l` and `out_r`.
    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32], p: &Settings) {
        let n = in_l.len().min(in_r.len()).min(out_l.len()).min(out_r.len());
        // A setting that is not a number is taken as its usual value.
        let usual = Settings::default();
        let sane = |v: f32, usual: f32| if v.is_finite() { v } else { usual };
        let p = &Settings {
            kind: p.kind.min(KINDS.len() - 1),
            size: sane(p.size, usual.size),
            damping: sane(p.damping, usual.damping),
            decay: sane(p.decay, usual.decay),
            predelay: sane(p.predelay, usual.predelay),
            low_cut: sane(p.low_cut, usual.low_cut),
        };
        let mut i = 0;
        while i < n {
            if self.idle {
                // Asleep: nothing to do until the first frame that is not silence.
                while i < n && in_l[i] == 0.0 && in_r[i] == 0.0 {
                    i += 1;
                }
                if i == n {
                    return;
                }
                self.idle = false;
                self.core.fade = 1.0;
                self.start(p);
            }
            self.core.leaving = p.kind != self.kind;
            if self.tick == 0 {
                self.control(p);
            }
            // As far as the next look at the slow things, in three steps.
            let run = (TICK - self.tick).min(n - i);
            let (xl, xr) = (&in_l[i..i + run], &in_r[i..i + run]);
            let (mut pl, mut pr, mut wl, mut wr, mut fades) = ([0.0f32; TICK], [0.0f32; TICK], [0.0f32; TICK], [0.0f32; TICK], [0.0f32; TICK]);
            let (c, mem) = (&mut self.core, &mut self.mem[..]);
            let sliding = c.sliding();
            let run = c.way_in(mem, sliding, xl, xr, &mut pl[..run], &mut pr[..run], &mut wl[..run], &mut wr[..run], &mut fades[..run]);
            let (xl, xr) = (&xl[..run], &xr[..run]);
            let (pl, pr, wl, wr, fades) = (&pl[..run], &pr[..run], &mut wl[..run], &mut wr[..run], &fades[..run]);
            if self.kind == SPRING {
                c.springs(mem, pl, pr, wl, wr);
            } else {
                c.network(mem, sliding, pl, pr, wl, wr);
            }
            c.w = c.w.wrapping_add(run);
            let (done, stop) = c.way_out(xl, xr, wl, wr, fades, &mut out_l[i..i + run], &mut out_r[i..i + run]);
            i += done;
            self.tick = (self.tick + done) % TICK;
            match stop {
                Stop::Done => {}
                // Whatever was worked out for the frames after this one goes
                // with the rest: they are done again from an empty reverb.
                Stop::Over => self.clear(),
                Stop::Faded => {
                    // What came in is kept, so that the new type's early
                    // reflections have something to reflect.
                    self.clear_tail();
                    self.core.fade = 0.0;
                    self.start(p);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_between_frames_never_makes_anything_louder() {
        // The gain of the four-frame read at every frequency and every
        // fraction of a frame: never more than 1, or a long tail could grow.
        let mut most = 0.0f64;
        for step in 0..=100 {
            let f = step as f32 / 100.0;
            let line = Span { at: 0, mask: 63 };
            let mut mem = [0.0f32; 64];
            line.write(&mut mem, 32, 1.0);
            let taps: Vec<f64> = (0..8).map(|n| line.read(&mem, 38 + n, 8.0 + f) as f64).collect();
            assert!((taps.iter().sum::<f64>() - 1.0).abs() < 1.0e-6, "the taps add up to 1");
            for bin in 0..=256 {
                let w = std::f64::consts::PI * bin as f64 / 256.0;
                let (re, im) = taps.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, &c)| (re + c * (w * n as f64).cos(), im + c * (w * n as f64).sin()));
                most = most.max((re * re + im * im).sqrt());
            }
        }
        assert!(most < 1.000_01, "{most}");
    }

    #[test]
    fn the_matrix_keeps_the_energy_and_reaches_every_line() {
        for i in 0..LINES {
            let mut x = [0.0f32; LINES];
            x[i] = 1.0;
            hadamard(&mut x);
            assert!(x.iter().all(|v| (v.abs() - 0.353_553_4).abs() < 1.0e-6));
            assert!((x.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1.0e-5);
        }
    }
}
