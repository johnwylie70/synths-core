//! The replayer: plays a `Song` by telling a set of voices what to do and
//! when.
//!
//! It makes no sound itself. Given a song it counts ticks and rows at the
//! song's tempo, works out every effect, and calls a `Voices` for each
//! thing a channel must do: start a sample, change its rate, its volume or
//! its panning, or stop. Every call carries the frame it happens at, so a
//! set of voices that honours the frame is accurate to the sample.
//!
//! Two sets of rules are built in, because songs depend on the quirks of
//! the tracker they were written in: ProTracker's (`pt`) and FastTracker
//! 2's (`ft2`). The song says which it wants.

pub mod ft2;
pub mod pt;
pub mod tables;

use crate::song::{Cell, Chord, ChordOrder, ChordRate, Rules, Song, C4_RATE, MAX_CHORD, NOTE_MAX};

/// What a channel plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// One of the song's samples, by index.
    Sample(usize),
    /// A synth patch: the index of the song's instrument that names it.
    Patch(usize),
}

/// Everything a voice needs in order to begin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Start {
    pub source: Source,
    /// The frame of the sample to begin at. At or past the end of what the
    /// sample plays (the end of its loop, or of the sample if it has none),
    /// a looped sample begins at the start of its loop and an unlooped one
    /// is silent.
    pub offset: u32,
    /// Frames of the sample to play each second (see `Voices::set_rate`).
    pub rate: f64,
    /// 0 to 1.
    pub volume: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
}

/// A chord for one voice to play by stepping quickly through its notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arpeggio {
    /// Each note, in semitones from the pitch the channel is playing.
    pub offsets: [i8; MAX_CHORD],
    /// How many of `offsets` are used, 1 to 8.
    pub count: u8,
    /// Steps a second. A chord timed in ticks has been worked out at the
    /// song's tempo when the chord began.
    pub rate: f32,
    pub order: ChordOrder,
}

impl Arpeggio {
    pub fn offsets(&self) -> &[i8] {
        &self.offsets[..(self.count as usize).min(MAX_CHORD)]
    }

    fn of(chord: &Chord, tempo: u16) -> Arpeggio {
        let rate = match chord.rate {
            ChordRate::PerTick(steps) => steps * tempo as f32 / 2.5,
            ChordRate::PerSecond(steps) => steps,
        };
        Arpeggio { offsets: chord.offsets, count: chord.count, rate, order: chord.order }
    }
}

/// The chord a channel is playing, as a row's event leaves it. True if it
/// changed. A chord lasts until an event stops it or the channel gets a new
/// note with no chord of its own.
pub(crate) fn chord_after_row(now: &mut Option<Arpeggio>, chord: Option<Chord>, cell: Cell, tempo: u16) -> bool {
    let next = match chord {
        Some(chord) if chord.is_off() => None,
        Some(chord) => Some(Arpeggio::of(&chord, tempo)),
        None if (1..=NOTE_MAX).contains(&cell.note) => None,
        None => return false,
    };
    let changed = *now != next;
    *now = next;
    changed
}

/// The voices a replayer drives: one for each channel of the song.
///
/// `frame` is when the thing happens, counted from the start of the block
/// the replayer was asked to process. Calls arrive in order of frame.
///
/// A channel plays one thing at a time: `play` replaces whatever the
/// channel was playing. Settings that arrive on the same frame as a `play`
/// belong to the new note and take effect at once, without smoothing.
pub trait Voices {
    /// Starts a sample (or a patch) on a channel.
    fn play(&mut self, frame: u32, channel: usize, start: &Start);

    /// Sets how fast the channel's sample plays, in frames of the sample a
    /// second. For a patch, `note_of_rate` turns it into a note. A rate of 0
    /// holds the sample still.
    fn set_rate(&mut self, frame: u32, channel: usize, rate: f64);

    /// Sets the channel's volume, 0 to 1. Envelopes, fades, tremolo and the
    /// song's global volume are already worked in. `ramp` is how many frames
    /// the change should be spread over: FastTracker 2 glides a volume to its
    /// new value over a whole tick, which is what makes its envelopes smooth,
    /// and takes about 5 milliseconds over a sudden change. 0 means at once,
    /// as on the Amiga; the voices may still take the edge off to avoid a click.
    fn set_volume(&mut self, frame: u32, channel: usize, volume: f32, ramp: u32);

    /// Sets the channel's panning, -1 (left) to 1 (right), spread over `ramp`
    /// frames as for `set_volume`. FastTracker 2 pans by the square root:
    /// with `p = (pan + 1) / 2`, the left is `sqrt(1 - p)` and the right
    /// `sqrt(p)`. The Amiga's channels are simply hard left or hard right.
    fn set_pan(&mut self, frame: u32, channel: usize, pan: f32, ramp: u32);

    /// Silences the channel and forgets what it was playing.
    fn stop(&mut self, frame: u32, channel: usize);

    /// The key was let go. Samples need nothing done: the replayer fades
    /// them with `set_volume`. A synth patch should begin its release.
    fn release(&mut self, _frame: u32, _channel: usize) {}

    /// Gives the channel's voice a chord to step through, or with `None`
    /// takes it away. The voice plays each offset in turn at the arpeggio's
    /// rate, on top of whatever rate it is set to, until told otherwise or
    /// until the channel's next `play` (which starts without one: the
    /// replayer sends the chord again after a `play` if it still holds).
    /// Voices that ignore this play the chord's root alone.
    fn set_arpeggio(&mut self, _frame: u32, _channel: usize, _arpeggio: Option<&Arpeggio>) {}

    /// ProTracker only: the sample to carry on with when the one playing
    /// next reaches its end or the end of its loop (at once if it has
    /// already ended). The new sample starts at its loop and repeats it; if
    /// it has no loop, or `sample` is `None`, the channel goes silent
    /// instead. This is how a ProTracker song swaps samples mid-note; voices
    /// that ignore it play such songs with the wrong sample here and there.
    fn queue(&mut self, _frame: u32, _channel: usize, _sample: Option<usize>) {}

    /// ProTracker only: switches the Amiga's low-pass "LED" filter.
    fn set_filter(&mut self, _frame: u32, _on: bool) {}
}

/// The note (60 is middle C, with fractions) that matches a sample rate,
/// taking C-4 at 8363 frames a second as middle C. Voices use it to play a
/// synth patch at the pitch the song asks for.
pub fn note_of_rate(rate: f64) -> f64 {
    60.0 + 12.0 * (rate / C4_RATE).log2()
}

/// Where the song has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    /// The place in the order list.
    pub order: u16,
    /// The pattern playing there.
    pub pattern: u16,
    pub row: u16,
    /// The tick within the row, from 0.
    pub tick: u16,
    /// Ticks in a row.
    pub speed: u16,
    /// Beats a minute: a tick lasts 2.5 / tempo seconds.
    pub tempo: u16,
    /// How many times the song has come back round to a row it had played.
    pub loops: u32,
}

/// Which rows have been played, so the song's end can be noticed: a song
/// has ended when it reaches a row it has played before.
pub(crate) struct Visited {
    bits: Vec<u64>,
    pub loops: u32,
    /// Play the song round and round (as trackers do), or stop at its end.
    pub repeat: bool,
    /// The song reached its end with `repeat` off, and plays no more.
    pub stopped: bool,
    /// The order list ran out and the song went back to its restart place:
    /// the next row played is the song beginning again.
    pub wrapped: bool,
    /// A place in the order to stay in, playing its pattern round and round.
    pub hold: Option<usize>,
}

impl Visited {
    const ROWS: usize = crate::song::MAX_ROWS;

    pub fn new(orders: usize) -> Visited {
        Visited { bits: vec![0; (orders.max(1) * Self::ROWS).div_ceil(64)], loops: 0, repeat: true, stopped: false, wrapped: false, hold: None }
    }

    /// The same record for a song that has replaced the one being played:
    /// nothing of the new song has been played yet.
    pub fn renewed(&self, orders: usize) -> Visited {
        Visited { loops: self.loops, repeat: self.repeat, stopped: self.stopped, hold: self.hold.map(|h| h.min(orders.max(1) - 1)), ..Visited::new(orders) }
    }

    /// Called with the place the next row is at, before it is played. If
    /// one place in the order is being played round and round, and the song
    /// has just left it (off the end of its pattern, or by a jump), this
    /// puts it back, at the top unless a pattern break named a row. Returns
    /// true while a place is being held: its rows come round again without
    /// the song having ended.
    pub fn keep(&mut self, order: &mut usize, row: &mut usize, song: &Song) -> bool {
        let Some(hold) = self.hold else { return false };
        if *order != hold {
            *order = hold.min(song.orders.len().saturating_sub(1));
            if *row >= song.rows_at(*order) as usize {
                *row = 0;
            }
        }
        self.wrapped = false;
        true
    }

    pub fn clear(&mut self) {
        self.bits.fill(0);
        self.wrapped = false;
    }

    /// Counts the song as having ended (a command stopped it).
    pub fn end(&mut self) {
        self.loops += 1;
        self.stopped = !self.repeat;
    }

    /// Notes that a row is about to be played, and says whether to play it.
    /// Coming to a row for the second time counts one more loop and starts
    /// the record afresh, unless a pattern loop is under way (`looping`):
    /// that plays rows again without the song having ended.
    pub fn visit(&mut self, order: usize, row: usize, looping: bool) -> bool {
        let i = order * Self::ROWS + row.min(Self::ROWS - 1);
        let Some(word) = self.bits.get(i / 64).copied() else { return true };
        let wrapped = core::mem::take(&mut self.wrapped);
        if wrapped || (word & (1 << (i % 64)) != 0 && !looping) {
            self.end();
            self.clear();
        }
        self.bits[i / 64] |= 1 << (i % 64);
        !self.stopped
    }

    /// Forgets rows `from..=to` of an order: a pattern loop is about to play
    /// them again, and that is not the song ending.
    pub fn forget(&mut self, order: usize, from: usize, to: usize) {
        for row in from..=to.min(Self::ROWS - 1) {
            let i = order * Self::ROWS + row;
            if let Some(word) = self.bits.get_mut(i / 64) {
                *word &= !(1 << (i % 64));
            }
        }
    }
}

enum Core {
    Pt(pt::Replayer),
    Ft2(Box<ft2::Replayer>),
}

/// Plays a song. Make one for a song, then call `process` for each block of
/// sound with the same song; nothing is allocated while it plays.
pub struct Player {
    core: Core,
    sample_rate: f64,
    /// Frames from the start of the next block to the next tick.
    until_tick: f64,
}

impl Player {
    /// A player at the start of the song, for output at `sample_rate` frames
    /// a second.
    pub fn new(song: &Song, sample_rate: f64) -> Player {
        let core = match song.rules {
            Rules::ProTracker => Core::Pt(pt::Replayer::new(song)),
            Rules::FastTracker2 => Core::Ft2(Box::new(ft2::Replayer::new(song))),
        };
        Player { core, sample_rate: sample_rate.max(1.0), until_tick: 0.0 }
    }

    /// Plays the next `frames` frames of the song: every tick that falls in
    /// them is worked out and its instructions sent to `voices`.
    pub fn process(&mut self, song: &Song, frames: u32, voices: &mut dyn Voices) {
        let end = frames as f64;
        while self.until_tick < end {
            let frame = self.until_tick as u32;
            self.until_tick += self.tick(song, frame, voices);
        }
        self.until_tick -= end;
    }

    /// Plays one tick, sending its instructions for `frame`, and returns how
    /// many frames (with a fraction) the tick lasts. For a host that keeps
    /// time itself; `process` is this in a loop.
    pub fn tick(&mut self, song: &Song, frame: u32, voices: &mut dyn Voices) -> f64 {
        let tempo = match &mut self.core {
            Core::Pt(r) => {
                r.tick(song, frame, voices);
                r.tempo()
            }
            Core::Ft2(r) => {
                r.tick(song, frame, self.sample_rate, voices);
                r.tempo()
            }
        };
        self.sample_rate * 2.5 / tempo.max(1) as f64
    }

    /// Jumps to a row of an order. Every channel is stopped and forgets what
    /// it was doing; the song's speed and tempo stay as they are.
    pub fn seek(&mut self, song: &Song, order: usize, row: usize, voices: &mut dyn Voices) {
        match &mut self.core {
            Core::Pt(r) => r.seek(song, order, row, voices),
            Core::Ft2(r) => r.seek(song, order, row, voices),
        }
        self.until_tick = 0.0;
    }

    /// Where the song is: the row most recently begun.
    pub fn position(&self) -> Position {
        match &self.core {
            Core::Pt(r) => r.position(),
            Core::Ft2(r) => r.position(),
        }
    }

    /// True once the song has played through to a row it had already
    /// played, or to a command that stops it. Unless `set_repeat(false)` was
    /// called, playing carries on regardless, as most songs are made to loop.
    pub fn ended(&self) -> bool {
        self.position().loops > 0
    }

    /// True if the song reached its end with repeat off and has stopped
    /// there; `seek` starts it again.
    pub fn stopped(&self) -> bool {
        match &self.core {
            Core::Pt(r) => r.visited.stopped,
            Core::Ft2(r) => r.visited.stopped,
        }
    }

    /// Stays in the place in the order that is playing now, going round its
    /// pattern again and again (`true`), or carries on through the song
    /// from wherever it is (`false`).
    pub fn hold_order(&mut self, on: bool) {
        let order = self.position().order as usize;
        let visited = match &mut self.core {
            Core::Pt(r) => &mut r.visited,
            Core::Ft2(r) => &mut r.visited,
        };
        if visited.hold.is_some() != on {
            // Which rows have been played says nothing about the song's
            // end while one pattern goes round: begin the record again.
            visited.clear();
        }
        visited.hold = on.then_some(order);
    }

    /// The song being played has been replaced by `song`, which has the
    /// same channels and rules and no fewer instruments or samples. The
    /// player carries on from where it is, reading the new song from its
    /// next row; if that place is not in the new song, from the top of the
    /// nearest pattern that is.
    pub fn retarget(&mut self, song: &Song) {
        match &mut self.core {
            Core::Pt(r) => r.retarget(song),
            Core::Ft2(r) => r.retarget(song),
        }
    }

    /// Chooses what happens at the song's end: carry on round (the default),
    /// or stop before the first row that would be played a second time. A
    /// stopped player sends nothing more, and leaves the voices as they are
    /// so that the last notes can ring; `seek` starts it again.
    pub fn set_repeat(&mut self, repeat: bool) {
        match &mut self.core {
            Core::Pt(r) => r.visited.repeat = repeat,
            Core::Ft2(r) => r.visited.repeat = repeat,
        }
    }
}

/// A channel's panning as the voices take it, from the song's 0 to 255.
#[inline]
pub(crate) fn pan_of(pan: u8) -> f32 {
    pan as f32 / 127.5 - 1.0
}
