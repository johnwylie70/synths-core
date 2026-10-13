//! The engine: parts, voices, the event queue and the mix.
//!
//! `render` never allocates, locks or waits. Everything it needs is made in
//! `Engine::new`.

use crate::event::*;
use crate::fx::{Chorus, Crusher, Delay, Limiter, Reverb};
use crate::lfo::FreeLfo;
use crate::math::{hz_to_note, Rng};
use crate::params::{self, *};
use crate::sample::{Sample, LOOP_PINGPONG};
use crate::voice::{Start, Voice, VoiceState, CONTROL};

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

/// Why a sample could not be loaded or changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SampleError {
    /// The slot number is past the last slot.
    BadSlot = -1,
    /// Not 1 or 2 channels, no frames, or a sample rate that is not positive.
    BadFormat = -2,
    /// There is no sample in the slot.
    Empty = -3,
    /// A null pointer.
    Null = -4,
}

struct Part {
    p: [f32; PART_PARAMS],
    lfo: [FreeLfo; 2],
    /// The last note played, for glide.
    last_note: f32,
    has_note: bool,
    /// What is left of a voice that was cut, fading out fast so it does not click.
    declick: [f32; 2],
    /// The same for a voice that ended during this chunk: it starts with the next.
    ended: [f32; 2],
    volume: f32,
    crusher: Crusher,
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
            declick: [0.0; 2],
            ended: [0.0; 2],
            volume: p[VOLUME],
            crusher: Crusher::default(),
            bus_l: [0.0; CONTROL],
            bus_r: [0.0; CONTROL],
            in_use: false,
        }
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
            part.has_note = false;
            part.volume = part.p[VOLUME];
        }
        for r in self.ramps.iter_mut() {
            r.live = false;
        }
        self.live_ramps = 0;
        self.queue.clear();
        self.delay.clear();
        self.reverb.clear();
        self.chorus.clear();
        self.limiter.clear();
        self.master = self.globals[MASTER_VOLUME - GLOBAL_BASE];
        self.quiet = u64::MAX / 2;
        self.asleep = true;
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
        if id == VOLUME && self.part_is_silent(part) {
            self.parts[part as usize].volume = value;
        }
        if id == MASTER_VOLUME && self.active_voices() == 0 {
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
        let sr = self.sample_rate;
        let glide = self.parts[pi].p[GLIDE];
        let mode = self.parts[pi].p[MODE] as u8;
        if mode != 0 {
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
        let unison = (self.parts[pi].p[UNISON] as usize).clamp(1, 8);
        let from = if self.parts[pi].has_note { self.parts[pi].last_note } else { note };
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
        }
        self.parts[pi].last_note = note;
        self.parts[pi].has_note = true;
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
                for v in self.voices.iter_mut() {
                    if v.state == VoiceState::Held && (all || v.part == part) {
                        let hit = if e.a != 0 { v.handle == e.a } else { (v.key - e.x).abs() < 0.005 };
                        if hit {
                            v.release();
                        }
                    }
                }
            }
            EV_ALL_NOTES_OFF => {
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
            EV_PARAM => self.set_param_gliding(part, e.a as usize, e.x, e.y),
            EV_PITCH_BEND => self.set_param_gliding(part, BEND, e.x, e.y),
            EV_VOICE_PITCH | EV_VOICE_FREQ | EV_VOICE_VOLUME | EV_VOICE_PAN | EV_VOICE_POSITION | EV_VOICE_KILL => {
                if e.a == 0 || !e.x.is_finite() {
                    return;
                }
                for v in self.voices.iter_mut() {
                    if !(v.active() && v.handle == e.a && (all || v.part == part)) {
                        continue;
                    }
                    match e.kind {
                        EV_VOICE_PITCH => v.set_pitch(e.x.clamp(-24.0, 160.0), e.y, sr),
                        EV_VOICE_FREQ => {
                            if e.x > 0.0 {
                                v.set_pitch(hz_to_note(e.x).clamp(-24.0, 160.0), e.y, sr);
                            }
                        }
                        EV_VOICE_VOLUME => v.volume = e.x.clamp(0.0, 4.0),
                        EV_VOICE_PAN => v.pan = e.x.clamp(-1.0, 1.0),
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

    // ---- sound ----

    /// Render the next frames into `left` and `right` (any length; the
    /// shorter of the two is used).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        let frames = left.len().min(right.len());
        let mut done = 0;
        while done < frames {
            while let Some(t) = self.queue.next_time() {
                if t > self.time {
                    break;
                }
                if let Some(e) = self.queue.pop() {
                    self.apply(&e);
                }
            }
            let mut n = (frames - done).min(CONTROL - self.tick_pos);
            if let Some(t) = self.queue.next_time() {
                n = n.min((t - self.time).min(CONTROL as u64) as usize);
            }
            if self.tick_pos == 0 && self.live_ramps > 0 {
                self.run_ramps();
            }
            self.chunk(&mut left[done..done + n], &mut right[done..done + n]);
            self.time += n as u64;
            self.tick_pos = (self.tick_pos + n) % CONTROL;
            done += n;
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
            // Asleep: no voice has sounded and no echo has rung for a while.
            self.asleep = self.quiet >= self.sleep_after;
            self.tick_loud = false;
        }
        if self.asleep {
            out_l.fill(0.0);
            out_r.fill(0.0);
            self.dirty = false;
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
                v.control(&part.p, &part.lfo, &self.samples, sr, to_tick);
            }
            if !part.in_use {
                part.in_use = true;
                part.bus_l[..n].fill(0.0);
                part.bus_r[..n].fill(0.0);
            }
            if v.render(&mut part.bus_l[..n], &mut part.bus_r[..n], &self.samples) {
                sounding += 1;
            } else {
                part.ended[0] += v.last[0];
                part.ended[1] += v.last[1];
                v.state = VoiceState::Free;
            }
        }

        let mut mix_l = [0.0f32; CONTROL];
        let mut mix_r = [0.0f32; CONTROL];
        let mut sends = [[0.0f32; CONTROL]; 6];
        let mut any_send = [false; 3];
        for part in self.parts.iter_mut() {
            let clicking = part.declick[0] != 0.0 || part.declick[1] != 0.0;
            if !part.in_use && !clicking {
                continue;
            }
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
            self.reverb.process(&sends[2][..n], &sends[3][..n], ml, mr, gp(REVERB_SIZE), gp(REVERB_DAMP));
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
