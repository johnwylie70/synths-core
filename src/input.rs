//! The live input: sound a host hands the engine as it renders, from a
//! microphone or a line in. It is cleaned, turned up and gated here, and
//! then plays through a part as if it were that part's one voice. This
//! file also holds the recorder, which writes the input, a part or the
//! whole output into a sample slot.

use crate::engine::OUTPUT_FRAMES;
use crate::filter::{self, Svf, FILTER_OFF};
use crate::math::{fast_sin, PI};
use crate::params::*;
use crate::sample::Sample;
use crate::voice::CONTROL;
use std::sync::atomic::{AtomicU32, Ordering};

/// Anything quieter than this (200 dB down) is taken as silence.
const TINY: f32 = 1.0e-10;
/// The loudest the input may be, after `input.gain`.
const LOUDEST: f32 = 4.0;
/// How long the gate takes to open, in seconds.
pub const GATE_ATTACK: f32 = 0.002;
/// The gate's measure of loudness falls to 37% in this long, in seconds.
pub const GATE_FALL: f32 = 0.03;
/// The gate closes when the loudness is this far under the level that opens it.
pub const GATE_CLOSE: f32 = 0.5;
/// At or below this many dB, `input.gate` is off.
pub const GATE_OFF: f32 = -99.5;

/// The live input's gate, and its way into a part.
pub struct Input {
    /// How loud the input has lately been: it jumps up and falls slowly.
    env: f32,
    env_keep: f32,
    open: bool,
    /// How far open the gate is, 0..1.
    gain: f32,
    attack: f32,
    /// The input is making sound (the gate is open, or still closing).
    pub sounding: bool,
    /// The input after its gain and gate, for the frames about to be rendered.
    pub pre_l: [f32; CONTROL],
    pub pre_r: [f32; CONTROL],
    stereo: bool,
    // Its way into the part: the part's filter and pan.
    filters: [[Svf; 2]; 2],
    filter_on: bool,
    g: f32,
    dg: f32,
    k: f32,
    kind: u8,
    slope: bool,
    pan: [f32; 2],
    dpan: [f32; 2],
    /// The next frames of the path start from scratch.
    fresh: bool,
    /// Where a host using the C interface puts the input.
    pub(crate) in_l: Box<[f32]>,
    pub(crate) in_r: Box<[f32]>,
}

#[inline]
fn clean(x: f32, gain: f32) -> f32 {
    if x.is_finite() {
        (x * gain).clamp(-LOUDEST, LOUDEST)
    } else {
        0.0
    }
}

impl Input {
    pub fn new(sample_rate: f32) -> Input {
        Input {
            env: 0.0,
            env_keep: (-1.0 / (GATE_FALL * sample_rate)).exp(),
            open: false,
            gain: 0.0,
            attack: 1.0 / (GATE_ATTACK * sample_rate).max(1.0),
            sounding: false,
            pre_l: [0.0; CONTROL],
            pre_r: [0.0; CONTROL],
            stereo: false,
            filters: [[Svf::default(); 2]; 2],
            filter_on: false,
            g: 0.0,
            dg: 0.0,
            k: 2.0,
            kind: FILTER_OFF,
            slope: false,
            pan: [0.0; 2],
            dpan: [0.0; 2],
            fresh: true,
            in_l: vec![0.0; OUTPUT_FRAMES].into_boxed_slice(),
            in_r: vec![0.0; OUTPUT_FRAMES].into_boxed_slice(),
        }
    }

    pub fn reset(&mut self) {
        self.env = 0.0;
        self.open = false;
        self.gain = 0.0;
        self.sounding = false;
        self.fresh = true;
    }

    /// The input is sounding but is not being let into its part: when it
    /// is let in again, its filter and pan start from scratch.
    #[inline]
    pub fn unheard(&mut self) {
        self.fresh = true;
    }

    /// Nothing is coming in and nothing is left of what came in before.
    #[inline]
    fn at_rest(&self) -> bool {
        !self.sounding && !self.open && self.env == 0.0 && self.gain == 0.0
    }

    /// Take in the input for the next frames: at most `n` of them, which is
    /// at most `CONTROL`. `left` and `right` hold at least `n` frames each
    /// (`right` is `None` for a mono input; both `None` is no input at all).
    /// `boundary` says these frames start a control period.
    ///
    /// Returns how many frames were taken and whether `pre_l` and `pre_r`
    /// were written. Fewer than `n` are taken when the input starts to
    /// sound part way through: the frames before that moment are returned
    /// first, so that it starts on its exact frame whatever the block size.
    pub fn stage(
        &mut self,
        left: Option<&[f32]>,
        right: Option<&[f32]>,
        n: usize,
        boundary: bool,
        g: &[f32; GLOBAL_PARAMS],
        sample_rate: f32,
    ) -> (usize, bool) {
        // The input stops sounding at the start of a control period, not
        // part way through one.
        if boundary && self.sounding && !self.open && self.gain == 0.0 {
            self.sounding = false;
            self.fresh = true;
        }
        let gain_in = g[INPUT_GAIN - GLOBAL_BASE];
        if self.at_rest() {
            // The quick way out: silence costs one look at each frame.
            let loud = |x: &[f32]| x[..n].iter().position(|v| (v * gain_in).abs() >= TINY);
            let first = match (left, right) {
                (Some(l), Some(r)) => match (loud(l), loud(r)) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                },
                (Some(l), None) => loud(l),
                _ => None,
            };
            match first {
                None => return (n, false),
                Some(i) if i > 0 => return (i, false),
                _ => {}
            }
        }
        let db = g[INPUT_GATE - GLOBAL_BASE];
        let gated = db > GATE_OFF;
        let opens = if gated { (db * 0.05 * core::f32::consts::LOG2_10).exp2() } else { 0.0 };
        let closes = opens * GATE_CLOSE;
        let release = 1.0 / (g[INPUT_GATE_RELEASE - GLOBAL_BASE] * sample_rate).max(1.0);
        self.stereo = right.is_some();
        for i in 0..n {
            let l = left.map_or(0.0, |x| clean(x[i], gain_in));
            let r = right.map_or(l, |x| clean(x[i], gain_in));
            let (l, r) = if l.abs().max(r.abs()) < TINY { (0.0, 0.0) } else { (l, r) };
            let level = l.abs().max(r.abs());
            let mut env = self.env * self.env_keep;
            if env < TINY {
                env = 0.0;
            }
            env = env.max(level);
            let open = if !gated {
                env > 0.0
            } else if self.open {
                env >= closes
            } else {
                env >= opens
            };
            if open && !self.sounding {
                if i > 0 {
                    // It starts here: hand back the silence before it first.
                    return (i, true);
                }
                self.sounding = true;
            }
            self.env = env;
            self.open = open;
            self.gain = if !gated {
                open as u32 as f32
            } else if open {
                (self.gain + self.attack).min(1.0)
            } else {
                (self.gain - release).max(0.0)
            };
            self.pre_l[i] = l * self.gain;
            self.pre_r[i] = r * self.gain;
        }
        (n, true)
    }

    /// Add the staged input to a part's sound, through the part's filter
    /// and pan. `p` is the part's patch, and `tick_pos` how far into the
    /// control period these frames start.
    pub fn monitor(&mut self, p: &[f32; PART_PARAMS], bus_l: &mut [f32], bus_r: &mut [f32], tick_pos: usize, sample_rate: f32) {
        let n = bus_l.len().min(bus_r.len()).min(CONTROL);
        if tick_pos == 0 || self.fresh {
            // Slow-moving values are looked at once a control period and
            // slide there in a straight line, as a voice's do.
            let left = (CONTROL - tick_pos) as f32;
            let jump = self.fresh;
            let was_on = self.filter_on && !jump;
            self.kind = p[FILTER_TYPE] as u8;
            self.filter_on = self.kind != FILTER_OFF;
            self.slope = p[FILTER_SLOPE] >= 0.5;
            if self.filter_on {
                let hz = p[CUTOFF].clamp(16.0, sample_rate * 0.45);
                let g = (PI * hz / sample_rate).tan();
                if was_on {
                    self.dg = (g - self.g) / left;
                } else {
                    self.g = g;
                    self.dg = 0.0;
                    self.filters = [[Svf::default(); 2]; 2];
                }
                self.k = 2.0 * (-6.5 * p[RESONANCE]).exp2();
                if self.slope {
                    self.k = self.k.sqrt();
                }
                for f in self.filters.iter_mut().flatten() {
                    f.tidy();
                }
            }
            let pan = p[PAN].clamp(-1.0, 1.0);
            let (gl, gr) = if self.stereo {
                // A stereo input keeps both sides at full until it is panned.
                ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
            } else if p[PAN_LAW] >= 0.5 {
                let right = (pan + 1.0) * 0.5;
                ((1.0 - right).sqrt(), right.sqrt())
            } else {
                let a = (pan + 1.0) * 0.125;
                (fast_sin(a + 0.25), fast_sin(a))
            };
            if jump {
                self.pan = [gl, gr];
                self.dpan = [0.0; 2];
            } else {
                self.dpan = [(gl - self.pan[0]) / left, (gr - self.pan[1]) / left];
            }
            self.fresh = false;
        }
        let mut a = self.pre_l;
        let mut b = self.pre_r;
        let (a, b) = (&mut a[..n], &mut b[..n]);
        if self.filter_on {
            let (g, dg, k, mix) = (self.g, self.dg, self.k, filter::outputs(self.kind));
            let sides = if self.stereo { 2 } else { 1 };
            for (side, buf) in [&mut *a, &mut *b].into_iter().take(sides).enumerate() {
                self.g = self.filters[side][0].process(buf, g, dg, k, mix);
                if self.slope {
                    self.filters[side][1].process(buf, g, dg, k, mix);
                }
            }
            if !self.stereo {
                b.copy_from_slice(a);
            }
        }
        let [mut gl, mut gr] = self.pan;
        let [dl, dr] = self.dpan;
        for i in 0..n {
            gl += dl;
            gr += dr;
            bus_l[i] += a[i] * gl;
            bus_r[i] += b[i] * gr;
        }
        self.pan = [gl, gr];
    }
}

/// What a recording takes in (the `y` of a `record` event).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    Input,
    Part(u16),
    Master,
}

/// How many numbers `Recorder::read` gives.
pub const RECORD_SIZE: usize = 4;

/// Writes sound into a sample slot whose memory was made beforehand.
pub struct Recorder {
    source: Option<Source>,
    slot: usize,
    frames: u32,
    /// For a display on another thread: recording or not, the slot, the
    /// frames recorded and the frames there is room for.
    pub(crate) shared: Box<[AtomicU32; RECORD_SIZE]>,
}

impl Default for Recorder {
    fn default() -> Recorder {
        Recorder { source: None, slot: 0, frames: 0, shared: Box::new([const { AtomicU32::new(0) }; RECORD_SIZE]) }
    }
}

impl Recorder {
    /// What is being recorded, if anything.
    #[inline]
    pub fn source(&self) -> Option<Source> {
        self.source
    }

    /// Recording or not (1 or 0), the slot, how many frames have been
    /// recorded, and how many the slot has room for. Safe to call while
    /// another thread renders.
    pub fn read(&self) -> [u32; RECORD_SIZE] {
        Recorder::read_shared(&self.shared)
    }

    pub(crate) fn read_shared(shared: &[AtomicU32; RECORD_SIZE]) -> [u32; RECORD_SIZE] {
        core::array::from_fn(|i| shared[i].load(Ordering::Relaxed))
    }

    /// Start recording into `slot`, from its first frame. False (and
    /// nothing changes) if the slot has no memory to record into.
    pub fn start(&mut self, samples: &mut [Option<Sample>], slot: usize, source: Source) -> bool {
        let s = match samples.get_mut(slot) {
            Some(Some(s)) if !s.data.is_empty() => s,
            _ => return false,
        };
        s.frames = 0;
        s.loop_mode = 0;
        s.loop_start = 0;
        s.loop_end = 0;
        self.source = Some(source);
        self.slot = slot;
        self.frames = 0;
        self.shared[1].store(slot as u32, Ordering::Relaxed);
        self.shared[2].store(0, Ordering::Relaxed);
        self.shared[3].store((s.data.len() / s.channels.max(1) as usize) as u32, Ordering::Relaxed);
        self.shared[0].store(1, Ordering::Relaxed);
        true
    }

    pub fn stop(&mut self) {
        self.source = None;
        self.shared[0].store(0, Ordering::Relaxed);
    }

    /// Add frames to the recording. It stops by itself when the slot is
    /// full, or if the slot has been emptied or loaded with something else.
    pub fn write(&mut self, samples: &mut [Option<Sample>], left: &[f32], right: &[f32]) {
        let s = match samples.get_mut(self.slot) {
            Some(Some(s)) if s.frames == self.frames => s,
            _ => return self.stop(),
        };
        let channels = s.channels.max(1) as usize;
        let room = s.data.len() / channels - self.frames as usize;
        let n = left.len().min(right.len()).min(room);
        let at = self.frames as usize * channels;
        if channels == 2 {
            for (i, f) in s.data[at..at + n * 2].chunks_exact_mut(2).enumerate() {
                f[0] = left[i];
                f[1] = right[i];
            }
        } else {
            for (i, f) in s.data[at..at + n].iter_mut().enumerate() {
                *f = 0.5 * (left[i] + right[i]);
            }
        }
        self.frames += n as u32;
        s.frames = self.frames;
        s.loop_end = self.frames;
        self.shared[2].store(self.frames, Ordering::Relaxed);
        if n == room {
            self.stop();
        }
    }

    /// Add frames of silence.
    pub fn write_silence(&mut self, samples: &mut [Option<Sample>], n: usize) {
        let zeros = [0.0f32; CONTROL];
        let n = n.min(CONTROL);
        self.write(samples, &zeros[..n], &zeros[..n]);
    }
}
