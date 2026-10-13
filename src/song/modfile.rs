//! Loading MOD files: ProTracker's format and the variants that share its
//! layout (Soundtracker's 15-sample files, StarTrekker's FLT4 and FLT8, and
//! the PC trackers' 6CHN, 8CHN, 16CH and so on).
//!
//! The layout: a 20-byte title; 31 (or 15) sample headers of 30 bytes; the
//! song length, a restart byte and 128 orders; a 4-letter tag saying which
//! variant it is (15-sample files have none); patterns of 64 rows with 4
//! bytes a cell; then the samples, 8 bits signed.
//!
//! A file can be cut short anywhere after the header and still load: what
//! is missing is silence.

use super::bytes::Reader;
use super::*;
use crate::replayer::tables::pt_base_periods;

const ROWS: usize = 64;

/// What a tag says about the file.
struct Flavour {
    channels: usize,
    samples: usize,
    /// Written by a 4-channel Amiga tracker, so played by ProTracker's rules.
    amiga: bool,
    /// StarTrekker's 8 channels: each pattern is stored as two 4-channel ones.
    paired: bool,
}

fn flavour_of(tag: &[u8]) -> Option<Flavour> {
    let digit = |b: u8| b.is_ascii_digit().then(|| (b - b'0') as usize);
    let f = |channels, amiga, paired| Some(Flavour { channels, samples: 31, amiga, paired });
    match tag {
        b"M.K." | b"M!K!" | b"M&K!" | b"N.T." | b"FLT4" => f(4, true, false),
        b"FLT8" => f(8, false, true),
        b"CD81" | b"OKTA" | b"OCTA" => f(8, false, false),
        [n, b'C', b'H', b'N'] => f(digit(*n).filter(|&n| n > 0)?, false, false),
        [a, b, b'C', b'H'] | [a, b, b'C', b'N'] => {
            let n = digit(*a)? * 10 + digit(*b)?;
            (1..=32).contains(&n).then_some(())?;
            f(n, false, false)
        }
        _ => None,
    }
}

/// True if the data has a MOD tag where one belongs.
pub fn has_tag(data: &[u8]) -> bool {
    data.get(1080..1084).is_some_and(|t| flavour_of(t).is_some())
}

/// A 15-sample Soundtracker file has nothing that says what it is, so it
/// has to look right all over before it is taken for one.
fn looks_like_15_samples(data: &[u8]) -> bool {
    if data.len() < 600 + 1024 {
        return false;
    }
    let texty = |s: &[u8]| s.iter().all(|&b| b == 0 || (0x20..0x7F).contains(&b));
    if !texty(&data[..20]) {
        return false;
    }
    let mut any = false;
    for i in 0..15 {
        let h = &data[20 + i * 30..50 + i * 30];
        let length = u16::from_be_bytes([h[22], h[23]]);
        if !texty(&h[..22]) || h[24] > 0x0F || h[25] > 64 || length > 0x8000 {
            return false;
        }
        any |= length > 0;
    }
    let length = data[470];
    let orders = &data[472..600];
    any && (1..=128).contains(&length) && orders.iter().all(|&o| o < 64)
}

/// The note a period in a pattern means. Inside the Amiga's three octaves
/// this is ProTracker's own search (the first table entry the period
/// reaches); outside, the nearest note.
fn note_of_period(period: u16) -> u8 {
    if period == 0 {
        return NOTE_NONE;
    }
    if (113..=856).contains(&period) {
        let i = pt_base_periods().iter().position(|&p| period >= p).unwrap_or(35);
        return 37 + i as u8;
    }
    let semis = (12.0 * (856.0 / period as f64).log2()).round() as i32;
    (37 + semis).clamp(1, NOTE_MAX as i32) as u8
}

pub fn load(data: &[u8]) -> Result<Song, LoadError> {
    let flavour = match data.get(1080..1084).and_then(flavour_of) {
        Some(f) => f,
        None if looks_like_15_samples(data) => Flavour { channels: 4, samples: 15, amiga: true, paired: false },
        None if data.len() < 600 => return Err(LoadError::TooShort),
        None => return Err(LoadError::NotRecognised),
    };
    let Flavour { channels, samples: sample_count, amiga, paired } = flavour;
    let old = sample_count == 15;

    let mut r = Reader::new(data);
    let mut song = Song { title: r.text(20), frequency_table: FrequencyTable::Amiga, ..Default::default() };

    struct Header {
        name: String,
        length: usize,
        finetune: i8,
        volume: u8,
        loop_start: usize,
        loop_len: usize,
    }
    let mut headers = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        let name = r.text(22);
        let length = r.u16_be() as usize * 2;
        let fine = r.u8() & 0x0F;
        let volume = r.u8().min(64);
        // Soundtracker counted the loop start in bytes; everyone since, in words.
        let loop_start = r.u16_be() as usize * if old { 1 } else { 2 };
        let loop_len = r.u16_be() as usize * 2;
        // The four bits are a signed number, -8 to 7; ours has 16 steps to each.
        let finetune = (if fine > 7 { fine as i8 - 16 } else { fine as i8 }) * 16;
        headers.push(Header { name, length, finetune, volume, loop_start, loop_len });
    }

    let length = (r.u8() as usize).clamp(1, 128);
    let restart = r.u8() as usize;
    let mut orders: Vec<u16> = r.take(128).iter().map(|&o| o as u16).collect();
    orders.resize(128, 0);
    if !old {
        r.skip(4);
    }
    let pattern_start = r.pos();

    // The file does not say how many patterns it has: it is one more than the
    // highest order. Orders past the song's length usually count too, unless
    // they are rubbish that would run past the end of the file.
    if paired {
        for o in &mut orders {
            *o /= 2;
        }
    }
    let stored = channels * ROWS * 4;
    let count_for = |orders: &[u16]| orders.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut pattern_count = count_for(&orders);
    if pattern_start + pattern_count * stored > data.len() {
        pattern_count = count_for(&orders[..length]);
    }
    orders.truncate(length);
    song.orders = orders;
    song.restart = if restart < length { restart as u16 } else { 0 };

    // Left, right, right, left, as the Amiga's four voices are wired.
    song.channel_pan = (0..channels).map(|c| if matches!(c % 4, 0 | 3) { 0 } else { 255 }).collect();

    let mut in_amiga_range = true;
    for p in 0..pattern_count {
        let mut pattern = Pattern::new(ROWS as u16);
        for row in 0..ROWS {
            for ch in 0..channels {
                // A paired pattern is two 4-channel patterns one after the other.
                let at = if paired {
                    pattern_start + p * stored + (ch / 4) * (ROWS * 16) + (row * 4 + ch % 4) * 4
                } else {
                    pattern_start + p * stored + (row * channels + ch) * 4
                };
                let Some(b) = data.get(at..at + 4) else { continue };
                if b == [0, 0, 0, 0] {
                    continue;
                }
                let period = ((b[0] & 0x0F) as u16) << 8 | b[1] as u16;
                if period != 0 && !(113..=856).contains(&period) {
                    in_amiga_range = false;
                }
                let cell = Cell {
                    note: note_of_period(period),
                    instrument: (b[0] & 0xF0) | (b[2] >> 4),
                    volume: 0,
                    effect: b[2] & 0x0F,
                    param: b[3],
                };
                if !cell.is_empty() {
                    pattern.events.push(Event { row: row as u16, channel: ch as u8, cell });
                }
            }
        }
        song.patterns.push(pattern);
    }

    r.seek(pattern_start.saturating_add(pattern_count * stored));
    for (i, h) in headers.into_iter().enumerate() {
        let bytes = r.take(h.length);
        let data: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
        let len = data.len();
        // A loop of one word is how the format says "no loop".
        let (mut loop_start, mut loop_len) = (h.loop_start, h.loop_len);
        if loop_len <= 2 {
            loop_len = 0;
        } else if loop_start + loop_len > len {
            // Some old files give the start in bytes; if that fits, it was meant.
            if !old && loop_start / 2 + loop_len <= len {
                loop_start /= 2;
            } else {
                loop_start = loop_start.min(len);
                loop_len = len - loop_start;
            }
        }
        song.samples.push(Sample {
            name: h.name.clone(),
            data: SampleData::Bits8(data),
            loop_start: loop_start as u32,
            loop_len: loop_len as u32,
            loop_kind: if loop_len > 0 { LoopKind::Forward } else { LoopKind::Off },
            volume: h.volume,
            finetune: h.finetune,
            relative_note: 0,
            pan: None,
        });
        song.instruments.push(Instrument::of_sample(&h.name, i as u16));
    }

    // Notes outside the Amiga's range mean a PC tracker wrote the file, even
    // if it has four channels, and ProTracker's quirks would be wrong for it.
    song.rules = if amiga && in_amiga_range { Rules::ProTracker } else { Rules::FastTracker2 };
    song.speed = 6;
    song.tempo = 125;
    song.tidy();
    Ok(song)
}
