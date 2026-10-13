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
    /// The pattern of steps it follows in place of its order: 1 to 64 for
    /// one of the engine's, `PATTERN_OWN`, or 0 for none.
    pub pattern: u8,
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
        pattern: 0,
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

    /// From the field of an `arp_pattern` event.
    pub fn set_pattern(&mut self, slot: f32) {
        self.pattern = if slot.is_finite() && slot >= 1.0 { slot.min(PATTERN_SLOTS as f32) as u8 } else { 0 };
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
        let which = at as usize % n;
        let list = &self.offsets[..n];
        // Which of the notes that is: as given, or the one with that many
        // below it (of two the same, the first given counts as the lower).
        let mut note = list[which];
        if self.order != ORDER_AS_GIVEN {
            for (i, &a) in list.iter().enumerate() {
                let below = list.iter().enumerate().filter(|&(j, &b)| b < a || (b == a && j < i)).count();
                if below == which {
                    note = a;
                }
            }
        }
        out[0] = note as i16 + 12 * (at / n as u32) as i16;
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

// ---------------------------------------------------------------- patterns

/// The most steps an arpeggio's pattern can have.
pub const MAX_PATTERN: usize = 32;
/// How many patterns the engine keeps for hosts to choose from.
pub const PATTERN_SLOTS: usize = 64;
/// How many bytes describe one step of a pattern.
pub const PATTERN_STEP_SIZE: usize = 4;
/// A step's flags: hold the last step's note on through this one.
pub const STEP_TIE: u8 = 1;
/// A step's flags: do not strike this note; slide the last one to it.
pub const STEP_SLIDE: u8 = 2;
/// The pattern number in an `ArpSpec` that means the arpeggiator was handed its steps directly.
pub const PATTERN_OWN: u8 = 255;
/// How loud a step with no accent is in a pattern that has accents, of 15.
pub const PLAIN_LEVEL: u8 = 10;
/// How long a slide into a step takes at most, in seconds.
pub const SLIDE_TIME: f32 = 0.06;

/// One step of a pattern: four bytes.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PatternStep {
    /// Which note of the chord, counted from 1 at its lowest; past the
    /// chord's last note it carries on an octave up. Below 0 counts down
    /// from the chord an octave below (-1 is its top note). 0 is a rest.
    pub note: i8,
    /// `STEP_TIE`, `STEP_SLIDE`.
    pub flags: u8,
    /// Octaves to move the note, -3 to 3.
    pub octave: i8,
    /// The low four bits: how hard it is struck, 1 to 15 (0: full). The
    /// high four: its own gate in tenths of a step, 1 to 10 (0: the arpeggio's).
    pub shape: u8,
}

impl PatternStep {
    pub fn is_rest(&self) -> bool {
        self.note == 0 && self.flags & STEP_TIE == 0
    }

    /// Does this step carry the last one's note on, and not strike its own?
    pub fn carries(&self) -> bool {
        self.flags & STEP_TIE != 0 || (self.flags & STEP_SLIDE != 0 && self.note != 0)
    }

    pub fn level(&self) -> f32 {
        match self.shape & 15 {
            0 => 1.0,
            n => n as f32 / 15.0,
        }
    }

    /// The step's own gate, if it has one.
    pub fn gate(&self) -> Option<f32> {
        match self.shape >> 4 {
            0 => None,
            n => Some(n.min(10) as f32 / 10.0),
        }
    }
}

/// The names of the patterns that are built in, and what each is.
pub const NAMED_PATTERNS: &[(&str, &str)] = &[
    ("bounce", "1 2 3 2"),
    ("octaves", "1 1' 1 1'"),
    ("root-fifth", "1 3 1' 3"),
    ("thumb", "1 2 1 3 1 4 1 3"),
    ("converge", "1 4 2 3"),
    ("diverge", "2 3 1 4"),
    ("offbeat", "- 1 - 2 - 3 - 2"),
    ("gallop", "1^ - 1 1 2^ - 2 2"),
    ("trance", "1^ - 3 2 - 1^ 3 -"),
    ("acid", "1^ 1 ~3 - 1' ~1 2^ -"),
];

/// What is wrong with a pattern's text, in words.
pub const PATTERN_HELP: &str = "an arp pattern is steps with spaces between: a chord note from 1 up, - for a rest, _ to hold the last note; after a note ' and , move it up and down an octave, ^ accents it and :1 to :9 sets its gate; ~ before a note slides into it; at most 32 steps; or the name of a built-in pattern";

/// An arpeggio's pattern: a short list of steps that goes round.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pattern {
    pub steps: [PatternStep; MAX_PATTERN],
    /// How many steps there are; 0 for no pattern.
    pub len: u8,
}

impl Pattern {
    pub const NONE: Pattern = Pattern { steps: [PatternStep { note: 0, flags: 0, octave: 0, shape: 0 }; MAX_PATTERN], len: 0 };

    pub fn steps(&self) -> &[PatternStep] {
        &self.steps[..(self.len as usize).min(MAX_PATTERN)]
    }

    /// A pattern of these steps (at most 32 are kept), each made safe.
    pub fn new(steps: &[PatternStep]) -> Pattern {
        let mut p = Pattern::NONE;
        for (slot, s) in p.steps.iter_mut().zip(steps) {
            *slot = PatternStep { note: s.note.clamp(-64, 64), flags: s.flags & (STEP_TIE | STEP_SLIDE), octave: s.octave.clamp(-3, 3), shape: s.shape };
        }
        p.len = steps.len().min(MAX_PATTERN) as u8;
        p
    }

    /// From the bytes a host hands over: four a step.
    pub fn from_bytes(bytes: &[u8]) -> Pattern {
        let mut steps = [PatternStep::default(); MAX_PATTERN];
        let mut n = 0;
        for (slot, b) in steps.iter_mut().zip(bytes.chunks_exact(PATTERN_STEP_SIZE)) {
            *slot = PatternStep { note: b[0] as i8, flags: b[1], octave: b[2] as i8, shape: b[3] };
            n += 1;
        }
        Pattern::new(&steps[..n])
    }

    /// From its text: `"1 - 3 2 _ 1 3 5^"`, or the name of a built-in
    /// pattern. `PATTERN_HELP` says what the text can be.
    pub fn parse(text: &str) -> Result<Pattern, &'static str> {
        let text = text.trim();
        let text = NAMED_PATTERNS.iter().find(|(name, _)| *name == text).map_or(text, |(_, steps)| steps);
        let mut p = Pattern::NONE;
        let mut accents = false;
        for token in text.split_ascii_whitespace() {
            if p.len as usize >= MAX_PATTERN {
                return Err(PATTERN_HELP);
            }
            let mut step = PatternStep::default();
            let mut rest = token.as_bytes();
            match token {
                "-" | "." => rest = &[],
                "_" => {
                    step.flags = STEP_TIE;
                    rest = &[];
                }
                _ => {
                    if let [b'~', more @ ..] = rest {
                        step.flags = STEP_SLIDE;
                        rest = more;
                    }
                    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
                    let n = rest[..digits].iter().fold(0u32, |n, b| (n * 10 + (b - b'0') as u32).min(1000));
                    if digits == 0 || n == 0 || n > MAX_PATTERN as u32 {
                        return Err(PATTERN_HELP);
                    }
                    step.note = n as i8;
                    rest = &rest[digits..];
                }
            }
            while let [mark, more @ ..] = rest {
                rest = more;
                match mark {
                    b'\'' if step.octave < 3 => step.octave += 1,
                    b',' if step.octave > -3 => step.octave -= 1,
                    b'^' => {
                        step.shape |= 15;
                        accents = true;
                    }
                    b':' => match rest {
                        [d @ b'1'..=b'9', more @ ..] if step.shape >> 4 == 0 => {
                            step.shape |= (d - b'0') << 4;
                            rest = more;
                        }
                        _ => return Err(PATTERN_HELP),
                    },
                    _ => return Err(PATTERN_HELP),
                }
            }
            p.steps[p.len as usize] = step;
            p.len += 1;
        }
        if p.len == 0 {
            return Err(PATTERN_HELP);
        }
        if accents {
            // The steps without an accent stand back for those with one.
            for s in p.steps[..p.len as usize].iter_mut().filter(|s| s.note != 0 && s.shape & 15 == 0) {
                s.shape |= PLAIN_LEVEL;
            }
        }
        Ok(p)
    }

    /// The pattern as text `parse` reads back as the same pattern, if text can say it.
    pub fn to_text(&self) -> Option<String> {
        let steps = self.steps();
        let accents = steps.iter().any(|s| s.note != 0 && s.shape & 15 != 0);
        let mut out = String::new();
        for s in steps {
            if !out.is_empty() {
                out.push(' ');
            }
            let (level, gate) = (s.shape & 15, s.shape >> 4);
            if s.note == 0 {
                if s.octave != 0 || s.shape != 0 || s.flags & STEP_SLIDE != 0 {
                    return None;
                }
                out.push(if s.flags & STEP_TIE != 0 { '_' } else { '-' });
                continue;
            }
            let plain = if accents { PLAIN_LEVEL } else { 0 };
            if s.note < 0 || s.note as usize > MAX_PATTERN || s.flags & STEP_TIE != 0 || gate > 9 || (level != plain && level != 15) || (level == 15 && !accents) {
                return None;
            }
            if s.flags & STEP_SLIDE != 0 {
                out.push('~');
            }
            out.push_str(&crate::math::small_int(s.note as u32));
            for _ in 0..s.octave.unsigned_abs() {
                out.push(if s.octave > 0 { '\'' } else { ',' });
            }
            if level == 15 {
                out.push('^');
            }
            if gate > 0 {
                out.push(':');
                out.push((b'0' + gate) as char);
            }
        }
        (!out.is_empty()).then_some(out)
    }
}

impl ArpSpec {
    /// How far from the note the arpeggio is on a pattern's step is, in
    /// semitones: its place in the chord's notes from the lowest up,
    /// carrying on into the octaves above and below.
    pub fn pattern_offset(&self, step: &PatternStep) -> i16 {
        let n = (self.count as usize).clamp(1, MAX_OFFSETS);
        let list = &self.offsets[..n];
        let place = if step.note > 0 { step.note as i32 - 1 } else { step.note as i32 };
        let which = place.rem_euclid(n as i32) as usize;
        let mut note = list[which.min(n - 1)];
        for (i, &a) in list.iter().enumerate() {
            if list.iter().enumerate().filter(|&(j, &b)| b < a || (b == a && j < i)).count() == which {
                note = a;
            }
        }
        note as i16 + 12 * (place.div_euclid(n as i32) + step.octave as i32) as i16
    }
}
