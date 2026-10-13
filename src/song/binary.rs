//! The packed form of a song: the whole model, samples and all, as one run
//! of bytes.
//!
//! The JSON form is for people and for editors. This one is for handing a
//! song across to the engine: it takes a fraction of the code to read, so a
//! WebAssembly build can leave the JSON reader out and have the host (which
//! reads JSON for nothing) pack the song first. `docs/song-format.md` gives
//! the layout byte by byte. Everything is little-endian.
//!
//! Reading is as careful as the module loaders: a file cut short or full of
//! nonsense loads as far as it makes sense, never panics, and never asks
//! for more memory than the bytes it was given could describe.

use super::bytes::Reader;
use super::*;

/// The four bytes a packed song begins with.
pub const MAGIC: &[u8; 4] = b"SYSG";
/// The version this code writes, and the highest it reads.
pub const VERSION: u16 = 1;

/// The four bytes that begin the instruments' `synth` texts, after the samples.
const SYNTHS: &[u8; 4] = b"SYNT";

/// The length that means "no text here" (an instrument that is not a patch).
const NO_TEXT: u16 = 0xFFFF;

pub fn is_binary(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

// ---------------------------------------------------------------- writing

fn put_u16(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_le_bytes());
}

fn put_text(out: &mut Vec<u8>, text: &str) {
    let bytes = &text.as_bytes()[..text.len().min(NO_TEXT as usize - 1)];
    put_u16(out, bytes.len() as u16);
    out.extend_from_slice(bytes);
}

fn put_envelope(out: &mut Vec<u8>, e: &Envelope) {
    let (loop_start, loop_end) = e.loop_points.unwrap_or((0, 0));
    let flags = e.enabled as u8 | (e.sustain.is_some() as u8) << 1 | (e.loop_points.is_some() as u8) << 2;
    let count = e.points.len().min(MAX_ENVELOPE_POINTS);
    out.extend_from_slice(&[flags, count as u8, e.sustain.unwrap_or(0), loop_start, loop_end]);
    for &(x, y) in &e.points[..count] {
        put_u16(out, x);
        put_u16(out, y);
    }
}

/// Packs a song.
pub fn save(song: &Song) -> Vec<u8> {
    let mut out = Vec::new();
    let o = &mut out;
    o.extend_from_slice(MAGIC);
    put_u16(o, VERSION);
    let flags = (song.rules == Rules::ProTracker) as u16 | ((song.frequency_table == FrequencyTable::Amiga) as u16) << 1;
    put_u16(o, flags);
    o.extend_from_slice(&[song.speed, song.tempo.min(255) as u8]);
    put_u16(o, song.restart);
    put_u16(o, song.channels() as u16);
    put_u16(o, song.orders.len() as u16);
    put_u16(o, song.patterns.len() as u16);
    put_u16(o, song.instruments.len() as u16);
    put_u16(o, song.samples.len() as u16);
    put_text(o, &song.title);
    o.extend_from_slice(&song.channel_pan);
    for &order in &song.orders {
        put_u16(o, order);
    }
    for p in &song.patterns {
        put_u16(o, p.rows);
        put_u32(o, p.events.len() as u32);
        for e in &p.events {
            let c = e.cell;
            put_u16(o, e.row);
            o.extend_from_slice(&[e.channel, e.chord.is_some() as u8, c.note, c.instrument, c.volume, c.effect, c.param]);
            if let Some(chord) = e.chord {
                let (kind, rate) = match chord.rate {
                    ChordRate::PerTick(n) => (0u8, n),
                    ChordRate::PerSecond(n) => (1, n),
                };
                o.extend_from_slice(&[chord.count.min(MAX_CHORD as u8), chord.order as u8, kind, 0]);
                o.extend_from_slice(&rate.to_le_bytes());
                o.extend(chord.offsets.iter().map(|&x| x as u8));
            }
        }
    }
    for i in &song.instruments {
        put_text(o, &i.name);
        match &i.patch {
            Some(patch) => put_text(o, patch),
            None => put_u16(o, NO_TEXT),
        }
        // Which sample each key plays: none, one for all, or one for each.
        let first = i.keymap.first().copied().unwrap_or(NO_SAMPLE);
        if i.keymap.is_empty() {
            o.push(0);
        } else if i.keymap.iter().all(|&k| k == first) {
            o.push(1);
            put_u16(o, first);
        } else {
            o.push(2);
            for k in 0..96 {
                put_u16(o, i.keymap.get(k).copied().unwrap_or(NO_SAMPLE));
            }
        }
        put_envelope(o, &i.volume_envelope);
        put_envelope(o, &i.pan_envelope);
        o.extend_from_slice(&[i.vibrato.waveform, i.vibrato.sweep, i.vibrato.depth, i.vibrato.rate]);
        put_u16(o, i.fadeout);
    }
    for s in &song.samples {
        put_text(o, &s.name);
        let wide = matches!(s.data, SampleData::Bits16(_));
        let flags = wide as u8 | (s.loop_kind as u8) << 1 | (s.pan.is_some() as u8) << 3;
        o.extend_from_slice(&[flags, s.volume, s.finetune as u8, s.relative_note as u8, s.pan.unwrap_or(128), 0, 0, 0]);
        put_u32(o, s.data.len() as u32);
        put_u32(o, s.loop_start);
        put_u32(o, s.loop_len);
        match &s.data {
            SampleData::Bits8(v) => o.extend(v.iter().map(|&x| x as u8)),
            SampleData::Bits16(v) => o.extend(v.iter().flat_map(|x| x.to_le_bytes())),
        }
    }
    // After everything a version 1 reader looks at: what hosts have written
    // about the instruments' synth sounds, if any has. An older reader stops
    // before this and is none the worse.
    if song.instruments.iter().any(|i| i.synth.is_some()) {
        o.extend_from_slice(SYNTHS);
        for i in &song.instruments {
            let text = i.synth.as_deref().unwrap_or("");
            put_u32(o, text.len() as u32);
            o.extend_from_slice(text.as_bytes());
        }
    }
    out
}

// ---------------------------------------------------------------- reading

fn get_text(r: &mut Reader) -> Option<String> {
    let len = r.u16_le();
    if len == NO_TEXT {
        return None;
    }
    // Text that is not UTF-8 is dropped, not guessed at.
    Some(core::str::from_utf8(r.take(len as usize)).unwrap_or("").to_string())
}

fn get_envelope(r: &mut Reader) -> Envelope {
    let [flags, count, sustain, loop_start, loop_end] = [r.u8(), r.u8(), r.u8(), r.u8(), r.u8()];
    let mut e = Envelope {
        enabled: flags & 1 != 0,
        points: Vec::new(),
        sustain: (flags & 2 != 0).then_some(sustain),
        loop_points: (flags & 4 != 0).then_some((loop_start, loop_end)),
    };
    for _ in 0..(count as usize).min(MAX_ENVELOPE_POINTS) {
        e.points.push((r.u16_le(), r.u16_le().min(64)));
    }
    e
}

/// Unpacks a song.
pub fn load(data: &[u8]) -> Result<Song, LoadError> {
    if !is_binary(data) {
        return Err(LoadError::NotRecognised);
    }
    if data.len() < 22 {
        return Err(LoadError::TooShort);
    }
    let mut r = Reader::new(data);
    r.skip(4);
    if r.u16_le() > VERSION {
        return Err(LoadError::Invalid("this packed song is newer than this player reads".to_string()));
    }
    let flags = r.u16_le();
    let mut song = Song {
        rules: if flags & 1 != 0 { Rules::ProTracker } else { Rules::FastTracker2 },
        frequency_table: if flags & 2 != 0 { FrequencyTable::Amiga } else { FrequencyTable::Linear },
        speed: r.u8(),
        tempo: r.u8() as u16,
        restart: r.u16_le(),
        ..Default::default()
    };
    let channels = r.u16_le() as usize;
    let orders = (r.u16_le() as usize).min(MAX_ORDERS);
    let patterns = (r.u16_le() as usize).min(MAX_PATTERNS);
    let instruments = (r.u16_le() as usize).min(MAX_INSTRUMENTS);
    let samples = (r.u16_le() as usize).min(MAX_SAMPLES);
    if channels == 0 || channels > MAX_CHANNELS {
        return Err(LoadError::Invalid("a packed song must have 1 to 64 channels".to_string()));
    }
    song.title = get_text(&mut r).unwrap_or_default();
    song.channel_pan = vec![128; channels];
    let pans = r.take(channels);
    song.channel_pan[..pans.len()].copy_from_slice(pans);
    song.orders = (0..orders).map(|_| r.u16_le()).collect();

    for _ in 0..patterns {
        let mut pattern = Pattern::new(r.u16_le());
        // Each event is at least 9 bytes, so the file bounds how many there are.
        let events = (r.u32_le() as usize).min(r.left() / 9);
        pattern.events.reserve(events);
        for _ in 0..events {
            let row = r.u16_le();
            let [channel, has_chord, note, instrument, volume, effect, param] = [r.u8(), r.u8(), r.u8(), r.u8(), r.u8(), r.u8(), r.u8()];
            let cell = Cell { note: if note > NOTE_OFF { 0 } else { note }, instrument, volume, effect: effect.min(fx::MAX), param };
            let chord = (has_chord != 0).then(|| {
                let [count, order, kind, _] = [r.u8(), r.u8(), r.u8(), r.u8()];
                let rate = f32::from_le_bytes([r.u8(), r.u8(), r.u8(), r.u8()]);
                let rate = if rate.is_finite() && rate > 0.0 { rate.min(1000.0) } else { 1.0 };
                let mut chord = Chord::OFF;
                for slot in &mut chord.offsets {
                    *slot = r.u8() as i8;
                }
                chord.count = count.min(MAX_CHORD as u8);
                chord.order = match order {
                    1 => ChordOrder::Down,
                    2 => ChordOrder::UpDown,
                    3 => ChordOrder::Written,
                    4 => ChordOrder::Random,
                    _ => ChordOrder::Up,
                };
                chord.rate = if kind == 1 { ChordRate::PerSecond(rate) } else { ChordRate::PerTick(rate) };
                chord
            });
            pattern.events.push(Event { row, channel, cell, chord });
        }
        song.patterns.push(pattern);
    }

    for _ in 0..instruments {
        let mut ins = Instrument { name: get_text(&mut r).unwrap_or_default(), patch: get_text(&mut r), ..Default::default() };
        match r.u8() {
            1 => ins.keymap = vec![r.u16_le(); 96],
            2 => ins.keymap = (0..96).map(|_| r.u16_le()).collect(),
            _ => {}
        }
        ins.volume_envelope = get_envelope(&mut r);
        ins.pan_envelope = get_envelope(&mut r);
        ins.vibrato = AutoVibrato { waveform: r.u8() & 3, sweep: r.u8(), depth: r.u8(), rate: r.u8() };
        ins.fadeout = r.u16_le();
        song.instruments.push(ins);
    }

    for _ in 0..samples {
        let name = get_text(&mut r).unwrap_or_default();
        let [flags, volume, finetune, relative, pan, _, _, _] = [r.u8(), r.u8(), r.u8(), r.u8(), r.u8(), r.u8(), r.u8(), r.u8()];
        let frames = r.u32_le() as usize;
        let loop_start = r.u32_le();
        let loop_len = r.u32_le();
        // The sound is as long as the file has bytes for, whatever it claims.
        let data = if flags & 1 != 0 {
            let bytes = r.take(frames.saturating_mul(2));
            SampleData::Bits16(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect())
        } else {
            SampleData::Bits8(r.take(frames).iter().map(|&b| b as i8).collect())
        };
        song.samples.push(Sample {
            name,
            data,
            loop_start,
            loop_len,
            loop_kind: match (flags >> 1) & 3 {
                1 => LoopKind::Forward,
                2 => LoopKind::PingPong,
                _ => LoopKind::Off,
            },
            volume,
            finetune: finetune as i8,
            relative_note: relative as i8,
            pan: (flags & 8 != 0).then_some(pan),
        });
    }

    if r.left() >= 4 && r.take(4) == SYNTHS {
        for ins in song.instruments.iter_mut() {
            let len = r.u32_le() as usize;
            let text = core::str::from_utf8(r.take(len)).unwrap_or("");
            ins.synth = (!text.is_empty()).then(|| text.to_string());
        }
    }

    song.tidy();
    Ok(song)
}
