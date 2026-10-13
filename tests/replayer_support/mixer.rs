//! A plain mixer that plays what the replayer tells it to, for tests and
//! for rendering songs to listen to. It is not the engine: it is the least
//! that honours the `Voices` contract, so a replayer bug cannot hide behind
//! clever sound code.
//!
//! The replayer's calls are queued with their frame and take effect at
//! exactly that frame when `render` runs.

use synths_core::replayer::{Source, Start, Voices};
use synths_core::song::{LoopKind, Song};

/// Something the replayer asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play(Start),
    Rate(f64),
    /// The volume, and how many frames to take getting there.
    Volume(f32, u32),
    Pan(f32, u32),
    Stop,
    Release,
    Queue(Option<usize>),
    Filter(bool),
}

/// A command, the frame it was for (counted from the start of the song)
/// and its channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Logged {
    pub frame: u64,
    pub channel: usize,
    pub command: Command,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    sample: Option<usize>,
    pos: f64,
    backwards: bool,
    rate: f64,
    volume: f32,
    volume_now: f32,
    /// How much the volume moves each frame on its way to `volume`.
    volume_step: f32,
    pan: f32,
    pan_now: f32,
    pan_step: f32,
    /// A sample waiting to take over at the end of this one's pass.
    queued: Option<Option<usize>>,
    /// The frame a note started on: settings that arrive on the same frame
    /// are its starting values and are not smoothed.
    fresh: Option<u64>,
}

pub struct Mixer<'a> {
    song: &'a Song,
    out_rate: f64,
    voices: Vec<Voice>,
    pending: Vec<Logged>,
    /// Frames rendered so far.
    time: u64,
    /// Everything the replayer has asked for.
    pub log: Vec<Logged>,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Smooth volume and panning changes over at least this many frames; 0
    /// to do exactly as the replayer asks.
    pub ramp: u32,
    /// Take the nearest frame of a sample and not a line between two.
    pub nearest: bool,
    /// FastTracker 2's panning (square root) and not a straight line.
    pub square_root_pan: bool,
    /// The loudness of one channel at full volume.
    pub gain: f32,
    /// Channels that are played but not heard.
    pub muted: Vec<bool>,
}

impl<'a> Mixer<'a> {
    pub fn new(song: &'a Song, out_rate: f64) -> Mixer<'a> {
        Mixer {
            song,
            out_rate,
            voices: vec![Voice::default(); song.channels()],
            pending: Vec::new(),
            time: 0,
            log: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
            ramp: (out_rate * 0.002) as u32,
            nearest: false,
            square_root_pan: song.rules == synths_core::song::Rules::FastTracker2,
            gain: 0.25,
            muted: vec![false; song.channels()],
        }
    }

    fn push(&mut self, frame: u32, channel: usize, command: Command) {
        let l = Logged { frame: self.time + frame as u64, channel, command };
        self.pending.push(l);
        self.log.push(l);
    }

    fn apply(&mut self, l: Logged) {
        let song = self.song;
        let Some(v) = self.voices.get_mut(l.channel) else { return };
        let fresh = v.fresh == Some(l.frame);
        match l.command {
            Command::Play(start) => {
                *v = Voice { rate: start.rate, volume: start.volume, volume_now: start.volume, pan: start.pan, pan_now: start.pan, fresh: Some(l.frame), ..Voice::default() };
                let Source::Sample(i) = start.source else { return };
                let Some(s) = song.samples.get(i) else { return };
                let end = s.play_end();
                v.sample = Some(i);
                v.pos = start.offset as f64;
                if start.offset >= end {
                    match s.loop_range() {
                        Some((a, _)) => v.pos = a as f64,
                        None => v.sample = None,
                    }
                }
            }
            Command::Rate(r) => v.rate = r,
            Command::Volume(x, ramp) => {
                v.volume = x;
                let ramp = ramp.max(self.ramp);
                if fresh || ramp == 0 {
                    v.volume_now = x;
                } else {
                    v.volume_step = (x - v.volume_now).abs() / ramp as f32;
                }
            }
            Command::Pan(x, ramp) => {
                v.pan = x;
                let ramp = ramp.max(self.ramp);
                if fresh || ramp == 0 {
                    v.pan_now = x;
                } else {
                    v.pan_step = (x - v.pan_now).abs() / ramp as f32;
                }
            }
            Command::Stop => v.sample = None,
            Command::Queue(q) => {
                if v.sample.is_none() {
                    start_queued(v, song, q);
                } else {
                    v.queued = Some(q);
                }
            }
            Command::Release | Command::Filter(_) => {}
        }
    }

    /// Mixes the next `frames` frames onto the end of `left` and `right`.
    /// Call it after the replayer has processed the same frames.
    pub fn render(&mut self, frames: u32) {
        let mut pending = core::mem::take(&mut self.pending);
        pending.sort_by_key(|l| l.frame);
        let mut next = 0;
        for _ in 0..frames {
            while next < pending.len() && pending[next].frame <= self.time {
                self.apply(pending[next]);
                next += 1;
            }
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for (c, v) in self.voices.iter_mut().enumerate() {
                v.volume_now += (v.volume - v.volume_now).clamp(-v.volume_step, v.volume_step);
                v.pan_now += (v.pan - v.pan_now).clamp(-v.pan_step, v.pan_step);
                let Some(s) = v.sample.and_then(|i| self.song.samples.get(i)) else { continue };
                let i = v.pos.floor();
                let frac = (v.pos - i) as f32;
                let i = i as usize;
                let a = s.data.get(i);
                let x = if self.nearest {
                    a
                } else {
                    // The frame after this one, which at the end of a loop is
                    // the loop's first (or, going back and forth, this one).
                    let j = match s.loop_range() {
                        Some((start, end)) if i + 1 >= end as usize => {
                            if s.loop_kind == LoopKind::PingPong { i } else { start as usize }
                        }
                        _ => i + 1,
                    };
                    let b = s.data.get(j);
                    a + (b - a) * frac
                };
                let p = (v.pan_now.clamp(-1.0, 1.0) + 1.0) * 0.5;
                let (gl, gr) = if self.square_root_pan { ((1.0 - p).sqrt(), p.sqrt()) } else { (1.0 - p, p) };
                let x = if self.muted[c] { 0.0 } else { x * v.volume_now * self.gain };
                l += x * gl;
                r += x * gr;

                let step = v.rate / self.out_rate;
                v.pos += if v.backwards { -step } else { step };
                advance(v, self.song);
            }
            self.left.push(l);
            self.right.push(r);
            self.time += 1;
        }
        // Anything for a later frame waits for the next call.
        self.pending = pending.split_off(next);
    }
}

/// Starts a queued sample in its loop, or silences the voice if it has none.
fn start_queued(v: &mut Voice, song: &Song, q: Option<usize>) {
    v.queued = None;
    v.backwards = false;
    match q.and_then(|i| song.samples.get(i).and_then(|s| s.loop_range()).map(|l| (i, l))) {
        Some((i, (start, _))) => {
            v.sample = Some(i);
            v.pos = start as f64;
        }
        None => v.sample = None,
    }
}

/// Keeps a voice's position inside its sample: loops it, turns it round or
/// ends it.
fn advance(v: &mut Voice, song: &Song) {
    let Some(s) = v.sample.and_then(|i| song.samples.get(i)) else { return };
    match s.loop_range() {
        None => {
            if v.pos >= s.data.len() as f64 {
                v.sample = None;
                if let Some(q) = v.queued {
                    start_queued(v, song, q);
                }
            }
        }
        Some((start, end)) => {
            let (start, end) = (start as f64, end as f64);
            let len = end - start;
            if s.loop_kind == LoopKind::PingPong {
                // Bounce between the ends until the position is inside.
                loop {
                    if !v.backwards && v.pos >= end {
                        v.pos = end - 1.0 - (v.pos - end);
                        v.backwards = true;
                    } else if v.backwards && v.pos < start {
                        v.pos = start + (start - v.pos) - 1.0;
                        v.backwards = false;
                    } else {
                        break;
                    }
                    if v.pos >= end + len || v.pos < start - len {
                        v.pos = start;
                        v.backwards = false;
                    }
                }
            } else if v.pos >= end {
                if let Some(q) = v.queued {
                    start_queued(v, song, q);
                } else {
                    v.pos = start + (v.pos - end) % len;
                }
            }
        }
    }
}

impl Voices for Mixer<'_> {
    fn play(&mut self, frame: u32, channel: usize, start: &Start) {
        self.push(frame, channel, Command::Play(*start));
    }
    fn set_rate(&mut self, frame: u32, channel: usize, rate: f64) {
        self.push(frame, channel, Command::Rate(rate));
    }
    fn set_volume(&mut self, frame: u32, channel: usize, volume: f32, ramp: u32) {
        self.push(frame, channel, Command::Volume(volume, ramp));
    }
    fn set_pan(&mut self, frame: u32, channel: usize, pan: f32, ramp: u32) {
        self.push(frame, channel, Command::Pan(pan, ramp));
    }
    fn stop(&mut self, frame: u32, channel: usize) {
        self.push(frame, channel, Command::Stop);
    }
    fn release(&mut self, frame: u32, channel: usize) {
        self.push(frame, channel, Command::Release);
    }
    fn queue(&mut self, frame: u32, channel: usize, sample: Option<usize>) {
        self.push(frame, channel, Command::Queue(sample));
    }
    fn set_filter(&mut self, frame: u32, on: bool) {
        self.push(frame, 0, Command::Filter(on));
    }
}

/// Plays a song from where the player is for `frames` frames, in blocks.
pub fn play(song: &Song, player: &mut synths_core::replayer::Player, mixer: &mut Mixer, frames: u64) {
    let mut left = frames;
    while left > 0 {
        let n = left.min(1024) as u32;
        player.process(song, n, mixer);
        mixer.render(n);
        left -= n as u64;
    }
}

/// A stereo 16-bit WAV file of the mixer's output.
pub fn wav_bytes(left: &[f32], right: &[f32], rate: u32) -> Vec<u8> {
    let size = left.len() as u32 * 4;
    let mut out = Vec::with_capacity(44 + size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + size).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&size.to_le_bytes());
    for (l, r) in left.iter().zip(right) {
        for x in [l, r] {
            out.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
    }
    out
}
