//! Taps: what a host reads to draw the sound. A scope (the most recent
//! output, for a waveform display), level meters, and which notes are
//! sounding (to light the keys of a keyboard).
//!
//! The audio side only ever stores single numbers here and the reading side
//! only ever loads them, so a display can read while sound is being made,
//! from another thread, with no lock. A reading taken mid-render may be a
//! few frames out; it is for drawing, not for arithmetic.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

/// How many frames of the master output the scope keeps.
pub const SCOPE_MASTER: usize = 4096;
/// How many frames of each part the scope keeps.
pub const SCOPE_PART: usize = 1024;
/// The source number that means the master output (or, for notes, every part).
pub const MASTER: u32 = 0xFFFF;
/// Numbers per level reading: peak left, peak right, RMS left, RMS right.
pub const LEVEL_SIZE: usize = 4;
/// Words per notes reading: 128 bits of notes held, then 128 of notes sounding.
pub const NOTES_SIZE: usize = 8;

fn words(n: usize) -> Box<[AtomicU32]> {
    (0..n).map(|_| AtomicU32::new(0)).collect()
}

pub struct Taps {
    parts: usize,
    /// The engine's time.
    time: AtomicU64,
    /// The time just past the last frame written to each scope: the master's
    /// at index `parts`. Frames from then on are silence.
    written: Box<[AtomicU64]>,
    master: Box<[AtomicU32]>,
    part_scopes: Box<[AtomicU32]>,
    levels: Box<[AtomicU32]>,
    notes: Box<[AtomicU32]>,
}

impl Taps {
    pub fn new(parts: usize) -> Taps {
        Taps {
            parts,
            time: AtomicU64::new(0),
            written: (0..parts + 1).map(|_| AtomicU64::new(0)).collect(),
            master: words(SCOPE_MASTER),
            part_scopes: words(parts * SCOPE_PART),
            levels: words((parts + 1) * LEVEL_SIZE),
            notes: words(parts * NOTES_SIZE),
        }
    }

    // ---- the audio side ----

    pub(crate) fn set_time(&self, time: u64) {
        self.time.store(time, Relaxed);
    }

    /// The ring and stamp of a source (a part, or `parts` for the master).
    fn ring(&self, source: usize) -> (&[AtomicU32], &AtomicU64) {
        if source >= self.parts {
            (&self.master[..], &self.written[self.parts])
        } else {
            (&self.part_scopes[source * SCOPE_PART..(source + 1) * SCOPE_PART], &self.written[source])
        }
    }

    /// Write the frames from `time` on: the mean of `left` and `right`.
    pub(crate) fn write_scope(&self, source: usize, time: u64, left: &[f32], right: &[f32]) {
        let (ring, stamp) = self.ring(source);
        let mask = ring.len() as u64 - 1;
        // If this source has been silent, the silence goes in first.
        let last = stamp.load(Relaxed);
        if last < time {
            let gap = (time - last).min(ring.len() as u64);
            for t in time - gap..time {
                ring[(t & mask) as usize].store(0, Relaxed);
            }
        }
        for (i, (l, r)) in left.iter().zip(right.iter()).enumerate() {
            ring[((time + i as u64) & mask) as usize].store((0.5 * (l + r)).to_bits(), Relaxed);
        }
        stamp.store(time + left.len() as u64, Relaxed);
    }

    pub(crate) fn write_levels(&self, source: usize, levels: [f32; LEVEL_SIZE]) {
        let at = source.min(self.parts) * LEVEL_SIZE;
        for (i, v) in levels.iter().enumerate() {
            self.levels[at + i].store(v.to_bits(), Relaxed);
        }
    }

    pub(crate) fn write_notes(&self, part: usize, bits: &[u32; NOTES_SIZE]) {
        if part < self.parts {
            for (i, w) in bits.iter().enumerate() {
                self.notes[part * NOTES_SIZE + i].store(*w, Relaxed);
            }
        }
    }

    // ---- the reading side ----

    /// The most recent frames of a part's sound (or the master's, for
    /// `MASTER`), oldest first, each the mean of left and right. Fills as
    /// much of `out` as the scope holds and returns how many that was.
    pub fn read_scope(&self, source: u32, out: &mut [f32]) -> usize {
        let source = if source == MASTER {
            self.parts
        } else if (source as usize) < self.parts {
            source as usize
        } else {
            return 0;
        };
        let (ring, stamp) = self.ring(source);
        let n = out.len().min(ring.len());
        let mask = ring.len() as u64 - 1;
        let now = self.time.load(Relaxed);
        let last = stamp.load(Relaxed);
        for (i, o) in out[..n].iter_mut().enumerate() {
            // Frame `t`; frames before the engine began, and since this
            // source last sounded, are silence.
            let back = (n - i) as u64;
            *o = if back > now {
                0.0
            } else {
                let t = now - back;
                if t >= last || last - t > ring.len() as u64 {
                    0.0
                } else {
                    f32::from_bits(ring[(t & mask) as usize].load(Relaxed))
                }
            };
        }
        n
    }

    /// The level of a part (or the master): peak left, peak right, RMS left,
    /// RMS right, each falling back slowly after the sound does.
    pub fn read_levels(&self, source: u32) -> Option<[f32; LEVEL_SIZE]> {
        let source = if source == MASTER {
            self.parts
        } else if (source as usize) < self.parts {
            source as usize
        } else {
            return None;
        };
        let mut v = [0.0; LEVEL_SIZE];
        for (i, o) in v.iter_mut().enumerate() {
            *o = f32::from_bits(self.levels[source * LEVEL_SIZE + i].load(Relaxed));
        }
        Some(v)
    }

    /// The notes of a part (or of every part together, for `MASTER`): bit
    /// `n % 32` of word `n / 32` is set while note `n` is held, and the same
    /// in words 4 to 7 while it is sounding at all (held or dying away).
    pub fn read_notes(&self, part: u32) -> Option<[u32; NOTES_SIZE]> {
        let mut v = [0u32; NOTES_SIZE];
        let range = if part == MASTER {
            0..self.parts
        } else if (part as usize) < self.parts {
            part as usize..part as usize + 1
        } else {
            return None;
        };
        for p in range {
            for (i, o) in v.iter_mut().enumerate() {
                *o |= self.notes[p * NOTES_SIZE + i].load(Relaxed);
            }
        }
        Some(v)
    }
}
