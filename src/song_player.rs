//! Playing a song inside the engine: the replayer, wired to real voices.
//!
//! `install` puts a song's samples into sample slots, sets a part up for
//! each of the song's channels, and installs a `SongPlayer` as the engine's
//! sequencer. From then on the engine calls it once for every tick of the
//! song, on the tick's exact frame, and it plays that tick through the
//! engine's voices. A host steers it with `song` events (play, pause, seek
//! and so on), which arrive on their exact frames like any other event, and
//! reads where it has got to through `Shared`, which is only ever single
//! numbers stored on one side and loaded on the other.
//!
//! A channel of the song is a part of the engine, and the channel's one
//! voice has the handle `channel + 1`. A channel playing a chord, or an
//! arpeggio whose notes are struck, has several voices with that handle,
//! which an arpeggiator of the engine starts and lets go; the player steps
//! it, on the frames the song's own rows and ticks give, so that its steps
//! keep time with the song whatever the song's tempo does.

use crate::engine::{Engine, MAX_SAMPLES, NUM_PARTS};
use crate::event::*;
use crate::params::*;
use crate::replayer::{note_of_rate, Arpeggio, Player, Source, Start, Voices as Channels};
use crate::sample::{LOOP_FORWARD, LOOP_OFF, LOOP_PINGPONG};
use crate::sequencer::{Sequencer, Trigger, Voices};
use crate::replayer::pan_of;
use crate::song::{ArpOrder, ArpRate, ArpStyle, BendShape, GateChange, LoopKind, Rules, Song, Time, C4_RATE, EFFECT_KEYS};
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::Arc;

/// No song is loaded.
pub const STATE_NONE: u32 = 0;
pub const STATE_STOPPED: u32 = 1;
pub const STATE_PLAYING: u32 = 2;
pub const STATE_PAUSED: u32 = 3;

/// How many numbers `Shared::position` gives.
pub const POSITION_SIZE: usize = 10;
/// How many numbers `Shared::info` gives.
pub const INFO_SIZE: usize = 8;

/// A part's volume for one channel of a song at full song volume: half, so
/// that the Amiga's two channels a side, flat out, just reach the level of
/// one synth note at full volume.
pub const CHANNEL_GAIN: f32 = 0.5;

/// The part a song's first channel plays on. A song has parts 32 to 63, a
/// channel each; parts 0 to 31 are the host's own, and a song never touches
/// them.
pub const FIRST_PART: u16 = 32;

/// A part number that means "no part": the instrument is not heard.
pub const NO_PART: u16 = 0xFFFF;

/// How long a looping sample takes to fade away when a song that does not
/// repeat reaches its end, in seconds.
pub const END_FADE: f32 = 0.25;

/// The cutoff that stands in for the Amiga's "LED" filter, in Hz.
const LED_FILTER_HZ: f32 = 3300.0;

/// Why a song could not be put into the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The song has more channels than there are parts from `first_part` on.
    Parts,
    /// The song has more samples than there are slots from `first_slot` on.
    Slots,
}

/// What a host can know about the loaded song without touching the player:
/// facts fixed when it was loaded, and where it is playing, kept up to date
/// by the audio side a tick at a time.
pub struct Shared {
    info: [u32; INFO_SIZE],
    /// For each instrument, the name of its synth patch if it is one.
    patches: Vec<Option<String>>,
    /// For each instrument, what its `synth` key held, as JSON text.
    synths: Vec<Option<String>>,
    position: [AtomicU32; POSITION_SIZE],
}

impl Shared {
    /// Channels, orders, patterns, instruments, samples, the first part, the
    /// first sample slot, and the rules (0 FastTracker 2, 1 ProTracker).
    pub fn info(&self) -> [u32; INFO_SIZE] {
        self.info
    }

    /// State (`STATE_...`), order, pattern, row, tick, ticks in a row, beats
    /// a minute, times the song has come round, rows in this pattern, and
    /// orders in the song. A reading taken while a tick is being played may
    /// mix two ticks' numbers; it is for a display.
    pub fn position(&self) -> [u32; POSITION_SIZE] {
        core::array::from_fn(|i| self.position[i].load(Relaxed))
    }

    /// The name of an instrument's synth patch (instruments count from 1).
    pub fn patch_name(&self, instrument: u32) -> Option<&str> {
        self.patches.get((instrument as usize).checked_sub(1)?)?.as_deref()
    }

    /// What a host wrote under an instrument's `synth` key, as JSON text
    /// (instruments count from 1).
    pub fn synth(&self, instrument: u32) -> Option<&str> {
        self.synths.get((instrument as usize).checked_sub(1)?)?.as_deref()
    }
}

/// A channel's place in the mix as the player has it: its volume, how far
/// its pan has been moved from where the song first had it, and its sends.
#[derive(Clone, Copy, PartialEq)]
struct Fader {
    volume: f32,
    pan: f32,
    reverb: f32,
    delay: f32,
}

impl Default for Fader {
    fn default() -> Fader {
        Fader { volume: 1.0, pan: 0.0, reverb: 0.0, delay: 0.0 }
    }
}

/// What one channel of the song has sounding.
#[derive(Clone, Copy)]
struct Channel {
    /// The part its voice is on: its own, or a synth patch's.
    part: u16,
    /// The voice is a synth patch's note and not a sample.
    patch: bool,
    /// The voice is a sample that goes round a loop, and so never ends by itself.
    looped: bool,
    /// The channel is to be heard: not muted, and not left out by a solo.
    heard: bool,
    rate: f64,
    volume: f32,
    pan: f32,
    /// The sample slot the voice plays and where it started in it.
    slot: Option<(u16, f64)>,
    /// The note began on this tick.
    fresh: bool,
    /// An arpeggiator of the engine is playing the channel's chord.
    arpeggiator: bool,
    /// How that arpeggiator is stepped, if it steps at all.
    stepper: Option<Stepper>,
    /// What else this row's event says: the number of one of the song's extras, until it is done.
    extra: u16,
    /// How far a bend has moved the note from the pitch the song gives it, in semitones.
    bend: f32,
    /// A bend that is under way.
    bending: Option<Bending>,
    /// The instrument the note is of, from 0 (`u16::MAX` for none).
    instrument: u16,
    /// The gate an event put on the channel: the extra that has it, or 0
    /// to go by the instrument; and whether an event switched the gate off.
    gate_from: u16,
    gate_off: bool,
    /// The gate has to be set again, and the part it was last set on.
    gate_due: bool,
    gated: u16,
    /// The channel's fader and pan in the song's mix, for a note on a synth
    /// patch's part, which carries them itself.
    gain: f32,
    turn: f32,
}

/// A bend in the song's time: where the pitch goes next and how long each leg takes.
#[derive(Clone, Copy)]
struct Bending {
    /// Each leg: how far from the note it ends, the time it takes, and how much of that time.
    legs: [(f32, Time, f64); 2],
    count: u8,
    at: u8,
    /// When the next leg begins, in ticks from the start of the tick being played.
    next: f64,
    counting: bool,
}

/// The steps of a channel's arpeggio, which the player takes in the song's time.
#[derive(Clone, Copy)]
struct Stepper {
    rate: ArpRate,
    /// When the next step is, in ticks from the start of the tick being played.
    next: f64,
    /// It has been through the end of a tick (one that began on this tick has not).
    counting: bool,
}

/// A step this close to the start of a tick, in ticks, is on it: sums of
/// step lengths that are not whole ticks (a triplet's) come out a hair off.
const ON_THE_TICK: f64 = 1.0e-4;

impl Channel {
    /// How loud a synth patch's note on the channel is, of what the song
    /// asks for: the song's volume and the channel's fader, or nothing if
    /// the channel is not to be heard.
    fn level(&self, song_volume: f32) -> f32 {
        if self.heard {
            song_volume * self.gain
        } else {
            0.0
        }
    }

    /// Where a synth patch's note on the channel is panned, the channel's pan in the mix added.
    fn panned(&self, pan: f32) -> f32 {
        (pan + self.turn).clamp(-1.0, 1.0)
    }

    fn new(part: u16) -> Channel {
        Channel {
            part,
            patch: false,
            looped: false,
            heard: true,
            rate: 0.0,
            volume: 0.0,
            pan: 0.0,
            slot: None,
            fresh: false,
            arpeggiator: false,
            stepper: None,
            extra: 0,
            bend: 0.0,
            bending: None,
            instrument: u16::MAX,
            gate_from: 0,
            gate_off: false,
            gate_due: false,
            gated: NO_PART,
            gain: 1.0,
            turn: 0.0,
        }
    }
}

/// The engine's voices as the replayer sees them.
struct Adapter<'a, 'b> {
    voices: &'a mut Voices<'b>,
    song: &'a Song,
    first_part: u16,
    first_slot: u16,
    channels: &'a mut [Channel],
    patch_parts: &'a [u16],
    /// How loud the song is (the `volume` command), for the voices of
    /// synth patches: their parts are the host's, so it is their own
    /// volume that carries it.
    volume: f32,
    /// For each sample, the first instrument that plays it.
    owners: &'a [u16],
    /// Ticks in a row, beats a minute and frames in a tick, as the last tick left them.
    speed: u16,
    tempo: u16,
    tick_length: f64,
}

impl Adapter<'_, '_> {
    fn own_part(&self, channel: usize) -> u16 {
        self.first_part + channel as u16
    }

    /// How many seconds a time in the song lasts just now.
    fn seconds(&self, time: &Time) -> f32 {
        (time.ticks(self.speed, self.tempo) * self.tick_length / self.voices.sample_rate() as f64) as f32
    }

    /// Start a sample on a channel's own part; false if nothing is there to play.
    fn start_sample(&mut self, channel: usize, sample: usize, offset: u32, rate: f64, volume: f32, pan: f32) -> bool {
        let Some(s) = self.song.samples.get(sample) else { return false };
        // A start at or past the end: into the loop if there is one.
        let position = match (offset >= s.play_end(), s.loop_range()) {
            (false, _) => offset,
            (true, Some((start, _))) => start,
            (true, None) => return false,
        };
        // A voice cannot be started at no speed: start it, then hold it.
        let trigger = Trigger {
            slot: self.first_slot + sample as u16,
            position: position as f64,
            rate: if rate > 0.0 { rate as f32 } else { 1.0 },
            volume,
            pan,
        };
        let part = self.own_part(channel);
        let started = self.voices.trigger(part, channel as u32 + 1, &trigger);
        if started && rate <= 0.0 {
            self.voices.set_rate(part, channel as u32 + 1, 0.0, 0.0);
        }
        if let Some(ch) = self.channels.get_mut(channel) {
            ch.slot = started.then_some((trigger.slot, trigger.position));
        }
        started
    }
}

impl Channels for Adapter<'_, '_> {
    fn play(&mut self, _frame: u32, channel: usize, start: &Start) {
        let Some(&was) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        let own = self.own_part(channel);
        let instrument = match start.source {
            Source::Patch(i) => i as u16,
            Source::Sample(s) => self.owners.get(s).copied().unwrap_or(u16::MAX),
        };
        // A glide: the note that is sounding is not started again; it
        // slides to this one. (With nothing sounding, the note just starts.)
        if let Some(time) = self.song.extra(was.extra).and_then(|x| x.glide) {
            let seconds = self.seconds(&time);
            let patch_part = match start.source {
                Source::Patch(i) => self.patch_parts.get(i).copied().unwrap_or(NO_PART),
                Source::Sample(_) => NO_PART,
            };
            let same = if was.patch { patch_part == was.part } else { patch_part == NO_PART && matches!(start.source, Source::Sample(_)) };
            if same && self.voices.sounding(was.part, handle) {
                if was.patch {
                    self.voices.set_pitch(was.part, handle, note_of_rate(start.rate) as f32, seconds);
                    self.voices.set_volume(was.part, handle, start.volume * was.level(self.volume));
                    self.voices.set_pan(was.part, handle, was.panned(start.pan));
                } else {
                    self.voices.set_rate(was.part, handle, start.rate as f32, seconds);
                    self.voices.set_volume(was.part, handle, start.volume);
                    self.voices.set_pan(was.part, handle, start.pan);
                }
                self.channels[channel] = Channel { rate: start.rate, volume: start.volume, pan: start.pan, bend: 0.0, bending: None, instrument, gate_due: true, ..was };
                return;
            }
        }
        // A channel plays one thing: whatever it had on another part goes.
        if was.part != own {
            self.voices.kill(was.part, handle);
        }
        let mut now = Channel {
            heard: was.heard,
            rate: start.rate,
            volume: start.volume,
            pan: start.pan,
            fresh: true,
            extra: was.extra,
            instrument,
            gate_from: was.gate_from,
            gate_off: was.gate_off,
            gate_due: true,
            gated: was.gated,
            gain: was.gain,
            turn: was.turn,
            ..Channel::new(own)
        };
        match start.source {
            Source::Sample(sample) => {
                if self.start_sample(channel, sample, start.offset, start.rate, start.volume, start.pan) {
                    now.looped = self.song.samples.get(sample).is_some_and(|s| s.loop_range().is_some());
                    now.slot = self.channels[channel].slot;
                } else {
                    self.voices.kill(own, handle);
                }
            }
            Source::Patch(instrument) => {
                self.voices.kill(own, handle);
                let part = self.patch_parts.get(instrument).copied().unwrap_or(NO_PART);
                if (part as usize) < NUM_PARTS {
                    self.voices.kill(part, handle);
                    self.voices.note_on(part, handle, note_of_rate(start.rate) as f32, 1.0);
                    self.voices.set_volume(part, handle, start.volume * was.level(self.volume));
                    self.voices.set_pan(part, handle, was.panned(start.pan));
                    // The channel's own meter shows the note, though it is on another part.
                    self.voices.meter_on(part, handle, own);
                    now.part = part;
                    now.patch = true;
                }
            }
        }
        self.channels[channel] = now;
    }

    fn set_rate(&mut self, _frame: u32, channel: usize, rate: f64) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.rate = rate;
        // (A bend that has moved the note stays on top of what the song does to its pitch.)
        if ch.patch {
            self.voices.set_pitch(ch.part, channel as u32 + 1, note_of_rate(rate) as f32 + ch.bend, 0.0);
        } else {
            self.voices.set_rate(ch.part, channel as u32 + 1, (rate * (ch.bend as f64 / 12.0).exp2()) as f32, 0.0);
        }
    }

    fn set_volume(&mut self, _frame: u32, channel: usize, volume: f32, ramp: u32) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.volume = volume;
        let volume = if ch.patch { volume * ch.level(self.volume) } else { volume };
        self.voices.ramp_volume(ch.part, channel as u32 + 1, volume, ramp);
    }

    fn set_pan(&mut self, _frame: u32, channel: usize, pan: f32, ramp: u32) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.pan = pan;
        let pan = if ch.patch { ch.panned(pan) } else { pan };
        self.voices.ramp_pan(ch.part, channel as u32 + 1, pan, ramp);
    }

    fn stop(&mut self, _frame: u32, channel: usize) {
        if let Some(ch) = self.channels.get_mut(channel) {
            self.voices.kill(ch.part, channel as u32 + 1);
            (ch.arpeggiator, ch.stepper) = (false, None);
        }
    }

    fn release(&mut self, _frame: u32, channel: usize) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        let handle = channel as u32 + 1;
        if ch.patch {
            // A patch has a release of its own; an arpeggiator ends with its note.
            self.voices.note_off(ch.part, handle);
            (ch.arpeggiator, ch.stepper) = (false, None);
        } else if ch.arpeggiator && ch.stepper.is_some() {
            // A sample is faded by the replayer. An arpeggiator strikes no
            // more notes; the one sounding is left to finish.
            self.voices.arp(ch.part, handle, 0.0, 0, &[]);
            (ch.arpeggiator, ch.stepper) = (false, None);
        }
    }

    fn set_arpeggio(&mut self, _frame: u32, channel: usize, arpeggio: Option<&Arpeggio>) {
        let Some(&was) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        let order = |order: ArpOrder| match order {
            ArpOrder::Up => crate::arp::ORDER_UP,
            ArpOrder::Down => crate::arp::ORDER_DOWN,
            ArpOrder::UpDown => crate::arp::ORDER_UP_DOWN,
            ArpOrder::Written => crate::arp::ORDER_AS_GIVEN,
            ArpOrder::Random => crate::arp::ORDER_RANDOM,
            ArpOrder::UpDownEnds => crate::arp::ORDER_UP_DOWN_ENDS,
            ArpOrder::Together => crate::arp::ORDER_TOGETHER,
        };
        let mut now = Channel { arpeggiator: false, stepper: None, ..was };
        match arpeggio {
            None => self.voices.arp(was.part, handle, 0.0, 0, &[]),
            // One voice stepping by itself, as songs have always had it.
            Some(a) if a.arp.is_some_and(|arp| arp.plain()) => self.voices.arp(was.part, handle, a.rate, order(a.order), a.offsets()),
            // Anything else is an arpeggiator's, stepped from here: it
            // takes over the note that has just begun, or the one sounding.
            Some(a) => {
                let (style, gate, octaves, swing, accents) = match a.arp {
                    Some(arp) => (arp.style, arp.gate, arp.octaves, arp.swing, arp.accents),
                    None => (ArpStyle::Strike, 1.0, 1, 0.0, 0),
                };
                self.voices.arp_notes(was.part, handle, -1.0, order(a.order), a.offsets());
                self.voices.arp_style(was.part, handle, if style == ArpStyle::Strike { gate } else { 0.0 }, octaves, swing, accents);
                self.voices.arp_pattern(was.part, handle, a.arp.and_then(|arp| self.song.arp_pattern(&arp)));
                if let Some((slot, position)) = was.slot.filter(|_| !was.patch) {
                    self.voices.arp_sample(was.part, handle, Some(slot), position);
                }
                now.arpeggiator = true;
                now.stepper = match (a.arp, was.stepper) {
                    // Notes together do not step. A chord given to a note
                    // that is sounding is struck there and then.
                    (None, _) => {
                        if was.arpeggiator && !was.fresh {
                            self.voices.arp_step(was.part, handle, 0.0);
                        }
                        None
                    }
                    // An arpeggio that is running keeps its place and its next step.
                    (Some(arp), Some(stepper)) if was.arpeggiator && !was.fresh => Some(Stepper { rate: arp.rate, ..stepper }),
                    // A new one takes its first step on this frame, once
                    // the tick has been played and its tempo is known.
                    (Some(arp), _) => Some(Stepper { rate: arp.rate, next: 0.0, counting: false }),
                };
            }
        }
        self.channels[channel] = now;
    }

    fn set_extra(&mut self, _frame: u32, channel: usize, extra: u16) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.extra = extra;
        match self.song.extra(extra).and_then(|x| x.gate.as_ref()) {
            None => return,
            Some(GateChange::Off) => (ch.gate_from, ch.gate_off) = (0, true),
            Some(GateChange::Instrument) => (ch.gate_from, ch.gate_off) = (0, false),
            Some(GateChange::On(_)) => (ch.gate_from, ch.gate_off) = (extra, false),
        }
        ch.gate_due = true;
    }

    fn queue(&mut self, _frame: u32, channel: usize, sample: Option<usize>) {
        let Some(&ch) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        if ch.patch {
            return;
        }
        let loops = sample.and_then(|s| self.song.samples.get(s)).is_some_and(|s| s.loop_range().is_some());
        if self.voices.sounding(ch.part, handle) {
            self.voices.queue_sample(ch.part, handle, sample.map(|s| self.first_slot + s as u16));
            self.channels[channel].looped = loops;
        } else if let Some(sample) = sample {
            // The old sample has run out: the new one's loop starts at once.
            self.channels[channel].looped = self.start_sample(channel, sample, u32::MAX, ch.rate, ch.volume, ch.pan) && loops;
        }
    }

    fn set_filter(&mut self, _frame: u32, on: bool) {
        // The Amiga's low-pass "LED" filter, near enough: each channel's own.
        for channel in 0..self.channels.len() {
            let part = self.own_part(channel);
            self.voices.set_param(part, CUTOFF, LED_FILTER_HZ, 0.0);
            self.voices.set_param(part, RESONANCE, 0.0, 0.0);
            self.voices.set_param(part, FILTER_TYPE, if on { 1.0 } else { 0.0 }, 0.0);
        }
    }
}

/// Voices that do nothing, for moving the replayer when there is no engine
/// to hand (a reset has already stopped every voice).
struct Nowhere;

impl Channels for Nowhere {
    fn play(&mut self, _: u32, _: usize, _: &Start) {}
    fn set_rate(&mut self, _: u32, _: usize, _: f64) {}
    fn set_volume(&mut self, _: u32, _: usize, _: f32, _: u32) {}
    fn set_pan(&mut self, _: u32, _: usize, _: f32, _: u32) {}
    fn stop(&mut self, _: u32, _: usize) {}
}

/// A song and its player, as the engine's sequencer.
pub struct SongPlayer {
    song: Song,
    player: Player,
    shared: Arc<Shared>,
    first_part: u16,
    first_slot: u16,
    channels: Vec<Channel>,
    /// For each instrument, the part its synth patch plays on.
    patch_parts: Vec<u16>,
    state: u32,
    repeat: bool,
    /// How loud the song is, 0..2 (the `volume` command).
    volume: f32,
    /// The channels that are muted, and those that are soloed.
    muted: Vec<bool>,
    solo: Vec<bool>,
    /// The place in the order now playing is played round and round.
    loop_pattern: bool,
    /// The song is a tracker module, played with those trackers' raw sound.
    raw: bool,
    /// The part of a frame the last tick's length left over.
    carry: f64,
    /// The tick being played: how many frames it has, how many of them
    /// have gone by, and its length with its fraction of a frame.
    tick_frames: u32,
    into: u32,
    tick_length: f64,
    /// The tempo of the tick being played, to notice when it changes.
    tempo: u16,
    /// Ticks in a row as the last tick left them, and what the engine was last told the tempo is.
    speed: u16,
    told: f32,
    /// For each sample, the first instrument that plays it.
    owners: Vec<u16>,
    /// Each channel's place in the mix as it is now, and as the engine was last told.
    mix: Vec<Fader>,
    mixed: Vec<Fader>,
    /// Where the song panned each channel to begin with, -1 to 1: a pan
    /// set while it plays is so far from this.
    base_pan: Vec<f32>,
    /// The song's loop as it is now: its start and its end, each an order and a row.
    loop_points: (Option<(usize, usize)>, Option<(usize, usize)>),
}

impl SongPlayer {
    fn new(song: Song, sample_rate: f32, first_part: u16, first_slot: u16) -> SongPlayer {
        let player = Player::new(&song, sample_rate as f64);
        let info = [
            song.channels() as u32,
            song.orders.len() as u32,
            song.patterns.len() as u32,
            song.instruments.len() as u32,
            song.samples.len() as u32,
            first_part as u32,
            first_slot as u32,
            (song.rules == Rules::ProTracker) as u32,
        ];
        let shared = Arc::new(Shared {
            info,
            patches: song.instruments.iter().map(|i| i.patch.clone()).collect(),
            synths: song.instruments.iter().map(|i| i.synth.clone()).collect(),
            position: core::array::from_fn(|_| AtomicU32::new(0)),
        });
        let channels = (0..song.channels()).map(|c| Channel::new(first_part + c as u16)).collect();
        let s = SongPlayer {
            patch_parts: vec![NO_PART; song.instruments.len()],
            muted: vec![false; song.channels()],
            solo: vec![false; song.channels()],
            loop_pattern: false,
            raw: false,
            song,
            player,
            shared,
            first_part,
            first_slot,
            channels,
            state: STATE_STOPPED,
            repeat: true,
            volume: 1.0,
            carry: 0.0,
            tick_frames: 0,
            into: 0,
            tick_length: 0.0,
            tempo: 0,
            speed: 0,
            told: 0.0,
            owners: Vec::new(),
            mix: Vec::new(),
            mixed: Vec::new(),
            base_pan: Vec::new(),
            loop_points: (None, None),
        };
        let mut s = s;
        s.find_owners();
        s.volume = s.song.volume;
        s.mix = (0..s.song.channels())
            .map(|c| {
                let m = s.song.channel_mix(c);
                Fader { volume: m.volume, pan: 0.0, reverb: m.reverb, delay: m.echo }
            })
            .collect();
        s.base_pan = s.song.channel_pan.iter().map(|&p| pan_of(p)).collect();
        s.mixed = vec![Fader::default(); s.song.channels()];
        for (ch, m) in s.channels.iter_mut().zip(&s.mix) {
            (ch.gain, ch.turn) = (m.volume, m.pan);
        }
        let place = |p: (u16, u16)| (p.0 as usize, p.1 as usize);
        s.loop_points = (s.song.loop_start.map(place), s.song.loop_end.map(place));
        s.publish();
        s
    }

    /// Tell the reading side where the song is.
    fn publish(&self) {
        let p = self.player.position();
        let numbers = [
            self.state,
            p.order as u32,
            p.pattern as u32,
            p.row as u32,
            p.tick as u32,
            p.speed as u32,
            p.tempo as u32,
            p.loops,
            self.song.rows_at(p.order as usize) as u32,
            self.song.orders.len() as u32,
        ];
        for (slot, n) in self.shared.position.iter().zip(numbers) {
            slot.store(n, Relaxed);
        }
    }

    /// Each channel's voice, for something that touches them all.
    fn each_voice(&self, mut f: impl FnMut(u16, u32, &Channel)) {
        for (c, ch) in self.channels.iter().enumerate() {
            f(ch.part, c as u32 + 1, ch);
        }
    }

    fn seek(&mut self, order: usize, row: usize, voices: &mut Voices<'_>) {
        self.ungate(voices);
        let mut adapter = Adapter {
            voices,
            song: &self.song,
            first_part: self.first_part,
            first_slot: self.first_slot,
            channels: &mut self.channels,
            patch_parts: &self.patch_parts,
            volume: self.volume,
            owners: &self.owners,
            speed: self.speed,
            tempo: self.tempo,
            tick_length: self.tick_length,
        };
        self.player.seek(&self.song, order, row, &mut adapter);
        self.player.set_repeat(self.repeat);
        // A pattern being played round and round: now the one sought.
        self.player.hold_order(self.loop_pattern);
        self.rewind();
    }

    /// The next call plays a tick, on its own frame, with nothing left of the last.
    fn rewind(&mut self) {
        self.carry = 0.0;
        self.tick_frames = 0;
        self.into = 0;
        for ch in &mut self.channels {
            (ch.arpeggiator, ch.stepper, ch.fresh) = (false, None, false);
            (ch.extra, ch.bend, ch.bending) = (0, 0.0, None);
            (ch.gate_from, ch.gate_off, ch.gate_due) = (0, false, false);
        }
    }

    /// Work out which instrument each sample belongs to.
    fn find_owners(&mut self) {
        self.owners = (0..self.song.samples.len()).map(|s| self.song.instruments.iter().position(|i| i.keymap.contains(&(s as u16))).map_or(u16::MAX, |i| i as u16)).collect();
    }

    /// Take away every gate the song has put on a part.
    fn ungate(&mut self, voices: &mut Voices<'_>) {
        for ch in &mut self.channels {
            if (ch.gated as usize) < NUM_PARTS {
                voices.set_param(ch.gated, GATE_DEPTH, 0.0, 0.0);
            }
            ch.gated = NO_PART;
        }
    }

    /// Move a channel's note so far from the pitch the song gives it, over some seconds.
    fn bend_to(voices: &mut Voices<'_>, c: usize, ch: &mut Channel, semitones: f32, seconds: f32) {
        ch.bend = semitones;
        if ch.patch {
            voices.set_pitch(ch.part, c as u32 + 1, note_of_rate(ch.rate) as f32 + semitones, seconds);
        } else if ch.rate > 0.0 {
            voices.set_rate(ch.part, c as u32 + 1, (ch.rate * (semitones as f64 / 12.0).exp2()) as f32, seconds);
        }
    }

    /// What is left of a row once its tick has been played: the vibrato,
    /// the bend and the gate its events asked for, and the beat.
    fn after_tick(&mut self, voices: &mut Voices<'_>) {
        let p = self.player.position();
        let per_tick = self.tick_length / voices.sample_rate() as f64;
        let seconds = |time: &Time| (time.ticks(p.speed, p.tempo) * per_tick) as f32;
        for c in 0..self.channels.len() {
            let mut ch = self.channels[c];
            if let Some(x) = self.song.extra(core::mem::take(&mut ch.extra)) {
                if let Some(v) = x.vibrato {
                    voices.set_vibrato(ch.part, c as u32 + 1, v.depth, v.rate, seconds(&v.delay));
                }
                if let Some(bend) = x.bend {
                    let legs = match bend.shape {
                        BendShape::To => [(bend.to, bend.over, 1.0), (0.0, bend.over, 0.0)],
                        BendShape::Slide => [(ch.bend + bend.to, bend.over, 1.0), (0.0, bend.over, 0.0)],
                        BendShape::Scoop => [(bend.to, bend.over, 0.0), (0.0, bend.over, 1.0)],
                        BendShape::Dip => [(bend.to, bend.over, 0.5), (0.0, bend.over, 0.5)],
                    };
                    let count = if matches!(bend.shape, BendShape::To | BendShape::Slide) { 1 } else { 2 };
                    ch.bending = Some(Bending { legs, count, at: 0, next: 0.0, counting: false });
                }
            }
            if core::mem::take(&mut ch.gate_due) {
                let gate = match (ch.gate_off, self.song.extra(ch.gate_from).and_then(|x| x.gate.as_ref())) {
                    (true, _) => None,
                    (false, Some(GateChange::On(gate))) => Some(gate),
                    _ => self.song.instruments.get(ch.instrument as usize).and_then(|i| i.gate.as_ref()),
                };
                if ch.gated != ch.part && (ch.gated as usize) < NUM_PARTS {
                    voices.set_param(ch.gated, GATE_DEPTH, 0.0, 0.0);
                    ch.gated = NO_PART;
                }
                match gate {
                    Some(g) if (ch.part as usize) < NUM_PARTS => {
                        voices.set_gate(ch.part, &g.levels);
                        // (Steps to a beat: a beat is four rows.)
                        let values = [(GATE_DEPTH, g.depth), (GATE_RATE, (4.0 / g.rate.rows()) as f32), (GATE_SMOOTH, g.smooth), (GATE_SWING, g.swing), (GATE_SENDS, g.sends as u8 as f32), (GATE_STEREO, g.stereo as u8 as f32)];
                        for (id, value) in values {
                            voices.set_param(ch.part, id, value, 0.0);
                        }
                        ch.gated = ch.part;
                    }
                    _ if ch.gated == ch.part => {
                        voices.set_param(ch.part, GATE_DEPTH, 0.0, 0.0);
                        ch.gated = NO_PART;
                    }
                    _ => {}
                }
            }
            self.channels[c] = ch;
        }
        // The engine's beat is the song's: four rows to the beat, and the
        // top of the pattern the start of a bar.
        let beats = 6.0 * p.tempo as f32 / p.speed.max(1) as f32;
        if beats != self.told {
            self.told = beats;
            // A delay timed in the song's measure keeps to it.
            if let Some(seconds) = self.delay_seconds(p.speed, p.tempo) {
                voices.set_param(0, DELAY_TIME, seconds, 0.0);
            }
            voices.set_tempo(beats);
        }
        if p.tick == 0 {
            voices.clock(p.row as f32 / 4.0);
        }
        self.speed = p.speed;
    }

    /// Begin every leg of the channels' bends that is due by now.
    fn take_bends(&mut self, voices: &mut Voices<'_>) {
        let p = self.player.position();
        for c in 0..self.channels.len() {
            let mut ch = self.channels[c];
            let Some(mut b) = ch.bending else { continue };
            while b.at < b.count && self.step_frame(b.next) <= self.into {
                let (to, time, part) = b.legs[b.at as usize];
                let ticks = time.ticks(p.speed, p.tempo) * part;
                SongPlayer::bend_to(voices, c, &mut ch, to, (ticks * self.tick_length / voices.sample_rate() as f64) as f32);
                b.next += ticks;
                b.at += 1;
            }
            ch.bending = (b.at < b.count).then_some(b);
            self.channels[c] = ch;
        }
    }

    /// The frame of the tick being played that a step `next` ticks into it falls on.
    fn step_frame(&self, next: f64) -> u32 {
        if next <= ON_THE_TICK {
            0
        } else if next >= 1.0 - ON_THE_TICK {
            // The next tick's, or a later one's.
            u32::MAX
        } else {
            (next * self.tick_length) as u32
        }
    }

    /// Take every step of the channels' arpeggios that is due by now.
    fn take_steps(&mut self, voices: &mut Voices<'_>) {
        let p = self.player.position();
        for c in 0..self.channels.len() {
            let ch = self.channels[c];
            let Some(mut stepper) = ch.stepper else { continue };
            // (No more than a tick's worth at once, whatever the rate.)
            for _ in 0..64 {
                if self.step_frame(stepper.next) > self.into {
                    break;
                }
                let ticks = stepper.rate.ticks(p.speed, p.tempo);
                voices.arp_step(ch.part, c as u32 + 1, (ticks * self.tick_length) as f32);
                stepper.next += ticks;
            }
            self.channels[c].stepper = Some(stepper);
        }
    }

    /// Bring every channel to the loudness the song's volume, the mutes
    /// and the solos give it. A channel's own part has its `volume` set; a
    /// synth patch's notes, on a part of the host's, carry it themselves.
    fn set_levels(&mut self, voices: &mut Voices<'_>) {
        let any_solo = self.solo.iter().any(|&s| s);
        let frames = (0.02 * voices.sample_rate()) as u32;
        for (c, ch) in self.channels.iter_mut().enumerate() {
            ch.heard = !self.muted[c] && (!any_solo || self.solo[c]);
            let (m, was) = (self.mix[c], self.mixed[c]);
            (ch.gain, ch.turn) = (m.volume, m.pan);
            let level = if ch.heard { self.volume } else { 0.0 };
            let own = self.first_part + c as u16;
            voices.set_param(own, VOLUME, level * CHANNEL_GAIN * m.volume, 0.02);
            // The rest of the channel's place in the mix, where it has moved.
            for (id, now, then) in [(PAN, m.pan, was.pan), (SEND_REVERB, m.reverb, was.reverb), (SEND_DELAY, m.delay, was.delay)] {
                if now != then {
                    voices.set_param(own, id, now, 0.02);
                }
            }
            // (While paused, the notes are held at nothing.)
            if ch.patch && self.state != STATE_PAUSED {
                voices.ramp_volume(ch.part, c as u32 + 1, ch.volume * ch.level(self.volume), frames);
                if m.pan != was.pan {
                    voices.ramp_pan(ch.part, c as u32 + 1, ch.panned(ch.pan), frames);
                }
            }
            self.mixed[c] = m;
        }
    }

    /// The seconds between the echoes of the song's delay, when the song
    /// gives the time in its own measure: `None` if it sets no delay, or
    /// gives it in seconds.
    fn delay_seconds(&self, speed: u16, tempo: u16) -> Option<f32> {
        match self.song.effects.echo_time? {
            Time::Seconds(_) => None,
            time => Some((time.ticks(speed, tempo) * 2.5 / tempo.max(1) as f64).clamp(0.001, 2.0) as f32),
        }
    }

    /// Put the song's own mix and effects into a newly loaded engine: at
    /// once, with nothing sounding to hear it move. A song that sets none
    /// of it changes nothing here.
    fn setup(&mut self, engine: &mut Engine) {
        for (c, m) in self.mix.iter().enumerate() {
            let own = self.first_part + c as u16;
            for (id, now, usual) in [(VOLUME, CHANNEL_GAIN * self.volume * m.volume, CHANNEL_GAIN), (PAN, m.pan, 0.0), (SEND_REVERB, m.reverb, 0.0), (SEND_DELAY, m.delay, 0.0)] {
                if now != usual {
                    engine.set_param(own, id, now);
                }
            }
            self.mixed[c] = *m;
        }
        self.set_effects(engine, true, true);
    }

    /// Give the engine the settings of the reverb and the echo that the
    /// song gives (`all`), or only the ones named in `which` of `EFFECT_KEYS`.
    fn set_effects(&self, engine: &mut Engine, all: bool, time: bool) {
        let fx = &self.song.effects;
        if all {
            for ((_, id), value) in EFFECT_KEYS.iter().zip(fx.values) {
                if let Some(value) = value {
                    engine.set_param(0, *id, value);
                }
            }
        }
        if time {
            let seconds = match fx.echo_time {
                Some(Time::Seconds(n)) => Some(n),
                Some(_) => self.delay_seconds(self.song.speed as u16, self.song.tempo),
                None => None,
            };
            if let Some(seconds) = seconds {
                engine.set_param(0, DELAY_TIME, seconds);
            }
        }
    }
}

impl Sequencer for SongPlayer {
    fn advance(&mut self, voices: &mut Voices<'_>) -> u32 {
        if self.state != STATE_PLAYING {
            // Nothing to do until an event says otherwise; look in now and then.
            return (voices.sample_rate() / 50.0) as u32;
        }
        if self.into >= self.tick_frames {
            self.tick_frames = self.play_tick(voices);
            self.into = 0;
            if self.state != STATE_PLAYING {
                self.told = 0.0;
                voices.set_tempo(0.0);
                return self.tick_frames;
            }
            // A tick has gone by for every arpeggio that was running. One
            // timed in seconds has as long to go as it had, though the
            // ticks may have changed length under it.
            let tempo = self.player.position().tempo;
            let stretch = if self.tempo != 0 { tempo as f64 / self.tempo as f64 } else { 1.0 };
            self.tempo = tempo;
            self.after_tick(voices);
            for ch in &mut self.channels {
                ch.fresh = false;
                if let Some(b) = &mut ch.bending {
                    if b.counting {
                        b.next -= 1.0;
                    }
                    b.counting = true;
                }
                let Some(stepper) = &mut ch.stepper else { continue };
                if stepper.counting {
                    stepper.next -= 1.0;
                    if matches!(stepper.rate, ArpRate::PerSecond(_)) {
                        stepper.next *= stretch;
                    }
                }
                stepper.counting = true;
            }
        }
        self.take_steps(voices);
        self.take_bends(voices);
        // Back at the next step that falls inside this tick, or at the next tick.
        let mut wait = self.tick_frames - self.into;
        for ch in &self.channels {
            if let Some(stepper) = &ch.stepper {
                wait = wait.min(self.step_frame(stepper.next).saturating_sub(self.into));
            }
            if let Some(b) = &ch.bending {
                wait = wait.min(self.step_frame(b.next).saturating_sub(self.into));
            }
        }
        let wait = wait.max(1);
        self.into += wait;
        wait
    }

    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        Some(self)
    }

    fn reset(&mut self) {
        self.player.seek(&self.song, 0, 0, &mut Nowhere);
        self.player.set_repeat(self.repeat);
        self.player.hold_order(self.loop_pattern);
        self.rewind();
        self.told = 0.0;
        self.state = STATE_STOPPED;
        self.publish();
    }

    fn event(&mut self, event: &Event, voices: &mut Voices<'_>) -> bool {
        let whole = |x: f32| if x.is_finite() && x > 0.0 { x as usize } else { 0 };
        let mut now = false;
        match event.a {
            SONG_PLAY => {
                if self.state == STATE_PAUSED {
                    // Let the held voices go on from where they were.
                    let song_volume = self.volume;
                    self.each_voice(|part, handle, ch| {
                        let level = if ch.heard { song_volume } else { 0.0 };
                        voices.set_volume(part, handle, if ch.patch { ch.volume * level } else { ch.volume });
                        if !ch.patch {
                            voices.set_rate(part, handle, ch.rate as f32, 0.0);
                        }
                    });
                } else if self.state == STATE_STOPPED && self.player.stopped() {
                    self.seek(0, 0, voices);
                }
                now = self.state != STATE_PLAYING;
                self.state = STATE_PLAYING;
            }
            SONG_STOP => {
                self.seek(0, 0, voices);
                self.state = STATE_STOPPED;
            }
            SONG_PAUSE => {
                if self.state == STATE_PLAYING {
                    // Hold every sample where it is, in silence.
                    self.each_voice(|part, handle, ch| {
                        voices.set_volume(part, handle, 0.0);
                        if !ch.patch {
                            voices.set_rate(part, handle, 0.0, 0.0);
                        }
                    });
                    self.state = STATE_PAUSED;
                }
            }
            SONG_SEEK => {
                self.seek(whole(event.x), whole(event.y), voices);
                if self.state == STATE_PAUSED {
                    self.state = STATE_STOPPED;
                }
                now = self.state == STATE_PLAYING;
            }
            SONG_REPEAT => {
                self.repeat = event.x != 0.0;
                self.player.set_repeat(self.repeat);
            }
            SONG_VOLUME => {
                let volume = if event.x.is_finite() { event.x.clamp(0.0, 2.0) } else { 1.0 };
                self.volume = volume;
                self.set_levels(voices);
            }
            SONG_MUTE | SONG_SOLO => {
                let list = if event.a == SONG_MUTE { &mut self.muted } else { &mut self.solo };
                let on = event.y != 0.0;
                if event.x < 0.0 {
                    list.fill(on);
                } else if let Some(slot) = list.get_mut(if event.x.is_finite() { event.x as usize } else { usize::MAX }) {
                    *slot = on;
                }
                self.set_levels(voices);
            }
            SONG_LOOP_PATTERN => {
                self.loop_pattern = event.x != 0.0;
                self.player.hold_order(self.loop_pattern);
            }
            SONG_CHANNEL_VOLUME | SONG_CHANNEL_PAN | SONG_CHANNEL_REVERB | SONG_CHANNEL_DELAY => {
                let channel = if event.x.is_finite() && event.x >= 0.0 { event.x as usize } else { usize::MAX };
                if let (Some(m), true) = (self.mix.get_mut(channel), event.y.is_finite()) {
                    match event.a {
                        SONG_CHANNEL_VOLUME => m.volume = event.y.clamp(0.0, 2.0),
                        SONG_CHANNEL_PAN => m.pan = event.y.clamp(-1.0, 1.0) - self.base_pan.get(channel).copied().unwrap_or(0.0),
                        SONG_CHANNEL_REVERB => m.reverb = event.y.clamp(0.0, 1.0),
                        _ => m.delay = event.y.clamp(0.0, 1.0),
                    }
                    self.set_levels(voices);
                }
            }
            SONG_LOOP_START | SONG_LOOP_END => {
                let place = (event.x.is_finite() && event.x >= 0.0).then(|| (event.x as usize, whole(event.y)));
                if event.a == SONG_LOOP_START {
                    self.loop_points.0 = place;
                } else {
                    self.loop_points.1 = place;
                }
                self.player.set_loop(self.loop_points.0, self.loop_points.1);
            }
            SONG_PATCH_PART => {
                let part = if event.y.is_finite() && event.y >= 0.0 && (event.y as usize) < NUM_PARTS { event.y as u16 } else { NO_PART };
                if let Some(slot) = whole(event.x).checked_sub(1).and_then(|i| self.patch_parts.get_mut(i)) {
                    *slot = part;
                }
            }
            _ => {}
        }
        if self.state != STATE_PLAYING && self.told != 0.0 {
            // The beat is the host's again.
            self.told = 0.0;
            voices.set_tempo(0.0);
        }
        self.publish();
        now
    }
}

impl SongPlayer {
    /// Play one tick of the song, on this frame. Returns how many whole
    /// frames it lasts.
    fn play_tick(&mut self, voices: &mut Voices<'_>) -> u32 {
        let mut adapter = Adapter {
            voices,
            song: &self.song,
            first_part: self.first_part,
            first_slot: self.first_slot,
            channels: &mut self.channels,
            patch_parts: &self.patch_parts,
            volume: self.volume,
            owners: &self.owners,
            speed: self.speed,
            tempo: self.tempo,
            tick_length: self.tick_length,
        };
        let length = self.player.tick(&self.song, 0, &mut adapter);
        // Step macros that keep time with the song go by once a tick.
        for c in 0..self.channels.len() {
            let ch = self.channels[c];
            voices.macro_tick(ch.part);
        }
        if self.player.stopped() {
            // The last notes are left to ring, but not for ever: a synth
            // patch's note is let go, to die away as its patch says, and a
            // sample that loops is faded out. A sample with an end plays to
            // it. An arpeggio takes no more steps.
            self.state = STATE_STOPPED;
            for (c, ch) in self.channels.iter_mut().enumerate() {
                (ch.arpeggiator, ch.stepper) = (false, None);
                if ch.patch {
                    voices.note_off(ch.part, c as u32 + 1);
                } else if ch.looped {
                    voices.fade(ch.part, c as u32 + 1, END_FADE);
                    ch.looped = false;
                }
            }
        }
        self.publish();
        self.tick_length = length;
        self.carry += length;
        let whole = self.carry.floor();
        self.carry -= whole;
        whole as u32
    }
}

/// Puts a song into an engine, ready to play (it starts stopped; a `song`
/// event with `SONG_PLAY` starts it). Its channels take the parts from
/// `first_part` and its samples the slots from `first_slot`; whatever was
/// in those slots is replaced, and a song already loaded is taken out
/// first. `raw` sets the channels' parts to play samples as the old
/// trackers did, without smoothing fast playback: right for MOD and XM.
///
/// This allocates, so it is for the thread that owns the engine between
/// renders, not for the audio path.
pub fn install(engine: &mut Engine, song: Song, first_part: u16, first_slot: u16, raw: bool) -> Result<Arc<Shared>, InstallError> {
    if first_part as usize + song.channels() > NUM_PARTS {
        return Err(InstallError::Parts);
    }
    if first_slot as usize + song.samples.len() > MAX_SAMPLES {
        return Err(InstallError::Slots);
    }
    remove(engine);

    for (i, s) in song.samples.iter().enumerate() {
        load_slot(engine, first_slot as usize + i, s);
    }

    // A channel's part plays its sample as it is: full on at once, and gone
    // at once when cut. The replayer does all the shaping.
    for c in 0..song.channels() {
        let part = first_part + c as u16;
        for (id, value) in [
            (VOLUME, CHANNEL_GAIN),
            (PAN, 0.0),
            (AMP_ATTACK, 0.0),
            (AMP_DECAY, 0.0),
            (AMP_SUSTAIN, 1.0),
            (AMP_RELEASE, 0.0),
            (VELOCITY_SENS, 0.0),
            // FastTracker 2's songs were balanced by its own pan law.
            (PAN_LAW, if song.rules == Rules::FastTracker2 { 1.0 } else { 0.0 }),
            (FILTER_TYPE, 0.0),
            (SAMPLE_ANTIALIAS, if raw { 0.0 } else { 1.0 }),
        ] {
            engine.set_param(part, id, value);
        }
    }

    let mut player = SongPlayer::new(song, engine.sample_rate(), first_part, first_slot);
    player.raw = raw;
    player.setup(engine);
    let shared = player.shared.clone();
    engine.song = Some(shared.clone());
    engine.set_sequencer(Some(Box::new(player)));
    Ok(shared)
}

/// Puts one of a song's samples into a sample slot.
fn load_slot(engine: &mut Engine, slot: usize, s: &crate::song::Sample) {
    let data = s.data.to_f32();
    // An empty sample leaves an empty slot, which plays nothing.
    if engine.load_sample(slot, &data, 1, C4_RATE as f32).is_err() {
        let _ = engine.free_sample(slot);
        return;
    }
    let (mode, start, end) = match (s.loop_kind, s.loop_range()) {
        (LoopKind::Forward, Some((a, b))) => (LOOP_FORWARD, a, b),
        (LoopKind::PingPong, Some((a, b))) => (LOOP_PINGPONG, a, b),
        _ => (LOOP_OFF, 0, 0),
    };
    let _ = engine.configure_sample(slot, 60.0, mode as u32, start, end);
}

/// Puts `song` in the place of the song that is loaded, for an editor
/// whose song changes while it plays. Returns true if the change was
/// seamless.
///
/// If the new song has the same channels and rules as the old one, and no
/// fewer instruments or samples, nothing stops: the notes that are
/// sounding carry on, the player keeps its place, tick and tempo, and from
/// its next row it reads the new song. Only the samples that differ are
/// loaded again. (If its place is not in the new song, it carries on from
/// the top of the nearest pattern that is.)
///
/// Otherwise the new song is installed afresh, and what the host had set
/// is put back: its volume, mutes, solos, repeat, pattern loop and the
/// parts of its synth patches, and its place, from the top of the row it
/// was on, playing if it was playing. The sounding notes are cut.
///
/// With no song loaded this is `install`. It allocates, so it is for
/// between renders.
pub fn replace(engine: &mut Engine, song: Song, first_slot: u16, raw: bool) -> Result<bool, InstallError> {
    if song.channels() > NUM_PARTS - FIRST_PART as usize {
        return Err(InstallError::Parts);
    }
    if first_slot as usize + song.samples.len() > MAX_SAMPLES {
        return Err(InstallError::Slots);
    }
    // What the host had set, in case the song has to be installed afresh.
    struct Kept {
        position: [u32; POSITION_SIZE],
        repeat: bool,
        volume: f32,
        muted: Vec<bool>,
        solo: Vec<bool>,
        loop_pattern: bool,
        patch_parts: Vec<u16>,
    }
    let mut song = Some(song);
    // What of the mix the new song changes, to be moved to (smoothly) once it is in place.
    let mut moves: Vec<(u32, f32, f32)> = Vec::new();
    let outcome = engine.with_sequencer(|engine, sequencer| {
        let player = sequencer.as_any()?.downcast_mut::<SongPlayer>()?;
        let new = song.as_ref()?;
        let fits = new.channels() == player.song.channels()
            && new.rules == player.song.rules
            && new.instruments.len() >= player.song.instruments.len()
            && new.samples.len() >= player.song.samples.len()
            && first_slot == player.first_slot
            && raw == player.raw;
        if !fits {
            return Some(Err(Kept {
                position: player.shared.position(),
                repeat: player.repeat,
                volume: player.volume,
                muted: player.muted.clone(),
                solo: player.solo.clone(),
                loop_pattern: player.loop_pattern,
                patch_parts: player.patch_parts.clone(),
            }));
        }
        let new = song.take()?;
        // A fader the edit moved goes where the new song has it; one it did
        // not is left where the host last put it.
        let old = &player.song;
        if old.volume != new.volume {
            moves.push((SONG_VOLUME, new.volume, 0.0));
        }
        for c in 0..new.channels() {
            let (was, now) = (old.channel_mix(c), new.channel_mix(c));
            let pans = (old.channel_pan.get(c).map(|&p| pan_of(p)).unwrap_or(0.0), new.channel_pan.get(c).map(|&p| pan_of(p)).unwrap_or(0.0));
            for (command, a, b) in [(SONG_CHANNEL_VOLUME, was.volume, now.volume), (SONG_CHANNEL_PAN, pans.0, pans.1), (SONG_CHANNEL_REVERB, was.reverb, now.reverb), (SONG_CHANNEL_DELAY, was.echo, now.echo)] {
                if a != b {
                    moves.push((command, c as f32, b));
                }
            }
        }
        for (command, a, b) in [(SONG_LOOP_START, old.loop_start, new.loop_start), (SONG_LOOP_END, old.loop_end, new.loop_end)] {
            if a != b {
                moves.push((command, b.map_or(-1.0, |p| p.0 as f32), b.map_or(0.0, |p| p.1 as f32)));
            }
        }
        let effects = (old.effects.values != new.effects.values, old.effects.echo_time != new.effects.echo_time);
        for (i, s) in new.samples.iter().enumerate() {
            if player.song.samples.get(i) != Some(s) {
                load_slot(engine, first_slot as usize + i, s);
            }
        }
        let fresh = SongPlayer::new(new, engine.sample_rate(), player.first_part, first_slot);
        player.song = fresh.song;
        player.shared = fresh.shared;
        player.owners = fresh.owners;
        // (What a channel's row or gate named was in the old song's lists.)
        for ch in &mut player.channels {
            (ch.extra, ch.gate_from, ch.gate_due) = (0, 0, true);
        }
        player.patch_parts.resize(player.song.instruments.len(), NO_PART);
        player.player.retarget(&player.song);
        // The effects the edit changed, and a delay in the song's measure worked out again.
        player.set_effects(engine, effects.0, effects.1);
        player.told = 0.0;
        player.publish();
        Some(Ok(player.shared.clone()))
    });
    match outcome.flatten() {
        Some(Ok(shared)) => {
            engine.song = Some(shared);
            for (command, x, y) in moves {
                engine.apply(&Event::song(0.0, command, x, y));
            }
            Ok(true)
        }
        kept => {
            let Some(song) = song.take() else { return Ok(false) };
            install(engine, song, FIRST_PART, first_slot, raw)?;
            let Some(Err(kept)) = kept else { return Ok(false) };
            let mut send = |command: u32, x: f32, y: f32| engine.apply(&Event::song(0.0, command, x, y));
            for (i, &part) in kept.patch_parts.iter().enumerate() {
                send(SONG_PATCH_PART, i as f32 + 1.0, if part == NO_PART { -1.0 } else { part as f32 });
            }
            send(SONG_REPEAT, kept.repeat as u32 as f32, 0.0);
            send(SONG_VOLUME, kept.volume, 0.0);
            for (c, (&muted, &solo)) in kept.muted.iter().zip(&kept.solo).enumerate() {
                send(SONG_MUTE, c as f32, muted as u32 as f32);
                send(SONG_SOLO, c as f32, solo as u32 as f32);
            }
            let [state, order, _, row, ..] = kept.position;
            send(SONG_SEEK, order as f32, row as f32);
            send(SONG_LOOP_PATTERN, kept.loop_pattern as u32 as f32, 0.0);
            if state == STATE_PLAYING {
                send(SONG_PLAY, 0.0, 0.0);
            }
            Ok(false)
        }
    }
}

/// Takes the loaded song out of the engine: its voices are cut and its
/// sample slots emptied. Does nothing if no song is loaded.
pub fn remove(engine: &mut Engine) {
    let Some(shared) = engine.song.take() else { return };
    let [channels, _, _, _, samples, first_part, first_slot, _] = shared.info();
    // Stopping cuts every voice the song has, on its own parts and on the
    // parts of synth patches, without a click.
    engine.apply(&Event::song(0.0, SONG_STOP, 0.0, 0.0));
    engine.set_sequencer(None);
    engine.song_tempo = 0.0;
    for c in 0..channels {
        engine.apply(&Event::all_notes_off(0.0, (first_part + c) as u16, true));
    }
    for i in 0..samples {
        let _ = engine.free_sample((first_slot + i) as usize);
    }
}

/// What a host can read of the engine's loaded song.
pub fn shared(engine: &Engine) -> Option<&Shared> {
    engine.song.as_deref()
}
