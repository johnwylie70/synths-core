//! Small maths helpers shared by the whole engine.

pub const PI: f32 = core::f32::consts::PI;
pub const TAU: f32 = core::f32::consts::TAU;

/// MIDI note number (69 = A4 = 440 Hz, fractions allowed) to frequency in Hz.
#[inline]
pub fn note_to_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) * (1.0 / 12.0)).exp2()
}

/// Frequency in Hz to a MIDI note number.
#[inline]
pub fn hz_to_note(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// A number of semitones to a frequency ratio.
#[inline]
pub fn semis_to_ratio(semis: f32) -> f32 {
    (semis * (1.0 / 12.0)).exp2()
}

/// sin(2 pi t) for t in 0..1, from a polynomial: the same answer on every
/// platform, and within 4 millionths of the true value.
#[inline]
pub fn fast_sin(t: f32) -> f32 {
    let x = if t < 0.25 {
        t
    } else if t < 0.75 {
        0.5 - t
    } else {
        t - 1.0
    };
    let x = x * TAU;
    let x2 = x * x;
    x * (1.0
        + x2 * (-1.0 / 6.0
            + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0)))))
}

/// A cheap tanh: exact at 0, reaches 1 at 3 and stays there.
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Very small numbers (denormals) make some processors crawl; turn them into zero.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-15 {
        0.0
    } else {
        x
    }
}

/// A small, fast random number generator (xorshift). Seeded, so the engine
/// makes the same noise every time it is given the same commands.
#[derive(Clone, Copy)]
pub struct Rng(pub u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A number from -1 to 1.
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        (self.next_u32() as i32) as f32 * (1.0 / 2_147_483_648.0)
    }

    /// A number from 0 to 1.
    #[inline]
    pub fn unipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// A whole number below 1000 as text, with no formatting machinery.
pub fn small_int(n: u32) -> String {
    let mut s = String::new();
    let n = n.min(999);
    if n >= 100 {
        s.push((b'0' + (n / 100) as u8) as char);
    }
    if n >= 10 {
        s.push((b'0' + (n / 10 % 10) as u8) as char);
    }
    s.push((b'0' + (n % 10) as u8) as char);
    s
}
