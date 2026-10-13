//! The per-note arpeggio: one voice stepping quickly through a few pitches,
//! so that it sounds like a chord. Unlike a part's step macro, which every
//! note of the part shares, an arpeggio is given to one note (by handle, or
//! to the next note of a part) and can be changed while the note sounds.

use crate::math::Rng;

/// The most offsets an arpeggio can have.
pub const MAX_OFFSETS: usize = 8;

pub const ORDER_UP: u8 = 0;
pub const ORDER_DOWN: u8 = 1;
pub const ORDER_UP_DOWN: u8 = 2;
pub const ORDER_AS_GIVEN: u8 = 3;
pub const ORDER_RANDOM: u8 = 4;

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
    /// Steps a second; below 0, one step on each `macro_tick` event instead.
    pub rate: f32,
}

impl ArpSpec {
    pub const NONE: ArpSpec = ArpSpec { offsets: [0; MAX_OFFSETS], count: 0, order: ORDER_UP, rate: DEFAULT_RATE };

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
        s.order = if order.is_finite() { order.clamp(0.0, ORDER_RANDOM as f32) as u8 } else { ORDER_UP };
        s.count = if count.is_finite() { count.clamp(0.0, MAX_OFFSETS as f32) as u8 } else { 0 };
        s.set_packed(0, packed);
        s
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
}

impl Default for Arp {
    fn default() -> Arp {
        Arp { spec: ArpSpec::NONE, seq: [0; 2 * MAX_OFFSETS], len: 0, pos: 0, period: 0.0, due: 0.0, offset: 0.0 }
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
