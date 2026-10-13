//! Samples: recorded sound, played back at any pitch.

pub const LOOP_OFF: u8 = 0;
pub const LOOP_FORWARD: u8 = 1;
pub const LOOP_PINGPONG: u8 = 2;

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
    fn looping(&self) -> u8 {
        if self.loop_end > self.loop_start && self.loop_end <= self.frames {
            self.loop_mode
        } else {
            LOOP_OFF
        }
    }

    /// The frame at index `v`, where `v` counts on past the end of the loop as
    /// if the loop were laid out end to end.
    #[inline]
    fn fetch(&self, mut v: i64, ch: usize, mode: u8, looped: bool) -> f32 {
        let ls = self.loop_start as i64;
        let le = self.loop_end as i64;
        let len = le - ls;
        match mode {
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
    /// The loop has been round at least once.
    looped: bool,
}

impl Player {
    pub fn start(&mut self, pos: f64) {
        self.pos = if pos.is_finite() && pos > 0.0 { pos } else { 0.0 };
        self.ended = false;
        self.looped = false;
    }

    /// Bring the position back inside the sample or its loop.
    #[inline]
    fn wrap(&mut self, s: &Sample, mode: u8) {
        match mode {
            LOOP_OFF => {
                if self.pos >= s.frames as f64 {
                    self.ended = true;
                }
            }
            LOOP_FORWARD => {
                let (ls, le) = (s.loop_start as f64, s.loop_end as f64);
                if self.pos >= le {
                    self.pos = ls + (self.pos - ls) % (le - ls);
                    self.looped = true;
                }
            }
            _ => {
                let (ls, le) = (s.loop_start as f64, s.loop_end as f64);
                let span = 2.0 * (le - ls);
                if self.pos >= le {
                    self.looped = true;
                }
                if self.pos >= ls + span {
                    self.pos = ls + (self.pos - ls) % span;
                }
            }
        }
    }

    /// Write the next samples into `left` (and `right`, if given; without it
    /// a stereo sample is mixed to one channel).
    pub fn render(&mut self, s: &Sample, left: &mut [f32], mut right: Option<&mut [f32]>) {
        let mode = s.looping();
        let stereo = s.channels == 2;
        self.wrap(s, mode);
        // Below this, the four frames around the position are plain neighbours.
        let plain_end = if mode == LOOP_OFF { s.frames as i64 } else { s.loop_end as i64 } - 2;
        for i in 0..left.len() {
            if self.ended {
                left[i] = 0.0;
                if let Some(r) = right.as_deref_mut() {
                    r[i] = 0.0;
                }
                continue;
            }
            let ip = self.pos as i64;
            let f = (self.pos - ip as f64) as f32;
            let (l, r);
            if ip >= 1 && ip < plain_end && (!self.looped || ip > s.loop_start as i64) {
                let at = ip as usize - 1;
                if stereo {
                    match s.data.get(at * 2..at * 2 + 8) {
                        Some(w) => {
                            l = cubic(w[0], w[2], w[4], w[6], f);
                            r = cubic(w[1], w[3], w[5], w[7], f);
                        }
                        None => {
                            l = 0.0;
                            r = 0.0;
                        }
                    }
                } else {
                    l = match s.data.get(at..at + 4) {
                        Some(w) => cubic(w[0], w[1], w[2], w[3], f),
                        None => 0.0,
                    };
                    r = l;
                }
            } else {
                let lp = self.looped;
                l = cubic(
                    s.fetch(ip - 1, 0, mode, lp),
                    s.fetch(ip, 0, mode, lp),
                    s.fetch(ip + 1, 0, mode, lp),
                    s.fetch(ip + 2, 0, mode, lp),
                    f,
                );
                r = if stereo {
                    cubic(
                        s.fetch(ip - 1, 1, mode, lp),
                        s.fetch(ip, 1, mode, lp),
                        s.fetch(ip + 1, 1, mode, lp),
                        s.fetch(ip + 2, 1, mode, lp),
                        f,
                    )
                } else {
                    l
                };
            }
            match right.as_deref_mut() {
                Some(rb) => {
                    left[i] = l;
                    rb[i] = r;
                }
                None => left[i] = if stereo { 0.5 * (l + r) } else { l },
            }
            self.inc += self.dinc;
            self.pos += self.inc.max(0.0) as f64;
            self.wrap(s, mode);
        }
    }
}
