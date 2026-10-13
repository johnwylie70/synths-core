//! The envelope: attack, decay, sustain, release.
//!
//! Each stage lasts a whole number of samples, so its timing is exact. The
//! curve bends each stage from a straight line (0) towards the quick-then-slow
//! shape of an analogue envelope (1).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// An envelope's settings, worked out from the patch for the sample rate.
#[derive(Clone, Copy)]
pub struct EnvShape {
    attack: u32,
    decay: u32,
    release: u32,
    inv_attack: f32,
    inv_decay: f32,
    inv_release: f32,
    pub sustain: f32,
    warp: f32,
}

impl EnvShape {
    /// `p` is attack, decay, sustain, release, curve (seconds, level, 0..1).
    pub fn new(p: &[f32], sample_rate: f32) -> EnvShape {
        let len = |seconds: f32| (seconds.max(0.0) * sample_rate + 0.5) as u32;
        let inv = |n: u32| if n > 0 { 1.0 / n as f32 } else { 0.0 };
        let (a, d, r) = (len(p[0]), len(p[1]), len(p[3]));
        EnvShape {
            attack: a,
            decay: d,
            release: r,
            inv_attack: inv(a),
            inv_decay: inv(d),
            inv_release: inv(r),
            sustain: p[2].clamp(0.0, 1.0),
            warp: 1.0 - 0.9 * p[4].clamp(0.0, 1.0),
        }
    }

    /// Give the release another length, in seconds.
    pub fn set_release(&mut self, seconds: f32, sample_rate: f32) {
        self.release = (seconds.max(0.0) * sample_rate + 0.5) as u32;
        self.inv_release = if self.release > 0 { 1.0 / self.release as f32 } else { 0.0 };
    }

    #[inline]
    fn bend(&self, x: f32) -> f32 {
        x / (x + self.warp * (1.0 - x))
    }
}

impl Default for EnvShape {
    fn default() -> EnvShape {
        EnvShape::new(&[0.0, 0.0, 1.0, 0.0, 0.0], 48000.0)
    }
}

#[derive(Clone, Copy)]
pub struct Env {
    pub stage: Stage,
    /// Samples into the current stage.
    pos: u32,
    /// The level the current attack or release started from.
    from: f32,
}

impl Default for Env {
    fn default() -> Env {
        Env { stage: Stage::Idle, pos: 0, from: 0.0 }
    }
}

impl Env {
    /// The note starts (or starts again, from wherever the level is now).
    pub fn gate_on(&mut self, s: &EnvShape) {
        self.from = self.level(s);
        self.stage = Stage::Attack;
        self.pos = 0;
    }

    /// The note is let go.
    pub fn gate_off(&mut self, s: &EnvShape) {
        if self.stage != Stage::Idle {
            self.from = self.level(s);
            self.stage = Stage::Release;
            self.pos = 0;
        }
    }

    /// Move on from any stage that has run its length.
    #[inline]
    fn settle(&mut self, s: &EnvShape) {
        if self.stage == Stage::Attack && self.pos >= s.attack {
            self.pos -= s.attack;
            self.stage = Stage::Decay;
        }
        if self.stage == Stage::Decay && self.pos >= s.decay {
            self.stage = Stage::Sustain;
        }
        if self.stage == Stage::Release && self.pos >= s.release {
            self.stage = Stage::Idle;
        }
    }

    /// The level now.
    #[inline]
    pub fn level(&mut self, s: &EnvShape) -> f32 {
        self.settle(s);
        match self.stage {
            Stage::Idle => 0.0,
            Stage::Attack => {
                self.from + (1.0 - self.from) * s.bend(self.pos as f32 * s.inv_attack)
            }
            Stage::Decay => 1.0 + (s.sustain - 1.0) * s.bend(self.pos as f32 * s.inv_decay),
            Stage::Sustain => s.sustain,
            Stage::Release => self.from * (1.0 - s.bend(self.pos as f32 * s.inv_release)),
        }
    }

    /// The level now, then step on one sample.
    #[inline]
    pub fn next(&mut self, s: &EnvShape) -> f32 {
        let l = self.level(s);
        if matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Release) {
            self.pos += 1;
        }
        l
    }

    /// Step on `n` samples, then the level.
    #[inline]
    pub fn advance(&mut self, n: u32, s: &EnvShape) -> f32 {
        if matches!(self.stage, Stage::Attack | Stage::Decay | Stage::Release) {
            self.pos = self.pos.saturating_add(n);
        }
        self.level(s)
    }

    /// True once the envelope can make no more sound.
    #[inline]
    pub fn finished(&self, s: &EnvShape) -> bool {
        self.stage == Stage::Idle || (self.stage == Stage::Sustain && s.sustain <= 0.0)
    }
}
