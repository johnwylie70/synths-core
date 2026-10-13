//! Loading XM files, FastTracker 2's format.
//!
//! The layout: a header with the song's settings and its order list; then
//! each pattern, packed; then each instrument, which carries its own
//! samples. Every block says how long its own header is, and that length is
//! what is believed, not the usual size. Sample sound is stored as the
//! difference from one frame to the next.
//!
//! A file can be cut short anywhere after the first header and still load:
//! what is missing is empty.

use super::bytes::Reader;
use super::*;

const MAGIC: &[u8] = b"Extended Module: ";

pub fn is_xm(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

fn envelope(points: &[u8], count: u8, sustain: u8, loop_start: u8, loop_end: u8, flags: u8) -> Envelope {
    let count = (count as usize).min(MAX_ENVELOPE_POINTS);
    let mut r = Reader::new(points);
    let points: Vec<(u16, u16)> = (0..count).map(|_| (r.u16_le(), r.u16_le().min(64))).collect();
    Envelope {
        enabled: flags & 1 != 0,
        points,
        sustain: (flags & 2 != 0).then_some(sustain),
        loop_points: (flags & 4 != 0).then_some((loop_start, loop_end)),
    }
}

#[cold]
pub fn load(data: &[u8]) -> Result<Song, LoadError> {
    if !is_xm(data) {
        return Err(LoadError::NotRecognised);
    }
    if data.len() < 80 {
        return Err(LoadError::TooShort);
    }
    let mut r = Reader::new(data);
    r.skip(MAGIC.len());
    let mut song = Song { title: r.text(20), ..Default::default() };
    r.skip(1 + 20);
    let version = r.u16_le();
    if version < 0x0104 {
        return Err(LoadError::Invalid("the XM file's version is older than this player reads".to_string()));
    }
    let header_start = r.pos();
    let header_size = r.u32_le() as usize;
    let length = r.u16_le() as usize;
    let restart = r.u16_le();
    let channels = r.u16_le() as usize;
    let pattern_count = r.u16_le() as usize;
    let instrument_count = r.u16_le() as usize;
    let flags = r.u16_le();
    let speed = r.u16_le();
    let tempo = r.u16_le();
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(LoadError::Invalid(text::join(&[&text::int(channels as i64), " channels"])));
    }
    if pattern_count > MAX_PATTERNS || instrument_count > MAX_INSTRUMENTS {
        return Err(LoadError::Invalid("too many patterns or instruments".to_string()));
    }
    let orders = r.take(length.clamp(1, MAX_ORDERS).min(header_size.saturating_sub(20)));
    song.orders = orders.iter().map(|&o| o as u16).collect();
    song.restart = restart;
    song.channel_pan = vec![128; channels];
    song.frequency_table = if flags & 1 != 0 { FrequencyTable::Linear } else { FrequencyTable::Amiga };
    song.speed = speed.clamp(1, 31) as u8;
    song.tempo = tempo;
    r.seek(header_start.saturating_add(header_size));

    for _ in 0..pattern_count {
        let start = r.pos();
        let header = r.u32_le() as usize;
        r.skip(1);
        let rows = r.u16_le();
        let packed = r.u16_le() as usize;
        r.seek(start.saturating_add(header));
        let mut pattern = Pattern::new(rows.clamp(1, MAX_ROWS as u16));
        let mut p = Reader::new(r.take(packed));
        // An empty pattern has no data at all.
        'rows: for row in 0..pattern.rows {
            for ch in 0..channels {
                if p.left() == 0 {
                    break 'rows;
                }
                // A first byte with its top bit set says which of the five
                // fields follow; without it, it is the note and all follow.
                let b = p.u8();
                let mut cell = Cell::default();
                let has = if b & 0x80 != 0 { b } else { 0x1F };
                if b & 0x80 == 0 {
                    cell.note = b;
                } else if has & 1 != 0 {
                    cell.note = p.u8();
                }
                if has & 2 != 0 {
                    cell.instrument = p.u8();
                }
                if has & 4 != 0 {
                    cell.volume = p.u8();
                }
                if has & 8 != 0 {
                    cell.effect = p.u8();
                }
                if has & 16 != 0 {
                    cell.param = p.u8();
                }
                if cell.note > NOTE_OFF {
                    cell.note = NOTE_NONE;
                }
                if !cell.is_empty() {
                    pattern.events.push(Event { row, channel: ch as u8, cell, chord: None, extra: 0 });
                }
            }
        }
        song.patterns.push(pattern);
    }

    for _ in 0..instrument_count {
        if r.left() < 4 {
            // The file ends early: the instruments that are missing are empty.
            song.instruments.push(Instrument::default());
            continue;
        }
        let start = r.pos();
        let size = r.u32_le() as usize;
        let mut instrument = Instrument { name: r.text(22), ..Default::default() };
        r.skip(1);
        let sample_count = (r.u16_le() as usize).min(64);
        if sample_count == 0 {
            r.seek(start.saturating_add(size));
            song.instruments.push(instrument);
            continue;
        }
        let sample_header = r.u32_le() as usize;
        let keys = r.take(96);
        let volume_points = r.take(48);
        let pan_points = r.take(48);
        let (volume_count, pan_count) = (r.u8(), r.u8());
        let (volume_sustain, volume_loop_start, volume_loop_end) = (r.u8(), r.u8(), r.u8());
        let (pan_sustain, pan_loop_start, pan_loop_end) = (r.u8(), r.u8(), r.u8());
        let (volume_flags, pan_flags) = (r.u8(), r.u8());
        instrument.volume_envelope =
            envelope(volume_points, volume_count, volume_sustain, volume_loop_start, volume_loop_end, volume_flags);
        instrument.pan_envelope = envelope(pan_points, pan_count, pan_sustain, pan_loop_start, pan_loop_end, pan_flags);
        instrument.vibrato = AutoVibrato { waveform: r.u8() & 3, sweep: r.u8(), depth: r.u8(), rate: r.u8() };
        instrument.fadeout = r.u16_le();
        r.seek(start.saturating_add(size));

        struct Header {
            length: usize,
            loop_start: usize,
            loop_len: usize,
            kind: u8,
            sample: Sample,
        }
        let mut headers = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            let at = r.pos();
            let length = r.u32_le() as usize;
            let loop_start = r.u32_le() as usize;
            let loop_len = r.u32_le() as usize;
            let volume = r.u8().min(64);
            let finetune = r.u8() as i8;
            let kind = r.u8();
            let pan = r.u8();
            let relative_note = r.u8() as i8;
            r.skip(1);
            let name = r.text(22);
            // A sample header is 40 bytes unless the instrument says otherwise.
            if sample_header != 0 {
                r.seek(at.saturating_add(sample_header));
            }
            let sample = Sample { name, volume, finetune, relative_note, pan: Some(pan), ..Default::default() };
            headers.push(Header { length, loop_start, loop_len, kind, sample });
        }

        let first = song.samples.len();
        for h in headers {
            if song.samples.len() >= MAX_SAMPLES {
                break;
            }
            let Header { length, loop_start, loop_len, kind, mut sample } = h;
            // Lengths are in bytes whatever the sample's width.
            let bytes = r.take(length);
            let wide = kind & 0x10 != 0;
            let stereo = kind & 0x20 != 0;
            let width = if wide { 2 } else { 1 };
            // A stereo sample is its left half then its right; the left is played.
            let frames = bytes.len() / width / if stereo { 2 } else { 1 };
            sample.data = if wide {
                let mut last = 0i16;
                SampleData::Bits16(
                    bytes[..frames * 2]
                        .chunks_exact(2)
                        .map(|b| {
                            last = last.wrapping_add(i16::from_le_bytes([b[0], b[1]]));
                            last
                        })
                        .collect(),
                )
            } else {
                let mut last = 0i8;
                SampleData::Bits8(
                    bytes[..frames]
                        .iter()
                        .map(|&b| {
                            last = last.wrapping_add(b as i8);
                            last
                        })
                        .collect(),
                )
            };
            let (mut loop_start, mut loop_len) = (loop_start / width, loop_len / width);
            loop_start = loop_start.min(frames);
            loop_len = loop_len.min(frames - loop_start);
            sample.loop_start = loop_start as u32;
            sample.loop_len = loop_len as u32;
            sample.loop_kind = match kind & 3 {
                _ if loop_len == 0 => LoopKind::Off,
                0 => LoopKind::Off,
                1 => LoopKind::Forward,
                _ => LoopKind::PingPong,
            };
            song.samples.push(sample);
        }
        let loaded = song.samples.len() - first;
        instrument.keymap = (0..96)
            .map(|k| match keys.get(k) {
                Some(&slot) if (slot as usize) < loaded => (first + slot as usize) as u16,
                _ => NO_SAMPLE,
            })
            .collect();
        song.instruments.push(instrument);
    }

    song.rules = Rules::FastTracker2;
    song.tidy();
    Ok(song)
}
