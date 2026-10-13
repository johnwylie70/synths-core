//! Step macros: the short looping tables that give chip music its sound.
//!
//! A macro belongs to a part. Each of its steps can change the wave, shift
//! the pitch, set the pulse width, move the filter and set the volume, and
//! the steps go by at a fixed rate (50 a second unless told otherwise). Three
//! pitch steps of 0, 4 and 7 semitones, looping, make one voice sound like a
//! chord; a step of noise followed by a falling pitch makes a drum.

/// The most steps a macro can have.
pub const MAX_STEPS: usize = 64;
/// How many numbers describe one step.
pub const STEP_SIZE: usize = 5;

/// The wave number in a step that means noise.
pub const STEP_NOISE: f32 = 4.0;
/// The flag that makes pulse width, cutoff and volume slide between steps.
pub const MACRO_SMOOTH: u32 = 1;

/// One step.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Step {
    /// Oscillator 1's wave: 0 sine, 1 triangle, 2 saw, 3 pulse, 4 noise;
    /// below 0 leaves it as the patch has it.
    pub wave: f32,
    /// Semitones added to the pitch.
    pub pitch: f32,
    /// The pulse width, 0..1; below 0 leaves it as the patch has it.
    pub pulse_width: f32,
    /// Semitones added to the filter cutoff.
    pub cutoff: f32,
    /// The volume, 0..1.
    pub volume: f32,
}

impl Step {
    /// A step that changes nothing.
    pub const NONE: Step = Step { wave: -1.0, pitch: 0.0, pulse_width: -1.0, cutoff: 0.0, volume: 1.0 };

    /// A step from its five numbers, made safe.
    pub fn from_numbers(v: &[f32]) -> Step {
        let ok = |x: f32, lo: f32, hi: f32, or: f32| if x.is_finite() { x.clamp(lo, hi) } else { or };
        Step {
            wave: ok(v[0], -1.0, STEP_NOISE, -1.0).round(),
            pitch: ok(v[1], -96.0, 96.0, 0.0),
            pulse_width: ok(v[2], -1.0, 1.0, -1.0),
            cutoff: ok(v[3], -120.0, 120.0, 0.0),
            volume: ok(v[4], 0.0, 1.0, 1.0),
        }
    }
}

/// A part's macro.
#[derive(Clone, Copy)]
pub struct Macro {
    pub steps: [Step; MAX_STEPS],
    /// How many steps there are; 0 means the part has no macro.
    pub len: u8,
    /// While the note is held, reaching `loop_end` goes back to `loop_start`.
    pub loop_start: u8,
    pub loop_end: u8,
    /// Where to jump when the note is let go; `len` or more for nowhere.
    pub release: u8,
    pub smooth: bool,
}

impl Macro {
    pub const fn none() -> Macro {
        Macro { steps: [Step::NONE; MAX_STEPS], len: 0, loop_start: 0, loop_end: 0, release: 255, smooth: false }
    }

    #[inline]
    fn loops(&self) -> bool {
        self.loop_end > self.loop_start && self.loop_end <= self.len
    }

    #[inline]
    pub fn has_release(&self) -> bool {
        self.release < self.len
    }

    /// The step after `pos`. `leaving` is true once the note has been let go
    /// and the macro has a release point: then the loop is left behind.
    #[inline]
    pub fn next(&self, pos: u8, leaving: bool) -> u8 {
        let n = pos + 1;
        if !leaving && self.loops() && n >= self.loop_end {
            self.loop_start
        } else if n >= self.len {
            // The last step holds.
            self.len.saturating_sub(1)
        } else {
            n
        }
    }
}

impl Default for Macro {
    fn default() -> Macro {
        Macro::none()
    }
}

/// Where a voice is in its part's macro.
#[derive(Clone, Copy, Default)]
pub struct MacroPlayer {
    pos: u8,
    /// How far through the current step, 0..1.
    clock: f32,
    /// Ticks waiting, when the macro steps on tick events.
    pub ticks: u8,
    released: bool,
    /// The note has just been let go: jump to the release point.
    jump: bool,
}

impl MacroPlayer {
    pub fn restart(&mut self) {
        *self = MacroPlayer::default();
    }

    pub fn release(&mut self) {
        if !self.released {
            self.released = true;
            self.jump = true;
        }
    }

    /// Move on by `steps` of clock time (or by the waiting ticks, when
    /// `sync`) and give the step to use now.
    pub fn advance(&mut self, m: &Macro, steps: f32, sync: bool) -> Step {
        if m.len == 0 {
            return Step::NONE;
        }
        if self.pos >= m.len {
            self.pos = m.len - 1;
        }
        let leaving = self.released && m.has_release();
        if self.jump {
            self.jump = false;
            if leaving {
                self.pos = m.release;
                self.clock = 0.0;
                self.ticks = 0;
                return m.steps[self.pos as usize];
            }
        }
        let mut whole = if sync {
            let t = self.ticks as usize;
            self.ticks = 0;
            self.clock = 0.0;
            t
        } else {
            // (A hair early rather than a control period late, when the
            // rounding of the clock leaves it just short of a whole step.)
            self.clock += steps.clamp(0.0, MAX_STEPS as f32);
            let w = (self.clock + 1.0e-4) as usize;
            self.clock = (self.clock - w as f32).max(0.0);
            w
        };
        whole = whole.min(MAX_STEPS);
        for _ in 0..whole {
            self.pos = m.next(self.pos, leaving);
        }
        let now = m.steps[self.pos as usize];
        if m.smooth && !sync {
            let then = m.steps[m.next(self.pos, leaving) as usize];
            let f = self.clock;
            let mix = |a: f32, b: f32| a + (b - a) * f;
            return Step {
                wave: now.wave,
                pitch: now.pitch,
                pulse_width: if now.pulse_width >= 0.0 && then.pulse_width >= 0.0 {
                    mix(now.pulse_width, then.pulse_width)
                } else {
                    now.pulse_width
                },
                cutoff: mix(now.cutoff, then.cutoff),
                volume: mix(now.volume, then.volume),
            };
        }
        now
    }
}
