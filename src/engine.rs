//! The engine: parts, voices, the event queue and the mix.
//!
//! `render` never allocates, locks or waits. Everything it needs is made in
//! `Engine::new`.

use crate::arp::ArpSpec;
use crate::event::*;
use crate::fx::{Chorus, Crusher, Delay, Limiter};
use crate::lfo::FreeLfo;
use crate::macros::{Macro, Step, MACRO_SMOOTH, MAX_STEPS, STEP_SIZE};
use crate::math::{hz_to_note, Rng};
use crate::params::{self, *};
use crate::reverb::{self, Reverb};
use crate::sample::{Sample, LOOP_PINGPONG};
use crate::sequencer::{Sequencer, Trigger, Voices};
use crate::taps::{Taps, NOTES_SIZE};
use crate::voice::{Start, Voice, VoiceState, CONTROL};
use crate::zones::{Picks, Zone, ZoneError, ZoneMaps};

mod arps;
mod io;

/// How many voices can sound at once.
pub const MAX_VOICES: usize = 64;
/// How many parts there are. Each has its own patch, volume, pan and sends.
pub const NUM_PARTS: usize = 64;
/// How many sample slots there are.
pub const MAX_SAMPLES: usize = 1024;
/// How many events the queue holds.
pub const QUEUE_CAPACITY: usize = 4096;
/// How many events fit in the buffer a host writes to.
pub const EVENT_BUFFER: usize = 256;
/// How many frames the engine's own output buffers hold.
pub const OUTPUT_FRAMES: usize = 4096;
/// The longest echo, in seconds.
const MAX_DELAY: f32 = 2.0;
/// How many parameter glides can run at once.
const MAX_RAMPS: usize = 64;

/// How often the notes that are sounding are written out for a display, in control periods.
const NOTES_EVERY: u32 = 16;

/// Why a sample or a macro could not be loaded or changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleError {
    /// The slot number is past the last slot (or the part past the last part).
    BadSlot = -1,
    /// Not 1 or 2 channels, no frames, or a sample rate that is not positive
    /// (or a macro with too many steps).
    BadFormat = -2,
    /// There is no sample in the slot.
    Empty = -3,
    /// A null pointer.
    Null = -4,
}

/// How many held notes a part remembers in mono and legato mode.
const MAX_HELD: usize = 16;

/// A note that is held down on a part in mono or legato mode.
#[derive(Clone, Copy, Default)]
struct HeldNote {
    note: f32,
    handle: u32,
    velocity: f32,
}

impl HeldNote {
    /// Does a note-off with this handle (or, with no handle, this note) mean this note?
    #[inline]
    fn is(&self, handle: u32, note: f32) -> bool {
        if handle != 0 {
            self.handle == handle
        } else {
            (self.note - note).abs() < 0.005
        }
    }
}

struct Part {
    p: [f32; PART_PARAMS],
    lfo: [FreeLfo; 2],
    /// The last note played, for glide.
    last_note: f32,
    has_note: bool,
    /// Mono and legato: the notes held down, the latest last.
    held: [HeldNote; MAX_HELD],
    held_len: usize,
    /// What is left of a voice that was cut, fading out fast so it does not click.
    declick: [f32; 2],
    /// The same for a voice that ended during this chunk: it starts with the next.
    ended: [f32; 2],
    volume: f32,
    crusher: Crusher,
    /// An arpeggio waiting for the part's next note.
    next_arp: ArpSpec,
    steps: Macro,
    /// Peak left and right, and mean square left and right, for the meters.
    meter: [f32; 4],
    bus_l: [f32; CONTROL],
    bus_r: [f32; CONTROL],
    in_use: bool,
}

impl Part {
    fn new() -> Part {
        let p = params::part_defaults();
        Part {
            p,
            lfo: [FreeLfo::default(); 2],
            last_note: 0.0,
            has_note: false,
            held: [HeldNote::default(); MAX_HELD],
            held_len: 0,
            declick: [0.0; 2],
            ended: [0.0; 2],
            volume: p[VOLUME],
            crusher: Crusher::default(),
            next_arp: ArpSpec::NONE,
            steps: Macro::none(),
            meter: [0.0; 4],
            bus_l: [0.0; CONTROL],
            bus_r: [0.0; CONTROL],
            in_use: false,
        }
    }
}

impl Part {
    /// Take a note out of the notes held down. With a handle, the notes with
    /// that handle; without, the notes at that pitch.
    fn forget(&mut self, handle: u32, note: f32) {
        let mut kept = 0;
        for i in 0..self.held_len {
            if !self.held[i].is(handle, note) {
                self.held[kept] = self.held[i];
                kept += 1;
            }
        }
        self.held_len = kept;
    }
}

/// A parameter on its way to a new value.
#[derive(Clone, Copy, Default)]
struct Ramp {
    live: bool,
    part: u16,
    id: u16,
    value: f32,
    target: f32,
    step: f32,
    ticks: u32,
}

pub struct Engine {
    sample_rate: f32,
    /// Frames rendered since the engine was made.
    time: u64,
    /// How far into the current control period we are.
    tick_pos: usize,
    /// An event has just changed something: voices must look again.
    dirty: bool,
    voices: Box<[Voice]>,
    parts: Box<[Part]>,
    globals: [f32; GLOBAL_PARAMS],
    samples: Box<[Option<Sample>]>,
    /// The parts' sample maps.
    zones: ZoneMaps,
    queue: Queue,
    ramps: Box<[Ramp]>,
    live_ramps: usize,
    rng: Rng,
    born: u64,
    delay: Delay,
    reverb: Reverb,
    chorus: Chorus,
    limiter: Limiter,
    master: f32,
    /// The smoothing of volume changes, per sample.
    smooth: f32,
    declick_decay: f32,
    delay_tone: (f32, f32),
    /// Frames of silence in a row, with no voice sounding.
    quiet: u64,
    sleep_after: u64,
    asleep: bool,
    /// Something has sounded in this control period.
    tick_loud: bool,
    /// The frame of the next arpeggio step of any voice or arpeggiator.
    arp_at: u64,
    /// The arpeggiators, and the number the last one made was given.
    arps: Box<[arps::Arpeggiator]>,
    arp_ids: u16,
    /// What a display reads: scope, meters, sounding notes.
    pub(crate) taps: Box<Taps>,
    /// The song that is loaded, as a host may read it while sound is made.
    #[cfg(feature = "songs")]
    pub(crate) song: Option<std::sync::Arc<crate::song_player::Shared>>,
    meter: [f32; 4],
    /// How much a meter's peak and mean square keep, per control period and per sample.
    peak_keep: f32,
    rms_keep: f32,
    rms_take: f32,
    notes_in: u32,
    /// Something that plays by itself, called at exact frames from `render`.
    sequencer: Option<Box<dyn Sequencer>>,
    /// The frame it next wants to be called on.
    sequencer_at: u64,
    /// The live input, the voice effects of the parts, and the recorder.
    pub(crate) input: crate::input::Input,
    vfx: crate::voicefx::VoiceFx,
    pub(crate) recorder: crate::input::Recorder,
    /// Buffers for hosts that use the C interface.
    pub(crate) out_l: Box<[f32]>,
    pub(crate) out_r: Box<[f32]>,
    pub(crate) event_buf: Box<[Event]>,
}

impl Engine {
    /// Make an engine. This is the only place memory is allocated, apart
    /// from loading samples.
    pub fn new(sample_rate: f32) -> Box<Engine> {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 192_000.0) } else { 48000.0 };
        let globals = params::global_defaults();
        Box::new(Engine {
            sample_rate: sr,
            time: 0,
            tick_pos: 0,
            dirty: false,
            voices: vec![Voice::new(); MAX_VOICES].into_boxed_slice(),
            parts: (0..NUM_PARTS).map(|_| Part::new()).collect(),
            globals,
            samples: (0..MAX_SAMPLES).map(|_| None).collect(),
            zones: ZoneMaps::new(),
            queue: Queue::new(QUEUE_CAPACITY),
            ramps: vec![Ramp::default(); MAX_RAMPS].into_boxed_slice(),
            live_ramps: 0,
            rng: Rng::new(0x5EED_1234),
            born: 0,
            delay: Delay::new(sr, MAX_DELAY),
            reverb: Reverb::new(sr),
            chorus: Chorus::new(sr),
            limiter: Limiter::new(sr),
            master: globals[MASTER_VOLUME - GLOBAL_BASE],
            smooth: 1.0 - (-1.0 / (0.005 * sr)).exp(),
            declick_decay: (-1.0 / (0.002 * sr)).exp(),
            delay_tone: (-1.0, 0.0),
            quiet: u64::MAX / 2,
            sleep_after: ((MAX_DELAY + 0.5) * sr) as u64,
            asleep: true,
            tick_loud: false,
            arp_at: u64::MAX,
            arps: vec![arps::Arpeggiator::NONE; crate::arp::MAX_ARPEGGIATORS].into_boxed_slice(),
            arp_ids: 0,
            taps: Box::new(Taps::new(NUM_PARTS)),
            #[cfg(feature = "songs")]
            song: None,
            meter: [0.0; 4],
            peak_keep: (-(CONTROL as f32) / (0.5 * sr)).exp(),
            rms_keep: (-(CONTROL as f32) / (0.1 * sr)).exp(),
            rms_take: 1.0 - (-1.0 / (0.1 * sr)).exp(),
            notes_in: 0,
            sequencer: None,
            sequencer_at: 0,
            input: crate::input::Input::new(sr),
            vfx: crate::voicefx::VoiceFx::new(sr),
            recorder: Default::default(),
            out_l: vec![0.0; OUTPUT_FRAMES].into_boxed_slice(),
            out_r: vec![0.0; OUTPUT_FRAMES].into_boxed_slice(),
            event_buf: vec![Event::default(); EVENT_BUFFER].into_boxed_slice(),
        })
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Frames rendered since the engine was made: the clock events are timed by.
    pub fn time(&self) -> u64 {
        self.time
    }

    /// How many voices are sounding.
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active()).count()
    }

    /// How many events are waiting in the queue.
    pub fn queued_events(&self) -> usize {
        self.queue.len()
    }

    /// Stop every sound at once and forget every waiting event. Patches and
    /// samples stay.
    pub fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.state = VoiceState::Free;
        }
        for part in self.parts.iter_mut() {
            part.declick = [0.0; 2];
            part.ended = [0.0; 2];
            part.next_arp = ArpSpec::NONE;
            part.has_note = false;
            part.held_len = 0;
            part.volume = part.p[VOLUME];
        }
        for r in self.ramps.iter_mut() {
            r.live = false;
        }
        self.live_ramps = 0;
        self.zones.rewind_all();
        self.arp_reset();
        self.queue.clear();
        self.delay.clear();
        self.reverb.clear();
        self.chorus.clear();
        self.limiter.clear();
        self.reset_io();
        self.master = self.globals[MASTER_VOLUME - GLOBAL_BASE];
        self.quiet = u64::MAX / 2;
        self.asleep = true;
        self.sequencer_at = self.time;
        if let Some(s) = self.sequencer.as_mut() {
            s.reset();
        }
        self.write_notes();
    }

    // ---- samples ----

    /// Put a sample in a slot, replacing whatever was there. `data` holds one
    /// number per channel per frame. It starts with no loop and plays back
    /// unchanged at note 60.
    pub fn load_sample(&mut self, slot: usize, data: &[f32], channels: u32, rate: f32) -> Result<(), SampleError> {
        if slot >= MAX_SAMPLES {
            return Err(SampleError::BadSlot);
        }
        if !(channels == 1 || channels == 2) || !rate.is_finite() || rate <= 0.0 {
            return Err(SampleError::BadFormat);
        }
        let frames = data.len() / channels as usize;
        if frames == 0 || frames > u32::MAX as usize / 4 {
            return Err(SampleError::BadFormat);
        }
        let clean: Box<[f32]> = data[..frames * channels as usize]
            .iter()
            .map(|&x| if x.is_finite() { x.clamp(-8.0, 8.0) } else { 0.0 })
            .collect();
        self.samples[slot] = Some(Sample {
            data: clean,
            frames: frames as u32,
            channels,
            rate,
            base_note: 60.0,
            loop_mode: 0,
            loop_start: 0,
            loop_end: frames as u32,
        });
        Ok(())
    }

    /// Set a sample's base note (the note at which it plays back unchanged)
    /// and its loop. `loop_end` is one past the last frame of the loop.
    pub fn configure_sample(
        &mut self,
        slot: usize,
        base_note: f32,
        loop_mode: u32,
        loop_start: u32,
        loop_end: u32,
    ) -> Result<(), SampleError> {
        let s = match self.samples.get_mut(slot) {
            None => return Err(SampleError::BadSlot),
            Some(None) => return Err(SampleError::Empty),
            Some(Some(s)) => s,
        };
        if !base_note.is_finite() {
            return Err(SampleError::BadFormat);
        }
        s.base_note = base_note.clamp(-60.0, 200.0);
        s.loop_end = loop_end.min(s.frames);
        s.loop_start = loop_start.min(s.loop_end);
        s.loop_mode = if s.loop_end > s.loop_start { loop_mode.min(LOOP_PINGPONG as u32) as u8 } else { 0 };
        Ok(())
    }

    /// Empty a slot. Voices playing the sample fall silent.
    pub fn free_sample(&mut self, slot: usize) -> Result<(), SampleError> {
        match self.samples.get_mut(slot) {
            None => Err(SampleError::BadSlot),
            Some(s) => {
                *s = None;
                Ok(())
            }
        }
    }

    // ---- sample maps ----

    /// Give a part its sample map, replacing any it had: a list of zones,
    /// each a sample slot for a range of keys and velocities (see
    /// `zones::Zone`). No zones takes the map away. `flags` is
    /// `zones::MAP_SILENT_OUTSIDE` or 0. Notes already sounding keep the
    /// zones they started with. It asks for no memory.
    pub fn set_zones(&mut self, part: usize, zones: &[Zone], flags: u32) -> Result<(), SampleError> {
        self.set_zones_with(part, zones.len(), flags, |i| zones[i])
    }

    /// `set_zones`, with zone `i` of `len` read by `zone(i)`.
    pub fn set_zones_with(&mut self, part: usize, len: usize, flags: u32, zone: impl Fn(usize) -> Zone) -> Result<(), SampleError> {
        self.zones.set_with(part, len, flags, zone).map_err(|e| match e {
            ZoneError::BadPart => SampleError::BadSlot,
            ZoneError::TooMany | ZoneError::BadSlot => SampleError::BadFormat,
        })
    }

    /// A part's zones as the engine holds them, in the order they were given.
    pub fn zones(&self, part: usize) -> &[Zone] {
        self.zones.zones(part)
    }

    /// A part's map flags.
    pub fn zone_flags(&self, part: usize) -> u32 {
        self.zones.flags(part)
    }

    /// Does a new note on this part go by its sample map?
    fn mapped(&self, part: usize) -> bool {
        !self.zones.zones(part).is_empty() && self.parts[part].p[OSC1_WAVE] as u8 == WAVE_SAMPLE
    }

    // ---- macros ----

    /// Give a part its step macro, replacing any it had. `numbers` holds
    /// five numbers a step: wave, pitch, pulse width, cutoff, volume (see
    /// `macros::Step`). No numbers at all takes the macro away.
    /// `part` may be `ALL_PARTS`.
    pub fn set_macro(
        &mut self,
        part: u16,
        numbers: &[f32],
        loop_start: u32,
        loop_end: u32,
        release: u32,
        flags: u32,
    ) -> Result<(), SampleError> {
        let count = numbers.len() / STEP_SIZE;
        if count > MAX_STEPS {
            return Err(SampleError::BadFormat);
        }
        if part != ALL_PARTS && part as usize >= NUM_PARTS {
            return Err(SampleError::BadSlot);
        }
        let mut m = Macro::none();
        for (i, v) in numbers.chunks_exact(STEP_SIZE).enumerate() {
            m.steps[i] = Step::from_numbers(v);
        }
        m.len = count as u8;
        m.loop_end = loop_end.min(count as u32) as u8;
        m.loop_start = loop_start.min(m.loop_end as u32) as u8;
        m.release = release.min(255) as u8;
        m.smooth = flags & MACRO_SMOOTH != 0;
        for (i, p) in self.parts.iter_mut().enumerate() {
            if part == ALL_PARTS || part as usize == i {
                p.steps = m;
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// What a display reads: the scope, the meters and the sounding notes.
    pub fn taps(&self) -> &Taps {
        &self.taps
    }

    // ---- parameters ----

    /// A parameter's value now. For an engine-wide parameter `part` is ignored.
    pub fn param(&self, part: usize, id: usize) -> f32 {
        match params::info(id) {
            Some(info) if info.is_global() => self.globals[id - GLOBAL_BASE],
            Some(_) => self.parts.get(part).map(|p| p.p[id]).unwrap_or(0.0),
            None => 0.0,
        }
    }

    /// Set a parameter at once. `part` may be `ALL_PARTS`.
    pub fn set_param(&mut self, part: u16, id: usize, value: f32) {
        self.set_param_gliding(part, id, value, 0.0);
    }

    fn store(&mut self, part: u16, id: usize, value: f32) {
        if id >= GLOBAL_BASE {
            self.globals[id - GLOBAL_BASE] = value;
        } else if let Some(p) = self.parts.get_mut(part as usize) {
            p.p[id] = value;
        }
    }

    fn part_is_silent(&self, part: u16) -> bool {
        !self.voices.iter().any(|v| v.active() && v.part == part)
    }

    fn set_param_gliding(&mut self, part: u16, id: usize, value: f32, glide: f32) {
        let info = match params::info(id) {
            Some(i) => i,
            None => return,
        };
        if !value.is_finite() {
            return;
        }
        if part == ALL_PARTS && !info.is_global() {
            for i in 0..NUM_PARTS as u16 {
                self.set_param_gliding(i, id, value, glide);
            }
            return;
        }
        let part = if info.is_global() { 0 } else { part };
        if part as usize >= NUM_PARTS {
            return;
        }
        let value = info.clean(value);
        self.dirty = true;
        // An older glide of the same parameter stops here.
        let mut slot = None;
        for (i, r) in self.ramps.iter_mut().enumerate() {
            if r.live && r.part == part && r.id == id as u16 {
                r.live = false;
                self.live_ramps -= 1;
                slot = Some(i);
            } else if !r.live && slot.is_none() {
                slot = Some(i);
            }
        }
        let ticks = (glide * self.sample_rate / CONTROL as f32) as u32;
        if glide > 0.0 && ticks > 0 && info.kind == Kind::Float {
            if let Some(i) = slot {
                let now = self.param(part as usize, id);
                self.ramps[i] = Ramp {
                    live: true,
                    part,
                    id: id as u16,
                    value: now,
                    target: value,
                    step: (value - now) / ticks as f32,
                    ticks,
                };
                self.live_ramps += 1;
                return;
            }
        }
        self.store(part, id, value);
        // With nothing sounding there is nothing to smooth: jump.
        if id == VOLUME && self.part_is_silent(part) && !self.input_sounds_on(part) {
            self.parts[part as usize].volume = value;
        }
        if id == MASTER_VOLUME && self.active_voices() == 0 && !self.input.sounding {
            self.master = value;
        }
    }

    fn run_ramps(&mut self) {
        for i in 0..MAX_RAMPS {
            let mut r = self.ramps[i];
            if !r.live {
                continue;
            }
            r.ticks -= 1;
            if r.ticks == 0 {
                r.value = r.target;
                r.live = false;
                self.live_ramps -= 1;
            } else {
                r.value += r.step;
            }
            self.ramps[i] = r;
            self.store(r.part, r.id as usize, r.value);
        }
    }

    // ---- events ----

    /// Queue an event for its time. False if the queue is full.
    pub fn push(&mut self, event: Event) -> bool {
        self.queue.push(event)
    }

    /// Cut a voice dead, leaving a very short fade so it does not click.
    fn cut(parts: &mut [Part], v: &mut Voice) {
        if v.active() {
            if let Some(p) = parts.get_mut(v.part as usize) {
                p.declick[0] += v.last[0];
                p.declick[1] += v.last[1];
            }
            v.state = VoiceState::Free;
            v.last = [0.0; 2];
        }
    }

    /// Find a voice for a new note: a free one, or else the one least missed.
    fn take_voice(&mut self) -> usize {
        let limit = (self.globals[VOICES - GLOBAL_BASE] as usize).clamp(1, MAX_VOICES);
        let mut free = None;
        let mut count = 0;
        // The oldest voice that has been let go, and the oldest of all.
        let mut released: Option<usize> = None;
        let mut oldest: Option<usize> = None;
        for (i, v) in self.voices.iter().enumerate() {
            match v.state {
                VoiceState::Free => {
                    if free.is_none() {
                        free = Some(i);
                    }
                }
                state => {
                    count += 1;
                    if state == VoiceState::Released && released.is_none_or(|j| v.born < self.voices[j].born) {
                        released = Some(i);
                    }
                    if oldest.is_none_or(|j| v.born < self.voices[j].born) {
                        oldest = Some(i);
                    }
                }
            }
        }
        if count < limit {
            if let Some(i) = free {
                return i;
            }
        }
        let i = released.or(oldest).unwrap_or(0);
        Engine::cut(&mut self.parts, &mut self.voices[i]);
        i
    }

    fn note_on(&mut self, part: u16, handle: u32, note: f32, velocity: f32) {
        let pi = part as usize;
        if pi >= NUM_PARTS || !note.is_finite() {
            return;
        }
        let note = note.clamp(-24.0, 160.0);
        let velocity = if velocity.is_finite() { velocity.clamp(0.0, 1.0) } else { 1.0 };
        if self.parts[pi].next_arp.wants_arpeggiator() {
            return self.arp_begin(part, handle, note, velocity);
        }
        let sr = self.sample_rate;
        let glide = self.parts[pi].p[GLIDE];
        let mode = self.parts[pi].p[MODE] as u8;
        if mode == 0 {
            self.parts[pi].held_len = 0;
        } else {
            // Remember the note, so that letting go of a later one comes back to it.
            let p = &mut self.parts[pi];
            p.forget(handle, note);
            if p.held_len == MAX_HELD {
                p.held.copy_within(1.., 0);
                p.held_len -= 1;
            }
            p.held[p.held_len] = HeldNote { note, handle, velocity };
            p.held_len += 1;
            if mode == 1 && self.mapped(pi) {
                // Mono, with a sample map: the new note has zones of its
                // own, so the sounding voices make way for new ones.
                self.cut_part(part, false);
            } else {
                // One note at a time: a voice that is already sounding moves to the new note.
                let mut found = false;
                for v in self.voices.iter_mut() {
                    if v.active() && v.part == part {
                        v.replay(note, velocity, handle, glide, mode == 1, sr);
                        found = true;
                    }
                }
                if found {
                    self.parts[pi].last_note = note;
                    return;
                }
            }
        }
        self.start_note(part, handle, note, velocity);
    }

    /// Cut the voices of a part (or only those still held), without a click.
    fn cut_part(&mut self, part: u16, held_only: bool) {
        for v in self.voices.iter_mut() {
            if v.active() && v.part == part && (!held_only || v.state == VoiceState::Held) {
                Engine::cut(&mut self.parts, v);
            }
        }
    }

    /// Start the voices of a new note: one for each unison voice, and with
    /// a sample map that many for each zone the note plays.
    fn start_note(&mut self, part: u16, handle: u32, note: f32, velocity: f32) {
        let pi = part as usize;
        let sr = self.sample_rate;
        let glide = self.parts[pi].p[GLIDE];
        let mapped = self.mapped(pi);
        let picks = if mapped { self.zones.pick(pi, note, velocity) } else { Picks::default() };
        let layers = if mapped { picks.count } else { 1 };
        let unison = (self.parts[pi].p[UNISON] as usize).clamp(1, 8);
        let from = if self.parts[pi].has_note { self.parts[pi].last_note } else { note };
        for layer in 0..layers {
            for i in 0..unison {
                let spread = if unison > 1 { i as f32 / (unison - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                let slot = self.take_voice();
                self.born += 1;
                let p = &self.parts[pi].p;
                let start = Start {
                    part,
                    handle,
                    note,
                    from,
                    glide: if from != note { glide } else { 0.0 },
                    velocity,
                    detune: spread * p[UNISON_DETUNE] * 0.01,
                    pan: spread * p[UNISON_SPREAD],
                    gain: 1.0 / (unison as f32).sqrt(),
                    random_phase: unison > 1,
                    seed: self.rng.next_u32(),
                    born: self.born,
                };
                self.voices[slot].start(&start, p, &self.samples, sr);
                if mapped {
                    let zone = self.zones.zone(picks.index[layer]);
                    self.voices[slot].use_zone(&zone, picks.gain[layer], p, &self.samples, sr);
                }
                // An arpeggio that was waiting for this note.
                let arp = self.parts[pi].next_arp;
                if arp.count > 0 {
                    let v = &mut self.voices[slot];
                    let mut rng = v.rng;
                    v.arp.set(&arp, self.time, sr, &mut rng);
                    v.rng = rng;
                    self.arp_at = self.time;
                }
            }
        }
        self.parts[pi].next_arp = ArpSpec::NONE;
        self.parts[pi].last_note = note;
        self.parts[pi].has_note = true;
    }

    /// Let go of a note on one part. In mono and legato mode, letting go of
    /// the sounding note while others are still held goes back to the latest
    /// of them.
    fn note_off(&mut self, part: u16, handle: u32, note: f32) {
        let sr = self.sample_rate;
        let pi = part as usize;
        if pi >= NUM_PARTS {
            return;
        }
        self.arp_note_off(part, handle, note);
        let p = &mut self.parts[pi];
        let mode = p.p[MODE] as u8;
        if mode != 0 && p.held_len > 0 {
            let was_sounding = p.held[p.held_len - 1].is(handle, note);
            p.forget(handle, note);
            if was_sounding && p.held_len > 0 {
                let back = p.held[p.held_len - 1];
                let glide = p.p[GLIDE];
                if mode == 1 && self.mapped(pi) {
                    // With a sample map the note gone back to has zones of its own.
                    self.cut_part(part, true);
                    self.start_note(part, back.handle, back.note, back.velocity);
                    return;
                }
                let p = &mut self.parts[pi];
                p.last_note = back.note;
                for v in self.voices.iter_mut() {
                    if v.state == VoiceState::Held && v.part == part {
                        v.replay(back.note, back.velocity, back.handle, glide, mode == 1, sr);
                    }
                }
                return;
            }
        }
        for v in self.voices.iter_mut() {
            if v.state == VoiceState::Held && v.part == part {
                let hit = if handle != 0 { v.handle == handle } else { (v.key - note).abs() < 0.005 };
                if hit {
                    v.release();
                }
            }
        }
    }

    /// Do what an event says, now.
    pub fn apply(&mut self, e: &Event) {
        self.dirty = true;
        self.quiet = 0;
        self.asleep = false;
        let sr = self.sample_rate;
        let part = e.part;
        let all = part == ALL_PARTS;
        match e.kind {
            EV_NOTE_ON => self.note_on(part, e.a, e.x, e.y),
            EV_NOTE_OFF => {
                if all {
                    for i in 0..NUM_PARTS as u16 {
                        self.note_off(i, e.a, e.x);
                    }
                } else {
                    self.note_off(part, e.a, e.x);
                }
            }
            EV_ALL_NOTES_OFF => {
                self.arp_end_part(part);
                for (i, p) in self.parts.iter_mut().enumerate() {
                    if all || i == part as usize {
                        p.held_len = 0;
                    }
                }
                for v in self.voices.iter_mut() {
                    if v.active() && (all || v.part == part) {
                        if e.a != 0 {
                            Engine::cut(&mut self.parts, v);
                        } else {
                            v.release();
                        }
                    }
                }
            }
            EV_SONG => self.sequencer_event(e),
            EV_RECORD => self.record_event(e),
            EV_MACRO_TICK => {
                for v in self.voices.iter_mut() {
                    if v.active() && (all || v.part == part) {
                        v.macro_player.ticks = v.macro_player.ticks.saturating_add(1);
                        let mut rng = v.rng;
                        if v.arp.tick(&mut rng) {
                            v.pitch_jump = true;
                        }
                        v.rng = rng;
                    }
                }
            }
            EV_ARP_STYLE | EV_ARP_BEAT | EV_ARP_NOTES | EV_ARP_STEP => {
                self.arp_event(e);
            }
            EV_ARP | EV_ARP_MORE => {
                if self.arp_event(e) {
                    return;
                }
                let first = e.kind == EV_ARP;
                if e.a == 0 {
                    // For the next note of the part.
                    if let Some(p) = self.parts.get_mut(part as usize) {
                        if first {
                            p.next_arp = ArpSpec::from_event(e.x, e.y, e.z, e.b);
                        } else {
                            p.next_arp.set_packed(4, e.b);
                        }
                    }
                    return;
                }
                let spec = ArpSpec::from_event(e.x, e.y, e.z, e.b);
                for v in self.voices.iter_mut() {
                    if v.active() && v.handle == e.a && (all || v.part == part) {
                        let mut rng = v.rng;
                        if first {
                            v.arp.set(&spec, self.time, sr, &mut rng);
                        } else {
                            v.arp.set_more(4, e.b, self.time, sr, &mut rng);
                        }
                        v.rng = rng;
                        v.pitch_jump = true;
                    }
                }
                self.arp_at = self.time;
            }
            EV_PARAM => self.set_param_gliding(part, e.a as usize, e.x, e.y),
            EV_PITCH_BEND => self.set_param_gliding(part, BEND, e.x, e.y),
            EV_VOICE_PITCH | EV_VOICE_FREQ | EV_VOICE_VOLUME | EV_VOICE_PAN | EV_VOICE_POSITION | EV_VOICE_KILL => {
                if e.a == 0 || (e.kind != EV_VOICE_KILL && !e.x.is_finite()) {
                    return;
                }
                // An arpeggiator with the handle is told too, for the notes it has yet to strike.
                match e.kind {
                    EV_VOICE_PITCH => self.arp_root(part, e.a, e.x.clamp(-24.0, 160.0)),
                    EV_VOICE_FREQ if e.x > 0.0 => self.arp_root(part, e.a, hz_to_note(e.x).clamp(-24.0, 160.0)),
                    EV_VOICE_VOLUME => self.arp_level(part, e.a, false, e.x.clamp(0.0, 4.0)),
                    EV_VOICE_PAN => self.arp_level(part, e.a, true, e.x.clamp(-1.0, 1.0)),
                    EV_VOICE_KILL => self.arp_end(part, e.a, false),
                    _ => {}
                }
                for v in self.voices.iter_mut() {
                    if !(v.active() && v.handle == e.a && (all || v.part == part)) {
                        continue;
                    }
                    match e.kind {
                        EV_VOICE_PITCH => v.set_pitch((e.x + v.arp.shift).clamp(-24.0, 160.0), e.y, sr),
                        EV_VOICE_FREQ => {
                            if e.x > 0.0 {
                                v.set_pitch((hz_to_note(e.x) + v.arp.shift).clamp(-24.0, 160.0), e.y, sr);
                            }
                        }
                        EV_VOICE_VOLUME => v.slide_volume(e.x.clamp(0.0, 4.0), 0.0),
                        EV_VOICE_PAN => v.slide_pan(e.x.clamp(-1.0, 1.0), 0.0),
                        EV_VOICE_POSITION => {
                            if let Some(p) = self.parts.get_mut(v.part as usize) {
                                p.declick[0] += v.last[0];
                                p.declick[1] += v.last[1];
                            }
                            v.last = [0.0; 2];
                            v.set_position(e.x);
                        }
                        _ => Engine::cut(&mut self.parts, v),
                    }
                }
            }
            _ => {}
        }
    }

    // ---- the sequencer hook ----

    /// Install a sequencer (or, with `None`, take it away), and get back
    /// the one that was there. `render` calls it on the next frame it
    /// renders, and after that whenever the sequencer asks. Voices it has
    /// started carry on; stop them first if they should not.
    ///
    /// This is not for the middle of a render, and what is handed back
    /// should be dropped away from the audio thread, since that frees memory.
    pub fn set_sequencer(&mut self, sequencer: Option<Box<dyn Sequencer>>) -> Option<Box<dyn Sequencer>> {
        self.sequencer_at = self.time;
        core::mem::replace(&mut self.sequencer, sequencer)
    }

    pub fn has_sequencer(&self) -> bool {
        self.sequencer.is_some()
    }

    /// Do something to the sequencer and the engine together, without
    /// disturbing when the sequencer is next due. Not for the middle of a
    /// render. `None` if there is no sequencer.
    pub fn with_sequencer<R>(&mut self, f: impl FnOnce(&mut Engine, &mut dyn Sequencer) -> R) -> Option<R> {
        let mut s = self.sequencer.take()?;
        let result = f(self, s.as_mut());
        self.sequencer = Some(s);
        Some(result)
    }

    /// Hand a `song` event to the sequencer, and call it at once if it asks.
    fn sequencer_event(&mut self, e: &Event) {
        if let Some(mut s) = self.sequencer.take() {
            if s.event(e, &mut Voices::new(self)) {
                let wait = s.advance(&mut Voices::new(self));
                self.sequencer_at = self.time + wait.max(1) as u64;
            }
            self.sequencer = Some(s);
        }
    }

    /// Call the sequencer for the frame the engine has reached.
    fn run_sequencer(&mut self) {
        if let Some(mut s) = self.sequencer.take() {
            let wait = s.advance(&mut Voices::new(self));
            self.sequencer_at = self.time + wait.max(1) as u64;
            self.sequencer = Some(s);
        }
    }

    /// Something has changed and something may be about to sound.
    fn wake(&mut self) {
        self.dirty = true;
        self.quiet = 0;
        self.asleep = false;
    }

    /// The note at which a sample plays at `rate` frames a second.
    fn note_for_rate(sample: &Sample, rate: f32) -> f32 {
        (sample.base_note + 12.0 * (rate / sample.rate).log2()).clamp(-24.0, 160.0)
    }

    /// Start a sample on a part as the only voice with its handle. See `Voices::trigger`.
    pub(crate) fn trigger(&mut self, part: u16, handle: u32, t: &Trigger) -> bool {
        let pi = part as usize;
        let sample = match self.samples.get(t.slot as usize) {
            Some(Some(s)) if pi < NUM_PARTS => s,
            _ => return false,
        };
        if !(t.rate.is_finite() && t.rate > 0.0) {
            return false;
        }
        let note = Engine::note_for_rate(sample, t.rate);
        self.wake();
        if handle != 0 {
            self.arp_end(part, handle, false);
            for v in self.voices.iter_mut() {
                if v.active() && v.part == part && v.handle == handle {
                    Engine::cut(&mut self.parts, v);
                }
            }
        }
        let slot = self.take_voice();
        self.born += 1;
        let start = Start {
            part,
            handle,
            note,
            from: note,
            glide: 0.0,
            velocity: 1.0,
            detune: 0.0,
            pan: 0.0,
            gain: 1.0,
            random_phase: false,
            seed: self.rng.next_u32(),
            born: self.born,
        };
        let v = &mut self.voices[slot];
        v.start(&start, &self.parts[pi].p, &self.samples, self.sample_rate);
        v.use_sample(t.slot, t.position);
        v.slide_volume(if t.volume.is_finite() { t.volume.clamp(0.0, 4.0) } else { 1.0 }, 0.0);
        v.slide_pan(if t.pan.is_finite() { t.pan.clamp(-1.0, 1.0) } else { 0.0 }, 0.0);
        true
    }

    /// Play the handle's voices at `rate` frames of their sample a second.
    pub(crate) fn voice_rate(&mut self, part: u16, handle: u32, rate: f32, glide: f32) {
        if handle == 0 || !(rate.is_finite() && rate >= 0.0) {
            return;
        }
        self.wake();
        if rate > 0.0 {
            self.arp_rate(part, handle, rate);
        }
        let sr = self.sample_rate;
        let glide = if glide.is_finite() { glide } else { 0.0 };
        for v in self.voices.iter_mut() {
            if !(v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part)) {
                continue;
            }
            // A rate of nothing holds the sample still where it is.
            v.set_hold(rate == 0.0);
            if rate == 0.0 {
                continue;
            }
            let slot = match self.parts.get(v.part as usize) {
                Some(p) => v.sample_slot(&p.p),
                None => continue,
            };
            if let Some(Some(sample)) = self.samples.get(slot) {
                v.set_pitch((Engine::note_for_rate(sample, rate) + v.arp.shift).clamp(-24.0, 160.0), glide, sr);
            }
        }
    }

    /// Give the handle's voices a sample to carry on with. See `Voices::queue_sample`.
    pub(crate) fn voice_queue(&mut self, part: u16, handle: u32, slot: Option<u16>) {
        if handle == 0 {
            return;
        }
        for v in self.voices.iter_mut() {
            if v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part) {
                v.queue_sample(slot);
            }
        }
    }

    /// Slide the volume (or, with `pan`, the pan) of the handle's voices to
    /// `value` over `frames` samples. See `Voices::ramp_volume`.
    pub(crate) fn voice_ramp(&mut self, part: u16, handle: u32, pan: bool, value: f32, frames: u32) {
        if handle == 0 || !value.is_finite() {
            return;
        }
        self.wake();
        self.arp_level(part, handle, pan, if pan { value.clamp(-1.0, 1.0) } else { value.clamp(0.0, 4.0) });
        for v in self.voices.iter_mut() {
            if v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part) {
                if pan {
                    v.slide_pan(value.clamp(-1.0, 1.0), frames as f32);
                } else {
                    v.slide_volume(value.clamp(0.0, 4.0), frames as f32);
                }
            }
        }
    }

    /// Fade the handle's voices to nothing over `seconds`, and end them.
    pub(crate) fn voice_fade(&mut self, part: u16, handle: u32, seconds: f32) {
        if handle == 0 {
            return;
        }
        self.wake();
        self.arp_end(part, handle, false);
        let sr = self.sample_rate;
        for v in self.voices.iter_mut() {
            if v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part) {
                if seconds > 0.0 {
                    v.fade_out(seconds, sr);
                } else {
                    Engine::cut(&mut self.parts, v);
                }
            }
        }
    }

    /// Is any voice with this handle still sounding on the part?
    pub(crate) fn voice_sounding(&self, part: u16, handle: u32) -> bool {
        handle != 0 && self.voices.iter().any(|v| v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part))
    }

    // ---- sound ----

    /// Render the next frames into `left` and `right` (any length; the
    /// shorter of the two is used).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.render_with(None, left, right);
    }

    /// `render`, with or without a live input (see `render_io`).
    #[inline(never)]
    fn render_with(&mut self, input: io::Frames<'_>, left: &mut [f32], right: &mut [f32]) {
        let frames = io::length(&input, left.len().min(right.len()));
        let mut done = 0;
        while done < frames {
            // The sequencer goes first, so that a host's event for the same
            // frame has the last word.
            if self.sequencer.is_some() && self.sequencer_at <= self.time {
                self.run_sequencer();
            }
            while let Some(t) = self.queue.next_time() {
                if t > self.time {
                    break;
                }
                if let Some(e) = self.queue.pop() {
                    self.apply(&e);
                }
            }
            // Arpeggio steps that fall on this frame; then find the next one.
            if self.arp_at <= self.time {
                let mut next = u64::MAX;
                for v in self.voices.iter_mut() {
                    if v.active() && v.arp.active() {
                        let mut rng = v.rng;
                        if v.arp.run(self.time, &mut rng) {
                            v.pitch_jump = true;
                            self.dirty = true;
                        }
                        v.rng = rng;
                        next = next.min(v.arp.next_frame());
                    }
                }
                self.arp_at = next.min(self.run_arps());
            }
            let mut n = (frames - done).min(CONTROL - self.tick_pos);
            if let Some(t) = self.queue.next_time() {
                n = n.min((t - self.time).min(CONTROL as u64) as usize);
            }
            n = n.min((self.arp_at - self.time).min(CONTROL as u64) as usize);
            if self.sequencer.is_some() {
                n = n.min((self.sequencer_at - self.time).min(CONTROL as u64) as usize);
            }
            if self.tick_pos == 0 && self.live_ramps > 0 {
                self.run_ramps();
            }
            let n = self.stage_input(&input, done, n);
            self.chunk(&mut left[done..done + n], &mut right[done..done + n]);
            self.time += n as u64;
            self.tick_pos = (self.tick_pos + n) % CONTROL;
            done += n;
        }
    }

    /// Let a meter fall back by one control period.
    fn fall(m: &mut [f32; 4], peak_keep: f32, rms_keep: f32) {
        m[0] *= peak_keep;
        m[1] *= peak_keep;
        m[2] *= rms_keep;
        m[3] *= rms_keep;
        for v in m.iter_mut() {
            if *v < 1.0e-9 {
                *v = 0.0;
            }
        }
    }

    /// Take some sound into a meter.
    fn measure(m: &mut [f32; 4], l: &[f32], r: &[f32], take: f32) {
        for (&a, &b) in l.iter().zip(r.iter()) {
            m[0] = m[0].max(a.abs());
            m[1] = m[1].max(b.abs());
            m[2] += a * a * take;
            m[3] += b * b * take;
        }
    }

    /// A meter as a host reads it: peaks, then RMS.
    fn levels(m: &[f32; 4]) -> [f32; 4] {
        [m[0], m[1], m[2].sqrt(), m[3].sqrt()]
    }

    /// Write out which notes each part is playing.
    fn write_notes(&mut self) {
        let mut bits = [[0u32; NOTES_SIZE]; NUM_PARTS];
        for v in self.voices.iter() {
            if !v.active() {
                continue;
            }
            // (With an arpeggio, the note it is on at this moment.)
            let note = (v.current_note() + v.arp.offset + 0.5).clamp(0.0, 127.0) as usize;
            if let Some(b) = bits.get_mut(v.part as usize) {
                let (word, bit) = (note / 32, 1u32 << (note % 32));
                if v.state == VoiceState::Held {
                    b[word] |= bit;
                }
                b[4 + word] |= bit;
            }
        }
        for (i, b) in bits.iter().enumerate() {
            self.taps.write_notes(i, b);
        }
    }

    /// Render up to `CONTROL` frames with nothing changing part way through.
    fn chunk(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len();
        let sr = self.sample_rate;
        let inv_sr = 1.0 / sr;

        if self.tick_pos == 0 {
            // The parts' free-running LFOs keep time even when nothing sounds.
            for part in self.parts.iter_mut() {
                for i in 0..2 {
                    let rate = part.p[LFO1_RATE + i * LFO_STRIDE];
                    part.lfo[i].advance(CONTROL as f32 * rate * inv_sr, &mut self.rng);
                }
            }
            // The meters fall back, whether or not their part is sounding.
            for (i, part) in self.parts.iter_mut().enumerate() {
                if part.meter != [0.0; 4] {
                    Engine::fall(&mut part.meter, self.peak_keep, self.rms_keep);
                    self.taps.write_levels(i, Engine::levels(&part.meter));
                }
            }
            if self.meter != [0.0; 4] {
                Engine::fall(&mut self.meter, self.peak_keep, self.rms_keep);
                self.taps.write_levels(NUM_PARTS, Engine::levels(&self.meter));
            }
            // Asleep: no voice has sounded and no echo has rung for a while.
            self.asleep = self.quiet >= self.sleep_after;
            self.tick_loud = false;
            if !self.asleep {
                self.notes_in = if self.notes_in == 0 { NOTES_EVERY - 1 } else { self.notes_in - 1 };
                if self.notes_in == NOTES_EVERY - 1 {
                    self.write_notes();
                }
            }
        }
        if self.asleep {
            out_l.fill(0.0);
            out_r.fill(0.0);
            self.dirty = false;
            self.taps.set_time(self.time + n as u64);
            self.record_silence(n);
            return;
        }

        let look = self.dirty || self.tick_pos == 0;
        self.dirty = false;
        let to_tick = CONTROL - self.tick_pos;
        let mut sounding = 0;
        for v in self.voices.iter_mut() {
            if !v.active() {
                continue;
            }
            let part = match self.parts.get_mut(v.part as usize) {
                Some(p) => p,
                None => {
                    v.state = VoiceState::Free;
                    continue;
                }
            };
            if look {
                v.control(&part.p, &part.lfo, &part.steps, &self.samples, sr, to_tick);
            }
            if !part.in_use {
                part.in_use = true;
                part.bus_l[..n].fill(0.0);
                part.bus_r[..n].fill(0.0);
            }
            // A voice that has finished is let go at the end of the control
            // period, not part way through, so that the moment does not
            // depend on how a host slices time.
            if v.render(&mut part.bus_l[..n], &mut part.bus_r[..n], &self.samples) || n != to_tick {
                sounding += 1;
            } else {
                part.ended[0] += v.last[0];
                part.ended[1] += v.last[1];
                v.state = VoiceState::Free;
            }
        }

        sounding += self.monitor_input(n);

        let mut mix_l = [0.0f32; CONTROL];
        let mut mix_r = [0.0f32; CONTROL];
        let mut sends = [[0.0f32; CONTROL]; 6];
        let mut any_send = [false; 3];
        let mut part_recorded = false;
        for (index, part) in self.parts.iter_mut().enumerate() {
            let clicking = part.declick[0] != 0.0 || part.declick[1] != 0.0;
            // Its voice effects may still be ringing after its voices have stopped.
            let ringing = self.vfx.live(index, self.tick_pos == 0);
            if !part.in_use && !clicking && !ringing {
                continue;
            }
            let driven = part.in_use;
            sounding += ringing as usize;
            if !part.in_use {
                part.bus_l[..n].fill(0.0);
                part.bus_r[..n].fill(0.0);
            }
            part.in_use = false;
            let bl = &mut part.bus_l[..n];
            let br = &mut part.bus_r[..n];
            if clicking {
                let [mut dl, mut dr] = part.declick;
                for i in 0..n {
                    bl[i] += dl;
                    br[i] += dr;
                    dl *= self.declick_decay;
                    dr *= self.declick_decay;
                    if dl.abs() + dr.abs() < 1.0e-5 {
                        dl = 0.0;
                        dr = 0.0;
                    }
                }
                part.declick = [dl, dr];
            }
            part.declick[0] += part.ended[0];
            part.declick[1] += part.ended[1];
            part.ended = [0.0; 2];
            // Nothing wild gets past here into the effects.
            for x in bl.iter_mut().chain(br.iter_mut()) {
                if x.is_nan() {
                    *x = 0.0;
                } else if x.abs() > 64.0 {
                    *x = x.clamp(-64.0, 64.0);
                }
            }
            let p = &part.p;
            self.vfx.process(index, p, bl, br, driven);
            if p[DRIVE] > 0.0 || p[CRUSH_BITS] < 16.0 || p[CRUSH_RATE] > 1.0 {
                part.crusher.process(bl, br, p[DRIVE], p[CRUSH_BITS], p[CRUSH_RATE] as u32);
            }
            let target = p[VOLUME];
            let mut vol = part.volume;
            if vol != target {
                for i in 0..n {
                    vol += (target - vol) * self.smooth;
                    if (vol - target).abs() < 1.0e-5 {
                        vol = target;
                    }
                    bl[i] *= vol;
                    br[i] *= vol;
                }
            } else if vol != 1.0 {
                for x in bl.iter_mut().chain(br.iter_mut()) {
                    *x *= vol;
                }
            }
            part.volume = vol;
            Engine::measure(&mut part.meter, bl, br, self.rms_take);
            self.taps.write_levels(index, Engine::levels(&part.meter));
            self.taps.write_scope(index, self.time, bl, br);
            if self.recorder.source() == Some(crate::input::Source::Part(index as u16)) {
                self.recorder.write(&mut self.samples, bl, br);
                part_recorded = true;
            }
            for i in 0..n {
                mix_l[i] += bl[i];
                mix_r[i] += br[i];
            }
            for (k, id) in [SEND_DELAY, SEND_REVERB, SEND_CHORUS].into_iter().enumerate() {
                let amount = p[id];
                if amount > 0.0 {
                    any_send[k] = true;
                    let (sl, sr2) = sends.split_at_mut(k * 2 + 1);
                    let (sl, sr2) = (&mut sl[k * 2], &mut sr2[0]);
                    for i in 0..n {
                        sl[i] += bl[i] * amount;
                        sr2[i] += br[i] * amount;
                    }
                }
            }
        }

        if !part_recorded {
            self.record_silence_of_part(n);
        }

        // The shared effects. Each keeps running after its input stops, for
        // its tail; `quiet` sends the whole engine to sleep once that is over.
        let g = &self.globals;
        let gp = |id: usize| g[id - GLOBAL_BASE];
        let ml = &mut mix_l[..n];
        let mr = &mut mix_r[..n];
        let _ = any_send;
        {
            let tone_hz = gp(DELAY_TONE);
            if self.delay_tone.0 != tone_hz {
                self.delay_tone = (tone_hz, 1.0 - (-crate::math::TAU * tone_hz * inv_sr).exp());
            }
            self.delay.process(
                &sends[0][..n],
                &sends[1][..n],
                ml,
                mr,
                gp(DELAY_TIME) * sr,
                gp(DELAY_FEEDBACK),
                self.delay_tone.1.clamp(0.0, 1.0),
                gp(DELAY_PINGPONG) >= 0.5,
            );
            let room = reverb::Settings {
                kind: gp(REVERB_TYPE) as usize,
                size: gp(REVERB_SIZE),
                damping: gp(REVERB_DAMP),
                decay: gp(REVERB_DECAY),
                predelay: gp(REVERB_PREDELAY),
                low_cut: gp(REVERB_LOW_CUT),
            };
            self.reverb.process(&sends[2][..n], &sends[3][..n], ml, mr, &room);
            self.chorus.process(&sends[4][..n], &sends[5][..n], ml, mr, gp(CHORUS_RATE), gp(CHORUS_DEPTH));
        }

        let target = gp(MASTER_VOLUME);
        let limit = gp(LIMITER) >= 0.5;
        let mut master = self.master;
        let mut peak = 0.0f32;
        for i in 0..n {
            if master != target {
                master += (target - master) * self.smooth;
                if (master - target).abs() < 1.0e-6 {
                    master = target;
                }
            }
            peak = peak.max(ml[i].abs()).max(mr[i].abs());
            let (l, r) = self.limiter.process(ml[i] * master, mr[i] * master, limit);
            out_l[i] = l;
            out_r[i] = r;
        }
        self.master = master;
        Engine::measure(&mut self.meter, out_l, out_r, self.rms_take);
        self.taps.write_levels(NUM_PARTS, Engine::levels(&self.meter));
        self.taps.write_scope(NUM_PARTS, self.time, out_l, out_r);
        if self.recorder.source() == Some(crate::input::Source::Master) {
            self.recorder.write(&mut self.samples, out_l, out_r);
        }
        self.taps.set_time(self.time + n as u64);
        // Silence is counted a whole control period at a time, so that when
        // the engine falls asleep does not depend on how a host slices time.
        if sounding > 0 || peak >= 1.0e-6 {
            self.tick_loud = true;
        }
        if self.tick_pos + n == CONTROL {
            if self.tick_loud {
                self.quiet = 0;
            } else {
                self.quiet += CONTROL as u64;
            }
        }
    }
}
