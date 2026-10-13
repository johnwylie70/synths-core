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
/// The version this code reads up to. Version 2 tells a chord (its notes
/// together) from an arpeggio (one at a time); in version 1 a `chord` was
/// always the fast one-voice arpeggio.
pub const VERSION: i64 = 2;

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

/// The orders of an arpeggio. The first five are the ones `chordOrder` had.
const ARP_ORDERS: [(&str, ArpOrder); 7] = [
    ("up", ArpOrder::Up),
    ("down", ArpOrder::Down),
    ("updown", ArpOrder::UpDown),
    ("written", ArpOrder::Written),
    ("random", ArpOrder::Random),
    ("updown2", ArpOrder::UpDownEnds),
    ("together", ArpOrder::Together),
];

/// The style an arpeggio has when it does not say: notes struck when it is
/// timed as music is, in note lengths or rows; one voice whose pitch jumps
/// when it is timed in ticks or seconds, as a sound chip's is.
fn default_style(rate: ArpRate) -> ArpStyle {
    match rate {
        ArpRate::Note(_) | ArpRate::PerRow(_) => ArpStyle::Strike,
        ArpRate::PerTick(_) | ArpRate::PerSecond(_) => ArpStyle::Slide,
    }
}

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
    if let Some(gate) = &i.gate {
        o.key("gate");
        write_gate(o, gate);
    }
    o.s("}");
}

/// A time: a note length as text, or `{"rows": 2}`, `{"ticks": 3}`, `{"seconds": 0.5}`.
fn write_time(o: &mut Out, time: &Time) {
    let (key, n) = match *time {
        Time::Note(length) => {
            o.quoted(&length.name());
            return;
        }
        Time::Rows(n) => ("rows", n),
        Time::Ticks(n) => ("ticks", n),
        Time::Seconds(n) => ("seconds", n),
    };
    o.s("{\"").s(key).s("\": ");
    text::push_number(&mut o.0, n as f64);
    o.s("}");
}

fn write_bend(o: &mut Out, bend: &Bend) {
    let slide = bend.shape == BendShape::Slide;
    o.key(if slide { "slide" } else { "bend" });
    let named = [("scoop", Bend::SCOOP), ("fall", Bend::FALL), ("dip", Bend::DIP)].into_iter().find(|(_, b)| b == bend);
    if let Some((name, _)) = named {
        o.s("\"").s(name).s("\"");
    } else if slide && bend.over == Time::Rows(1.0) {
        text::push_number(&mut o.0, bend.to as f64);
    } else {
        o.s("{");
        match bend.shape {
            BendShape::Scoop => o.s("\"shape\": \"scoop\", "),
            BendShape::Dip => o.s("\"shape\": \"dip\", "),
            _ => o,
        };
        o.s("\"to\": ");
        text::push_number(&mut o.0, bend.to as f64);
        o.key("over");
        write_time(o, &bend.over);
        o.s("}");
    }
}

/// A gate: its pattern alone when everything else is as it is by default.
fn write_gate(o: &mut Out, g: &Gate) {
    let plain = Gate::new(&g.levels);
    let pattern = |o: &mut Out| match crate::gate::to_text(&g.levels) {
        Some(text) => {
            o.quoted(&text);
        }
        None => {
            o.s("[");
            for (i, level) in g.levels.iter().enumerate() {
                o.s(if i > 0 { ", " } else { "" });
                text::push_number(&mut o.0, *level as f64);
            }
            o.s("]");
        }
    };
    if *g == plain {
        return pattern(o);
    }
    o.s("{\"pattern\": ");
    pattern(o);
    let number = |o: &mut Out, key: &str, n: f32| {
        o.key(key);
        text::push_number(&mut o.0, n as f64);
    };
    if g.rate != plain.rate {
        o.key("rate").quoted(&g.rate.name());
    }
    if g.depth != plain.depth {
        number(o, "depth", g.depth);
    }
    if g.smooth != plain.smooth {
        number(o, "smooth", g.smooth);
    }
    if g.swing != 0.0 {
        number(o, "swing", g.swing);
    }
    if !g.sends {
        o.key("sends").s("false");
    }
    if g.stereo {
        o.key("stereo").s("true");
    }
    o.s("}");
}

/// An arpeggio's pattern: as its text when text can say it, or step by step.
fn write_arp_pattern(o: &mut Out, pattern: &crate::arp::Pattern) {
    use crate::arp::{STEP_SLIDE, STEP_TIE};
    if let Some(text) = pattern.to_text() {
        o.quoted(&text);
        return;
    }
    o.s("[");
    for (i, step) in pattern.steps().iter().enumerate() {
        o.s(if i > 0 { ", " } else { "" });
        if step.is_rest() && step.octave == 0 && step.shape == 0 && step.flags == 0 {
            o.s("\"-\"");
        } else if step.note == 0 && step.flags == STEP_TIE && step.octave == 0 && step.shape == 0 {
            o.s("\"_\"");
        } else {
            o.s("{\"note\": ").n(step.note as i64);
            if step.octave != 0 {
                o.key("octave").n(step.octave as i64);
            }
            if step.shape & 15 != 0 {
                o.key("level").n((step.shape & 15) as i64);
            }
            if step.shape >> 4 != 0 {
                o.key("gate");
                text::push_number(&mut o.0, (step.shape >> 4).min(10) as f64 / 10.0);
            }
            if step.flags & STEP_SLIDE != 0 {
                o.key("slide").s("true");
            }
            if step.flags & STEP_TIE != 0 {
                o.key("tie").s("true");
            }
            o.s("}");
        }
    }
    o.s("]");
}

fn write_event(o: &mut Out, e: &Event, song: &Song) {
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
    if c.effect == fx::TONE_PORTA {
        // A glide to the note, at the speed the channel last had or at this one.
        o.key("glide");
        if c.param == 0 {
            o.s("true");
        } else {
            o.n(c.param as i64);
        }
    } else if c.effect != 0 || c.param != 0 {
        o.key("fx").s("\"");
        o.0.push(effect_letter(c.effect));
        text::push_hex(&mut o.0, c.param as u32, 2, true);
        o.s("\"");
    }
    match e.chord {
        None => {}
        Some(chord) if chord.is_off() => {
            o.key("arp").s("\"off\"");
        }
        Some(Chord { arp: None, .. }) => {
            o.key("chord").list(e.chord.iter().flat_map(|c| c.offsets()).map(|&x| x as i64));
        }
        Some(chord @ Chord { arp: Some(arp), .. }) => {
            o.key("arp").s("{\"chord\": ").list(chord.offsets().iter().map(|&x| x as i64));
            let number = |o: &mut Out, key: &str, n: f32| {
                o.key(key);
                text::push_number(&mut o.0, n as f64);
            };
            match arp.rate {
                ArpRate::Note(_) if arp.rate == Arp::SIXTEENTHS.rate => {}
                ArpRate::Note(length) => {
                    o.key("rate").quoted(&length.name());
                }
                ArpRate::PerRow(n) => number(o, "perRow", n),
                ArpRate::PerTick(n) => number(o, "perTick", n),
                ArpRate::PerSecond(n) => number(o, "hz", n),
            }
            if arp.order != ArpOrder::Up {
                let name = ARP_ORDERS.iter().find(|(_, order)| *order == arp.order).map_or("up", |(n, _)| n);
                o.key("order").s("\"").s(name).s("\"");
            }
            if arp.style != default_style(arp.rate) {
                o.key("style").s(if arp.style == ArpStyle::Strike { "\"strike\"" } else { "\"slide\"" });
            }
            if arp.octaves != 1 {
                o.key("octaves").n(arp.octaves as i64);
            }
            if arp.gate != Arp::FAST.gate {
                number(o, "gate", arp.gate);
            }
            if arp.swing != 0.0 {
                number(o, "swing", arp.swing);
            }
            if arp.accents != 0 {
                o.key("accent").list(arp.accent_list().into_iter().map(|x| x as i64));
            }
            if let Some(pattern) = song.arp_pattern(&arp) {
                o.key("pattern");
                write_arp_pattern(o, pattern);
            }
            o.s("}");
        }
    }
    if let Some(x) = song.extra(e.extra) {
        if let Some(glide) = &x.glide {
            o.key("glide");
            write_time(o, glide);
        }
        if let Some(bend) = &x.bend {
            write_bend(o, bend);
        }
        if let Some(v) = &x.vibrato {
            o.key("vibrato");
            if *v == Vibrato::LEAD {
                o.s("true");
            } else if v.depth == 0.0 {
                o.s("false");
            } else {
                o.s("{\"depth\": ");
                text::push_number(&mut o.0, v.depth as f64);
                o.key("rate");
                text::push_number(&mut o.0, v.rate as f64);
                o.key("delay");
                write_time(o, &v.delay);
                o.s("}");
            }
        }
        match &x.gate {
            None => {}
            Some(GateChange::Off) => {
                o.key("gate").s("\"off\"");
            }
            Some(GateChange::Instrument) => {
                o.key("gate").s("\"instrument\"");
            }
            Some(GateChange::On(gate)) => {
                o.key("gate");
                write_gate(o, gate);
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
    // A song with no chord in it says nothing an older reader would get wrong.
    let chords = song.patterns.iter().any(|p| p.events.iter().any(|e| e.chord.is_some() || e.extra != 0)) || song.instruments.iter().any(|i| i.gate.is_some());
    o.s("{\n  \"format\": \"").s(FORMAT).s("\",\n  \"version\": ").n(if chords { VERSION } else { 1 });
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
            write_event(o, e, song);
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

/// The notes of a chord: a name ("min", "Cm7"), a list of semitone offsets
/// or a list of note names. A name that begins with its root note gives the
/// event its note if it has none.
fn read_notes(v: &Value, cell: &mut Cell) -> Result<Chord, LoadError> {
    let bad = || invalid("\"chord\" must be a name like \"min\" or \"Cm7\", up to 8 offsets like [0, 3, 7], note names, or \"off\"");
    match v {
        Value::String(s) => {
            let Some((root, offsets)) = Chord::named(s) else { return bad() };
            if let Some(root) = root {
                if cell.note == NOTE_NONE {
                    cell.note = 49 + root;
                } else if cell.note > NOTE_MAX || (cell.note - 1) % 12 != root {
                    return invalid_text(text::join(&["the chord \"", s, "\" is not built on the event's note"]));
                }
            }
            Ok(Chord::new(offsets))
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
            Ok(Chord::new(&offsets))
        }
        _ => bad(),
    }
}

fn read_order(name: &str, key: &str, known: &[(&str, ArpOrder)]) -> Result<ArpOrder, LoadError> {
    match known.iter().find(|(n, _)| *n == name) {
        Some((_, order)) => Ok(*order),
        None => {
            let mut what = text::join(&["\"", key, "\" must be"]);
            for (i, (n, _)) in known.iter().enumerate() {
                what.push_str(if i == 0 { " \"" } else if i + 1 == known.len() { " or \"" } else { ", \"" });
                what.push_str(n);
                what.push('"');
            }
            invalid_text(what)
        }
    }
}

/// An arpeggio as an event's `arp` has it: `{"chord": ..., "rate": "1/16", ...}`.
fn read_arp(v: &Value, cell: &mut Cell, patterns: &mut Vec<crate::arp::Pattern>) -> Result<Chord, LoadError> {
    if !matches!(v, Value::Object(_)) {
        return invalid("\"arp\" must be {\"chord\": ..., \"rate\": \"1/16\", ...} or \"off\"");
    }
    let Some(notes) = v.get("chord") else { return invalid("an \"arp\" needs a \"chord\": the notes it plays") };
    let mut chord = read_notes(notes, cell)?;
    let note = match v.get("rate") {
        None | Some(Value::Null) => None,
        Some(r) => match r.as_str().and_then(NoteLength::parse) {
            Some(length) => Some(ArpRate::Note(length)),
            None => return invalid("an arp's \"rate\" must be a note length: \"1/1\" to \"1/64\", with t for a triplet (\"1/8t\") or a dot for dotted (\"1/8.\")"),
        },
    };
    let rates = [
        note,
        number(v, "perRow", 0.01, 64.0)?.map(ArpRate::PerRow),
        number(v, "perTick", 0.01, 64.0)?.map(ArpRate::PerTick),
        number(v, "hz", 0.1, 1000.0)?.map(ArpRate::PerSecond),
    ];
    if rates.iter().flatten().count() > 1 {
        return invalid("an arp has one of \"rate\" (a note length), \"perRow\", \"perTick\" and \"hz\", not more");
    }
    let mut arp = Arp { rate: rates.into_iter().flatten().next().unwrap_or(Arp::SIXTEENTHS.rate), ..Arp::FAST };
    if let Some(name) = text(v, "order")? {
        arp.order = read_order(name, "order", &ARP_ORDERS)?;
    }
    arp.style = match text(v, "style")? {
        None => default_style(arp.rate),
        Some("slide") => ArpStyle::Slide,
        Some("strike") => ArpStyle::Strike,
        Some(_) => return invalid("an arp's \"style\" must be \"strike\" (each step a new note) or \"slide\" (one voice whose pitch jumps)"),
    };
    arp.octaves = int(v, "octaves", 1, 4, 1)? as u8;
    arp.gate = number(v, "gate", 0.05, 1.0)?.unwrap_or(Arp::FAST.gate);
    arp.swing = number(v, "swing", 0.0, 0.75)?.unwrap_or(0.0);
    let accents = list(v, "accent")?;
    for (i, level) in accents.iter().enumerate() {
        match level.as_i64() {
            Some(level) if i < 8 && (0..=15).contains(&level) && (level > 0 || i + 1 < accents.len()) => arp.accents |= (level as u32) << (4 * i),
            _ => return invalid("an arp's \"accent\" must be up to 8 levels from 0 (a rest) to 15 (full), the last not 0"),
        }
    }
    if let Some(p) = v.get("pattern").filter(|p| **p != Value::Null) {
        if v.get("order").is_some() || v.get("accent").is_some() {
            return invalid("an arp's \"pattern\" takes the place of its \"order\" and \"accent\": give the pattern alone");
        }
        let pattern = read_arp_pattern(p)?;
        let at = patterns.iter().position(|x| *x == pattern).unwrap_or_else(|| {
            patterns.push(pattern);
            patterns.len() - 1
        });
        if at >= 255 {
            return invalid("a song can have at most 255 different arp patterns");
        }
        arp.pattern = at as u8 + 1;
    }
    chord.arp = Some(arp);
    Ok(chord)
}

/// An arpeggio's pattern: its text (or the name of a built-in one), or a
/// list of steps, each a chord note's number, "-" (a rest), "_" (a tie) or
/// `{"note": 3, "octave": 1, "level": 15, "gate": 0.3, "slide": true}`.
fn read_arp_pattern(v: &Value) -> Result<crate::arp::Pattern, LoadError> {
    use crate::arp::{Pattern, PatternStep, MAX_PATTERN, PATTERN_HELP, STEP_SLIDE, STEP_TIE};
    let bad = || invalid_text(text::join(&["an arp's \"pattern\": ", PATTERN_HELP, "; or a list of steps"]));
    match v {
        Value::String(s) => Pattern::parse(s).or_else(|_| bad()),
        Value::Array(items) if !items.is_empty() && items.len() <= MAX_PATTERN => {
            let mut steps = Vec::with_capacity(items.len());
            for item in items {
                let mut step = PatternStep::default();
                match item {
                    Value::String(s) if s == "-" || s == "." => {}
                    Value::String(s) if s == "_" => step.flags = STEP_TIE,
                    Value::Object(_) => {
                        step.note = int(item, "note", -32, 32, 0)? as i8;
                        step.octave = int(item, "octave", -3, 3, 0)? as i8;
                        let gate = number(item, "gate", 0.1, 1.0)?.map_or(0, |g| (g * 10.0).round() as u8);
                        step.shape = int(item, "level", 0, 15, 0)? as u8 | gate << 4;
                        if item.get("slide") == Some(&Value::Bool(true)) {
                            step.flags |= STEP_SLIDE;
                        }
                        if item.get("tie") == Some(&Value::Bool(true)) {
                            step.flags |= STEP_TIE;
                        }
                    }
                    n => match n.as_i64() {
                        Some(n) if (-32..=32).contains(&n) => step.note = n as i8,
                        _ => return bad(),
                    },
                }
                steps.push(step);
            }
            Ok(Pattern::new(&steps))
        }
        _ => bad(),
    }
}

/// A length of time: a note length ("1/16", "1/8t", "1/4."), or
/// `{"rows": n}`, `{"ticks": n}` or `{"seconds": n}`.
fn read_time(v: &Value, key: &str) -> Result<Time, LoadError> {
    let bad = || invalid_text(text::join(&["\"", key, "\" takes a time: a note length like \"1/16\", \"1/8t\" or \"1/4.\", or {\"rows\": n}, {\"ticks\": n} or {\"seconds\": n}"]));
    match v {
        Value::String(s) => NoteLength::parse(s).map(Time::Note).ok_or(()).or_else(|_| bad()),
        Value::Object(fields) if fields.len() == 1 => match (number(v, "rows", 0.0, 1024.0)?, number(v, "ticks", 0.0, 8192.0)?, number(v, "seconds", 0.0, 600.0)?) {
            (Some(n), None, None) => Ok(Time::Rows(n)),
            (None, Some(n), None) => Ok(Time::Ticks(n)),
            (None, None, Some(n)) => Ok(Time::Seconds(n)),
            _ => bad(),
        },
        _ => bad(),
    }
}

/// How far a bend goes: semitones, or a note name measured from the event's note.
fn read_bend_to(v: &Value, cell: &Cell) -> Result<f32, LoadError> {
    match v {
        Value::String(_) if (1..=NOTE_MAX).contains(&cell.note) => match read_note(v) {
            Ok(n) if (1..=NOTE_MAX).contains(&n) => Ok(n as f32 - cell.note as f32),
            _ => invalid("a bend's \"to\" must be semitones (a number from -96 to 96) or a note name"),
        },
        Value::String(_) => invalid("a bend to a note name needs the event to have a \"note\""),
        n => match n.as_f64() {
            Some(n) if (-96.0..=96.0).contains(&n) => Ok(n as f32),
            _ => invalid("a bend's \"to\" must be semitones (a number from -96 to 96) or a note name"),
        },
    }
}

/// A gate: its pattern as text (or a built-in pattern's name), or
/// `{"pattern": ..., "rate": "1/16", "depth": 1, "smooth": 0.008, "swing": 0, "sends": true, "stereo": false}`.
fn read_gate(v: &Value) -> Result<Gate, LoadError> {
    let bad = || invalid_text(text::join(&["a \"gate\": ", crate::gate::GATE_HELP, "; or a list of levels from 0 to 1"]));
    let pattern = |p: &Value| -> Result<Gate, LoadError> {
        match p {
            Value::String(s) => crate::gate::parse(s).map(|(levels, n)| Gate::new(&levels[..n])).or_else(|_| bad()),
            Value::Array(items) if !items.is_empty() && items.len() <= 32 => {
                let mut levels = Vec::with_capacity(items.len());
                for item in items {
                    match item.as_f64() {
                        Some(x) if (0.0..=1.0).contains(&x) => levels.push(x as f32),
                        _ => return bad(),
                    }
                }
                Ok(Gate::new(&levels))
            }
            _ => bad(),
        }
    };
    let Value::Object(_) = v else { return pattern(v) };
    let Some(p) = v.get("pattern") else { return invalid("a \"gate\" needs a \"pattern\"") };
    let mut gate = pattern(p)?;
    if let Some(rate) = v.get("rate") {
        gate.rate = match rate.as_str().and_then(NoteLength::parse) {
            Some(length) => length,
            None => return invalid("a gate's \"rate\" must be a note length: \"1/1\" to \"1/64\", with t for a triplet or a dot for dotted"),
        };
    }
    gate.depth = number(v, "depth", 0.0, 1.0)?.unwrap_or(1.0);
    gate.smooth = match v.get("smooth") {
        Some(Value::String(s)) if s == "hard" => 0.001,
        _ => number(v, "smooth", 0.001, 0.5)?.unwrap_or(0.008),
    };
    gate.swing = number(v, "swing", 0.0, 0.75)?.unwrap_or(0.0);
    gate.sends = v.get("sends") != Some(&Value::Bool(false));
    gate.stereo = v.get("stereo") == Some(&Value::Bool(true));
    Ok(gate)
}

/// What else an event says: `glide`, `bend`, `slide`, `vibrato`, `gate`.
/// A plain glide (true, or a tracker's speed) is the cell's own effect.
fn read_extra(e: &Value, cell: &mut Cell) -> Result<Extra, LoadError> {
    let mut x = Extra::default();
    match e.get("glide") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(v @ (Value::Bool(true) | Value::Number(_))) => {
            let speed = match v.as_i64() {
                None => 0,
                Some(n) if (1..=255).contains(&n) => n as u8,
                Some(_) => return invalid("\"glide\" as a number is a tracker's glide speed, 1 to 255; or give true, or a time like \"1/16\""),
            };
            if cell.effect != 0 || cell.param != 0 {
                return invalid("an event has \"glide\": true (or a speed) or an \"fx\", not both: that glide is the effect 3xx");
            }
            (cell.effect, cell.param) = (fx::TONE_PORTA, speed);
        }
        Some(v) => x.glide = Some(read_time(v, "glide")?),
    }
    let shaped = |shape: &str| match shape {
        "scoop" => Some(Bend::SCOOP),
        "fall" => Some(Bend::FALL),
        "dip" => Some(Bend::DIP),
        "to" => Some(Bend { shape: BendShape::To, to: 0.0, over: Bend::FALL.over }),
        _ => None,
    };
    let no_shape = || invalid("a \"bend\" is \"scoop\", \"fall\" or \"dip\", or {\"to\": semitones or a note, \"over\": a time}, with \"shape\" if it is a scoop or a dip");
    match e.get("bend") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) => x.bend = Some(shaped(s).filter(|_| s != "to").map_or_else(no_shape, Ok)?),
        Some(v @ Value::Object(_)) => {
            let mut bend = match v.get("shape") {
                None => Bend { shape: BendShape::To, to: f32::NAN, over: Bend::FALL.over },
                Some(shape) => shape.as_str().and_then(shaped).map_or_else(no_shape, Ok)?,
            };
            if let Some(to) = v.get("to") {
                bend.to = read_bend_to(to, cell)?;
            }
            if bend.to.is_nan() {
                return invalid("a \"bend\" needs a \"to\": how far, in semitones, or the note to bend to");
            }
            if let Some(over) = v.get("over") {
                bend.over = read_time(over, "over")?;
            }
            x.bend = Some(bend);
        }
        Some(_) => return no_shape().map(|_| x),
    }
    match e.get("slide") {
        None | Some(Value::Null) => {}
        Some(_) if x.bend.is_some() => return invalid("an event has a \"bend\" or a \"slide\", not both"),
        Some(v @ Value::Object(_)) => {
            let Some(to) = v.get("to") else { return invalid("a \"slide\" needs a \"to\": how far, in semitones") };
            let over = v.get("over").map_or(Ok(Time::Rows(1.0)), |t| read_time(t, "over"))?;
            x.bend = Some(Bend { shape: BendShape::Slide, to: read_bend_to(to, cell)?, over });
        }
        Some(v) => x.bend = Some(Bend { shape: BendShape::Slide, to: read_bend_to(v, cell)?, over: Time::Rows(1.0) }),
    }
    match e.get("vibrato") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(on)) => x.vibrato = Some(if *on { Vibrato::LEAD } else { Vibrato::OFF }),
        Some(v @ Value::Object(_)) => {
            let delay = v.get("delay").map_or(Ok(Vibrato::LEAD.delay), |t| read_time(t, "delay"))?;
            x.vibrato = Some(Vibrato { depth: number(v, "depth", 0.0, 12.0)?.unwrap_or(Vibrato::LEAD.depth), rate: number(v, "rate", 0.1, 40.0)?.unwrap_or(Vibrato::LEAD.rate), delay });
        }
        Some(_) => return invalid("an event's \"vibrato\" is true, false, or {\"depth\": semitones, \"rate\": times a second, \"delay\": a time}"),
    }
    match e.get("gate") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "off" => x.gate = Some(GateChange::Off),
        Some(Value::String(s)) if s == "instrument" => x.gate = Some(GateChange::Instrument),
        Some(v) => x.gate = Some(GateChange::On(read_gate(v)?)),
    }
    Ok(x)
}

/// What an event says of chords: `chord`, its notes together; `arp`, its
/// notes one at a time; or, from a song written before the two were told
/// apart (version 1, or `chordSpeed`, `chordHz` or `chordOrder` beside the
/// chord), a `chord` that is the fast one-voice arpeggio.
fn read_chord(e: &Value, cell: &mut Cell, version: i64, patterns: &mut Vec<crate::arp::Pattern>) -> Result<Option<Chord>, LoadError> {
    if e.get("arp").is_none() && e.get("chord").is_none() {
        return Ok(None);
    }
    let (speed, hz, order) = (number(e, "chordSpeed", 0.01, 64.0)?, number(e, "chordHz", 0.1, 1000.0)?, text(e, "chordOrder")?);
    let old = speed.is_some() || hz.is_some() || order.is_some();
    let off = |v: &Value| v.as_str() == Some("off");
    match (e.get("arp"), e.get("chord")) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => invalid("an event has a \"chord\" (its notes together) or an \"arp\" (one at a time), not both: an arp's notes are its own \"chord\""),
        (Some(_), None) if old => invalid("\"chordSpeed\", \"chordHz\" and \"chordOrder\" go with \"chord\"; an \"arp\" has \"perTick\", \"hz\" and \"order\" of its own"),
        (Some(v), None) | (None, Some(v)) if off(v) => Ok(Some(Chord::OFF)),
        (Some(v), None) => read_arp(v, cell, patterns).map(Some),
        (None, Some(v)) => {
            let mut chord = read_notes(v, cell)?;
            if old || version < 2 {
                let mut arp = Arp::FAST;
                arp.rate = match (speed, hz) {
                    (Some(_), Some(_)) => return invalid("give \"chordSpeed\" (steps a tick) or \"chordHz\" (steps a second), not both"),
                    (Some(n), None) => ArpRate::PerTick(n),
                    (None, Some(n)) => ArpRate::PerSecond(n),
                    (None, None) => ArpRate::PerTick(1.0),
                };
                if let Some(name) = order {
                    arp.order = read_order(name, "chordOrder", &ARP_ORDERS[..5])?;
                }
                chord.arp = Some(arp);
            }
            Ok(Some(chord))
        }
    }
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

    let (mut arp_patterns, mut extras) = (Vec::new(), Vec::new());
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
            let chord = read_chord(e, &mut cell, version, &mut arp_patterns)?;
            let x = read_extra(e, &mut cell)?;
            let extra = if x.is_empty() {
                0
            } else {
                let at = extras.iter().position(|y| *y == x).unwrap_or_else(|| {
                    extras.push(x);
                    extras.len() - 1
                });
                if at >= u16::MAX as usize {
                    return invalid("a song can have at most 65535 different glides, bends, vibratos and gates");
                }
                at as u16 + 1
            };
            pattern.events.push(Event { row, channel, cell, chord, extra });
        }
    }

    song.arp_patterns = arp_patterns;
    song.extras = extras;

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
        if let Some(gate) = i.get("gate").filter(|g| **g != Value::Null) {
            ins.gate = Some(read_gate(gate)?);
        }
    }

    Ok(())
}
