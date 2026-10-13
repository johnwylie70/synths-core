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

// The limits are spelled out in the reader's messages.
const _: () = assert!(MAX_ENVELOPE_POINTS == 12 && MAX_PATTERNS == 256 && MAX_ORDERS == 256 && MAX_SAMPLES == 4096 && MAX_INSTRUMENTS == 255);

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
    let bytes = text.as_bytes();
    let first = bytes.iter().find(|b| !b.is_ascii_whitespace());
    first == Some(&b'{') && bytes.windows(FORMAT.len()).any(|w| w == FORMAT.as_bytes())
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

/// The JSON being written: text, with shorthands for the pieces a song is
/// made of. (No `format!` here: see `text`.)
struct Out(String);

impl Out {
    fn s(&mut self, text: &str) -> &mut Out {
        self.0.push_str(text);
        self
    }

    fn n(&mut self, n: i64) -> &mut Out {
        text::push_int(&mut self.0, n);
        self
    }

    /// `, "key": ` (the comma is left out with `first`).
    fn key(&mut self, key: &str) -> &mut Out {
        self.s(", \"").s(key).s("\": ")
    }

    fn quoted(&mut self, text: &str) -> &mut Out {
        let q = quote(text);
        self.s(&q)
    }

    /// A list of whole numbers: `[1, 2, 3]`.
    fn list(&mut self, numbers: impl Iterator<Item = i64>) -> &mut Out {
        self.s("[");
        for (i, n) in numbers.enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.n(n);
        }
        self.s("]")
    }
}

fn write_envelope(o: &mut Out, key: &str, e: &Envelope) {
    if e.points.is_empty() {
        return;
    }
    o.key(key).s("{\"points\": [");
    for (i, (x, y)) in e.points.iter().enumerate() {
        o.s(if i > 0 { ", [" } else { "[" }).n(*x as i64).s(", ").n(*y as i64).s("]");
    }
    o.s("]");
    if let Some(p) = e.sustain {
        o.key("sustain").n(p as i64);
    }
    if let Some((a, b)) = e.loop_points {
        o.key("loop").list([a as i64, b as i64].into_iter());
    }
    if !e.enabled {
        o.key("on").s("false");
    }
    o.s("}");
}

fn write_instrument(o: &mut Out, i: &Instrument) {
    o.s("{\"name\": ").quoted(&i.name);
    if let Some(patch) = &i.patch {
        o.key("patch").quoted(patch);
    }
    if let Some(synth) = &i.synth {
        o.key("synth").s(synth);
    }
    let first = i.keymap.first().copied().unwrap_or(NO_SAMPLE);
    if !i.keymap.is_empty() && first != NO_SAMPLE && i.keymap.iter().all(|&k| k == first) {
        o.key("sample").n(first as i64);
    } else if !i.keymap.is_empty() {
        // Runs of keys that share a sample.
        o.key("zones").s("[");
        let mut from = 0;
        let mut any = false;
        while from < i.keymap.len() {
            let sample = i.keymap[from];
            let to = from + i.keymap[from..].iter().take_while(|&&k| k == sample).count() - 1;
            if sample != NO_SAMPLE {
                o.s(if any { ", " } else { "" });
                o.s("{\"from\": ").quoted(&note_name(from as u8 + 1));
                o.key("to").quoted(&note_name(to as u8 + 1));
                o.key("sample").n(sample as i64).s("}");
                any = true;
            }
            from = to + 1;
        }
        o.s("]");
    }
    write_envelope(o, "volumeEnvelope", &i.volume_envelope);
    write_envelope(o, "panEnvelope", &i.pan_envelope);
    if i.vibrato != AutoVibrato::default() {
        let v = i.vibrato;
        o.key("vibrato").s("{\"wave\": \"").s(VIBRATO_WAVES[(v.waveform & 3) as usize]).s("\"");
        o.key("sweep").n(v.sweep as i64).key("depth").n(v.depth as i64).key("rate").n(v.rate as i64).s("}");
    }
    if i.fadeout != 0 {
        o.key("fadeout").n(i.fadeout as i64);
    }
    o.s("}");
}

fn write_event(o: &mut Out, e: &Event) {
    let c = e.cell;
    o.s("{\"row\": ").n(e.row as i64).key("ch").n(e.channel as i64);
    if c.note != NOTE_NONE {
        o.key("note").quoted(&note_name(c.note));
    }
    if c.instrument != 0 {
        o.key("ins").n(c.instrument as i64);
    }
    match c.volume {
        0 => {}
        0x10..=0x50 => {
            o.key("vol").n(c.volume as i64 - 0x10);
        }
        0x60..=0xFF => {
            o.key("vol").s("\"");
            o.0.push(VOLUME_COMMANDS[(c.volume >> 4) as usize - 6]);
            text::push_hex(&mut o.0, (c.volume & 15) as u32, 1, true);
            o.s("\"");
        }
        // Not a volume or a command; kept so the song reads back the same.
        raw => {
            o.key("vol").s("\"#");
            text::push_hex(&mut o.0, raw as u32, 2, true);
            o.s("\"");
        }
    }
    if c.effect != 0 || c.param != 0 {
        o.key("fx").s("\"");
        o.0.push(effect_letter(c.effect));
        text::push_hex(&mut o.0, c.param as u32, 2, true);
        o.s("\"");
    }
    match e.chord {
        None => {}
        Some(chord) if chord.is_off() => {
            o.key("chord").s("\"off\"");
        }
        Some(chord) => {
            o.key("chord").list(chord.offsets().iter().map(|&x| x as i64));
            match chord.rate {
                ChordRate::PerTick(1.0) => {}
                ChordRate::PerTick(n) => {
                    o.key("chordSpeed");
                    text::push_number(&mut o.0, n as f64);
                }
                ChordRate::PerSecond(n) => {
                    o.key("chordHz");
                    text::push_number(&mut o.0, n as f64);
                }
            }
            if chord.order != ChordOrder::Up {
                let name = CHORD_ORDERS.iter().find(|(_, order)| *order == chord.order).map_or("up", |(n, _)| n);
                o.key("chordOrder").s("\"").s(name).s("\"");
            }
        }
    }
    o.s("}");
}

fn write_sample(o: &mut Out, s: &Sample, blobs: Option<&mut Vec<Blob>>) {
    o.s("{\"name\": ").quoted(&s.name);
    match &s.data {
        SampleData::Bits8(v) if v.len() <= INLINE_FRAMES => {
            o.key("wave").list(v.iter().map(|&x| x as i64));
        }
        data => {
            let rate = c4_rate(s.relative_note, s.finetune).round().clamp(1.0, 4.0e9) as u32;
            let file = wav::write(data, rate);
            match blobs {
                Some(blobs) => {
                    let name = text::join(&["samples/", &hex(&sha256(&file))[..16], ".wav"]);
                    o.key("file").quoted(&name);
                    if !blobs.iter().any(|b| b.name == name) {
                        blobs.push(Blob { name, data: file });
                    }
                }
                // All in one file: the WAV goes in as base64 text.
                None => {
                    o.key("data").quoted(&super::base64::encode(&file));
                }
            }
        }
    }
    if s.volume != 64 {
        o.key("volume").n(s.volume as i64);
    }
    if let Some(pan) = s.pan {
        o.key("pan").n(pan as i64);
    }
    if s.loop_kind != LoopKind::Off {
        o.key("loop").s("{\"start\": ").n(s.loop_start as i64).key("length").n(s.loop_len as i64);
        o.key("kind").s(if s.loop_kind == LoopKind::PingPong { "\"pingpong\"}" } else { "\"forward\"}" });
    }
    if s.relative_note != 0 || s.finetune != 0 {
        o.key("transpose").n(s.relative_note as i64).key("finetune").n(s.finetune as i64);
    }
    o.s("}");
}

/// Writes a song as JSON and sample files.
pub fn save(song: &Song) -> Saved {
    let mut blobs: Vec<Blob> = Vec::new();
    let json = write(song, Some(&mut blobs));
    Saved { json, blobs }
}

/// Writes a song as one JSON text with nothing beside it: the samples too
/// big to be lists of numbers go in as WAV files in base64 (`"data"`).
/// It is a third bigger than the files would be, and is one thing to keep.
pub fn save_in_one(song: &Song) -> String {
    write(song, None)
}

fn write(song: &Song, mut blobs: Option<&mut Vec<Blob>>) -> String {
    let mut out = Out(String::new());
    let o = &mut out;
    o.s("{\n  \"format\": \"").s(FORMAT).s("\",\n  \"version\": ").n(VERSION);
    o.s(",\n  \"title\": ").quoted(&song.title);
    o.s(",\n  \"rules\": ").s(if song.rules == Rules::ProTracker { "\"protracker\"" } else { "\"ft2\"" });
    o.s(",\n  \"slides\": ").s(if song.frequency_table == FrequencyTable::Amiga { "\"amiga\"" } else { "\"linear\"" });
    o.s(",\n  \"speed\": ").n(song.speed as i64).s(",\n  \"tempo\": ").n(song.tempo as i64);
    if song.restart != 0 {
        o.s(",\n  \"restart\": ").n(song.restart as i64);
    }
    o.s(",\n  \"channels\": ").n(song.channels() as i64);
    if song.channel_pan.iter().any(|&p| p != 128) {
        o.s(",\n  \"pan\": ").list(song.channel_pan.iter().map(|&p| p as i64));
    }
    o.s(",\n  \"order\": ").list(song.orders.iter().map(|&p| p as i64));

    o.s(",\n  \"patterns\": [");
    for (i, p) in song.patterns.iter().enumerate() {
        o.s(if i == 0 { "\n" } else { ",\n" });
        o.s("    {\"rows\": ").n(p.rows as i64).s(", \"events\": [");
        for (j, e) in p.events.iter().enumerate() {
            o.s(if j == 0 { "\n      " } else { ",\n      " });
            write_event(o, e);
        }
        o.s(if p.events.is_empty() { "]}" } else { "\n    ]}" });
    }
    o.s(if song.patterns.is_empty() { "]" } else { "\n  ]" });

    o.s(",\n  \"instruments\": [");
    for (i, ins) in song.instruments.iter().enumerate() {
        o.s(if i == 0 { "\n    " } else { ",\n    " });
        write_instrument(o, ins);
    }
    o.s(if song.instruments.is_empty() { "]" } else { "\n  ]" });

    o.s(",\n  \"samples\": [");
    for (i, s) in song.samples.iter().enumerate() {
        o.s(if i == 0 { "\n    " } else { ",\n    " });
        write_sample(o, s, blobs.as_deref_mut());
    }
    o.s(if song.samples.is_empty() { "]\n}\n" } else { "\n  ]\n}\n" });
    out.0
}

// ---------------------------------------------------------------- reading

#[inline(never)]
fn invalid_text<T>(what: String) -> Result<T, LoadError> {
    Err(LoadError::Invalid(what))
}

/// The error for a mistake in a song, kept out of line: there are dozens of
/// places that make one, and each would otherwise carry its own copy.
#[inline(never)]
fn bad(what: &str) -> LoadError {
    LoadError::Invalid(what.to_string())
}

fn invalid<T>(what: &str) -> Result<T, LoadError> {
    Err(bad(what))
}

/// A whole number under a key, within limits; `default` if the key is absent.
fn int(v: &Value, key: &str, min: i64, max: i64, default: i64) -> Result<i64, LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(n) => match n.as_i64() {
            Some(n) if (min..=max).contains(&n) => Ok(n),
            _ => invalid_text(text::join(&["\"", key, "\" must be a whole number from ", &text::int(min), " to ", &text::int(max)])),
        },
    }
}

fn text<'a>(v: &'a Value, key: &str) -> Result<Option<&'a str>, LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => invalid_text(text::join(&["\"", key, "\" must be text"])),
    }
}

fn list<'a>(v: &'a Value, key: &str) -> Result<&'a [Value], LoadError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => invalid_text(text::join(&["\"", key, "\" must be a list"])),
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
                return match (raw.len(), text::hex_value(raw.as_bytes())) {
                    (2, Some(n)) => Ok(n as u8),
                    _ => bad(),
                };
            }
            let &[command, digit] = s.as_bytes() else { return bad() };
            let index = VOLUME_COMMANDS.iter().position(|&c| c as u8 == command.to_ascii_uppercase());
            match (index, text::digit36(digit).filter(|&x| x < 16)) {
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
        Value::String(s) => {
            let &[effect, high, low] = s.as_bytes() else { return bad() };
            match (text::digit36(effect), text::hex_value(&[high, low])) {
                (Some(e), Some(p)) => Ok((e, p as u8)),
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
            _ => {
                let mut what = text::join(&["\"", key, "\" must be a number from "]);
                text::push_number(&mut what, min);
                what.push_str(" to ");
                text::push_number(&mut what, max);
                invalid_text(what)
            }
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
                    return invalid_text(text::join(&["the chord \"", s, "\" is not built on the event's note"]));
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
        return invalid("an envelope can have at most 12 points");
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
    let mut song = Song::default();
    fill(&v, blob, &mut song)?;
    song.tidy();
    Ok(song)
}

/// Reads the JSON into a song. The song is the caller's, so that the many
/// places a mistake can stop the reading need not each clear it up.
#[inline(never)]
fn fill(v: &Value, blob: &dyn Fn(&str) -> Option<Vec<u8>>, song: &mut Song) -> Result<(), LoadError> {
    if text(v, "format")? != Some(FORMAT) {
        return Err(LoadError::NotRecognised);
    }
    let version = int(v, "version", 1, i64::MAX, 1)?;
    if version > VERSION {
        return invalid_text(text::join(&["this song is format version ", &text::int(version), ", newer than this player reads"]));
    }
    song.title = text(v, "title")?.unwrap_or("").to_string();
    song.rules = match text(v, "rules")? {
        None | Some("ft2") => Rules::FastTracker2,
        Some("protracker") => Rules::ProTracker,
        Some(other) => return invalid_text(text::join(&["\"rules\" must be \"ft2\" or \"protracker\", not \"", other, "\""])),
    };
    song.frequency_table = match text(v, "slides")? {
        None | Some("linear") => FrequencyTable::Linear,
        Some("amiga") => FrequencyTable::Amiga,
        Some(other) => return invalid_text(text::join(&["\"slides\" must be \"linear\" or \"amiga\", not \"", other, "\""])),
    };
    song.speed = int(v, "speed", 1, 31, 6)? as u8;
    song.tempo = int(v, "tempo", 32, 255, 125)? as u16;
    let channels = int(v, "channels", 1, MAX_CHANNELS as i64, 4)? as usize;
    song.channel_pan = vec![128; channels];
    let pans = list(v, "pan")?;
    if pans.len() > channels {
        return invalid("\"pan\" has more entries than there are channels");
    }
    for (slot, p) in song.channel_pan.iter_mut().zip(pans) {
        match p.as_i64() {
            Some(p) if (0..=255).contains(&p) => *slot = p as u8,
            _ => return invalid("a channel's \"pan\" must be 0 (left) to 255 (right)"),
        }
    }

    let patterns = list(v, "patterns")?;
    if patterns.len() > MAX_PATTERNS {
        return invalid("a song can have at most 256 patterns");
    }
    for p in patterns {
        song.patterns.push(Pattern::new(int(p, "rows", 1, MAX_ROWS as i64, 64)? as u16));
        let Some(pattern) = song.patterns.last_mut() else { break };
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
    }

    let orders = list(v, "order")?;
    if orders.len() > MAX_ORDERS {
        return invalid("\"order\" can have at most 256 entries");
    }
    for o in orders {
        match o.as_i64() {
            Some(o) if (0..MAX_PATTERNS as i64).contains(&o) => song.orders.push(o as u16),
            _ => return invalid("\"order\" must be a list of pattern numbers"),
        }
    }
    song.restart = int(v, "restart", 0, (song.orders.len() as i64 - 1).max(0), 0)? as u16;

    let samples = list(v, "samples")?;
    if samples.len() > MAX_SAMPLES {
        return invalid("a song can have at most 4096 samples");
    }
    for s in samples {
        song.samples.push(Sample::default());
        let Some(sample) = song.samples.last_mut() else { break };
        sample.name = text(s, "name")?.unwrap_or("").to_string();
        let mut tuning = (0, 0);
        if let Some(file) = text(s, "file")? {
            let bytes = blob(file).ok_or_else(|| LoadError::MissingSample(file.to_string()))?;
            let (data, rate) = wav::read(&bytes)?;
            sample.data = data;
            tuning = tuning_for(rate as f64);
        } else if let Some(data) = text(s, "data")? {
            // The WAV file itself, as base64 text: a song all in one file.
            let bytes = super::base64::decode(data).ok_or_else(|| bad("a sample's \"data\" must be a WAV file as base64 text"))?;
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
                Some(other) => return invalid_text(text::join(&["a loop's \"kind\" must be \"forward\" or \"pingpong\", not \"", other, "\""])),
            };
        }
    }

    let instruments = list(v, "instruments")?;
    if instruments.len() > MAX_INSTRUMENTS {
        return invalid("a song can have at most 255 instruments");
    }
    let last_sample = song.samples.len() as i64 - 1;
    for i in instruments {
        song.instruments.push(Instrument::default());
        let Some(ins) = song.instruments.last_mut() else { break };
        ins.name = text(i, "name")?.unwrap_or("").to_string();
        ins.patch = text(i, "patch")?.map(str::to_string);
        // A host's own description of the synth sound: kept, not read.
        ins.synth = i.get("synth").filter(|v| **v != Value::Null).map(Value::to_text);
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
            if let Some(keys) = ins.keymap.get_mut(from as usize - 1..to as usize) {
                keys.fill(sample);
            }
        }
        ins.volume_envelope = read_envelope(i.get("volumeEnvelope"))?;
        ins.pan_envelope = read_envelope(i.get("panEnvelope"))?;
        if let Some(vib) = i.get("vibrato") {
            let waveform = match text(vib, "wave")? {
                None => 0,
                Some(name) => match VIBRATO_WAVES.iter().position(|&w| w == name) {
                    Some(w) => w as u8,
                    None => return invalid("a vibrato's \"wave\" must be \"sine\", \"square\", \"ramp-up\" or \"ramp-down\""),
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
    }

    Ok(())
}
