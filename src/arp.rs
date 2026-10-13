//! The per-note arpeggio: one voice stepping quickly through a few pitches,
//! so that it sounds like a chord. Unlike a part's step macro, which every
//! note of the part shares, an arpeggio is given to one note (by handle, or
//! to the next note of a part) and can be changed while the note sounds.
//!
//! That is `Arp`, which lives in a voice and is what the `arp` event has
//! always made. An arpeggio that is asked for more (notes struck afresh at
//! each step, steps counted in beats, several octaves, the notes all
//! together) is played by an arpeggiator instead, which lives in the engine
//! (`engine/arps.rs`) and starts and lets go of notes as a player would.
//! `ArpSpec` describes both, and works out which notes a step has.

use crate::math::Rng;

/// The most offsets an arpeggio can have.
pub const MAX_OFFSETS: usize = 8;

pub const ORDER_UP: u8 = 0;
pub const ORDER_DOWN: u8 = 1;
pub const ORDER_UP_DOWN: u8 = 2;
pub const ORDER_AS_GIVEN: u8 = 3;
pub const ORDER_RANDOM: u8 = 4;
/// Up and then down, playing the top and the bottom note twice.
pub const ORDER_UP_DOWN_ENDS: u8 = 5;
/// Every note at once, on each step.
pub const ORDER_TOGETHER: u8 = 6;

/// The most octaves an arpeggio can run up through.
pub const MAX_OCTAVES: u8 = 4;
/// The shortest a struck note is held, as a part of its step.
pub const MIN_GATE: f32 = 0.05;
/// The furthest every second step can be put back, as a part of a step.
pub const MAX_SWING: f32 = 0.75;
/// How many arpeggiators can run at once.
pub const MAX_ARPEGGIATORS: usize = 32;
/// The most voices one arpeggiator has sounding: the notes of its step, and
/// as many notes of earlier steps, still dying away, as there is room for.
pub const ARPEGGIATOR_VOICES: usize = 8;

/// The steps every second an arpeggio takes when it is not told.
pub const DEFAULT_RATE: f32 = 50.0;

/// What a host asks for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ArpSpec {
    /// Semitones above (or below) the note, whole numbers.
    pub offsets: [i8; MAX_OFFSETS],
    /// How many of them are used; 0 means no arpeggio.
    pub count: u8,
    pub order: u8,
    /// Steps a second; below 0, one step on each `macro_tick` event instead
    /// (or, for an arpeggiator, on each `arp_step` event).
    pub rate: f32,
    /// How much of each step a note is held before it is let go, `MIN_GATE`
    /// to 1 (held until the next is struck). 0: nothing is struck after the
    /// first note; its pitch jumps from step to step.
    pub gate: f32,
    /// How many octaves the pattern is played through, each above the last.
    pub octaves: u8,
    /// How far every second step is put back, as a part of a step.
    pub swing: f32,
    /// Steps to a beat of the engine's `tempo`; 0 to go by `rate`.
    pub beat: f32,
    /// How hard each step is struck: four bits a step, the first step
    /// lowest, 15 for the note's own velocity and 0 for a rest. The pattern
    /// is as long as its last level that is not 0; none at all means every
    /// step at full.
    pub levels: u32,
    /// An event other than `arp` and `arp_more` has had a say in it.
    pub shaped: bool,
}

impl ArpSpec {
    pub const NONE: ArpSpec = ArpSpec {
        offsets: [0; MAX_OFFSETS],
        count: 0,
        order: ORDER_UP,
        rate: DEFAULT_RATE,
        gate: 0.0,
        octaves: 1,
        swing: 0.0,
        beat: 0.0,
        levels: 0,
        shaped: false,
    };

    /// From the fields of an `arp` event: rate, order, count and the first
    /// four offsets packed a byte each.
    pub fn from_event(rate: f32, order: f32, count: f32, packed: u32) -> ArpSpec {
        let mut s = ArpSpec::NONE;
        s.rate = if !rate.is_finite() || rate == 0.0 {
            DEFAULT_RATE
        } else if rate < 0.0 {
            -1.0
        } else {
            rate.clamp(0.1, 1000.0)
        };
        s.order = if order.is_finite() { order.clamp(0.0, ORDER_TOGETHER as f32) as u8 } else { ORDER_UP };
        s.count = if count.is_finite() { count.clamp(0.0, MAX_OFFSETS as f32) as u8 } else { 0 };
        s.set_packed(0, packed);
        s
    }

    /// From the fields of an `arp_notes` event: the notes and the order of
    /// `from_event`, and its rate unless that is 0, with the rest left alone.
    pub fn set_notes(&mut self, rate: f32, order: f32, count: f32, packed: u32) {
        let new = ArpSpec::from_event(rate, order, count, packed);
        if rate.is_finite() && rate != 0.0 {
            self.rate = new.rate;
        }
        self.order = new.order;
        self.count = new.count;
        self.offsets[..4].copy_from_slice(&new.offsets[..4]);
        self.shaped = true;
    }

    /// From the fields of an `arp_style` event.
    pub fn set_style(&mut self, gate: f32, octaves: f32, swing: f32, levels: u32) {
        self.gate = if gate.is_finite() && gate > 0.0 { gate.clamp(MIN_GATE, 1.0) } else { 0.0 };
        self.octaves = if octaves.is_finite() { octaves.clamp(1.0, MAX_OCTAVES as f32) as u8 } else { 1 };
        self.swing = if swing.is_finite() { swing.clamp(0.0, MAX_SWING) } else { 0.0 };
        self.levels = levels;
        self.shaped = true;
    }

    /// From the field of an `arp_beat` event.
    pub fn set_beat(&mut self, steps: f32) {
        self.beat = if steps.is_finite() && steps > 0.0 { steps.clamp(0.01, 64.0) } else { 0.0 };
        self.shaped = true;
    }

    /// Is this for an arpeggiator to play, and not a voice by itself?
    #[inline]
    pub fn wants_arpeggiator(&self) -> bool {
        self.count > 0 && (self.shaped || self.order > ORDER_RANDOM)
    }

    /// Are notes struck afresh at each step (and not one note's pitch moved)?
    #[inline]
    pub fn struck(&self) -> bool {
        self.gate > 0.0 || self.order == ORDER_TOGETHER
    }

    /// How long a step lasts by the clock, in frames; 0 if it has no clock
    /// and steps when it is told to.
    pub fn period(&self, sample_rate: f32, tempo: f32) -> f64 {
        if self.beat > 0.0 {
            (sample_rate as f64 * 60.0 / (tempo.max(1.0) as f64 * self.beat as f64)).max(1.0)
        } else if self.rate > 0.0 {
            (sample_rate / self.rate).max(1.0) as f64
        } else {
            0.0
        }
    }

    /// The notes of step `k`, in semitones from the note the arpeggio is
    /// on, put into `out`. Returns how many there are: one, or with
    /// `ORDER_TOGETHER` all of them.
    pub fn notes(&self, k: u32, rng: &mut Rng, out: &mut [i16; MAX_OFFSETS]) -> usize {
        let n = (self.count as usize).min(MAX_OFFSETS);
        if n == 0 {
            return 0;
        }
        let octaves = self.octaves.clamp(1, MAX_OCTAVES) as u32;
        if self.order == ORDER_TOGETHER {
            // The whole chord, an octave higher at each step.
            let up = 12 * (k % octaves) as i16;
            for (o, &offset) in out.iter_mut().zip(&self.offsets[..n]) {
                *o = offset as i16 + up;
            }
            return n;
        }
        let mut list = self.offsets;
        if self.order != ORDER_AS_GIVEN {
            for i in 1..n {
                let mut j = i;
                while j > 0 && list[j - 1] > list[j] {
                    list.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
        // The pattern is the notes, then the notes an octave up, and so on.
        let len = n as u32 * octaves;
        let at = match self.order {
            ORDER_DOWN => len - 1 - k % len,
            ORDER_UP_DOWN => {
                let round = (2 * len - 2).max(1);
                let p = k % round;
                if p < len {
                    p
                } else {
                    round - p
                }
            }
            ORDER_UP_DOWN_ENDS => {
                let p = k % (2 * len);
                if p < len {
                    p
                } else {
                    2 * len - 1 - p
                }
            }
            ORDER_RANDOM => rng.next_u32() % len,
            _ => k % len,
        };
        out[0] = list[at as usize % n] as i16 + 12 * (at / n as u32) as i16;
        1
    }

    /// How hard step `k` is struck, 0..1; 0 is a rest.
    pub fn level(&self, k: u32) -> f32 {
        if self.levels == 0 {
            return 1.0;
        }
        let len = 8 - self.levels.leading_zeros() / 4;
        ((self.levels >> (4 * (k % len))) & 15) as f32 / 15.0
    }

    /// Set four offsets, starting at `first`, from a byte each of `packed`
    /// (the lowest byte is the first).
    pub fn set_packed(&mut self, first: usize, packed: u32) {
        for (i, b) in packed.to_le_bytes().iter().enumerate() {
            if let Some(o) = self.offsets.get_mut(first + i) {
                *o = *b as i8;
            }
        }
    }
}

/// A voice's arpeggio.
#[derive(Clone, Copy)]
pub struct Arp {
    spec: ArpSpec,
    /// The offsets in the order they are played.
    seq: [i8; 2 * MAX_OFFSETS],
    len: u8,
    pos: u8,
    /// Frames between steps; 0 when it steps on tick events.
    period: f64,
    /// The time of the next step, in frames.
    due: f64,
    /// The offset now, in semitones.
    pub offset: f32,
    /// The arpeggiator that started this voice, or 0 for none.
    pub owner: u16,
    /// How far above its arpeggiator's note the voice was struck, in
    /// semitones, and the note it was struck at: a voice that has since
    /// been given another note (mono mode does that) has left it behind.
    pub shift: f32,
    pub shift_key: f32,
}

impl Default for Arp {
    fn default() -> Arp {
        Arp { spec: ArpSpec::NONE, seq: [0; 2 * MAX_OFFSETS], len: 0, pos: 0, period: 0.0, due: 0.0, offset: 0.0, owner: 0, shift: 0.0, shift_key: 0.0 }
    }
}

impl Arp {
    pub fn spec(&self) -> ArpSpec {
        self.spec
    }

    /// Start (or change, or with a count of 0 stop) the arpeggio at time
    /// `now`. Its first step sounds at once.
    pub fn set(&mut self, spec: &ArpSpec, now: u64, sample_rate: f32, rng: &mut Rng) {
        self.spec = *spec;
        let n = (spec.count as usize).min(MAX_OFFSETS);
        let mut sorted = spec.offsets;
        // A sort of at most eight numbers, with no memory needed.
        for i in 1..n {
            let mut j = i;
            while j > 0 && sorted[j - 1] > sorted[j] {
                sorted.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut len = 0;
        let mut put = |seq: &mut [i8; 2 * MAX_OFFSETS], v: i8| {
            seq[len] = v;
            len += 1;
        };
        match spec.order {
            ORDER_UP => (0..n).for_each(|i| put(&mut self.seq, sorted[i])),
            ORDER_DOWN => (0..n).rev().for_each(|i| put(&mut self.seq, sorted[i])),
            ORDER_UP_DOWN => {
                (0..n).for_each(|i| put(&mut self.seq, sorted[i]));
                // Back down without playing the top or the bottom twice.
                (1..n.saturating_sub(1)).rev().for_each(|i| put(&mut self.seq, sorted[i]));
            }
            _ => (0..n).for_each(|i| put(&mut self.seq, spec.offsets[i])),
        }
        self.len = len as u8;
        self.pos = 0;
        self.period = if spec.rate > 0.0 { (sample_rate / spec.rate).max(1.0) as f64 } else { 0.0 };
        self.due = now as f64 + self.period;
        self.offset = 0.0;
        if self.len > 0 {
            if spec.order == ORDER_RANDOM {
                self.pos = (rng.next_u32() % self.len as u32) as u8;
            }
            self.offset = self.seq[self.pos as usize] as f32;
        }
    }

    /// Change four of the offsets without starting the pattern again.
    pub fn set_more(&mut self, first: usize, packed: u32, now: u64, sample_rate: f32, rng: &mut Rng) {
        let (pos, due) = (self.pos, self.due);
        let mut spec = self.spec;
        spec.set_packed(first, packed);
        self.set(&spec, now, sample_rate, rng);
        if self.len > 0 && spec.order != ORDER_RANDOM {
            self.pos = pos % self.len;
            self.offset = self.seq[self.pos as usize] as f32;
        }
        if due > now as f64 {
            self.due = due;
        }
    }

    #[inline]
    pub fn active(&self) -> bool {
        self.len > 0
    }

    /// How far from its arpeggiator's note a voice started with `key` is to stay.
    #[inline]
    pub fn shift_from(&self, key: f32) -> f32 {
        if key == self.shift_key {
            self.shift
        } else {
            0.0
        }
    }

    /// Hand the arpeggio over to an arpeggiator: this one stops where it
    /// is, with the pitch left where it has got to. Gives the step it was
    /// on and the time of its next (0 if it stepped on ticks).
    pub fn hand_over(&mut self) -> (u8, f64) {
        let at = (self.pos, if self.period > 0.0 { self.due } else { 0.0 });
        self.len = 0;
        self.spec.count = 0;
        at
    }

    /// The frame of the next step by the clock, or `u64::MAX` if there is none.
    #[inline]
    pub fn next_frame(&self) -> u64 {
        if self.len > 0 && self.period > 0.0 {
            self.due as u64
        } else {
            u64::MAX
        }
    }

    fn step(&mut self, rng: &mut Rng) {
        if self.len == 0 {
            return;
        }
        self.pos = if self.spec.order == ORDER_RANDOM {
            (rng.next_u32() % self.len as u32) as u8
        } else if self.pos + 1 >= self.len {
            0
        } else {
            self.pos + 1
        };
        self.offset = self.seq[self.pos as usize] as f32;
    }

    /// Take the step that is due at `now`, if one is. True if it did.
    pub fn run(&mut self, now: u64, rng: &mut Rng) -> bool {
        if self.next_frame() > now {
            return false;
        }
        self.due += self.period;
        if self.due <= now as f64 {
            // Far behind (it should not happen): start counting again from here.
            self.due = now as f64 + self.period;
        }
        self.step(rng);
        true
    }

    /// A tick event: step, if this arpeggio steps on ticks. True if it did.
    pub fn tick(&mut self, rng: &mut Rng) -> bool {
        if self.len > 0 && self.period == 0.0 {
            self.step(rng);
            true
        } else {
            false
        }
    }
}
