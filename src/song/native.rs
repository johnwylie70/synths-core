//! Our own song format: a JSON document, with the samples beside it as WAV
//! files named after what is in them.
//!
//! `docs/song-format.md` is the description of the format; this is the code
//! that reads and writes it. What is written reads back as exactly the same
//! song. The writer lays the JSON out by hand, one pattern event to a line,
//! so that a song can be read in a text editor and two versions compared
//! line by line.

use super::json::{self, quote, Value};
use super::sha256::{hex, sha256};
use super::*;

pub const FORMAT: &str = "synths-song";
pub const VERSION: i64 = 1;

/// A sample with this many frames or fewer, in 8 bits, is written into the
/// JSON itself as a list of numbers; anything bigger goes in a WAV file.
pub const INLINE_FRAMES: usize = 256;

/// A file that goes with a song's JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct Blob {
    /// Its path beside the JSON, such as `samples/0f3a9c1b5d7e2a44.wav`.
    pub name: String,
    pub data: Vec<u8>,
}

/// A saved song: its JSON and the sample files the JSON names.
#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub json: String,
    pub blobs: Vec<Blob>,
}

/// True if the text looks like a song in this format.
pub fn is_native(text: &str) -> bool {
    text.trim_start().starts_with('{') && text.contains(FORMAT)
}

// ---------------------------------------------------------------- writing

const VOLUME_COMMANDS: [char; 10] = ['-', '+', 'D', 'U', 'S', 'V', 'P', 'L', 'R', 'M'];
const VIBRATO_WAVES: [&str; 4] = ["sine", "square", "ramp-up", "ramp-down"];

const CHORD_ORDERS: [(&str, ChordOrder); 5] = [
    ("up", ChordOrder::Up),
    ("down", ChordOrder::Down),
    ("updown", ChordOrder::UpDown),
    ("written", ChordOrder::Written),
    ("random", ChordOrder::Random),
];

fn effect_letter(effect: u8) -> char {
    char::from_digit(effect as u32, 36).unwrap_or('0').to_ascii_uppercase()
}

/// The rate a sample plays at for C-4, given its tuning.
fn c4_rate(relative_note: i8, finetune: i8) -> f64 {
    C4_RATE * ((relative_note as f64 + finetune as f64 / 128.0) / 12.0).exp2()
}

/// The tuning that plays a sample recorded at `rate` at its own speed on C-4.
fn tuning_for(rate: f64) -> (i8, i8) {
    let semitones = 12.0 * (rate.max(1.0) / C4_RATE).log2();
    let whole = semitones.round();
    let fine = ((semitones - whole) * 128.0).round();
    (whole.clamp(-96.0, 96.0) as i8, fine.clamp(-128.0, 127.0) as i8)
}

fn write_envelope(e: &Envelope) -> String {
    let points: Vec<String> = e.points.iter().map(|(x, y)| format!("[{x}, {y}]")).collect();
    let mut s = format!("{{\"points\": [{}]", points.join(", "));
    if let Some(p) = e.sustain {
        s += &format!(", \"sustain\": {p}");
    }
    if let Some((a, b)) = e.loop_points {
        s += &format!(", \"loop\": [{a}, {b}]");
    }
    if !e.enabled {
        s += ", \"on\": false";
    }
    s + "}"
}

fn write_instrument(i: &Instrument) -> String {
    let mut s = format!("{{\"name\": {}", quote(&i.name));
    if let Some(patch) = &i.patch {
        s += &format!(", \"patch\": {}", quote(patch));
    }
    let first = i.keymap.first().copied().unwrap_or(NO_SAMPLE);
    if !i.keymap.is_empty() && first != NO_SAMPLE && i.keymap.iter().all(|&k| k == first) {
        s += &format!(", \"sample\": {first}");
    } else if !i.keymap.is_empty() {
        // Runs of keys that share a sample.
        let mut zones = Vec::new();
        let mut from = 0;
        while from < i.keymap.len() {
            let sample = i.keymap[from];
            let to = from + i.keymap[from..].iter().take_while(|&&k| k == sample).count() - 1;
            if sample != NO_SAMPLE {
                zones.push(format!(
                    "{{\"from\": {}, \"to\": {}, \"sample\": {sample}}}",
                    quote(&note_name(from as u8 + 1)),
                    quote(&note_name(to as u8 + 1))
                ));
            }
            from = to + 1;
        }
        s += &format!(", \"zones\": [{}]", zones.join(", "));
    }
    if !i.volume_envelope.points.is_empty() {
        s += &format!(", \"volumeEnvelope\": {}", write_envelope(&i.volume_envelope));
    }
    if !i.pan_envelope.points.is_empty() {
        s += &format!(", \"panEnvelope\": {}", write_envelope(&i.pan_envelope));
    }
    if i.vibrato != AutoVibrato::default() {
        let v = i.vibrato;
        s += &format!(
            ", \"vibrato\": {{\"wave\": \"{}\", \"sweep\": {}, \"depth\": {}, \"rate\": {}}}",
            VIBRATO_WAVES[(v.waveform & 3) as usize],
            v.sweep,
            v.depth,
            v.rate
        );
    }
    if i.fadeout != 0 {
        s += &format!(", \"fadeout\": {}", i.fadeout);
    }
    s + "}"
}

fn write_event(e: &Event) -> String {
    let c = e.cell;
    let mut s = format!("{{\"row\": {}, \"ch\": {}", e.row, e.channel);
    if c.note != NOTE_NONE {
        s += &format!(", \"note\": \"{}\"", note_name(c.note));
    }
    if c.instrument != 0 {
        s += &format!(", \"ins\": {}", c.instrument);
    }
    match c.volume {
        0 => {}
        0x10..=0x50 => s += &format!(", \"vol\": {}", c.volume - 0x10),
        0x60..=0xFF => {
            s += &format!(", \"vol\": \"{}{:X}\"", VOLUME_COMMANDS[(c.volume >> 4) as usize - 6], c.volume & 15)
        }
        // Not a volume or a command; kept so the song reads back the same.
        raw => s += &format!(", \"vol\": \"#{raw:02X}\""),
    }
    if c.effect != 0 || c.param != 0 {
        s += &format!(", \"fx\": \"{}{:02X}\"", effect_letter(c.effect), c.param);
    }
    match e.chord {
        None => {}
        Some(chord) if chord.is_off() => s += ", \"chord\": \"off\"",
        Some(chord) => {
            let offsets: Vec<String> = chord.offsets().iter().map(|o| o.to_string()).collect();
            s += &format!(", \"chord\": [{}]", offsets.join(", "));
            match chord.rate {
                ChordRate::PerTick(1.0) => {}
                ChordRate::PerTick(n) => s += &format!(", \"chordSpeed\": {n}"),
                ChordRate::PerSecond(n) => s += &format!(", \"chordHz\": {n}"),
            }
            if chord.order != ChordOrder::Up {
                let name = CHORD_ORDERS.iter().find(|(_, o)| *o == chord.order).map_or("up", |(n, _)| n);
                s += &format!(", \"chordOrder\": \"{name}\"");
            }
        }
    }
    s + "}"
}

/// Writes a song as JSON and sample files.
pub fn save(song: &Song) -> Saved {
    let mut blobs: Vec<Blob> = Vec::new();
    let mut o = String::new();
    o += "{\n";
    o += &format!("  \"format\": \"{FORMAT}\",\n  \"version\": {VERSION},\n");
    o += &format!("  \"title\": {},\n", quote(&song.title));
    o += &format!("  \"rules\": \"{}\",\n", if song.rules == Rules::ProTracker { "protracker" } else { "ft2" });
    let slides = if song.frequency_table == FrequencyTable::Amiga { "amiga" } else { "linear" };
    o += &format!("  \"slides\": \"{slides}\",\n");
    o += &format!("  \"speed\": {},\n  \"tempo\": {},\n", song.speed, song.tempo);
    if song.restart != 0 {
        o += &format!("  \"restart\": {},\n", song.restart);
    }
    o += &format!("  \"channels\": {},\n", song.channels());
    if song.channel_pan.iter().any(|&p| p != 128) {
        let pans: Vec<String> = song.channel_pan.iter().map(|p| p.to_string()).collect();
        o += &format!("  \"pan\": [{}],\n", pans.join(", "));
    }
    let orders: Vec<String> = song.orders.iter().map(|p| p.to_string()).collect();
    o += &format!("  \"order\": [{}],\n", orders.join(", "));

    o += "  \"patterns\": [";
    for (i, p) in song.patterns.iter().enumerate() {
        o += if i == 0 { "\n" } else { ",\n" };
        if p.events.is_empty() {
            o += &format!("    {{\"rows\": {}, \"events\": []}}", p.rows);
            continue;
        }
        o += &format!("    {{\"rows\": {}, \"events\": [\n", p.rows);
        let lines: Vec<String> = p.events.iter().map(|e| format!("      {}", write_event(e))).collect();
        o += &lines.join(",\n");
        o += "\n    ]}";
    }
    o += if song.patterns.is_empty() { "],\n" } else { "\n  ],\n" };

    o += "  \"instruments\": [";
    for (i, ins) in song.instruments.iter().enumerate() {
        o += if i == 0 { "\n    " } else { ",\n    " };
        o += &write_instrument(ins);
    }
    o += if song.instruments.is_empty() { "],\n" } else { "\n  ],\n" };

    o += "  \"samples\": [";
    for (i, s) in song.samples.iter().enumerate() {
        o += if i == 0 { "\n    " } else { ",\n    " };
        o += &format!("{{\"name\": {}", quote(&s.name));
        match &s.data {
            SampleData::Bits8(v) if v.len() <= INLINE_FRAMES => {
                let numbers: Vec<String> = v.iter().map(|x| x.to_string()).collect();
                o += &format!(", \"wave\": [{}]", numbers.join(", "));
            }
            data => {
                let rate = c4_rate(s.relative_note, s.finetune).round().clamp(1.0, 4.0e9) as u32;
                let file = wav::write(data, rate);
                let name = format!("samples/{}.wav", &hex(&sha256(&file))[..16]);
                o += &format!(", \"file\": {}", quote(&name));
                if !blobs.iter().any(|b| b.name == name) {
                    blobs.push(Blob { name, data: file });
                }
            }
        }
        if s.volume != 64 {
            o += &format!(", \"volume\": {}", s.volume);
        }
        if let Some(pan) = s.pan {
            o += &format!(", \"pan\": {pan}");
        }
        if s.loop_kind != LoopKind::Off {
            let kind = if s.loop_kind == LoopKind::PingPong { "pingpong" } else { "forward" };
            o += &format!(", \"loop\": {{\"start\": {}, \"length\": {}, \"kind\": \"{kind}\"}}", s.loop_start, s.loop_len);
        }
        if s.relative_note != 0 || s.finetune != 0 {
            o += &format!(", \"transpose\": {}, \"finetune\": {}", s.relative_note, s.finetune);
        }
        o += "}";
    }
    o += if song.samples.is_empty() { "]\n" } else { "\n  ]\n" };
    o += "}\n";
    Saved { json: o, blobs }
}

// ---------------------------------------------------------------- reading

fn invalid<T>(what: impl Into<String>) -> Result<T, LoadError> {
    Err(LoadError::Invalid(what.into()))
}

/// A whole number under a key, within limits; `default` if the key is absent.
fn int(v: &Value, key: &str, min: i64, max: i64, default: i64) -> Result<i64, LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(n) => match n.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(n),
            _ => invalid(format!("\"{key}\" must be a whole number from {min} to {max}")),
        },
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Result<Option<&'a str>, LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => invalid(format!("\"{key}\" must be text")),
    }
}

fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => invalid(format!("\"{key}\" must be a list")),
    }
}

/// A note written as a name ("C-4", "off") or a number.
fn read_note(v: &Value) -> Result<u8, LoadError> {
    let note = match v {
        Value::String(s) => parse_note(s),
        n => n.as_i64().filter(|n| (0..=NOTE_OFF as i64).contains(n)).map(|n| n as u8),
    };
    note.map_or_else(|| invalid("a note must be a name like \"C-4\" or \"off\", or a number from 0 to 97"), Ok)
}

fn read_volume(v: &Value) -> Result<u8, LoadError> {
    let bad = || invalid("\"vol\" must be 0 to 64, or a command like \"+2\" or \"P8\"");
    match v {
        Value::String(s) => {
            if let Some(raw) = s.strip_prefix('#') {
                // A raw byte, two hex digits.
                return u8::from_str_radix(raw, 16).or_else(|_| bad());
            }
            let &[command, digit] = s.as_bytes() else { return bad() };
            let index = VOLUME_COMMANDS.iter().position(|&c| c == (command as char).to_ascii_uppercase());
            match (index, (digit as char).to_digit(16)) {
                (Some(i), Some(x)) => Ok(((i as u8 + 6) << 4) | x as u8),
                _ => bad(),
            }
        }
        n => match n.as_i64() {
            Some(n) if (0..=64).contains(&n) => Ok(0x10 + n as u8),
            _ => bad(),
        },
    }
}

/// An effect as "A04" (a letter or digit, then two hex digits) or [10, 4].
fn read_effect(v: &Value) -> Result<(u8, u8), LoadError> {
    let bad = || invalid("\"fx\" must be an effect like \"A04\", or [number, number]");
    match v {
        Value::String(s) if s.len() == 3 && s.is_ascii() => {
            let effect = (s.as_bytes()[0] as char).to_digit(36);
            let param = u8::from_str_radix(&s[1..], 16);
            match (effect, param) {
                (Some(e), Ok(p)) => Ok((e as u8, p)),
                _ => bad(),
            }
        }
        Value::Array(a) if a.len() == 2 => match (a[0].as_i64(), a[1].as_i64()) {
            (Some(e), Some(p)) if (0..=fx::MAX as i64).contains(&e) && (0..=255).contains(&p) => Ok((e as u8, p as u8)),
            _ => bad(),
        },
        _ => bad(),
    }
}

/// A number under a key, within limits, if the key is there.
fn number(v: &Value, key: &str, min: f64, max: f64) -> Result<Option<f32>, LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => match n.as_f64() {
            Some(n) if n >= min && n <= max => Ok(Some(n as f32)),
            _ => invalid(format!("\"{key}\" must be a number from {min} to {max}")),
        },
    }
}

/// An event's chord: a name ("min", "Cm7"), a list of semitone offsets, a
/// list of note names, or "off". A name that begins with its root note
/// gives the event its note if it has none.
fn read_chord(e: &Value, cell: &mut Cell) -> Result<Option<Chord>, LoadError> {
    let Some(v) = e.get("chord") else { return Ok(None) };
    let bad = || invalid("\"chord\" must be a name like \"min\" or \"Cm7\", up to 8 offsets like [0, 3, 7], note names, or \"off\"");
    let mut chord = match v {
        Value::String(s) if s == "off" => return Ok(Some(Chord::OFF)),
        Value::String(s) => {
            let Some((root, offsets)) = Chord::named(s) else { return bad() };
            if let Some(root) = root {
                if cell.note == NOTE_NONE {
                    cell.note = 49 + root;
                } else if cell.note > NOTE_MAX || (cell.note - 1) % 12 != root {
                    return invalid(format!("the chord \"{s}\" is not built on the event's note"));
                }
            }
            Chord::new(offsets)
        }
        Value::Array(items) if !items.is_empty() && items.len() <= MAX_CHORD => {
            let mut offsets = Vec::with_capacity(items.len());
            for item in items {
                let offset = match item {
                    // A note name: how far it is from the event's own note.
                    Value::String(_) if (1..=NOTE_MAX).contains(&cell.note) => {
                        read_note(item).ok().filter(|n| (1..=NOTE_MAX).contains(n)).map(|n| n as i64 - cell.note as i64)
                    }
                    Value::String(_) => return invalid("a chord of note names needs the event to have a \"note\""),
                    n => n.as_i64(),
                };
                match offset {
                    Some(o) if (-96..=96).contains(&o) => offsets.push(o as i8),
                    _ => return bad(),
                }
            }
            Chord::new(&offsets)
        }
        _ => return bad(),
    };
    let speed = number(e, "chordSpeed", 0.01, 64.0)?;
    let hz = number(e, "chordHz", 0.1, 1000.0)?;
    chord.rate = match (speed, hz) {
        (Some(_), Some(_)) => return invalid("give \"chordSpeed\" (steps a tick) or \"chordHz\" (steps a second), not both"),
        (Some(n), None) => ChordRate::PerTick(n),
        (None, Some(n)) => ChordRate::PerSecond(n),
        (None, None) => ChordRate::PerTick(1.0),
    };
    if let Some(name) = text(e, "chordOrder")? {
        chord.order = match CHORD_ORDERS.iter().find(|(n, _)| *n == name) {
            Some((_, order)) => *order,
            None => return invalid("\"chordOrder\" must be \"up\", \"down\", \"updown\", \"written\" or \"random\""),
        };
    }
    Ok(Some(chord))
}

fn read_envelope(v: Option<&Value>) -> Result<Envelope, LoadError> {
    let Some(v) = v else { return Ok(Envelope::default()) };
    let mut e = Envelope { enabled: v.get("on") != Some(&Value::Bool(false)), ..Default::default() };
    for p in list(v, "points")? {
        match p.as_array().map(|p| (p.len(), p.first().and_then(Value::as_i64), p.get(1).and_then(Value::as_i64))) {
            Some((2, Some(x), Some(y))) if (0..=65535).contains(&x) && (0..=64).contains(&y) => {
                e.points.push((x as u16, y as u16))
            }
            _ => return invalid("an envelope point must be [tick, value from 0 to 64]"),
        }
    }
    if e.points.len() > MAX_ENVELOPE_POINTS {
        return invalid(format!("an envelope can have at most {MAX_ENVELOPE_POINTS} points"));
    }
    let last = e.points.len() as i64 - 1;
    if v.get("sustain").is_some() {
        e.sustain = Some(int(v, "sustain", 0, last, 0)? as u8);
    }
    let ends = list(v, "loop")?;
    if !ends.is_empty() {
        match (ends.len(), ends[0].as_i64(), ends.get(1).and_then(Value::as_i64)) {
            (2, Some(a), Some(b)) if 0 <= a && a <= b && b <= last => e.loop_points = Some((a as u8, b as u8)),
            _ => return invalid("an envelope's \"loop\" must be [first point, last point]"),
        }
    }
    Ok(e)
}

/// Reads a song. `blob` is asked for each sample file the song names, by
/// the path in the JSON, and gives the file's bytes if it has them.
pub fn load(source: &str, blob: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Song, LoadError> {
    let v = json::parse(source).map_err(LoadError::Invalid)?;
    if text(&v, "format")? != Some(FORMAT) {
        return Err(LoadError::NotRecognised);
    }
    let version = int(&v, "version", 1, i64::MAX, 1)?;
    if version > VERSION {
        return invalid(format!("this song is format version {version}, newer than this player ({VERSION})"));
    }
    let mut song = Song { title: text(&v, "title")?.unwrap_or("").to_string(), ..Default::default() };
    song.rules = match text(&v, "rules")? {
        None | Some("ft2") => Rules::FastTracker2,
        Some("protracker") => Rules::ProTracker,
        Some(other) => return invalid(format!("\"rules\" must be \"ft2\" or \"protracker\", not \"{other}\"")),
    };
    song.frequency_table = match text(&v, "slides")? {
        None | Some("linear") => FrequencyTable::Linear,
        Some("amiga") => FrequencyTable::Amiga,
        Some(other) => return invalid(format!("\"slides\" must be \"linear\" or \"amiga\", not \"{other}\"")),
    };
    song.speed = int(&v, "speed", 1, 31, 6)? as u8;
    song.tempo = int(&v, "tempo", 32, 255, 125)? as u16;
    let channels = int(&v, "channels", 1, MAX_CHANNELS as i64, 4)? as usize;
    song.channel_pan = vec![128; channels];
    let pans = list(&v, "pan")?;
    if pans.len() > channels {
        return invalid("\"pan\" has more entries than there are channels");
    }
    for (slot, p) in song.channel_pan.iter_mut().zip(pans) {
        match p.as_i64() {
            Some(p) if (0..=255).contains(&p) => *slot = p as u8,
            _ => return invalid("a channel's \"pan\" must be 0 (left) to 255 (right)"),
        }
    }

    let patterns = list(&v, "patterns")?;
    if patterns.len() > MAX_PATTERNS {
        return invalid(format!("a song can have at most {MAX_PATTERNS} patterns"));
    }
    for p in patterns {
        let mut pattern = Pattern::new(int(p, "rows", 1, MAX_ROWS as i64, 64)? as u16);
        for e in list(p, "events")? {
            let row = int(e, "row", 0, pattern.rows as i64 - 1, 0)? as u16;
            let channel = int(e, "ch", 0, channels as i64 - 1, 0)? as u8;
            let mut cell = Cell { instrument: int(e, "ins", 0, 255, 0)? as u8, ..Default::default() };
            if let Some(n) = e.get("note") {
                cell.note = read_note(n)?;
            }
            if let Some(vol) = e.get("vol") {
                cell.volume = read_volume(vol)?;
            }
            if let Some(effect) = e.get("fx") {
                (cell.effect, cell.param) = read_effect(effect)?;
            }
            let chord = read_chord(e, &mut cell)?;
            pattern.events.push(Event { row, channel, cell, chord });
        }
        song.patterns.push(pattern);
    }

    let orders = list(&v, "order")?;
    if orders.len() > MAX_ORDERS {
        return invalid(format!("\"order\" can have at most {MAX_ORDERS} entries"));
    }
    for o in orders {
        match o.as_i64() {
            Some(o) if (0..MAX_PATTERNS as i64).contains(&o) => song.orders.push(o as u16),
            _ => return invalid("\"order\" must be a list of pattern numbers"),
        }
    }
    song.restart = int(&v, "restart", 0, (song.orders.len() as i64 - 1).max(0), 0)? as u16;

    let samples = list(&v, "samples")?;
    if samples.len() > MAX_SAMPLES {
        return invalid(format!("a song can have at most {MAX_SAMPLES} samples"));
    }
    for s in samples {
        let mut sample = Sample { name: text(s, "name")?.unwrap_or("").to_string(), ..Default::default() };
        let mut tuning = (0, 0);
        if let Some(file) = text(s, "file")? {
            let bytes = blob(file).ok_or_else(|| LoadError::MissingSample(file.to_string()))?;
            let (data, rate) = wav::read(&bytes)?;
            sample.data = data;
            tuning = tuning_for(rate as f64);
        } else {
            let wave = list(s, "wave")?;
            let mut data = Vec::with_capacity(wave.len());
            for x in wave {
                match x.as_i64() {
                    Some(x) if (-128..=127).contains(&x) => data.push(x as i8),
                    _ => return invalid("a sample's \"wave\" must be a list of numbers from -128 to 127"),
                }
            }
            sample.data = SampleData::Bits8(data);
        }
        // A sample that gives neither plays at its recorded speed on C-4.
        if s.get("transpose").is_some() || s.get("finetune").is_some() {
            tuning = (int(s, "transpose", -96, 96, 0)? as i8, int(s, "finetune", -128, 127, 0)? as i8);
        }
        (sample.relative_note, sample.finetune) = tuning;
        sample.volume = int(s, "volume", 0, 64, 64)? as u8;
        if s.get("pan").is_some() {
            sample.pan = Some(int(s, "pan", 0, 255, 128)? as u8);
        }
        if let Some(l) = s.get("loop") {
            let frames = sample.data.len() as i64;
            let start = int(l, "start", 0, frames, 0)?;
            let length = int(l, "length", 0, frames - start, frames - start)?;
            sample.loop_start = start as u32;
            sample.loop_len = length as u32;
            sample.loop_kind = match text(l, "kind")? {
                None | Some("forward") => LoopKind::Forward,
                Some("pingpong") => LoopKind::PingPong,
                Some(other) => return invalid(format!("a loop's \"kind\" must be \"forward\" or \"pingpong\", not \"{other}\"")),
            };
        }
        song.samples.push(sample);
    }

    let instruments = list(&v, "instruments")?;
    if instruments.len() > MAX_INSTRUMENTS {
        return invalid(format!("a song can have at most {MAX_INSTRUMENTS} instruments"));
    }
    let last_sample = song.samples.len() as i64 - 1;
    for i in instruments {
        let mut ins = Instrument { name: text(i, "name")?.unwrap_or("").to_string(), ..Default::default() };
        ins.patch = text(i, "patch")?.map(str::to_string);
        if i.get("sample").is_some() {
            ins.keymap = vec![int(i, "sample", 0, last_sample, 0)? as u16; 96];
        }
        let zones = list(i, "zones")?;
        if !zones.is_empty() {
            ins.keymap.resize(96, NO_SAMPLE);
        }
        for z in zones {
            let from = z.get("from").map_or(Ok(1), read_note)?;
            let to = z.get("to").map_or(Ok(NOTE_MAX), read_note)?;
            if from == 0 || to > NOTE_MAX || from > to {
                return invalid("a zone must run \"from\" a note \"to\" a higher one");
            }
            let sample = int(z, "sample", 0, last_sample, 0)? as u16;
            ins.keymap[from as usize - 1..to as usize].fill(sample);
        }
        ins.volume_envelope = read_envelope(i.get("volumeEnvelope"))?;
        ins.pan_envelope = read_envelope(i.get("panEnvelope"))?;
        if let Some(vib) = i.get("vibrato") {
            let waveform = match text(vib, "wave")? {
                None => 0,
                Some(name) => match VIBRATO_WAVES.iter().position(|&w| w == name) {
                    Some(w) => w as u8,
                    None => return invalid(format!("a vibrato's \"wave\" must be one of {VIBRATO_WAVES:?}")),
                },
            };
            ins.vibrato = AutoVibrato {
                waveform,
                sweep: int(vib, "sweep", 0, 255, 0)? as u8,
                depth: int(vib, "depth", 0, 255, 0)? as u8,
                rate: int(vib, "rate", 0, 255, 0)? as u8,
            };
        }
        ins.fadeout = int(i, "fadeout", 0, 65535, 0)? as u16;
        song.instruments.push(ins);
    }

    song.tidy();
    Ok(song)
}
