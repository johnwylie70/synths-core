//! The sequencer hook: a place for something that plays music by itself (a
//! tracker song player) to live inside the engine.
//!
//! A host can only reach the engine between blocks. A sequencer is called
//! from inside `render`, on the exact frame it asked for, however that frame
//! falls within a block, and plays its voices directly through `Voices`.
//! Like everything else on the audio path it must not allocate, lock or wait.

use crate::engine::Engine;
use crate::event::*;

/// Something the engine calls at exact frames while it renders.
pub trait Sequencer: Send {
    /// Called on the frame the sequencer asked for (the first time, on the
    /// next frame rendered after it was installed). Do whatever is due now
    /// through `voices`, and return how many frames from now to be called
    /// again; 0 counts as 1. A tracker returns the length of its tick, and
    /// keeps any fraction of a frame to add to the next.
    fn advance(&mut self, voices: &mut Voices<'_>) -> u32;

    /// The engine was reset: every voice has been stopped. The next call to
    /// `advance` comes on the next frame rendered.
    fn reset(&mut self) {}

    /// A `song` event from the host has come due. Return true to have
    /// `advance` called straight away, on this same frame, whenever it was
    /// next due (a song told to play starts on the event's frame).
    fn event(&mut self, _event: &Event, _voices: &mut Voices<'_>) -> bool {
        false
    }
}

/// How to start a sample on a part: what a tracker channel needs for a note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trigger {
    /// The sample slot to play.
    pub slot: u16,
    /// Where to start, in frames of the sample.
    pub position: f64,
    /// How fast to play, in frames of the sample per second.
    pub rate: f32,
    /// The voice's volume, 0..4 (1 is unchanged), as `voice_volume` sets it.
    pub volume: f32,
    /// The voice's pan, -1..1, added to the part's.
    pub pan: f32,
}

/// The engine as a sequencer sees it, at the frame it was called for.
pub struct Voices<'a> {
    engine: &'a mut Engine,
}

impl<'a> Voices<'a> {
    pub(crate) fn new(engine: &'a mut Engine) -> Voices<'a> {
        Voices { engine }
    }

    /// The frame this call is for, as `Engine::time` counts them.
    pub fn time(&self) -> u64 {
        self.engine.time()
    }

    pub fn sample_rate(&self) -> f32 {
        self.engine.sample_rate()
    }

    /// Carry out any event a host could send, now. Its time is ignored.
    /// Whatever events the engine gains later can be sent this way too.
    pub fn event(&mut self, event: &Event) {
        self.engine.apply(event);
    }

    /// Start a sample on a part, as its only voice with this handle: any
    /// voice the handle already has there is cut (without a click) on the
    /// same frame. The voice uses the part's patch (envelopes, filter,
    /// effects) but plays this sample as oscillator 1 whatever the patch
    /// says, so one part can play a different sample on every note. False if
    /// the slot is empty or the part does not exist.
    pub fn trigger(&mut self, part: u16, handle: u32, t: &Trigger) -> bool {
        self.engine.trigger(part, handle, t)
    }

    /// Start a note as a `note_on` event does.
    pub fn note_on(&mut self, part: u16, handle: u32, note: f32, velocity: f32) {
        self.engine.apply(&Event::note_on(0.0, part, handle, note, velocity));
    }

    /// Let go of the voices with a handle, as a `note_off` event does.
    pub fn note_off(&mut self, part: u16, handle: u32) {
        self.engine.apply(&Event::note_off(0.0, part, handle, 0.0));
    }

    /// Play the handle's voices at `rate` frames of their sample per second,
    /// sliding there over `glide` seconds. (A tracker works this out from
    /// its period.) A rate of 0 holds the sample still where it is until a
    /// rate above 0 lets it go on.
    pub fn set_rate(&mut self, part: u16, handle: u32, rate: f32, glide: f32) {
        self.engine.voice_rate(part, handle, rate, glide);
    }

    /// Move the handle's voices to a note, sliding there over `glide` seconds.
    pub fn set_pitch(&mut self, part: u16, handle: u32, note: f32, glide: f32) {
        self.engine.apply(&Event::voice_pitch(0.0, part, handle, note, glide));
    }

    /// Move the handle's voices to a frequency in Hz.
    pub fn set_freq(&mut self, part: u16, handle: u32, hz: f32, glide: f32) {
        self.engine.apply(&Event::voice_freq(0.0, part, handle, hz, glide));
    }

    /// Set the volume of the handle's voices, 0..4.
    pub fn set_volume(&mut self, part: u16, handle: u32, volume: f32) {
        self.engine.apply(&Event::voice_volume(0.0, part, handle, volume));
    }

    /// Set the pan of the handle's voices, -1..1, added to the part's.
    pub fn set_pan(&mut self, part: u16, handle: u32, pan: f32) {
        self.engine.apply(&Event::voice_pan(0.0, part, handle, pan));
    }

    /// Jump to a position in the sample, in frames.
    pub fn set_position(&mut self, part: u16, handle: u32, frames: f32) {
        self.engine.apply(&Event::voice_position(0.0, part, handle, frames));
    }

    /// Cut the handle's voices at once, without a click.
    pub fn kill(&mut self, part: u16, handle: u32) {
        self.engine.apply(&Event::voice_kill(0.0, part, handle));
    }

    /// When the sample the handle's voices are playing next reaches the end
    /// of its loop, or its end, carry on with the sample in `slot`: from the
    /// start of that sample's loop, round and round, at the same pitch. With
    /// `None`, or a slot that is empty or has no loop, the voices end there
    /// instead. (This is how a ProTracker song swaps samples in mid-note.)
    /// It happens within a control period (16 frames) of the moment.
    pub fn queue_sample(&mut self, part: u16, handle: u32, slot: Option<u16>) {
        self.engine.voice_queue(part, handle, slot);
    }

    /// Give the handle's voices an arpeggio, as the `arp` and `arp_more`
    /// events do: up to 8 semitone `offsets` to step through, `rate` steps a
    /// second (0: 50; below 0: one step for each `macro_tick`), in an `order`
    /// (0 up, 1 down, 2 up and down, 3 as given, 4 random). No offsets stops
    /// it. With handle 0 it is for the part's next `note_on`.
    pub fn arp(&mut self, part: u16, handle: u32, rate: f32, order: u8, offsets: &[i8]) {
        self.engine.apply(&Event::arp(0.0, part, handle, rate, order, offsets));
        if offsets.len() > 4 {
            self.engine.apply(&Event::arp_more(0.0, part, handle, &offsets[4..]));
        }
    }

    /// Fade the handle's voices to nothing in a straight line over `seconds`
    /// and end them there. The handle still steers them until then.
    pub fn fade(&mut self, part: u16, handle: u32, seconds: f32) {
        self.engine.voice_fade(part, handle, seconds);
    }

    /// Is any voice with this handle still sounding on the part?
    pub fn sounding(&self, part: u16, handle: u32) -> bool {
        self.engine.voice_sounding(part, handle)
    }

    /// Set a parameter of a part (or, with `ALL_PARTS`, of every part), gliding over `glide` seconds.
    pub fn set_param(&mut self, part: u16, id: usize, value: f32, glide: f32) {
        self.engine.apply(&Event::param(0.0, part, id, value, glide));
    }

    /// Step the macros of a part's voices (or, with `ALL_PARTS`, of every
    /// part's) by one tick, for parts with `macro.sync` on.
    pub fn macro_tick(&mut self, part: u16) {
        self.engine.apply(&Event::macro_tick(0.0, part));
    }
}
