//! FastTracker 2's rules: how an XM is played, and a MOD with more than
//! four channels, and our own songs.
//!
//! FT2 is full of behaviour nobody designed: effects that share a memory
//! by accident, tables read past their ends, checks made against the wrong
//! flag. Songs were written by ear against all of it, so all of it is kept,
//! and each odd case below says that it is one.
//!
//! How it is put together: reading a row and running a tick only change a
//! channel's numbers and mark what changed (`status`). When every channel
//! has had its turn the marks are sent to the voices in one go (`send`).
//! That order matters: it is why a global volume command does not reach the
//! channels to its left until something else touches their volume.
//!
//! Periods are FT2's own: a 16-bit number that falls as the pitch rises. In
//! linear mode 64 of them make a semitone; in Amiga mode they are four times
//! ProTracker's.
//!
//! Behaviour was checked against ft2-clone (BSD 3-Clause, Olav Sorensen)
//! and the OpenMPT test modules. Not done: FT2's MIDI vibrato.

use super::tables::{ft2_linear_period, AUTO_VIBRATO_SINE, FT2_AMIGA_PERIODS, HALF_SINE};
use super::{chord_after_row, pan_of, Arpeggio, Position, Source, Start, Visited, Voices};
use crate::song::{fx, Cell, Envelope, FrequencyTable, Instrument, Song, C4_RATE, NOTE_OFF};

// What has changed on a channel since the voices were last told.
const VOLUME: u8 = 1;
const PAN: u8 = 2;
const PERIOD: u8 = 4;
const TRIGGER: u8 = 8;
const RELEASE: u8 = 16;
/// The volume change is a sudden one (a new note, a set volume, a cut) and
/// not a step of a slide or an envelope.
const QUICK: u8 = 32;
/// The channel's chord began, changed or ended.
const CHORD: u8 = 64;

/// The highest instrument number FT2 takes notice of.
const MAX_INSTRUMENT: u8 = 128;

/// Where a channel is along one of its instrument's envelopes.
#[derive(Clone, Copy, Default)]
struct EnvelopePos {
    tick: u16,
    /// The point being headed for.
    point: u8,
    /// The envelope's height now, times 256, and how much it changes a tick.
    value: i16,
    delta: i16,
}

fn point_of(env: &Envelope, i: usize) -> (i16, i16) {
    env.points.get(i).map_or((0, 0), |&(x, y)| (x as i16, y as i16))
}

impl EnvelopePos {
    fn restart(&mut self) {
        // One before 0: the first step brings it to the first point.
        self.tick = u16::MAX;
        self.point = 0;
    }

    /// Moves one tick along the envelope and gives its height, 0 to 16384.
    fn step(&mut self, env: &Envelope, key_off: bool) -> i16 {
        let count = env.points.len();
        let mut arrived = false;
        self.tick = self.tick.wrapping_add(1);
        let mut point = self.point as usize;
        if self.tick as i32 == point_of(env, point).0 as i32 {
            // On a point: take its height exactly and aim at the next one.
            self.value = point_of(env, point).1 << 8;
            if let Some((start, end)) = env.loop_points {
                // The loop turns back at its end, unless that is also the
                // sustain point and the key has been let go.
                let released_here = key_off && env.sustain == Some(point as u8);
                if point == end as usize && !released_here {
                    point = start as usize;
                    self.tick = point_of(env, point).0 as u16;
                    self.value = point_of(env, point).1 << 8;
                }
            }
            point += 1;
            if point >= count {
                self.delta = 0;
            } else if !key_off && env.sustain == Some(point as u8 - 1) {
                // Wait here until the key is let go.
                self.delta = 0;
            } else {
                self.point = point as u8;
                let (x0, y0) = point_of(env, point - 1);
                let (x1, y1) = point_of(env, point);
                let width = x1.wrapping_sub(x0);
                if width > 0 {
                    self.delta = (((y1 - y0) as i8 as i16) << 8) / width;
                    arrived = true;
                } else {
                    self.delta = 0;
                }
            }
        }
        if arrived {
            return self.value;
        }
        self.value = self.value.wrapping_add(self.delta);
        // Rounding can carry the line a little past 0 or 64; stop it there.
        let top = (self.value >> 8) as u8;
        if top > 64 {
            self.delta = 0;
            return if top <= 160 { 64 << 8 } else { 0 };
        }
        self.value
    }

    /// Lxx: puts the envelope at a tick.
    fn jump_to(&mut self, env: &Envelope, tick: u8) {
        let count = env.points.len();
        let tick = tick as i16;
        self.tick = (tick as u16).wrapping_sub(1);
        // The stretch of the envelope the tick falls in begins at `from`.
        let from = (1..count).find(|&p| tick < point_of(env, p).0).map(|p| p - 1);
        let mut point = count.saturating_sub(1);
        let mut on_a_point = true;
        if let Some(from) = from {
            let (x0, y0) = point_of(env, from);
            let (x1, y1) = point_of(env, from + 1);
            let into = tick - x0;
            point = from;
            if into == 0 {
                // Exactly on a point: FT2 leaves the height as it was.
                on_a_point = false;
            } else if x1 - x0 > 0 {
                self.delta = (((y1 - y0) as i8 as i16) << 8) / (x1 - x0);
                self.value = (y0 << 8).wrapping_add(self.delta.wrapping_mul(into - 1));
                point = from + 1;
                on_a_point = false;
            }
        }
        if on_a_point {
            self.delta = 0;
            self.value = point_of(env, point).1 << 8;
        }
        self.point = point as u8;
    }
}

#[derive(Clone, Default)]
struct Channel {
    /// The cell being played.
    cell_note: u8,
    cell_instrument: u8,
    volume_column: u8,
    effect: u8,
    param: u8,

    /// The instrument number last seen, the instrument last started (an
    /// index into the song's; `None` for one that does not exist), and the
    /// note and sample it is playing.
    instrument_number: u8,
    instrument: Option<usize>,
    note: u8,
    sample: Option<usize>,
    relative_note: i8,
    finetune: i8,
    /// The sample's own volume and panning, for an instrument number to
    /// bring back.
    default_volume: u8,
    default_pan: u8,
    /// The channel's panning before any sample has had a say.
    channel_pan: u8,

    /// The volume slides act on, and the volume heard (tremolo and tremor
    /// change only the second).
    volume: u8,
    out_volume: u8,
    out_pan: u8,
    /// The same split for the period: slides act on `period`, vibrato and
    /// arpeggio change only `out_period`, and the instrument's own vibrato
    /// is added in `final_period`.
    period: u16,
    out_period: u16,
    final_period: u16,
    final_volume: f32,
    final_pan: u8,

    status: u8,
    /// The chord the channel is playing (our own songs only).
    chord: Option<Arpeggio>,
    /// The global volume changed after this channel had its turn; its
    /// volume is sent on the next tick.
    volume_due: bool,
    start_at: u32,
    key_off: bool,
    volume_envelope: EnvelopePos,
    pan_envelope: EnvelopePos,
    /// 32768 is full; key off starts it falling.
    fadeout: u16,
    fadeout_speed: u16,
    auto_vibrato_pos: u8,
    auto_vibrato_depth: u16,
    auto_vibrato_sweep: u16,

    porta_speed: u16,
    porta_target: u16,
    /// 0: nowhere to go; 1: the period must rise; 2: it must fall.
    porta_direction: u8,
    glissando: bool,
    vibrato_speed: u8,
    vibrato_depth: u8,
    vibrato_pos: u8,
    tremolo_speed: u8,
    tremolo_depth: u8,
    tremolo_pos: u8,
    /// Low four bits: vibrato's shape; high four: tremolo's.
    waveforms: u8,
    tremor: u8,
    tremor_pos: u8,
    retrig_speed: u8,
    retrig_volume: u8,
    retrig_count: u8,
    loop_row: u8,
    loop_count: u8,

    // What each effect did last time, for when it is given 0.
    offset_memory: u8,
    volume_slide_memory: u8,
    fine_volume_up_memory: u8,
    fine_volume_down_memory: u8,
    porta_up_memory: u8,
    porta_down_memory: u8,
    fine_porta_up_memory: u8,
    fine_porta_down_memory: u8,
    extra_fine_up_memory: u8,
    extra_fine_down_memory: u8,
    global_slide_memory: u8,
    pan_slide_memory: u8,
}

pub struct Replayer {
    channels: Vec<Channel>,
    linear: bool,
    speed: u16,
    tempo: u16,
    /// Counts down from `speed` to 1 through a row.
    counter: u16,
    global_volume: u8,
    /// The order and row that will be read next.
    order: usize,
    row: usize,
    break_row: usize,
    jump: bool,
    jump_out_of_range: bool,
    loop_back: bool,
    delay_asked: u8,
    delay_left: u8,
    pub(crate) visited: Visited,
    shown: Position,
}

impl Replayer {
    pub fn new(song: &Song) -> Replayer {
        let channels = song
            .channel_pan
            .iter()
            .map(|&pan| Channel {
                channel_pan: pan,
                default_pan: pan,
                out_pan: pan,
                final_pan: pan,
                status: VOLUME,
                ..Default::default()
            })
            .collect();
        Replayer {
            channels,
            linear: song.frequency_table == FrequencyTable::Linear,
            speed: song.speed as u16,
            tempo: song.tempo,
            counter: 1,
            global_volume: 64,
            order: 0,
            row: 0,
            break_row: 0,
            jump: false,
            jump_out_of_range: false,
            loop_back: false,
            delay_asked: 0,
            delay_left: 0,
            visited: Visited::new(song.orders.len()),
            shown: Position { speed: song.speed as u16, tempo: song.tempo, ..Default::default() },
        }
    }

    pub fn tempo(&self) -> u16 {
        self.tempo
    }

    pub fn position(&self) -> Position {
        Position {
            tick: self.speed.saturating_sub(self.counter),
            speed: self.speed,
            tempo: self.tempo,
            loops: self.visited.loops,
            ..self.shown
        }
    }

    pub fn seek(&mut self, song: &Song, order: usize, row: usize, voices: &mut dyn Voices) {
        for (c, ch) in self.channels.iter_mut().enumerate() {
            voices.stop(0, c);
            let pan = ch.channel_pan;
            *ch = Channel {
                channel_pan: pan,
                default_pan: pan,
                out_pan: pan,
                final_pan: pan,
                status: VOLUME,
                ..Default::default()
            };
        }
        self.order = order.min(song.orders.len().saturating_sub(1));
        self.row = row.min(song.rows_at(self.order) as usize - 1);
        self.counter = 1;
        self.break_row = 0;
        self.jump = false;
        self.jump_out_of_range = false;
        self.loop_back = false;
        self.delay_asked = 0;
        self.delay_left = 0;
        self.visited.clear();
        self.visited.stopped = false;
        self.shown.order = self.order as u16;
        self.shown.pattern = song.orders.get(self.order).copied().unwrap_or(0);
        self.shown.row = self.row as u16;
    }

    pub fn tick(&mut self, song: &Song, frame: u32, sample_rate: f64, voices: &mut dyn Voices) {
        if self.visited.stopped {
            return;
        }
        for ch in &mut self.channels {
            if core::mem::take(&mut ch.volume_due) {
                ch.status |= VOLUME;
            }
        }
        self.counter = self.counter.wrapping_sub(1);
        let first = self.counter == 0;
        if first {
            self.counter = self.speed;
        }
        if first && self.delay_left == 0 {
            let looping = self.channels.iter().any(|ch| ch.loop_count > 0);
            if !self.visited.visit(self.order, self.row, looping) {
                return;
            }
            self.shown.order = self.order as u16;
            self.shown.pattern = song.orders.get(self.order).copied().unwrap_or(0);
            self.shown.row = self.row as u16;
            let events = song.pattern_at(self.order).map_or(&[][..], |p| p.row(self.row as u16));
            let mut next = events.iter().peekable();
            for c in 0..self.channels.len() {
                let event = next.next_if(|e| e.channel as usize == c);
                let cell = event.map_or(Cell::default(), |e| e.cell);
                if chord_after_row(&mut self.channels[c].chord, event.and_then(|e| e.chord), cell, self.tempo) {
                    self.channels[c].status |= CHORD;
                }
                self.read_cell(song, c, cell);
                self.shape(song, c);
            }
        } else {
            for c in 0..self.channels.len() {
                self.later_tick_effects(song, c);
                self.shape(song, c);
            }
        }
        self.advance(song);
        self.send(song, frame, sample_rate, voices);
    }

    /// On a row's last tick, works out which row comes next.
    fn advance(&mut self, song: &Song) {
        if self.counter != 1 {
            return;
        }
        self.row += 1;
        if self.delay_asked > 0 {
            self.delay_left = self.delay_asked;
            self.delay_asked = 0;
        }
        if self.delay_left > 0 {
            self.delay_left -= 1;
            if self.delay_left > 0 {
                self.row -= 1;
            }
        }
        if self.loop_back {
            // The row to go back to is left in `break_row`, so the next
            // pattern starts there too: FT2's pattern loop bug, which songs
            // use to start a pattern part-way down.
            self.loop_back = false;
            self.row = self.break_row;
        }
        if self.row >= song.rows_at(self.order) as usize || self.jump {
            self.row = self.break_row;
            self.break_row = 0;
            self.jump = false;
            if self.jump_out_of_range {
                self.jump_out_of_range = false;
                self.order = 0;
            } else {
                self.order += 1;
                if self.order >= song.orders.len() {
                    self.order = song.restart as usize;
                    self.visited.wrapped = true;
                }
            }
            // The row may be past the end of the new pattern. FT2 plays it
            // anyway (it is empty here) and moves on after it.
        }
    }

    /// Tells the voices what changed on each channel this tick.
    fn send(&mut self, song: &Song, frame: u32, sample_rate: f64, voices: &mut dyn Voices) {
        // A sudden change takes 5 milliseconds; any other glides for the tick.
        let quick = (sample_rate / 200.0) as u32;
        let slow = (sample_rate * 2.5 / self.tempo.max(1) as f64) as u32;
        for (c, ch) in self.channels.iter_mut().enumerate() {
            let status = core::mem::take(&mut ch.status);
            if status == 0 {
                continue;
            }
            let rate = rate_of(ch.final_period, self.linear);
            let pan = pan_of(ch.final_pan);
            if status & TRIGGER != 0 {
                let patch = ch.instrument.filter(|&i| song.instruments[i].patch.is_some());
                let source = match (patch, ch.sample) {
                    (Some(i), _) => Some(Source::Patch(i)),
                    // A start at or past the end of the sample plays nothing.
                    (None, Some(s)) if ch.start_at < song.samples[s].play_end() => Some(Source::Sample(s)),
                    _ => None,
                };
                match source {
                    Some(source) => {
                        let start = Start { source, offset: ch.start_at, rate, volume: ch.final_volume, pan };
                        voices.play(frame, c, &start);
                        // A new voice starts without a chord: give it the channel's.
                        if ch.chord.is_some() {
                            voices.set_arpeggio(frame, c, ch.chord.as_ref());
                        }
                    }
                    None => voices.stop(frame, c),
                }
            } else {
                if status & VOLUME != 0 {
                    voices.set_volume(frame, c, ch.final_volume, if status & QUICK != 0 { quick } else { slow });
                }
                if status & PAN != 0 {
                    voices.set_pan(frame, c, pan, if status & QUICK != 0 { quick } else { slow });
                }
                if status & PERIOD != 0 {
                    voices.set_rate(frame, c, rate);
                }
            }
            if status & CHORD != 0 && !(status & TRIGGER != 0 && ch.chord.is_some()) {
                voices.set_arpeggio(frame, c, ch.chord.as_ref());
            }
            if status & RELEASE != 0 {
                voices.release(frame, c);
            }
        }
    }

    fn period_at(&self, index: usize) -> u16 {
        if self.linear {
            ft2_linear_period(index)
        } else {
            FT2_AMIGA_PERIODS[index.min(FT2_AMIGA_PERIODS.len() - 1)]
        }
    }

    /// The period of the semitone a period is in, moved up by `semitones`
    /// (for arpeggio and glissando). FT2 only looks through the first eight
    /// octaves, and stops just short of their top.
    fn semitone_period(&self, period: u16, semitones: u8, finetune: i8) -> u16 {
        let fine = (finetune >> 3) as i32 + 16;
        let reached = |note: i32| period >= self.period_at((note * 16 + fine - 8).max(0) as usize);
        let note = (1..96).find(|&n| reached(n)).unwrap_or(96) - 1;
        let mut index = note * 16 + fine + semitones as i32 * 16;
        if index >= 8 * 12 * 16 + 14 {
            index = 8 * 12 * 16 + 15;
        }
        self.period_at(index as usize)
    }

    /// Reads a channel's cell at the start of a row.
    fn read_cell(&mut self, song: &Song, c: usize, cell: Cell) {
        let ch = &mut self.channels[c];
        ch.volume_column = cell.volume;
        // Arpeggio and vibrato leave the pitch wherever they got to. Put it
        // back, unless the vibrato carries on.
        let was_vibrato = matches!(ch.effect, fx::VIBRATO | fx::VIBRATO_VOL_SLIDE);
        let is_vibrato = matches!(cell.effect, fx::VIBRATO | fx::VIBRATO_VOL_SLIDE);
        if (ch.effect == 0 && ch.param != 0) || (was_vibrato && !is_vibrato) {
            ch.out_period = ch.period;
            ch.status |= PERIOD;
        }
        ch.effect = cell.effect;
        ch.param = cell.param;
        ch.cell_note = cell.note;
        ch.cell_instrument = cell.instrument;

        let mut instrument = cell.instrument;
        if instrument > MAX_INSTRUMENT {
            instrument = 0;
        } else if instrument > 0 {
            ch.instrument_number = instrument;
        }

        let extended = cell.effect == fx::EXTENDED;
        if extended && (0xD1..=0xDF).contains(&cell.param) {
            // A delayed note: nothing happens until its tick.
            return;
        }
        // E90 goes straight to starting the note, skipping the cases below.
        if !(extended && cell.param == 0x90) {
            let column_porta = cell.volume & 0xF0 == 0xF0;
            if column_porta || matches!(cell.effect, fx::TONE_PORTA | fx::TONE_PORTA_VOL_SLIDE) {
                if column_porta {
                    if cell.volume & 0x0F > 0 {
                        ch.porta_speed = (cell.volume & 0x0F) as u16 * 64;
                    }
                } else if cell.effect == fx::TONE_PORTA && cell.param != 0 {
                    ch.porta_speed = cell.param as u16 * 4;
                }
                // The note is where to slide to, not a note to start.
                self.aim_portamento(song, c, cell.note);
                if instrument > 0 {
                    self.restore_volume(c);
                    if cell.note != NOTE_OFF {
                        self.start_instrument(song, c);
                    }
                }
                self.first_tick_effects(song, c);
                return;
            }
            if cell.effect == fx::KEY_OFF && cell.param == 0 {
                // K00 lets the key go at once, and any note beside it is lost.
                self.key_off(song, c);
                if instrument > 0 {
                    self.restore_volume(c);
                }
                self.first_tick_effects(song, c);
                return;
            }
            if cell.note == 0 {
                if instrument > 0 {
                    self.restore_volume(c);
                    self.start_instrument(song, c);
                }
                self.first_tick_effects(song, c);
                return;
            }
        }
        if cell.note == NOTE_OFF {
            self.key_off(song, c);
        } else {
            self.start_note(song, c, cell.note, cell.effect, cell.param);
        }
        if instrument > 0 {
            self.restore_volume(c);
            if cell.note != NOTE_OFF {
                self.start_instrument(song, c);
            }
        }
        self.first_tick_effects(song, c);
    }

    /// Starts a note: picks its sample and works out its period. A note of
    /// 0 means the channel's last note again.
    fn start_note(&mut self, song: &Song, c: usize, note: u8, effect: u8, param: u8) {
        if note == NOTE_OFF {
            self.key_off(song, c);
            return;
        }
        let ch = &mut self.channels[c];
        let note = if note == 0 { ch.note } else { note };
        if note == 0 {
            return;
        }
        ch.note = note;
        let instrument = (ch.instrument_number as usize).checked_sub(1).filter(|&i| playable(song.instruments.get(i)));
        ch.instrument = instrument;
        let sample = instrument.and_then(|i| song.instruments[i].sample_for(note));
        ch.sample = sample;
        // An instrument that is not there plays nothing at volume 0; a key
        // with no sample in an instrument that is plays nothing at full volume.
        let (relative, volume, pan, finetune) = match sample.map(|s| &song.samples[s]) {
            Some(s) => (s.relative_note, s.volume, s.pan.unwrap_or(ch.channel_pan), s.finetune),
            None => (0, if instrument.is_some() { 64 } else { 0 }, ch.channel_pan, 0),
        };
        ch.relative_note = relative;
        // The note the sample really plays; outside ten octaves it is dropped.
        let real = (note as i8).wrapping_add(relative) as u8;
        if real >= 120 {
            return;
        }
        ch.default_volume = volume;
        ch.default_pan = pan;
        ch.finetune = if effect == fx::EXTENDED && param & 0xF0 == 0x50 {
            ((param & 0x0F) as i16 * 16 - 128) as i8
        } else {
            finetune
        };
        if real != 0 {
            let index = (real as usize - 1) * 16 + ((ch.finetune >> 3) as i32 + 16) as usize;
            let period = self.period_at(index);
            let ch = &mut self.channels[c];
            ch.period = period;
            ch.out_period = period;
        }
        let ch = &mut self.channels[c];
        ch.status |= PERIOD | VOLUME | PAN | TRIGGER;
        if effect == fx::SAMPLE_OFFSET {
            if param > 0 {
                ch.offset_memory = ch.param;
            }
            ch.start_at = (ch.offset_memory as u32) << 8;
        } else {
            ch.start_at = 0;
        }
    }

    /// An instrument number: the sample's own volume and panning come back.
    fn restore_volume(&mut self, c: usize) {
        let ch = &mut self.channels[c];
        ch.volume = ch.default_volume;
        ch.out_volume = ch.default_volume;
        ch.out_pan = ch.default_pan;
        ch.status |= VOLUME | PAN | QUICK;
    }

    /// Starts the instrument's part of a note again: envelopes, fade and
    /// its vibrato go back to the beginning.
    fn start_instrument(&mut self, song: &Song, c: usize) {
        let ch = &mut self.channels[c];
        if ch.waveforms & 0x04 == 0 {
            ch.vibrato_pos = 0;
        }
        if ch.waveforms & 0x40 == 0 {
            ch.tremolo_pos = 0;
        }
        ch.retrig_count = 0;
        ch.tremor_pos = 0;
        ch.key_off = false;
        let Some(ins) = ch.instrument.map(|i| &song.instruments[i]) else {
            ch.fadeout_speed = 0;
            ch.fadeout = 32768;
            return;
        };
        if ins.volume_envelope.enabled {
            ch.volume_envelope.restart();
        }
        if ins.pan_envelope.enabled {
            ch.pan_envelope.restart();
        }
        ch.fadeout_speed = ins.fadeout;
        ch.fadeout = 32768;
        if ins.vibrato.depth > 0 {
            ch.auto_vibrato_pos = 0;
            if ins.vibrato.sweep > 0 {
                ch.auto_vibrato_depth = 0;
                ch.auto_vibrato_sweep = ((ins.vibrato.depth as u16) << 8) / ins.vibrato.sweep as u16;
            } else {
                ch.auto_vibrato_depth = (ins.vibrato.depth as u16) << 8;
                ch.auto_vibrato_sweep = 0;
            }
        }
    }

    fn key_off(&mut self, song: &Song, c: usize) {
        let ch = &mut self.channels[c];
        ch.key_off = true;
        ch.status |= RELEASE;
        let ins = ch.instrument.map(|i| &song.instruments[i]);
        let none = Envelope::default();
        let volume_envelope = ins.map_or(&none, |i| &i.volume_envelope);
        let pan_envelope = ins.map_or(&none, |i| &i.pan_envelope);
        if volume_envelope.enabled {
            // If the envelope was waiting past its point, step it back so
            // that it meets the point again and moves on from it.
            let x = point_of(volume_envelope, ch.volume_envelope.point as usize).0 as u16;
            if ch.volume_envelope.tick >= x {
                ch.volume_envelope.tick = x.wrapping_sub(1);
            }
        } else {
            // With no envelope, letting go is silence at once.
            ch.volume = 0;
            ch.out_volume = 0;
            ch.status |= VOLUME | QUICK;
        }
        // FT2 does this when the panning envelope is off, which is the wrong
        // way round; the test modules need it kept.
        if !pan_envelope.enabled {
            let x = point_of(pan_envelope, ch.pan_envelope.point as usize).0 as u16;
            if ch.pan_envelope.tick >= x {
                ch.pan_envelope.tick = x.wrapping_sub(1);
            }
        }
    }

    /// A note beside a tone portamento sets where the slide is going.
    fn aim_portamento(&mut self, song: &Song, c: usize, note: u8) {
        if note == 0 {
            return;
        }
        if note == NOTE_OFF {
            self.key_off(song, c);
            return;
        }
        let ch = &self.channels[c];
        let index = (note as i32 - 1 + ch.relative_note as i32) * 16 + (ch.finetune >> 3) as i32 + 16;
        if !(0..(10 * 12 * 16 + 16)).contains(&index) {
            return;
        }
        let target = self.period_at(index as usize);
        let ch = &mut self.channels[c];
        ch.porta_target = target;
        ch.porta_direction = match target.cmp(&ch.period) {
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
            core::cmp::Ordering::Less => 2,
        };
    }

    /// The volume column and effects that act once, when the row is read.
    fn first_tick_effects(&mut self, song: &Song, c: usize) {
        let ch = &mut self.channels[c];
        // What the volume column came to; multi retrig looks at it.
        let mut column = ch.volume_column;
        let low = ch.volume_column & 0x0F;
        match ch.volume_column >> 4 {
            1..=5 => {
                column = (column - 16).min(64);
                ch.volume = column;
                ch.out_volume = column;
                ch.status |= VOLUME | QUICK;
            }
            0x8 => {
                column = ch.volume.saturating_sub(low);
                ch.volume = column;
                ch.out_volume = column;
                ch.status |= VOLUME;
            }
            0x9 => {
                column = (ch.volume + low).min(64);
                ch.volume = column;
                ch.out_volume = column;
                ch.status |= VOLUME;
            }
            0xA => {
                column = low * 4;
                if column != 0 {
                    ch.vibrato_speed = column;
                }
            }
            0xC => {
                column = low << 4;
                ch.out_pan = column;
                ch.status |= PAN;
            }
            _ => {}
        }

        let (effect, param) = (ch.effect, ch.param);
        if effect == 0 && param == 0 {
            return;
        }
        match effect {
            fx::SET_PAN => {
                ch.out_pan = param;
                ch.status |= PAN;
            }
            fx::SET_VOLUME => {
                ch.volume = param.min(64);
                ch.out_volume = ch.volume;
                ch.status |= VOLUME | QUICK;
            }
            fx::MULTI_RETRIG => {
                if param & 0x0F != 0 {
                    ch.retrig_speed = param & 0x0F;
                }
                if param >> 4 != 0 {
                    ch.retrig_volume = param >> 4;
                }
                // Only when the volume column came to 0, which is how a
                // volume beside Rxy puts off the first retrigger.
                if column == 0 {
                    self.multi_retrig(song, c);
                }
            }
            fx::EXTRA_FINE_PORTA => match param >> 4 {
                1 => {
                    let by = recall(&mut ch.extra_fine_up_memory, param & 0x0F);
                    slide_period_up(ch, by as u16);
                }
                2 => {
                    let by = recall(&mut ch.extra_fine_down_memory, param & 0x0F);
                    slide_period_down(ch, by as u16);
                }
                _ => {}
            },
            fx::POSITION_JUMP => {
                // A jump before the start or past the end goes to the start.
                match (param as usize).checked_sub(1) {
                    Some(order) if order < song.orders.len() => self.order = order,
                    _ => self.jump_out_of_range = true,
                }
                self.break_row = 0;
                self.jump = true;
            }
            fx::PATTERN_BREAK => {
                let row = (param >> 4) as usize * 10 + (param & 0x0F) as usize;
                self.break_row = if row <= 63 { row } else { 0 };
                self.jump = true;
            }
            fx::EXTENDED => self.first_tick_extended(c, param >> 4, param & 0x0F),
            fx::SPEED => {
                if param >= 32 {
                    self.tempo = param as u16;
                } else {
                    // Speed 0 stops FT2 moving on; call that the song's end.
                    if param == 0 {
                        self.visited.end();
                    }
                    self.speed = param as u16;
                    self.counter = param as u16;
                }
            }
            fx::GLOBAL_VOLUME => {
                self.global_volume = param.min(64);
                self.global_volume_changed(c);
            }
            fx::ENVELOPE_POS => {
                let Some(ins) = ch.instrument.map(|i| &song.instruments[i]) else { return };
                if ins.volume_envelope.enabled {
                    ch.volume_envelope.jump_to(&ins.volume_envelope, param);
                }
                // FT2 means to ask whether the panning envelope has a sustain
                // point and asks the volume envelope: another slip to keep.
                if ins.volume_envelope.sustain.is_some() {
                    ch.pan_envelope.jump_to(&ins.pan_envelope, param);
                }
            }
            _ => {}
        }
    }

    fn first_tick_extended(&mut self, c: usize, command: u8, x: u8) {
        let ch = &mut self.channels[c];
        match command {
            0x1 => {
                let by = recall(&mut ch.fine_porta_up_memory, x);
                slide_period_up(ch, by as u16 * 4);
            }
            0x2 => {
                let by = recall(&mut ch.fine_porta_down_memory, x);
                slide_period_down(ch, by as u16 * 4);
            }
            0x3 => ch.glissando = x != 0,
            0x4 => ch.waveforms = (ch.waveforms & 0xF0) | x,
            0x6 => {
                if x == 0 {
                    ch.loop_row = self.row as u8;
                    return;
                }
                if ch.loop_count == 0 {
                    ch.loop_count = x;
                } else {
                    ch.loop_count -= 1;
                    if ch.loop_count == 0 {
                        return;
                    }
                }
                self.break_row = ch.loop_row as usize;
                self.loop_back = true;
                self.visited.forget(self.order, self.break_row, self.row);
            }
            0x7 => ch.waveforms = (x << 4) | (ch.waveforms & 0x0F),
            0xA => {
                let by = recall(&mut ch.fine_volume_up_memory, x);
                ch.volume = (ch.volume + by).min(64);
                ch.out_volume = ch.volume;
                ch.status |= VOLUME;
            }
            0xB => {
                let by = recall(&mut ch.fine_volume_down_memory, x);
                ch.volume = ch.volume.saturating_sub(by);
                ch.out_volume = ch.volume;
                ch.status |= VOLUME;
            }
            0xC if x == 0 => {
                ch.volume = 0;
                ch.out_volume = 0;
                ch.status |= VOLUME | QUICK;
            }
            0xE if self.delay_left == 0 => self.delay_asked = x + 1,
            _ => {}
        }
    }

    /// The volume column and effects that act on every tick after a row's
    /// first.
    fn later_tick_effects(&mut self, song: &Song, c: usize) {
        // How far into the row this tick is.
        let tick = self.speed.wrapping_sub(self.counter) as u8;
        let counter = self.counter;
        let ch = &mut self.channels[c];
        let low = ch.volume_column & 0x0F;
        match ch.volume_column >> 4 {
            0x6 => {
                ch.volume = ch.volume.saturating_sub(low);
                ch.out_volume = ch.volume;
                ch.status |= VOLUME;
            }
            0x7 => {
                ch.volume = (ch.volume + low).min(64);
                ch.out_volume = ch.volume;
                ch.status |= VOLUME;
            }
            0xB => {
                if low > 0 {
                    ch.vibrato_depth = low;
                }
                vibrato(ch);
            }
            0xD => {
                // A slide left by 0 throws the panning hard left: the sum
                // never carries, and FT2 takes that for having gone below 0.
                ch.out_pan = if low == 0 { 0 } else { ch.out_pan.saturating_sub(low) };
                ch.status |= PAN;
            }
            0xE => {
                ch.out_pan = ch.out_pan.saturating_add(low);
                ch.status |= PAN;
            }
            0xF => self.tone_portamento(c),
            _ => {}
        }

        let ch = &mut self.channels[c];
        let (effect, param) = (ch.effect, ch.param);
        if effect == 0 && param == 0 {
            return;
        }
        match effect {
            fx::ARPEGGIO => {
                // FT2 looks the step up in a 16-entry table with a tick that
                // can be 31; past the end it reads the vibrato table, which
                // is all "third note" but for one "first note".
                let at = counter & 31;
                let step = match at {
                    0..=15 => at % 3,
                    16 => 0,
                    _ => 2,
                };
                let (base, finetune) = (ch.period, ch.finetune);
                let period = match step {
                    0 => base,
                    1 => self.semitone_period(base, param >> 4, finetune),
                    _ => self.semitone_period(base, param & 0x0F, finetune),
                };
                let ch = &mut self.channels[c];
                ch.out_period = period;
                ch.status |= PERIOD;
            }
            fx::PORTA_UP => {
                let by = recall(&mut ch.porta_up_memory, param);
                slide_period_up(ch, by as u16 * 4);
            }
            fx::PORTA_DOWN => {
                let by = recall(&mut ch.porta_down_memory, param);
                slide_period_down(ch, by as u16 * 4);
            }
            fx::TONE_PORTA => self.tone_portamento(c),
            fx::VIBRATO => {
                if param & 0x0F > 0 {
                    ch.vibrato_depth = param & 0x0F;
                }
                if param & 0xF0 > 0 {
                    ch.vibrato_speed = (param & 0xF0) >> 2;
                }
                vibrato(ch);
            }
            fx::TONE_PORTA_VOL_SLIDE => {
                self.tone_portamento(c);
                volume_slide(&mut self.channels[c], param);
            }
            fx::VIBRATO_VOL_SLIDE => {
                vibrato(ch);
                volume_slide(ch, param);
            }
            fx::TREMOLO => tremolo(ch, param),
            fx::VOL_SLIDE => volume_slide(ch, param),
            fx::EXTENDED => {
                let x = param & 0x0F;
                match param >> 4 {
                    0x9 if x != 0 && tick % x == 0 => {
                        self.start_note(song, c, 0, 0, 0);
                        self.start_instrument(song, c);
                    }
                    0xC if tick == x => {
                        ch.volume = 0;
                        ch.out_volume = 0;
                        ch.status |= VOLUME | QUICK;
                    }
                    0xD if tick == x => self.delayed_note(song, c),
                    _ => {}
                }
            }
            fx::GLOBAL_VOL_SLIDE => {
                let by = recall(&mut ch.global_slide_memory, param);
                self.global_volume = if by & 0xF0 == 0 {
                    self.global_volume.saturating_sub(by)
                } else {
                    (self.global_volume + (by >> 4)).min(64)
                };
                self.global_volume_changed(c);
            }
            fx::KEY_OFF if tick == param & 31 => self.key_off(song, c),
            fx::PAN_SLIDE => {
                let by = recall(&mut ch.pan_slide_memory, param);
                ch.out_pan = if by & 0xF0 == 0 { ch.out_pan.saturating_sub(by) } else { ch.out_pan.saturating_add(by >> 4) };
                ch.status |= PAN;
            }
            fx::MULTI_RETRIG => self.multi_retrig(song, c),
            fx::TREMOR => {
                let param = recall(&mut ch.tremor, param);
                // The low seven bits count the ticks left in this phase; the
                // top bit says whether it is the sounding one.
                let mut sounding = ch.tremor_pos & 0x80 != 0;
                let mut left = (ch.tremor_pos & 0x7F).wrapping_sub(1);
                if left & 0x80 != 0 {
                    sounding = !sounding;
                    left = if sounding { param >> 4 } else { param & 0x0F };
                }
                ch.tremor_pos = left | if sounding { 0x80 } else { 0 };
                ch.out_volume = if sounding { ch.volume } else { 0 };
                ch.status |= VOLUME | QUICK;
            }
            _ => {}
        }
    }

    /// The global volume was changed by channel `c`. That channel and those
    /// after it have yet to work out their volume this tick, and will use
    /// the new one. Those before it already have: they stay as they are
    /// until the next tick. So every note on a row can be at a different
    /// global volume, and songs lean on it.
    fn global_volume_changed(&mut self, c: usize) {
        for (i, ch) in self.channels.iter_mut().enumerate() {
            if i < c {
                ch.volume_due = true;
            } else {
                ch.status |= VOLUME;
            }
        }
    }

    /// 3xx, 5xy and the volume column's Mx: slides the period to its target.
    fn tone_portamento(&mut self, c: usize) {
        let ch = &mut self.channels[c];
        match ch.porta_direction {
            0 => return,
            1 => {
                ch.period = ch.period.wrapping_add(ch.porta_speed);
                if ch.period >= ch.porta_target {
                    ch.period = ch.porta_target;
                }
            }
            _ => {
                ch.period = ch.period.wrapping_sub(ch.porta_speed);
                if ch.period as i16 <= ch.porta_target as i16 {
                    // Arriving from above leaves the direction pointing the
                    // other way; a later slide notices and stays put.
                    ch.porta_direction = 1;
                    ch.period = ch.porta_target;
                }
            }
        }
        let (period, glissando, finetune) = (ch.period, ch.glissando, ch.finetune);
        let period = if glissando { self.semitone_period(period, 0, finetune) } else { period };
        let ch = &mut self.channels[c];
        ch.out_period = period;
        ch.status |= PERIOD;
    }

    /// Rxy: starts the note again every so many ticks, changing its volume
    /// each time.
    fn multi_retrig(&mut self, song: &Song, c: usize) {
        let ch = &mut self.channels[c];
        let count = ch.retrig_count + 1;
        if count < ch.retrig_speed {
            ch.retrig_count = count;
            return;
        }
        ch.retrig_count = 0;
        let v = ch.volume as i16;
        let v = match ch.retrig_volume {
            0x1 => v - 1,
            0x2 => v - 2,
            0x3 => v - 4,
            0x4 => v - 8,
            0x5 => v - 16,
            0x6 => (v >> 1) + (v >> 3) + (v >> 4),
            0x7 => v >> 1,
            0x9 => v + 1,
            0xA => v + 2,
            0xB => v + 4,
            0xC => v + 8,
            0xD => v + 16,
            0xE => (v >> 1) + v,
            0xF => v + v,
            _ => v,
        };
        ch.volume = v.clamp(0, 64) as u8;
        ch.out_volume = ch.volume;
        self.apply_column_volume_or_pan(c);
        self.start_note(song, c, 0, 0, 0);
    }

    /// A volume or a panning in the volume column, applied again when a
    /// note is restarted mid-row.
    fn apply_column_volume_or_pan(&mut self, c: usize) {
        let ch = &mut self.channels[c];
        match ch.volume_column {
            0x10..=0x50 => {
                ch.volume = ch.volume_column - 0x10;
                ch.out_volume = ch.volume;
            }
            0xC0..=0xCF => ch.out_pan = (ch.volume_column & 0x0F) << 4,
            _ => {}
        }
    }

    /// EDx: the row's note, started now. With no note in the row the last
    /// note starts again, and a key off beside a delay restarts the
    /// envelopes and nothing else.
    fn delayed_note(&mut self, song: &Song, c: usize) {
        let (note, instrument) = (self.channels[c].cell_note, self.channels[c].cell_instrument);
        self.start_note(song, c, note, 0, 0);
        if instrument > 0 {
            self.restore_volume(c);
        }
        self.start_instrument(song, c);
        self.apply_column_volume_or_pan(c);
    }

    /// Shapes the channel's sound for this tick: fade, envelopes and the
    /// instrument's own vibrato turn its volume, panning and period into
    /// what is heard.
    fn shape(&mut self, song: &Song, c: usize) {
        let global = self.global_volume as u32;
        let ch = &mut self.channels[c];
        let ins: Option<&Instrument> = ch.instrument.map(|i| &song.instruments[i]);

        if ch.key_off {
            if ch.fadeout_speed > ch.fadeout {
                ch.fadeout = 0;
                ch.fadeout_speed = 0;
            } else {
                ch.fadeout -= ch.fadeout_speed;
            }
            ch.status |= VOLUME;
        }

        let mut volume = (global * ch.out_volume as u32 * ch.fadeout as u32) as f32 / (64.0 * 64.0 * 32768.0);
        if let Some(env) = ins.map(|i| &i.volume_envelope).filter(|e| e.enabled) {
            let height = ch.volume_envelope.step(env, ch.key_off) as u16;
            volume *= (height as f32 / 16384.0).min(1.0);
            ch.status |= VOLUME;
        }
        ch.final_volume = volume.min(1.0);

        if let Some(env) = ins.map(|i| &i.pan_envelope).filter(|e| e.enabled) {
            let height = ch.pan_envelope.step(env, ch.key_off) as i32 - (32 << 8);
            // The envelope has most room to move the panning at the centre,
            // and none at the edges.
            let room = (128 - (ch.out_pan as i32 - 128).abs()) << 3;
            ch.final_pan = ch.out_pan.wrapping_add(((height * room) >> 16) as u8);
            ch.status |= PAN;
        } else {
            ch.final_pan = ch.out_pan;
        }

        let vibrato = ins.map(|i| i.vibrato).filter(|v| v.depth > 0);
        if let Some(vibrato) = vibrato {
            let depth = if ch.auto_vibrato_sweep > 0 {
                // The wobble grows to its full depth, but only while the key
                // is held: let go early and it drops back to one step's worth.
                let mut depth = ch.auto_vibrato_sweep;
                if !ch.key_off {
                    depth = depth.wrapping_add(ch.auto_vibrato_depth);
                    if depth >> 8 > vibrato.depth as u16 {
                        depth = (vibrato.depth as u16) << 8;
                        ch.auto_vibrato_sweep = 0;
                    }
                    ch.auto_vibrato_depth = depth;
                }
                depth
            } else {
                ch.auto_vibrato_depth
            };
            ch.auto_vibrato_pos = ch.auto_vibrato_pos.wrapping_add(vibrato.rate);
            let pos = ch.auto_vibrato_pos as i32;
            let wave = match vibrato.waveform {
                1 => {
                    if pos > 127 {
                        64
                    } else {
                        -64
                    }
                }
                2 => (((pos >> 1) + 64) & 127) - 64,
                3 => ((64 - (pos >> 1)) & 127) - 64,
                _ => AUTO_VIBRATO_SINE[pos as usize] as i32,
            };
            // FT2 multiplies by the depth as a signed 16-bit number.
            let bend = ((wave * depth as i16 as i32) >> 14) as i16;
            let period = ch.out_period.wrapping_add(bend as u16);
            ch.final_period = if period >= 32000 { 0 } else { period };
            ch.status |= PERIOD;
        } else {
            ch.final_period = ch.out_period;
        }
    }
}

/// An instrument with something to play: samples, or a synth patch.
fn playable(instrument: Option<&Instrument>) -> bool {
    instrument.is_some_and(|i| !i.keymap.is_empty() || i.patch.is_some())
}

/// How many frames of a sample a second a period plays.
fn rate_of(period: u16, linear: bool) -> f64 {
    if period == 0 {
        return 0.0;
    }
    if !linear {
        return C4_RATE * 1712.0 / period as f64;
    }
    // 768 periods to the octave, counted down from 9216 (twelve octaves).
    // FT2 works the octave out as a shift of at most 31 places, so a period
    // past the top wraps round to a very low note instead of a very high one.
    let up = 9216u16.wrapping_sub(period) as u32;
    let (octave, within) = (up / 768, up % 768);
    let shift = 14u32.wrapping_sub(octave) & 31;
    C4_RATE * 256.0 * (within as f64 / 768.0).exp2() / (1u64 << shift) as f64
}

/// An effect given 0 does what it did last time.
fn recall(memory: &mut u8, param: u8) -> u8 {
    if param != 0 {
        *memory = param;
    }
    *memory
}

/// Raises the pitch. The floor is a period of 1.
fn slide_period_up(ch: &mut Channel, by: u16) {
    ch.period = ch.period.wrapping_sub(by);
    if (ch.period as i16) < 1 {
        ch.period = 1;
    }
    ch.out_period = ch.period;
    ch.status |= PERIOD;
}

/// Lowers the pitch. FT2 checks the ceiling as a signed number, so a
/// period past 32767 is not caught and carries on round.
fn slide_period_down(ch: &mut Channel, by: u16) {
    ch.period = ch.period.wrapping_add(by);
    if ch.period as i16 >= 32000 {
        ch.period = 32000 - 1;
    }
    ch.out_period = ch.period;
    ch.status |= PERIOD;
}

/// The height of a vibrato or tremolo wave, 0 to 255, at a position.
fn wave(shape: u8, pos: u8, flip_ramp: bool) -> u16 {
    let step = (pos >> 2) & 31;
    match shape & 3 {
        0 => HALF_SINE[step as usize] as u16,
        1 if flip_ramp => (!(step << 3)) as u16,
        1 => (step << 3) as u16,
        _ => 255,
    }
}

fn vibrato(ch: &mut Channel) {
    let second_half = ch.vibrato_pos >= 128;
    let depth = ((wave(ch.waveforms, ch.vibrato_pos, second_half) * ch.vibrato_depth as u16) >> 5) as u8 as u16;
    ch.out_period = if second_half { ch.period.wrapping_sub(depth) } else { ch.period.wrapping_add(depth) };
    ch.status |= PERIOD;
    ch.vibrato_pos = ch.vibrato_pos.wrapping_add(ch.vibrato_speed);
}

fn tremolo(ch: &mut Channel, param: u8) {
    if param & 0x0F > 0 {
        ch.tremolo_depth = param & 0x0F;
    }
    if param & 0xF0 > 0 {
        ch.tremolo_speed = (param & 0xF0) >> 2;
    }
    // The ramp shape checks the vibrato's position for which half it is in,
    // not the tremolo's: a slip FT2 shares with ProTracker.
    let height = wave(ch.waveforms >> 4, ch.tremolo_pos, ch.vibrato_pos >= 128);
    let depth = ((height * ch.tremolo_depth as u16) >> 6) as u8;
    ch.out_volume = if ch.tremolo_pos >= 128 { ch.volume.saturating_sub(depth) } else { (ch.volume + depth).min(64) };
    ch.status |= VOLUME;
    ch.tremolo_pos = ch.tremolo_pos.wrapping_add(ch.tremolo_speed);
}

fn volume_slide(ch: &mut Channel, param: u8) {
    let by = recall(&mut ch.volume_slide_memory, param);
    ch.volume = if by & 0xF0 == 0 { ch.volume.saturating_sub(by) } else { (ch.volume + (by >> 4)).min(64) };
    ch.out_volume = ch.volume;
    ch.status |= VOLUME;
}
