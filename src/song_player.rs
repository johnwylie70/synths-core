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
//! voice has the handle `channel + 1`.

use crate::engine::{Engine, MAX_SAMPLES, NUM_PARTS};
use crate::event::*;
use crate::params::*;
use crate::replayer::{note_of_rate, Arpeggio, Player, Source, Start, Voices as Channels};
use crate::sample::{LOOP_FORWARD, LOOP_OFF, LOOP_PINGPONG};
use crate::sequencer::{Sequencer, Trigger, Voices};
use crate::song::{ChordOrder, LoopKind, Rules, Song, C4_RATE};
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
}

/// What one channel of the song has sounding.
#[derive(Clone, Copy)]
struct Channel {
    /// The part its voice is on: its own, or a synth patch's.
    part: u16,
    /// The voice is a synth patch's note and not a sample.
    patch: bool,
    rate: f64,
    volume: f32,
    pan: f32,
}

/// The engine's voices as the replayer sees them.
struct Adapter<'a, 'b> {
    voices: &'a mut Voices<'b>,
    song: &'a Song,
    first_part: u16,
    first_slot: u16,
    channels: &'a mut [Channel],
    patch_parts: &'a [u16],
}

impl Adapter<'_, '_> {
    fn own_part(&self, channel: usize) -> u16 {
        self.first_part + channel as u16
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
        started
    }
}

impl Channels for Adapter<'_, '_> {
    fn play(&mut self, _frame: u32, channel: usize, start: &Start) {
        let Some(&was) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        let own = self.own_part(channel);
        // A channel plays one thing: whatever it had on another part goes.
        if was.part != own {
            self.voices.kill(was.part, handle);
        }
        let mut now = Channel { part: own, patch: false, rate: start.rate, volume: start.volume, pan: start.pan };
        match start.source {
            Source::Sample(sample) => {
                if !self.start_sample(channel, sample, start.offset, start.rate, start.volume, start.pan) {
                    self.voices.kill(own, handle);
                }
            }
            Source::Patch(instrument) => {
                self.voices.kill(own, handle);
                let part = self.patch_parts.get(instrument).copied().unwrap_or(NO_PART);
                if (part as usize) < NUM_PARTS {
                    self.voices.kill(part, handle);
                    self.voices.note_on(part, handle, note_of_rate(start.rate) as f32, 1.0);
                    self.voices.set_volume(part, handle, start.volume);
                    self.voices.set_pan(part, handle, start.pan);
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
        if ch.patch {
            self.voices.set_pitch(ch.part, channel as u32 + 1, note_of_rate(rate) as f32, 0.0);
        } else {
            self.voices.set_rate(ch.part, channel as u32 + 1, rate as f32, 0.0);
        }
    }

    fn set_volume(&mut self, _frame: u32, channel: usize, volume: f32, ramp: u32) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.volume = volume;
        self.voices.ramp_volume(ch.part, channel as u32 + 1, volume, ramp);
    }

    fn set_pan(&mut self, _frame: u32, channel: usize, pan: f32, ramp: u32) {
        let Some(ch) = self.channels.get_mut(channel) else { return };
        ch.pan = pan;
        self.voices.ramp_pan(ch.part, channel as u32 + 1, pan, ramp);
    }

    fn stop(&mut self, _frame: u32, channel: usize) {
        if let Some(ch) = self.channels.get(channel) {
            self.voices.kill(ch.part, channel as u32 + 1);
        }
    }

    fn release(&mut self, _frame: u32, channel: usize) {
        // A sample is faded by the replayer; a patch has a release of its own.
        if let Some(ch) = self.channels.get(channel).filter(|ch| ch.patch) {
            self.voices.note_off(ch.part, channel as u32 + 1);
        }
    }

    fn set_arpeggio(&mut self, _frame: u32, channel: usize, arpeggio: Option<&Arpeggio>) {
        let Some(ch) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        match arpeggio {
            None => self.voices.arp(ch.part, handle, 0.0, 0, &[]),
            Some(a) => {
                let order = match a.order {
                    ChordOrder::Up => crate::arp::ORDER_UP,
                    ChordOrder::Down => crate::arp::ORDER_DOWN,
                    ChordOrder::UpDown => crate::arp::ORDER_UP_DOWN,
                    ChordOrder::Written => crate::arp::ORDER_AS_GIVEN,
                    ChordOrder::Random => crate::arp::ORDER_RANDOM,
                };
                self.voices.arp(ch.part, handle, a.rate, order, a.offsets());
            }
        }
    }

    fn queue(&mut self, _frame: u32, channel: usize, sample: Option<usize>) {
        let Some(&ch) = self.channels.get(channel) else { return };
        let handle = channel as u32 + 1;
        if ch.patch {
            return;
        }
        if self.voices.sounding(ch.part, handle) {
            self.voices.queue_sample(ch.part, handle, sample.map(|s| self.first_slot + s as u16));
        } else if let Some(sample) = sample {
            // The old sample has run out: the new one's loop starts at once.
            self.start_sample(channel, sample, u32::MAX, ch.rate, ch.volume, ch.pan);
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
    /// The part of a frame the last tick's length left over.
    carry: f64,
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
            position: core::array::from_fn(|_| AtomicU32::new(0)),
        });
        let channels = (0..song.channels())
            .map(|c| Channel { part: first_part + c as u16, patch: false, rate: 0.0, volume: 0.0, pan: 0.0 })
            .collect();
        let s = SongPlayer {
            patch_parts: vec![NO_PART; song.instruments.len()],
            song,
            player,
            shared,
            first_part,
            first_slot,
            channels,
            state: STATE_STOPPED,
            repeat: true,
            carry: 0.0,
        };
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
        let mut adapter = Adapter {
            voices,
            song: &self.song,
            first_part: self.first_part,
            first_slot: self.first_slot,
            channels: &mut self.channels,
            patch_parts: &self.patch_parts,
        };
        self.player.seek(&self.song, order, row, &mut adapter);
        self.player.set_repeat(self.repeat);
        self.carry = 0.0;
    }
}

impl Sequencer for SongPlayer {
    fn advance(&mut self, voices: &mut Voices<'_>) -> u32 {
        if self.state != STATE_PLAYING {
            // Nothing to do until an event says otherwise; look in now and then.
            return (voices.sample_rate() / 50.0) as u32;
        }
        let mut adapter = Adapter {
            voices,
            song: &self.song,
            first_part: self.first_part,
            first_slot: self.first_slot,
            channels: &mut self.channels,
            patch_parts: &self.patch_parts,
        };
        let length = self.player.tick(&self.song, 0, &mut adapter);
        // Step macros that keep time with the song go by once a tick.
        for c in 0..self.channels.len() {
            let ch = self.channels[c];
            voices.macro_tick(ch.part);
        }
        if self.player.stopped() {
            // The last notes are left to ring.
            self.state = STATE_STOPPED;
        }
        self.publish();
        self.carry += length;
        let whole = self.carry.floor();
        self.carry -= whole;
        whole as u32
    }

    fn reset(&mut self) {
        self.player.seek(&self.song, 0, 0, &mut Nowhere);
        self.player.set_repeat(self.repeat);
        self.carry = 0.0;
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
                    self.each_voice(|part, handle, ch| {
                        voices.set_volume(part, handle, ch.volume);
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
                for c in 0..self.channels.len() {
                    voices.set_param(self.first_part + c as u16, VOLUME, volume * CHANNEL_GAIN, 0.02);
                }
            }
            SONG_PATCH_PART => {
                let part = if event.y.is_finite() && event.y >= 0.0 && (event.y as usize) < NUM_PARTS { event.y as u16 } else { NO_PART };
                if let Some(slot) = whole(event.x).checked_sub(1).and_then(|i| self.patch_parts.get_mut(i)) {
                    *slot = part;
                }
            }
            _ => {}
        }
        self.publish();
        now
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
        let slot = first_slot as usize + i;
        let data = s.data.to_f32();
        // An empty sample leaves an empty slot, which plays nothing.
        if engine.load_sample(slot, &data, 1, C4_RATE as f32).is_err() {
            let _ = engine.free_sample(slot);
            continue;
        }
        let (mode, start, end) = match (s.loop_kind, s.loop_range()) {
            (LoopKind::Forward, Some((a, b))) => (LOOP_FORWARD, a, b),
            (LoopKind::PingPong, Some((a, b))) => (LOOP_PINGPONG, a, b),
            _ => (LOOP_OFF, 0, 0),
        };
        let _ = engine.configure_sample(slot, 60.0, mode as u32, start, end);
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

    let player = SongPlayer::new(song, engine.sample_rate(), first_part, first_slot);
    let shared = player.shared.clone();
    engine.song = Some(shared.clone());
    engine.set_sequencer(Some(Box::new(player)));
    Ok(shared)
}

/// Takes the loaded song out of the engine: its voices are cut and its
/// sample slots emptied. Does nothing if no song is loaded.
pub fn remove(engine: &mut Engine) {
    let Some(shared) = engine.song.take() else { return };
    let [channels, _, _, _, samples, first_part, first_slot, _] = shared.info();
    engine.set_sequencer(None);
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
