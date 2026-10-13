//! Samples: recorded sound, played back at any pitch.

pub const LOOP_OFF: u8 = 0;
pub const LOOP_FORWARD: u8 = 1;
pub const LOOP_PINGPONG: u8 = 2;

/// A loop: which way it goes round, its first frame, and one past its last.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Loop {
    pub mode: u8,
    pub start: u32,
    pub end: u32,
}

/// A loaded sample.
pub struct Sample {
    /// The sound, one number per channel per frame (left, right, left, ...).
    pub data: Box<[f32]>,
    pub frames: u32,
    pub channels: u32,
    /// The sample rate it was recorded at.
    pub rate: f32,
    /// The note at which it plays back unchanged.
    pub base_note: f32,
    pub loop_mode: u8,
    /// The first frame of the loop.
    pub loop_start: u32,
    /// One past the last frame of the loop.
    pub loop_end: u32,
}

impl Sample {
    /// The loop mode, or off if the loop points make no sense.
    #[inline]
    pub fn looping(&self) -> u8 {
        if self.loop_end > self.loop_start && self.loop_end <= self.frames {
            self.loop_mode
        } else {
            LOOP_OFF
        }
    }

    /// The sample's own loop, as a voice plays it.
    #[inline]
    pub fn own_loop(&self) -> Loop {
        Loop { mode: self.looping(), start: self.loop_start, end: self.loop_end }
    }

    /// A loop of someone else's choosing, brought inside this sample: an
    /// end past the last frame is brought back to it, and a loop with no
    /// frames in it is no loop.
    #[inline]
    pub fn fit_loop(&self, mode: u8, start: u32, end: u32) -> Loop {
        let end = end.min(self.frames);
        let start = start.min(end);
        Loop { mode: if end > start { mode.min(LOOP_PINGPONG) } else { LOOP_OFF }, start, end }
    }

    /// The frame at index `v`, where `v` counts on past the end of the loop as
    /// if the loop were laid out end to end.
    #[inline]
    fn fetch(&self, mut v: i64, ch: usize, lp: &Loop, looped: bool) -> f32 {
        let ls = lp.start as i64;
        let le = lp.end as i64;
        let len = le - ls;
        match lp.mode {
            LOOP_FORWARD => {
                if v >= le {
                    v = ls + (v - le) % len;
                } else if v < ls && looped {
                    v = le - 1 - (ls - 1 - v) % len;
                }
            }
            LOOP_PINGPONG => {
                if v >= le {
                    let o = (v - le) % (2 * len);
                    v = if o < len { le - 1 - o } else { ls + (o - len) };
                } else if v < ls && looped {
                    let o = (ls - 1 - v) % (2 * len);
                    v = if o < len { ls + o } else { le - 1 - (o - len) };
                }
            }
            _ => {}
        }
        if v < 0 || v >= self.frames as i64 {
            return 0.0;
        }
        self.data.get(v as usize * self.channels as usize + ch).copied().unwrap_or(0.0)
    }
}

/// A smooth curve through four neighbouring samples (Catmull-Rom); `f` is how
/// far between `p1` and `p2`.
#[inline]
fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, f: f32) -> f32 {
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);
    ((c3 * f + c2) * f + c1) * f + p1
}

/// Above this many frames of the sample per frame of output, the smoothing
/// against aliasing stops getting wider (it would cost too much).
const WIDEST: f32 = 8.0;

/// Where a voice is in a sample.
#[derive(Clone, Copy, Default)]
pub struct Player {
    /// The position in frames. In a ping-pong loop it counts forwards through
    /// the loop and then on through its mirror image.
    pub pos: f64,
    /// How many frames to move each output sample.
    pub inc: f32,
    pub dinc: f32,
    /// A one-shot sample has reached its end.
    pub ended: bool,
    /// Play backwards.
    pub reverse: bool,
    /// Smooth the sample when it is played faster than it was recorded.
    pub antialias: bool,
    /// The position has reached the end of the loop since this was last
    /// cleared (the voice clears it).
    pub wrapped: bool,
    /// The loop has been round at least once.
    looped: bool,
    /// The last value played before the end, fading out so the end does not click.
    tail: [f32; 2],
}

impl Player {
    pub fn start(&mut self, pos: f64) {
        self.pos = if pos.is_finite() && pos > 0.0 { pos } else { 0.0 };
        self.ended = false;
        self.looped = false;
        self.tail = [0.0; 2];
    }

    /// Start a note that plays backwards, `offset` frames in from where it
    /// would otherwise begin: the last frame of a one-shot, or the end of a loop.
    pub fn start_reversed(&mut self, s: &Sample, lp: &Loop, offset: f64) {
        let offset = if offset.is_finite() && offset > 0.0 { offset } else { 0.0 };
        match lp.mode {
            LOOP_OFF => self.start((s.frames as f64 - 1.0 - offset).max(0.0)),
            LOOP_FORWARD => {
                let (ls, le) = (lp.start as f64, lp.end as f64);
                self.start(le - 1.0 - offset % (le - ls));
                self.looped = true;
            }
            _ => {
                // The way back is the second half of a ping-pong loop.
                let (ls, le) = (lp.start as f64, lp.end as f64);
                self.start(le + offset % (le - ls));
                self.looped = true;
            }
        }
    }

    /// Bring the position back inside the sample or its loop. `back` is true
    /// when the position is moving towards the start.
    #[inline]
    fn wrap(&mut self, s: &Sample, lp: &Loop, back: bool) {
        match lp.mode {
            LOOP_OFF => {
                if self.pos >= s.frames as f64 || self.pos < 0.0 {
                    self.ended = true;
                }
            }
            LOOP_FORWARD => {
                let (ls, le) = (lp.start as f64, lp.end as f64);
                if back {
                    if self.pos < ls {
                        self.pos = le - (ls - self.pos) % (le - ls);
                        if self.pos >= le {
                            self.pos = ls;
                        }
                        self.looped = true;
                        self.wrapped = true;
                    }
                } else if self.pos >= le {
                    self.pos = ls + (self.pos - ls) % (le - ls);
                    self.looped = true;
                    self.wrapped = true;
                }
            }
            _ => {
                let (ls, le) = (lp.start as f64, lp.end as f64);
                let span = 2.0 * (le - ls);
                if self.pos >= le {
                    self.wrapped |= !self.looped;
                    self.looped = true;
                }
                if self.pos >= ls + span {
                    self.pos = ls + (self.pos - ls) % span;
                    self.wrapped = true;
                }
            }
        }
    }

    /// The sample at the position, from a smooth curve through the four
    /// frames around it.
    #[inline]
    fn read(&self, s: &Sample, lp: &Loop, plain_end: i64) -> (f32, f32) {
        let stereo = s.channels == 2;
        let ip = self.pos as i64;
        let f = (self.pos - ip as f64) as f32;
        if ip >= 1 && ip < plain_end && (!self.looped || ip > lp.start as i64) {
            // The four frames are plain neighbours.
            let at = ip as usize - 1;
            if stereo {
                match s.data.get(at * 2..at * 2 + 8) {
                    Some(w) => (cubic(w[0], w[2], w[4], w[6], f), cubic(w[1], w[3], w[5], w[7], f)),
                    None => (0.0, 0.0),
                }
            } else {
                let l = match s.data.get(at..at + 4) {
                    Some(w) => cubic(w[0], w[1], w[2], w[3], f),
                    None => 0.0,
                };
                (l, l)
            }
        } else {
            let round = self.looped;
            let l = cubic(
                s.fetch(ip - 1, 0, lp, round),
                s.fetch(ip, 0, lp, round),
                s.fetch(ip + 1, 0, lp, round),
                s.fetch(ip + 2, 0, lp, round),
                f,
            );
            let r = if stereo {
                cubic(
                    s.fetch(ip - 1, 1, lp, round),
                    s.fetch(ip, 1, lp, round),
                    s.fetch(ip + 1, 1, lp, round),
                    s.fetch(ip + 2, 1, lp, round),
                    f,
                )
            } else {
                l
            };
            (l, r)
        }
    }

    /// The sample at the position when it is being played `width` times
    /// faster than it was recorded: a weighted average of the frames within
    /// `width` either side, the nearest counting most. That takes out most of
    /// what is too high to survive the speeding up, which would otherwise
    /// fold back down as harsh tones that were never in the sample.
    #[inline]
    fn read_wide(&self, s: &Sample, lp: &Loop, width: f32) -> (f32, f32) {
        let stereo = s.channels == 2;
        let lo = (self.pos - width as f64).ceil() as i64;
        let hi = (self.pos + width as f64).floor() as i64;
        let inv = 1.0 / width;
        let limit = if lp.mode == LOOP_OFF { s.frames as i64 } else { lp.end as i64 };
        let floor = if self.looped { lp.start as i64 } else { 0 };
        let (mut l, mut r, mut total) = (0.0f32, 0.0f32, 0.0f32);
        if lo >= floor && hi < limit {
            let mut d = (lo as f64 - self.pos) as f32 * inv;
            let (lo, hi) = (lo as usize, hi as usize);
            if stereo {
                if let Some(frames) = s.data.get(lo * 2..hi * 2 + 2) {
                    for w in frames.chunks_exact(2) {
                        let weight = 1.0 - d.abs();
                        l += w[0] * weight;
                        r += w[1] * weight;
                        total += weight;
                        d += inv;
                    }
                }
            } else if let Some(frames) = s.data.get(lo..hi + 1) {
                for &x in frames {
                    let weight = 1.0 - d.abs();
                    l += x * weight;
                    total += weight;
                    d += inv;
                }
            }
        } else {
            let round = self.looped;
            for j in lo..=hi {
                let weight = 1.0 - ((j as f64 - self.pos) as f32 * inv).abs();
                l += s.fetch(j, 0, lp, round) * weight;
                if stereo {
                    r += s.fetch(j, 1, lp, round) * weight;
                }
                total += weight;
            }
        }
        let norm = if total > 0.0 { 1.0 / total } else { 0.0 };
        if stereo {
            (l * norm, r * norm)
        } else {
            (l * norm, l * norm)
        }
    }

    /// Write the next samples into `left` (and `right`, if given; without it
    /// a stereo sample is mixed to one channel). `lp` is the loop to play:
    /// the sample's own, or another that fits inside it.
    pub fn render(&mut self, s: &Sample, lp: &Loop, left: &mut [f32], mut right: Option<&mut [f32]>) {
        let mode = lp.mode;
        let stereo = s.channels == 2;
        // A ping-pong loop already goes both ways: there, playing backwards
        // only changes where the note starts.
        let back = self.reverse && mode != LOOP_PINGPONG;
        self.wrap(s, lp, back);
        // Below this, the four frames around the position are plain neighbours.
        let plain_end = if mode == LOOP_OFF { s.frames as i64 } else { lp.end as i64 } - 2;
        for i in 0..left.len() {
            if self.ended {
                self.tail = [self.tail[0] * 0.97, self.tail[1] * 0.97];
                match right.as_deref_mut() {
                    Some(r) => {
                        left[i] = self.tail[0];
                        r[i] = self.tail[1];
                    }
                    None => left[i] = if stereo { 0.5 * (self.tail[0] + self.tail[1]) } else { self.tail[0] },
                }
                continue;
            }
            let (l, r) = if self.antialias && self.inc > 1.0 {
                let wide = self.read_wide(s, lp, self.inc.min(WIDEST));
                if self.inc < 2.0 {
                    // Up to an octave above its own speed, cross over gradually.
                    let mix = self.inc - 1.0;
                    let fine = self.read(s, lp, plain_end);
                    (fine.0 + (wide.0 - fine.0) * mix, fine.1 + (wide.1 - fine.1) * mix)
                } else {
                    wide
                }
            } else {
                self.read(s, lp, plain_end)
            };
            match right.as_deref_mut() {
                Some(rb) => {
                    left[i] = l;
                    rb[i] = r;
                }
                None => left[i] = if stereo { 0.5 * (l + r) } else { l },
            }
            self.inc += self.dinc;
            let step = self.inc.max(0.0) as f64;
            self.pos += if back { -step } else { step };
            self.wrap(s, lp, back);
            if self.ended {
                self.tail = [l, r];
            }
        }
    }
}
