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
/// The highest version this code reads. It writes version 1 unless the
/// song has a chord or an arpeggio that version 1 could not say (`CHORD_NEW`).
pub const VERSION: u16 = 2;

/// What follows an event: nothing, a chord as version 1 had it (16 bytes:
/// always the fast one-voice arpeggio), or a chord or arpeggio in full (32).
const CHORD_NONE: u8 = 0;
const CHORD_OLD: u8 = 1;
const CHORD_NEW: u8 = 2;
/// Added to it: the number of the event's extra follows (a `u16`).
const HAS_EXTRA: u8 = 4;
/// The order byte of an arpeggio that follows a pattern: its number is where the accents would be.
const ORDER_PATTERN: u8 = 7;

/// The lists after the samples, each begun by its four letters.
const ARP_PATTERNS: &[u8; 4] = b"ARPP";
const EXTRAS: &[u8; 4] = b"XTRA";
const GATES: &[u8; 4] = b"IGAT";
const MIXER: &[u8; 4] = b"MIXR";

fn put_f32(out: &mut Vec<u8>, x: f32) {
    out.extend_from_slice(&x.to_le_bytes());
}

fn put_time(out: &mut Vec<u8>, time: &Time) {
    let (kind, length, value) = match *time {
        Time::Note(length) => (0u8, length, 0.0),
        Time::Rows(n) => (1, NoteLength { division: 0, feel: Feel::Plain }, n),
        Time::Ticks(n) => (2, NoteLength { division: 0, feel: Feel::Plain }, n),
        Time::Seconds(n) => (3, NoteLength { division: 0, feel: Feel::Plain }, n),
    };
    out.extend_from_slice(&[kind, length.division, length.feel as u8, 0]);
    put_f32(out, value);
}

fn put_gate(out: &mut Vec<u8>, g: &Gate) {
    let n = g.levels.len().min(32);
    out.extend_from_slice(&[n as u8, g.sends as u8 | (g.stereo as u8) << 1, g.rate.division, g.rate.feel as u8]);
    put_f32(out, g.depth);
    put_f32(out, g.smooth);
    put_f32(out, g.swing);
    for &level in &g.levels[..n] {
        put_f32(out, level);
    }
}

/// Can version 1's record say this chord? It can say "off", and an
/// arpeggio of one voice timed in ticks or seconds with nothing newer.
fn fits_old(chord: &Chord) -> bool {
    match chord.arp {
        None => chord.is_off(),
        Some(arp) => arp.plain() && arp.gate == Arp::FAST.gate && arp.accents == 0,
    }
}

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
    let new = song.patterns.iter().any(|p| p.events.iter().any(|e| e.extra != 0 || e.chord.is_some_and(|c| !fits_old(&c)))) || song.instruments.iter().any(|i| i.gate.is_some());
    put_u16(o, if new { 2 } else { 1 });
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
            let kind = match &e.chord {
                None => CHORD_NONE,
                Some(chord) if fits_old(chord) => CHORD_OLD,
                Some(_) => CHORD_NEW,
            };
            let extra = if e.extra != 0 { HAS_EXTRA } else { 0 };
            o.extend_from_slice(&[e.channel, kind | extra, c.note, c.instrument, c.volume, c.effect, c.param]);
            if e.extra != 0 {
                put_u16(o, e.extra);
            }
            let Some(chord) = e.chord else { continue };
            let arp = chord.arp.unwrap_or(Arp::FAST);
            let (unit, rate, length) = match arp.rate {
                ArpRate::PerTick(n) => (0u8, n, None),
                ArpRate::PerSecond(n) => (1, n, None),
                ArpRate::PerRow(n) => (2, n, None),
                ArpRate::Note(length) => (3, 0.0, Some(length)),
            };
            let count = chord.count.min(MAX_CHORD as u8);
            if kind == CHORD_OLD {
                o.extend_from_slice(&[count, arp.order as u8, unit, 0]);
                o.extend_from_slice(&rate.to_le_bytes());
            } else {
                let length = length.unwrap_or(NoteLength { division: 0, feel: Feel::Plain });
                let order = if arp.pattern != 0 { ORDER_PATTERN } else { arp.order as u8 };
                o.extend_from_slice(&[count, chord.arp.is_some() as u8, order, arp.style as u8]);
                o.extend_from_slice(&[unit, length.division, length.feel as u8, arp.octaves]);
                o.extend_from_slice(&rate.to_le_bytes());
                o.extend_from_slice(&arp.gate.to_le_bytes());
                o.extend_from_slice(&arp.swing.to_le_bytes());
                put_u32(o, if arp.pattern != 0 { arp.pattern as u32 } else { arp.accents });
            }
            o.extend(chord.offsets.iter().map(|&x| x as u8));
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
    // Then what version 2 added, each list only if there is something in it.
    if !song.arp_patterns.is_empty() {
        o.extend_from_slice(ARP_PATTERNS);
        put_u16(o, song.arp_patterns.len() as u16);
        for p in &song.arp_patterns {
            o.push(p.len);
            o.extend(p.steps().iter().flat_map(|s| [s.note as u8, s.flags, s.octave as u8, s.shape]));
        }
    }
    if !song.extras.is_empty() {
        o.extend_from_slice(EXTRAS);
        put_u16(o, song.extras.len() as u16);
        for x in &song.extras {
            o.push(x.glide.is_some() as u8 | (x.bend.is_some() as u8) << 1 | (x.vibrato.is_some() as u8) << 2 | (x.gate.is_some() as u8) << 3);
            if let Some(glide) = &x.glide {
                put_time(o, glide);
            }
            if let Some(bend) = &x.bend {
                o.extend_from_slice(&[bend.shape as u8, 0, 0, 0]);
                put_f32(o, bend.to);
                put_time(o, &bend.over);
            }
            if let Some(vibrato) = &x.vibrato {
                put_f32(o, vibrato.depth);
                put_f32(o, vibrato.rate);
                put_time(o, &vibrato.delay);
            }
            match &x.gate {
                None => {}
                Some(GateChange::Off) => o.push(0),
                Some(GateChange::Instrument) => o.push(1),
                Some(GateChange::On(gate)) => {
                    o.push(2);
                    put_gate(o, gate);
                }
            }
        }
    }
    if song.instruments.iter().any(|i| i.gate.is_some()) {
        o.extend_from_slice(GATES);
        for i in &song.instruments {
            o.push(i.gate.is_some() as u8);
            if let Some(gate) = &i.gate {
                put_gate(o, gate);
            }
        }
    }
    // The mix, the effects and the loop: last, and passed over by a reader that does not know them.
    if song.has_mix() {
        o.extend_from_slice(MIXER);
        put_f32(o, song.volume);
        o.push(song.mix.len().min(MAX_CHANNELS) as u8);
        for m in song.mix.iter().take(MAX_CHANNELS) {
            for x in [m.volume, m.reverb, m.echo] {
                put_f32(o, x);
            }
        }
        // Which of the effects' settings the song gives (bit 11: the echo's time), then each.
        let fx = &song.effects;
        let given = fx.values.iter().enumerate().fold((fx.echo_time.is_some() as u16) << 11, |bits, (i, v)| bits | (v.is_some() as u16) << i);
        put_u16(o, given);
        for v in fx.values.iter().flatten() {
            put_f32(o, *v);
        }
        if let Some(time) = &fx.echo_time {
            put_time(o, time);
        }
        o.push(song.loop_start.is_some() as u8 | (song.loop_end.is_some() as u8) << 1);
        for (order, row) in [song.loop_start, song.loop_end].into_iter().flatten() {
            put_u16(o, order);
            put_u16(o, row);
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

fn get_f32(r: &mut Reader) -> f32 {
    f32::from_le_bytes([r.u8(), r.u8(), r.u8(), r.u8()])
}

fn get_length(division: u8, feel: u8) -> NoteLength {
    NoteLength { division: if division.is_power_of_two() && division <= 64 { division } else { 16 }, feel: [Feel::Plain, Feel::Triplet, Feel::Dotted][feel.min(2) as usize] }
}

fn get_time(r: &mut Reader) -> Time {
    let [kind, division, feel, _] = [r.u8(), r.u8(), r.u8(), r.u8()];
    let value = within(get_f32(r), 0.0, 1.0e4, 0.0);
    match kind {
        1 => Time::Rows(value),
        2 => Time::Ticks(value),
        3 => Time::Seconds(value),
        _ => Time::Note(get_length(division, feel)),
    }
}

fn get_gate(r: &mut Reader) -> Gate {
    let [n, flags, division, feel] = [r.u8(), r.u8(), r.u8(), r.u8()];
    let (depth, smooth, swing) = (within(get_f32(r), 0.0, 1.0, 1.0), within(get_f32(r), 0.001, 0.5, 0.008), within(get_f32(r), 0.0, 0.75, 0.0));
    let levels = (0..n.min(32)).map(|_| within(get_f32(r), 0.0, 1.0, 0.0)).collect();
    Gate { levels, rate: get_length(division, feel), depth, smooth, swing, sends: flags & 1 != 0, stereo: flags & 2 != 0 }
}

/// A number from the file, kept only if it makes sense.
fn within(x: f32, min: f32, max: f32, or: f32) -> f32 {
    if x.is_finite() && x >= min {
        x.min(max)
    } else {
        or
    }
}

/// The chord after an event: as version 1 wrote it, or in full.
fn get_chord(r: &mut Reader, full: bool) -> Chord {
    let order = |n: u8, most: u8| match n.min(most) {
        1 => ArpOrder::Down,
        2 => ArpOrder::UpDown,
        3 => ArpOrder::Written,
        4 => ArpOrder::Random,
        5 => ArpOrder::UpDownEnds,
        6 => ArpOrder::Together,
        _ => ArpOrder::Up,
    };
    let mut chord = Chord::OFF;
    let mut arp = Arp::FAST;
    if full {
        let [count, is_arp, how, style] = [r.u8(), r.u8(), r.u8(), r.u8()];
        let [unit, division, feel, octaves] = [r.u8(), r.u8(), r.u8(), r.u8()];
        let rate = within(get_f32(r), 0.01, 1000.0, 1.0);
        arp.order = order(how, 6);
        arp.style = if style == 1 { ArpStyle::Strike } else { ArpStyle::Slide };
        arp.rate = match unit {
            1 => ArpRate::PerSecond(rate),
            2 => ArpRate::PerRow(rate.min(64.0)),
            3 => ArpRate::Note(NoteLength {
                division: if division.is_power_of_two() && division <= 64 { division } else { 16 },
                feel: [Feel::Plain, Feel::Triplet, Feel::Dotted][feel.min(2) as usize],
            }),
            _ => ArpRate::PerTick(rate.min(64.0)),
        };
        arp.octaves = octaves.clamp(1, 4);
        arp.gate = within(get_f32(r), 0.05, 1.0, Arp::FAST.gate);
        arp.swing = within(get_f32(r), 0.0, 0.75, 0.0);
        arp.accents = r.u32_le();
        if how == ORDER_PATTERN {
            (arp.order, arp.pattern, arp.accents) = (ArpOrder::Up, arp.accents.min(255) as u8, 0);
        }
        chord.count = count.min(MAX_CHORD as u8);
        chord.arp = (is_arp != 0 && count != 0).then_some(arp);
    } else {
        let [count, how, unit, _] = [r.u8(), r.u8(), r.u8(), r.u8()];
        let rate = get_f32(r);
        let rate = if rate.is_finite() && rate > 0.0 { rate.min(1000.0) } else { 1.0 };
        arp.order = order(how, 4);
        arp.rate = if unit == 1 { ArpRate::PerSecond(rate) } else { ArpRate::PerTick(rate) };
        chord.count = count.min(MAX_CHORD as u8);
        chord.arp = (count != 0).then_some(arp);
    }
    for slot in &mut chord.offsets {
        *slot = r.u8() as i8;
    }
    chord
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
            let extra = if has_chord & HAS_EXTRA != 0 { r.u16_le() } else { 0 };
            let has_chord = has_chord & 3;
            let chord = (has_chord != CHORD_NONE).then(|| get_chord(&mut r, has_chord == CHORD_NEW));
            pattern.events.push(Event { row, channel, cell, chord, extra });
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

    while r.left() >= 4 {
        let tag = r.take(4);
        if tag == SYNTHS {
            for ins in song.instruments.iter_mut() {
                let len = r.u32_le() as usize;
                let text = core::str::from_utf8(r.take(len)).unwrap_or("");
                ins.synth = (!text.is_empty()).then(|| text.to_string());
            }
        } else if tag == ARP_PATTERNS {
            // (Each list is bounded by what is left of the file.)
            for _ in 0..(r.u16_le() as usize).min(255).min(r.left()) {
                let len = (r.u8() as usize).min(crate::arp::MAX_PATTERN);
                song.arp_patterns.push(crate::arp::Pattern::from_bytes(r.take(len * 4)));
            }
        } else if tag == EXTRAS {
            for _ in 0..(r.u16_le() as usize).min(r.left()) {
                let mask = r.u8();
                let mut x = Extra::default();
                if mask & 1 != 0 {
                    x.glide = Some(get_time(&mut r));
                }
                if mask & 2 != 0 {
                    let shape = [BendShape::To, BendShape::Scoop, BendShape::Dip, BendShape::Slide][(r.u8() & 3) as usize];
                    r.skip(3);
                    x.bend = Some(Bend { shape, to: within(get_f32(&mut r), -96.0, 96.0, 0.0), over: get_time(&mut r) });
                }
                if mask & 4 != 0 {
                    x.vibrato = Some(Vibrato { depth: within(get_f32(&mut r), 0.0, 12.0, 0.0), rate: within(get_f32(&mut r), 0.0, 40.0, 6.0), delay: get_time(&mut r) });
                }
                if mask & 8 != 0 {
                    x.gate = Some(match r.u8() {
                        0 => GateChange::Off,
                        1 => GateChange::Instrument,
                        _ => GateChange::On(get_gate(&mut r)),
                    });
                }
                song.extras.push(x);
            }
        } else if tag == GATES {
            for ins in song.instruments.iter_mut() {
                if r.u8() != 0 {
                    ins.gate = Some(get_gate(&mut r));
                }
            }
        } else if tag == MIXER {
            song.volume = within(get_f32(&mut r), 0.0, 2.0, 1.0);
            for _ in 0..(r.u8() as usize).min(r.left() / 12) {
                let [volume, reverb, echo] = [get_f32(&mut r), get_f32(&mut r), get_f32(&mut r)];
                song.mix.push(ChannelMix { volume: within(volume, 0.0, 2.0, 1.0), reverb: within(reverb, 0.0, 1.0, 0.0), echo: within(echo, 0.0, 1.0, 0.0) });
            }
            let given = r.u16_le();
            for (i, (_, id)) in EFFECT_KEYS.iter().enumerate() {
                if given & (1 << i) != 0 {
                    let x = get_f32(&mut r);
                    // (Kept only if it is a number: the parameter's own range does the rest.)
                    song.effects.values[i] = crate::params::info(*id).filter(|_| x.is_finite()).map(|info| info.clean(x));
                }
            }
            if given & (1 << 11) != 0 {
                song.effects.echo_time = Some(get_time(&mut r));
            }
            let has = r.u8();
            if has & 1 != 0 {
                song.loop_start = Some((r.u16_le(), r.u16_le()));
            }
            if has & 2 != 0 {
                song.loop_end = Some((r.u16_le(), r.u16_le()));
            }
        } else {
            break;
        }
    }

    song.tidy();
    Ok(song)
}
