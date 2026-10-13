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

    /// The sequencer as itself, for code that knows which kind it is and
    /// wants more of it than this trait offers (the song player's song is
    /// replaced this way). `None` unless it is written.
    fn as_any(&mut self) -> Option<&mut dyn core::any::Any> {
        None
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

    /// Take the volume of the handle's voices to `volume` (0..4) in a
    /// straight line over `frames` frames, starting on this frame. With 0
    /// frames, or for a voice started on this same frame, it is there at
    /// once. (FastTracker 2 slides a volume over a whole tick.) A later
    /// `set_volume` or `ramp_volume` takes over from wherever it has got to.
    pub fn ramp_volume(&mut self, part: u16, handle: u32, volume: f32, frames: u32) {
        self.engine.voice_ramp(part, handle, false, volume, frames);
    }

    /// The same for the pan of the handle's voices, -1..1.
    pub fn ramp_pan(&mut self, part: u16, handle: u32, pan: f32, frames: u32) {
        self.engine.voice_ramp(part, handle, true, pan, frames);
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

    /// Start a chord: several notes at once on one part under one handle,
    /// each `offsets` semitones from `note` (the note itself is 0, and is
    /// only played if it is listed; at most 8). The handle then means the
    /// chord: its volume, pan and pitch move every note of it, each keeping
    /// its place in the chord, and `note_off` lets them all go. It takes a
    /// voice for each note (times the part's unison).
    pub fn chord(&mut self, part: u16, handle: u32, note: f32, velocity: f32, offsets: &[i8]) {
        self.arp(part, 0, -1.0, crate::arp::ORDER_TOGETHER, offsets);
        self.note_on(part, handle, note, velocity);
    }

    /// Change the notes and the order of the handle's arpeggio, and its
    /// rate unless that is 0, as the `arp_notes` event does: from its next
    /// step, without starting the pattern again.
    pub fn arp_notes(&mut self, part: u16, handle: u32, rate: f32, order: u8, offsets: &[i8]) {
        self.engine.apply(&Event::arp_notes(0.0, part, handle, rate, order, offsets));
        if offsets.len() > 4 {
            self.engine.apply(&Event::arp_more(0.0, part, handle, &offsets[4..]));
        }
    }

    /// Set how the handle's arpeggio plays, as the `arp_style` event does:
    /// `gate` is how much of each step a note is held (0: one note whose
    /// pitch jumps), `octaves` how many octaves the pattern runs through,
    /// `swing` how far every second step is put back, and `levels` how hard
    /// each step is struck (0: all full).
    pub fn arp_style(&mut self, part: u16, handle: u32, gate: f32, octaves: u8, swing: f32, levels: u32) {
        self.engine.apply(&Event::arp_style(0.0, part, handle, gate, octaves, swing, levels));
    }

    /// Have the handle's arpeggio keep time with the `tempo` parameter, so
    /// many `steps` to a beat (0: go by its own rate again).
    pub fn arp_beat(&mut self, part: u16, handle: u32, steps: f32) {
        self.engine.apply(&Event::arp_beat(0.0, part, handle, steps));
    }

    /// Have the handle's arpeggiator follow a pattern of steps of its own
    /// in place of its order (`None`: go by its order again), from its next
    /// step. Send it after the arpeggio is given.
    pub fn arp_pattern(&mut self, part: u16, handle: u32, pattern: Option<&crate::arp::Pattern>) {
        self.engine.arp_own_pattern(part, handle, pattern);
    }

    /// Take the next step of the handle's arpeggio now, lasting `frames`
    /// frames (0: as long as the last): for an arpeggio with no clock of
    /// its own (a rate below 0), which a sequencer steps in its own time.
    pub fn arp_step(&mut self, part: u16, handle: u32, frames: f32) {
        self.engine.apply(&Event::arp_step(0.0, part, handle, frames));
    }

    /// Have the handle's arpeggiator play a sample slot as oscillator 1 of
    /// every note it strikes, from `position` frames in, whatever the part's
    /// patch says: `trigger`, for the notes of an arpeggio or a chord.
    /// `None` goes back to the patch. Send it after the arpeggio is given
    /// and before its first step.
    pub fn arp_sample(&mut self, part: u16, handle: u32, slot: Option<u16>, position: f64) {
        self.engine.arp_sample(part, handle, slot, position);
    }

    /// Give a part's trance gate its pattern: a level for each step, 0
    /// (closed) to 1 (open), at most 32. The part's `gate.` parameters say
    /// how deep, how fast and how smooth it is. False if there are too many.
    pub fn set_gate(&mut self, part: u16, levels: &[f32]) -> bool {
        self.engine.set_gate(part, levels).is_ok()
    }

    /// Say which beat it is on this frame, as the `clock` event does: the
    /// trance gates count their steps from it.
    pub fn clock(&mut self, beat: f32) {
        self.engine.apply(&Event::clock(0.0, beat));
    }

    /// Give the handle's voices a vibrato, as the `voice_vibrato` event
    /// does: `depth` semitones each way at `rate` Hz, after `delay` seconds.
    pub fn set_vibrato(&mut self, part: u16, handle: u32, depth: f32, rate: f32, delay: f32) {
        self.engine.apply(&Event::voice_vibrato(0.0, part, handle, depth, rate, delay));
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
