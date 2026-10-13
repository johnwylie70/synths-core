//! ProTracker's rules: how a 4-channel Amiga MOD is played.
//!
//! This follows what ProTracker 2.3's replay routine does, tick by tick,
//! including the things it does by accident that songs came to rely on. The
//! Amiga's sound chip (Paula) is why some of it is odd: the routine writes a
//! period and a volume to the chip's registers, and what the chip has last
//! been told is what sounds. So "the period" below is two things: the one a
//! channel remembers and the one it last sent.
//!
//! Behaviour was checked against pt2-clone (BSD 3-Clause, Olav Sorensen) and
//! the OpenMPT test modules. Not done: EFx (invert loop), which rewrites
//! the sample as it plays, and E8x.

use super::tables::{HALF_SINE, PT_PERIOD_TABLE, PT_ROW};
use super::{chord_after_row, pan_of, Arpeggio, Position, Source, Start, Visited, Voices};
use crate::song::{fx, Cell, Song, NOTE_MAX};

/// Half the PAL Amiga's clock: a period of 1 would play this many frames a
/// second.
const PAULA_CLOCK: f64 = 3_546_894.6;

const MIN_PERIOD: u16 = 113;
const MAX_PERIOD: u16 = 856;

/// What a period sounds like. Paula takes 0 to mean 65536.
fn rate_of(period: u16) -> f64 {
    PAULA_CLOCK / if period == 0 { 65536.0 } else { period as f64 }
}

/// One finetune's row of the period table. It runs on past its own end into
/// the next row, as it did in ProTracker's memory.
fn row_of(finetune: u8) -> &'static [u16] {
    &PT_PERIOD_TABLE[(finetune & 15) as usize * PT_ROW..]
}

/// The first entry in a finetune's row that a period reaches: the note it
/// is, or the next one up if it lies between two.
fn index_in(row: &[u16], period: u16) -> usize {
    row[..PT_ROW].iter().position(|&p| period >= p).unwrap_or(PT_ROW - 2)
}

#[derive(Clone, Default)]
struct Channel {
    /// The cell being played.
    note: u8,
    effect: u8,
    param: u8,
    /// The sample the channel's instrument plays.
    sample: Option<usize>,
    /// Where in the sample the next start begins, in frames, and how many
    /// words there are from there to its end. 9xx moves both, and they stay
    /// moved until the next instrument number.
    start: u32,
    length: u32,
    finetune: u8,
    volume: i16,
    period: u16,
    /// The period a tone portamento is heading for; 0 when there is none.
    wanted: u16,
    porta_upwards: bool,
    porta_speed: u8,
    vibrato: u8,
    vibrato_pos: u8,
    tremolo: u8,
    tremolo_pos: u8,
    /// Low four bits: vibrato's shape; high four: tremolo's.
    waveforms: u8,
    glissando: bool,
    offset: u8,
    loop_row: u8,
    loop_count: u8,
    pan: f32,
    /// What the voice was last told.
    sent_period: u16,
    sent_volume: i16,
    /// The chord the channel is playing (our own songs only).
    chord: Option<Arpeggio>,
    /// A sample was started on this tick.
    started: bool,
    /// A sample has been started at some time. Until then the chip's channel
    /// is switched off, and giving it a sample to move to does nothing.
    switched_on: bool,
}

pub struct Replayer {
    channels: Vec<Channel>,
    speed: u16,
    tempo: u16,
    /// A tempo change waits for the next tick, as the Amiga's timer did.
    next_tempo: Option<u16>,
    tick: u16,
    /// The order and row that will be read next.
    order: usize,
    row: usize,
    break_row: usize,
    jump: bool,
    loop_back: bool,
    /// A pattern delay asked for on this row, and the repeats still to go.
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
            .map(|&p| Channel { pan: pan_of(p), sent_volume: -1, ..Default::default() })
            .collect();
        Replayer {
            channels,
            speed: song.speed as u16,
            tempo: song.tempo,
            next_tempo: None,
            tick: u16::MAX - 1,
            order: 0,
            row: 0,
            break_row: 0,
            jump: false,
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
        Position { tick: if self.tick < self.speed { self.tick } else { 0 }, speed: self.speed, tempo: self.tempo, loops: self.visited.loops, ..self.shown }
    }

    /// See `Player::retarget`.
    pub fn retarget(&mut self, song: &Song) {
        self.visited = self.visited.renewed(song.orders.len());
        if self.order >= song.orders.len() || self.row >= song.rows_at(self.order) as usize {
            self.order = self.order.min(song.orders.len().saturating_sub(1));
            self.row = 0;
        }
    }

    pub fn seek(&mut self, song: &Song, order: usize, row: usize, voices: &mut dyn Voices) {
        for (c, ch) in self.channels.iter_mut().enumerate() {
            voices.stop(0, c);
            *ch = Channel { pan: ch.pan, sent_volume: -1, ..Default::default() };
        }
        self.order = order.min(song.orders.len().saturating_sub(1));
        self.row = row.min(song.rows_at(self.order) as usize - 1);
        self.tick = u16::MAX - 1;
        self.break_row = 0;
        self.jump = false;
        self.loop_back = false;
        self.delay_asked = 0;
        self.delay_left = 0;
        self.visited.clear();
        self.visited.stopped = false;
        self.shown.order = self.order as u16;
        self.shown.pattern = song.orders.get(self.order).copied().unwrap_or(0);
        self.shown.row = self.row as u16;
    }

    pub fn tick(&mut self, song: &Song, frame: u32, voices: &mut dyn Voices) {
        if self.visited.stopped {
            return;
        }
        if let Some(tempo) = self.next_tempo.take() {
            self.tempo = tempo;
        }
        self.tick = self.tick.wrapping_add(1);
        if self.tick < self.speed {
            for c in 0..self.channels.len() {
                self.run_effects(c, frame, voices);
            }
            if self.jump {
                self.next_order(song);
            }
            return;
        }
        self.tick = 0;

        if self.delay_left == 0 {
            self.play_row(song, frame, voices);
        } else {
            // The row is being held by a pattern delay: its effects run again
            // from the top, but its notes are not read again.
            for c in 0..self.channels.len() {
                self.run_effects(c, frame, voices);
            }
        }

        // Work out the next row now, on the row's first tick. A jump takes
        // effect at once, so a delay on the same row holds the row jumped to.
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
            self.loop_back = false;
            self.row = self.break_row;
            self.break_row = 0;
        }
        if self.row >= song.rows_at(self.order) as usize || self.jump {
            self.next_order(song);
        }
    }

    fn next_order(&mut self, song: &Song) {
        self.row = self.break_row;
        self.break_row = 0;
        self.jump = false;
        self.order = self.order.wrapping_add(1) & 127;
        if self.order >= song.orders.len() {
            self.order = 0;
            self.visited.wrapped = true;
        }
        if self.row >= song.rows_at(self.order) as usize {
            self.row = 0;
        }
    }

    fn play_row(&mut self, song: &Song, frame: u32, voices: &mut dyn Voices) {
        let held = self.visited.keep(&mut self.order, &mut self.row, song);
        let looping = held || self.channels.iter().any(|ch| ch.loop_count > 0);
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
            let chord_changed = chord_after_row(&mut self.channels[c].chord, event.and_then(|e| e.chord), cell, self.tempo);
            self.channels[c].started = false;
            self.play_cell(song, c, cell, frame, voices);
            let ch = &mut self.channels[c];
            // A note that starts takes the chord with it; this is for a chord
            // that begins or ends on a note already sounding.
            if chord_changed && !(ch.started && ch.chord.is_some()) {
                voices.set_arpeggio(frame, c, ch.chord.as_ref());
            }
            send_volume(ch, ch.volume, c, frame, voices);
            // An instrument number that did not start a note still changes
            // which sample the channel is on: the chip moves to it when the
            // one playing comes round.
            if cell.instrument != 0 && !ch.started && ch.switched_on {
                voices.queue(frame, c, ch.sample);
            }
        }
    }

    fn play_cell(&mut self, song: &Song, c: usize, cell: Cell, frame: u32, voices: &mut dyn Voices) {
        let ch = &mut self.channels[c];
        if cell.is_empty() {
            send_period(ch, ch.period, c, frame, voices);
        }
        ch.note = if cell.note <= NOTE_MAX { cell.note } else { 0 };
        ch.effect = cell.effect & 15;
        ch.param = cell.param;

        if (1..=31).contains(&cell.instrument) {
            let sample = song.instrument(cell.instrument).and_then(|i| i.sample_for(49));
            ch.sample = sample;
            ch.start = 0;
            ch.length = 0;
            if let Some(s) = sample.and_then(|i| song.samples.get(i)) {
                ch.finetune = ((s.finetune >> 4) & 15) as u8;
                ch.volume = s.volume as i16;
                // With a loop that starts after the beginning, the first pass
                // stops at the loop's end; otherwise it plays the lot.
                ch.length = match s.loop_range() {
                    Some((start, end)) if start > 0 => end / 2,
                    _ => s.data.len() as u32 / 2,
                };
            } else {
                ch.finetune = 0;
                ch.volume = 0;
            }
        }

        if ch.note == 0 {
            self.first_tick_effects(c, frame, voices);
            return;
        }
        if ch.effect == fx::EXTENDED && ch.param >> 4 == 5 {
            ch.finetune = ch.param & 15;
            self.start_note(c, frame, voices);
        } else if ch.effect == fx::TONE_PORTA || ch.effect == fx::TONE_PORTA_VOL_SLIDE {
            aim_portamento(ch);
            self.first_tick_effects(c, frame, voices);
        } else if ch.effect == fx::SAMPLE_OFFSET {
            // The offset is applied here and again inside `start_note`: once
            // for this note, and once more that only later notes feel.
            self.first_tick_effects(c, frame, voices);
            self.start_note(c, frame, voices);
        } else {
            self.start_note(c, frame, voices);
        }
    }

    /// Gives the channel its note's period and, unless the note is delayed,
    /// starts the sample.
    fn start_note(&mut self, c: usize, frame: u32, voices: &mut dyn Voices) {
        let ch = &mut self.channels[c];
        let i = (ch.note as usize).clamp(37, 72) - 37;
        ch.period = row_of(ch.finetune)[i];
        let delayed = ch.effect == fx::EXTENDED && ch.param >> 4 == 0xD;
        if !delayed {
            if ch.waveforms & 0x04 == 0 {
                ch.vibrato_pos = 0;
            }
            if ch.waveforms & 0x40 == 0 {
                ch.tremolo_pos = 0;
            }
            restart_sample(ch, c, frame, voices);
        }
        self.first_tick_effects(c, frame, voices);
    }

    /// The effects that act once, when a row is read.
    fn first_tick_effects(&mut self, c: usize, frame: u32, voices: &mut dyn Voices) {
        let ch = &mut self.channels[c];
        match ch.effect {
            fx::SAMPLE_OFFSET => {
                if ch.param > 0 {
                    ch.offset = ch.param;
                }
                let words = (ch.offset as u32) << 7;
                if words < ch.length {
                    ch.length -= words;
                    ch.start += words * 2;
                } else {
                    ch.length = 1;
                }
            }
            fx::POSITION_JUMP => {
                self.order = (ch.param as usize).wrapping_sub(1);
                self.break_row = 0;
                self.jump = true;
            }
            fx::PATTERN_BREAK => {
                let row = (ch.param >> 4) as usize * 10 + (ch.param & 15) as usize;
                self.break_row = if row > 63 { 0 } else { row };
                self.jump = true;
            }
            fx::EXTENDED => self.extended(c, frame, voices),
            fx::SPEED => {
                if ch.param == 0 {
                    // F00 stops the song in ProTracker.
                    self.visited.end();
                } else if ch.param < 32 {
                    self.speed = ch.param as u16;
                } else {
                    self.next_tempo = Some(ch.param as u16);
                }
            }
            fx::SET_VOLUME => ch.volume = ch.param.min(64) as i16,
            _ => send_period(ch, ch.period, c, frame, voices),
        }
    }

    /// The effects that act on every tick but a row's first (and on the
    /// first too, when a pattern delay is repeating the row), followed by
    /// the volume being sent.
    fn run_effects(&mut self, c: usize, frame: u32, voices: &mut dyn Voices) {
        let tick = self.tick;
        let ch = &mut self.channels[c];
        if ch.effect != 0 || ch.param != 0 {
            match ch.effect {
                fx::ARPEGGIO => {
                    let step = match tick % 3 {
                        1 => (ch.param >> 4) as usize,
                        2 => (ch.param & 15) as usize,
                        _ => 0,
                    };
                    if step == 0 && tick % 3 == 0 {
                        send_period(ch, ch.period, c, frame, voices);
                    } else {
                        // Counted along the table from the channel's note. A
                        // step off the end of the row reads what lies beyond.
                        let row = row_of(ch.finetune);
                        if let Some(i) = row[..PT_ROW].iter().position(|&p| ch.period >= p) {
                            let period = row.get(i + step).copied().unwrap_or(0);
                            send_period(ch, period, c, frame, voices);
                        }
                    }
                }
                fx::PORTA_UP => slide_up(ch, ch.param, c, frame, voices),
                fx::PORTA_DOWN => slide_down(ch, ch.param, c, frame, voices),
                fx::TONE_PORTA => {
                    if ch.param > 0 {
                        ch.porta_speed = ch.param;
                        ch.param = 0;
                    }
                    tone_portamento(ch, c, frame, voices);
                }
                fx::VIBRATO => {
                    if ch.param & 0x0F > 0 {
                        ch.vibrato = (ch.vibrato & 0xF0) | (ch.param & 0x0F);
                    }
                    if ch.param & 0xF0 > 0 {
                        ch.vibrato = (ch.param & 0xF0) | (ch.vibrato & 0x0F);
                    }
                    vibrato(ch, c, frame, voices);
                }
                fx::TONE_PORTA_VOL_SLIDE => {
                    tone_portamento(ch, c, frame, voices);
                    volume_slide(ch);
                }
                fx::VIBRATO_VOL_SLIDE => {
                    vibrato(ch, c, frame, voices);
                    volume_slide(ch);
                }
                fx::EXTENDED => self.extended(c, frame, voices),
                effect => {
                    send_period(ch, ch.period, c, frame, voices);
                    if effect == fx::TREMOLO {
                        tremolo(ch, c, frame, voices);
                        return;
                    } else if effect == fx::VOL_SLIDE {
                        volume_slide(ch);
                    }
                }
            }
        }
        let ch = &mut self.channels[c];
        send_volume(ch, ch.volume, c, frame, voices);
    }

    /// The E effects. Most check the tick themselves.
    fn extended(&mut self, c: usize, frame: u32, voices: &mut dyn Voices) {
        let tick = self.tick;
        let ch = &mut self.channels[c];
        let x = ch.param & 15;
        match ch.param >> 4 {
            0x0 if tick == 0 => voices.set_filter(frame, x & 1 == 0),
            0x1 if tick == 0 => slide_up(ch, x, c, frame, voices),
            0x2 if tick == 0 => slide_down(ch, x, c, frame, voices),
            0x3 => ch.glissando = x != 0,
            0x4 => ch.waveforms = (ch.waveforms & 0xF0) | x,
            0x5 => ch.finetune = x,
            0x6 if tick == 0 => {
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
            0x9 => {
                // Not on the first tick if the row has a note: that note has
                // only just started.
                if x > 0 && !(tick == 0 && ch.note != 0) && tick % x as u16 == 0 {
                    restart_sample(ch, c, frame, voices);
                }
            }
            0xA if tick == 0 => ch.volume = (ch.volume + x as i16).min(64),
            0xB if tick == 0 => ch.volume = (ch.volume - x as i16).max(0),
            0xC if tick == x as u16 => ch.volume = 0,
            0xD if tick == x as u16 && ch.note != 0 => restart_sample(ch, c, frame, voices),
            0xE if tick == 0 && self.delay_left == 0 => self.delay_asked = x + 1,
            _ => {}
        }
    }
}

fn send_period(ch: &mut Channel, period: u16, c: usize, frame: u32, voices: &mut dyn Voices) {
    if ch.sent_period != period {
        ch.sent_period = period;
        voices.set_rate(frame, c, rate_of(period));
    }
}

fn send_volume(ch: &mut Channel, volume: i16, c: usize, frame: u32, voices: &mut dyn Voices) {
    if ch.sent_volume != volume {
        ch.sent_volume = volume;
        voices.set_volume(frame, c, volume as f32 / 64.0, 0);
    }
}

/// Starts the channel's sample from where 9xx has left its start.
fn restart_sample(ch: &mut Channel, c: usize, frame: u32, voices: &mut dyn Voices) {
    ch.started = true;
    ch.switched_on = true;
    ch.sent_period = ch.period;
    let Some(sample) = ch.sample else {
        voices.stop(frame, c);
        return;
    };
    // The chip keeps the volume it had, but the row's volume is sent straight
    // after, so the note may as well start at it.
    ch.sent_volume = ch.volume;
    let start = Start {
        source: Source::Sample(sample),
        // One word left means the offset ran past the end: nothing of the
        // sample plays, only its loop if it has one.
        offset: if ch.length <= 1 { u32::MAX } else { ch.start },
        rate: rate_of(ch.period),
        volume: ch.sent_volume as f32 / 64.0,
        pan: ch.pan,
    };
    voices.play(frame, c, &start);
    if ch.chord.is_some() {
        voices.set_arpeggio(frame, c, ch.chord.as_ref());
    }
}

/// Sets where a tone portamento is going: the row's note, in the channel's
/// finetune.
fn aim_portamento(ch: &mut Channel) {
    let row = row_of(ch.finetune);
    let note = row_of(0)[(ch.note as usize).clamp(37, 72) - 37];
    let mut i = index_in(row, note);
    if ch.finetune & 8 != 0 && i > 0 {
        i -= 1;
    }
    ch.wanted = row[i];
    ch.porta_upwards = false;
    if ch.period == ch.wanted {
        ch.wanted = 0;
    } else if ch.period > ch.wanted {
        ch.porta_upwards = true;
    }
}

fn tone_portamento(ch: &mut Channel, c: usize, frame: u32, voices: &mut dyn Voices) {
    if ch.wanted == 0 {
        return;
    }
    let speed = ch.porta_speed as u16;
    if ch.porta_upwards {
        ch.period = ch.period.saturating_sub(speed);
        if ch.period <= ch.wanted {
            ch.period = ch.wanted;
            ch.wanted = 0;
        }
    } else {
        ch.period = ch.period.saturating_add(speed);
        if ch.period >= ch.wanted {
            ch.period = ch.wanted;
            ch.wanted = 0;
        }
    }
    let period = if ch.glissando {
        let row = row_of(ch.finetune);
        row[index_in(row, ch.period)]
    } else {
        ch.period
    };
    send_period(ch, period, c, frame, voices);
}

/// 1xx and E1x. The floor is checked on the low 12 bits only, so a slide
/// that goes below zero wraps round and is not caught.
fn slide_up(ch: &mut Channel, amount: u8, c: usize, frame: u32, voices: &mut dyn Voices) {
    ch.period = ch.period.wrapping_sub(amount as u16);
    if ch.period & 0xFFF < MIN_PERIOD {
        ch.period = (ch.period & 0xF000) | MIN_PERIOD;
    }
    send_period(ch, ch.period & 0xFFF, c, frame, voices);
}

fn slide_down(ch: &mut Channel, amount: u8, c: usize, frame: u32, voices: &mut dyn Voices) {
    ch.period = ch.period.wrapping_add(amount as u16);
    if ch.period & 0xFFF > MAX_PERIOD {
        ch.period = (ch.period & 0xF000) | MAX_PERIOD;
    }
    send_period(ch, ch.period & 0xFFF, c, frame, voices);
}

/// The height of a vibrato or tremolo wave, 0 to 255, at a position; the
/// position's top bit says which half of the wave it is in.
fn wave(shape: u8, pos: u8, falling: bool) -> u16 {
    let step = (pos >> 2) & 31;
    match shape & 3 {
        0 => HALF_SINE[step as usize] as u16,
        1 if falling => 255 - ((step as u16) << 3),
        1 => (step as u16) << 3,
        _ => 255,
    }
}

fn vibrato(ch: &mut Channel, c: usize, frame: u32, voices: &mut dyn Voices) {
    let second_half = ch.vibrato_pos >= 128;
    let depth = (wave(ch.waveforms, ch.vibrato_pos, second_half) * (ch.vibrato & 15) as u16) >> 7;
    let period = if second_half { ch.period.wrapping_sub(depth) } else { ch.period.wrapping_add(depth) };
    send_period(ch, period, c, frame, voices);
    ch.vibrato_pos = ch.vibrato_pos.wrapping_add((ch.vibrato >> 2) & 0x3C);
}

fn tremolo(ch: &mut Channel, c: usize, frame: u32, voices: &mut dyn Voices) {
    if ch.param & 0x0F > 0 {
        ch.tremolo = (ch.tremolo & 0xF0) | (ch.param & 0x0F);
    }
    if ch.param & 0xF0 > 0 {
        ch.tremolo = (ch.param & 0xF0) | (ch.tremolo & 0x0F);
    }
    // The ramp shape looks at the vibrato's position to see which half it is
    // in, not the tremolo's: a slip in ProTracker that is now how it sounds.
    let depth = (wave(ch.waveforms >> 4, ch.tremolo_pos, ch.vibrato_pos >= 128) * (ch.tremolo & 15) as u16) >> 6;
    let volume = if ch.tremolo_pos < 128 {
        (ch.volume + depth as i16).min(64)
    } else {
        (ch.volume - depth as i16).max(0)
    };
    send_volume(ch, volume, c, frame, voices);
    ch.tremolo_pos = ch.tremolo_pos.wrapping_add((ch.tremolo >> 2) & 0x3C);
}

fn volume_slide(ch: &mut Channel) {
    if ch.param & 0xF0 == 0 {
        ch.volume = (ch.volume - (ch.param & 15) as i16).max(0);
    } else {
        ch.volume = (ch.volume + (ch.param >> 4) as i16).min(64);
    }
}
