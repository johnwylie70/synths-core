//! Songs: one in-memory form that tracker modules (MOD, XM) and our own song
//! format all load into, and that the replayer plays.
//!
//! A song is an order list of patterns. A pattern is a grid of rows and
//! channels, stored sparsely: only the cells that hold something. A cell
//! names a note, an instrument, a volume command and an effect, in the
//! FastTracker 2 numbering, which is a superset of ProTracker's.
//!
//! `docs/song-format.md` describes the JSON form of this model.

#[cfg(feature = "json")]
pub mod base64;
pub mod binary;
pub mod bytes;
#[cfg(feature = "json")]
pub mod json;
pub mod modfile;
#[cfg(feature = "json")]
pub mod native;
#[cfg(feature = "json")]
pub mod sha256;
pub mod text;
#[cfg(feature = "json")]
pub mod wav;
pub mod xm;

/// No note in this cell.
pub const NOTE_NONE: u8 = 0;
/// The highest note, B-7. Note 1 is C-0 and note 49 is C-4, which plays a
/// sample at 8363 frames a second when the sample is not transposed.
pub const NOTE_MAX: u8 = 96;
/// Key off: let go of the note.
pub const NOTE_OFF: u8 = 97;

/// The rate a sample plays at for C-4, in frames a second.
pub const C4_RATE: f64 = 8363.0;

/// A key in an instrument's map that has no sample.
pub const NO_SAMPLE: u16 = 0xFFFF;

/// The most channels a song can have.
pub const MAX_CHANNELS: usize = 64;
/// The most rows a pattern can have.
pub const MAX_ROWS: usize = 256;
/// The most patterns, orders and instruments a song can have.
pub const MAX_PATTERNS: usize = 256;
pub const MAX_ORDERS: usize = 256;
pub const MAX_INSTRUMENTS: usize = 255;
/// The most samples a song can have.
pub const MAX_SAMPLES: usize = 4096;
/// The most frames one sample can have (256 MB of 16-bit sound).
pub const MAX_SAMPLE_FRAMES: usize = 1 << 27;

/// Effect numbers: 0 to 15 are the ProTracker effects 0 to F, and the rest
/// are FastTracker 2's letters (G is 16, and so on).
pub mod fx {
    pub const ARPEGGIO: u8 = 0x0;
    pub const PORTA_UP: u8 = 0x1;
    pub const PORTA_DOWN: u8 = 0x2;
    pub const TONE_PORTA: u8 = 0x3;
    pub const VIBRATO: u8 = 0x4;
    pub const TONE_PORTA_VOL_SLIDE: u8 = 0x5;
    pub const VIBRATO_VOL_SLIDE: u8 = 0x6;
    pub const TREMOLO: u8 = 0x7;
    pub const SET_PAN: u8 = 0x8;
    pub const SAMPLE_OFFSET: u8 = 0x9;
    pub const VOL_SLIDE: u8 = 0xA;
    pub const POSITION_JUMP: u8 = 0xB;
    pub const SET_VOLUME: u8 = 0xC;
    pub const PATTERN_BREAK: u8 = 0xD;
    pub const EXTENDED: u8 = 0xE;
    pub const SPEED: u8 = 0xF;
    pub const GLOBAL_VOLUME: u8 = 16;
    pub const GLOBAL_VOL_SLIDE: u8 = 17;
    pub const KEY_OFF: u8 = 20;
    pub const ENVELOPE_POS: u8 = 21;
    pub const PAN_SLIDE: u8 = 25;
    pub const MULTI_RETRIG: u8 = 27;
    pub const TREMOR: u8 = 29;
    pub const EXTRA_FINE_PORTA: u8 = 33;
    /// The highest effect number (Z).
    pub const MAX: u8 = 35;
}

/// What one channel does on one row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// 0 for none, 1 to 96 for C-0 to B-7, 97 for key off.
    pub note: u8,
    /// 0 for none, otherwise an instrument number counted from 1.
    pub instrument: u8,
    /// The volume column, as FastTracker 2 stores it: 0 for nothing, 0x10 to
    /// 0x50 to set the volume to 0 to 64, and 0x60 upwards for its commands.
    pub volume: u8,
    /// The effect (`fx::...`).
    pub effect: u8,
    /// The effect's number.
    pub param: u8,
}

impl Cell {
    pub fn is_empty(&self) -> bool {
        *self == Cell::default()
    }
}

/// The most notes a chord can have.
pub const MAX_CHORD: usize = 8;

/// How fast an arpeggio's notes are stepped through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArpRate {
    /// So many steps in each tick of the song.
    PerTick(f32),
    /// So many steps a second, whatever the song's tempo.
    PerSecond(f32),
    /// So many steps in each row of the pattern.
    PerRow(f32),
    /// A step for each note of this length, counting four rows to the beat
    /// as trackers do: a sixteenth is a row.
    Note(NoteLength),
}

/// The length of a note: a half, a quarter, an eighth and so on, plain,
/// dotted (half as long again) or as one of a triplet (two thirds as long).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteLength {
    /// How many make a whole note: 1, 2, 4, 8, 16, 32 or 64.
    pub division: u8,
    pub feel: Feel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Feel {
    #[default]
    Plain,
    Triplet,
    Dotted,
}

impl NoteLength {
    /// How many rows the note lasts: a whole note is sixteen.
    pub fn rows(&self) -> f64 {
        let plain = 16.0 / self.division.max(1) as f64;
        match self.feel {
            Feel::Plain => plain,
            Feel::Triplet => plain * 2.0 / 3.0,
            Feel::Dotted => plain * 1.5,
        }
    }

    /// The note a text means: "1/16", "1/8t" (of a triplet), "1/4." (dotted).
    pub fn parse(text: &str) -> Option<NoteLength> {
        let rest = text::trim(text).strip_prefix("1/")?;
        let (number, feel) = match rest.as_bytes().last()? {
            b't' | b'T' => (&rest[..rest.len() - 1], Feel::Triplet),
            b'.' | b'd' => (&rest[..rest.len() - 1], Feel::Dotted),
            _ => (rest, Feel::Plain),
        };
        let division = match number {
            "1" => 1,
            "2" => 2,
            "4" => 4,
            "8" => 8,
            "16" => 16,
            "32" => 32,
            "64" => 64,
            _ => return None,
        };
        Some(NoteLength { division, feel })
    }

    /// The text `parse` reads.
    pub fn name(&self) -> String {
        let end = match self.feel {
            Feel::Plain => "",
            Feel::Triplet => "t",
            Feel::Dotted => ".",
        };
        text::join(&["1/", &text::int(self.division as i64), end])
    }
}

impl ArpRate {
    /// How many ticks a step lasts, in a song with `speed` ticks to the row
    /// at `tempo` beats a minute (a tick lasts 2.5 / tempo seconds).
    pub fn ticks(&self, speed: u16, tempo: u16) -> f64 {
        let ticks = match *self {
            ArpRate::PerTick(n) => 1.0 / n as f64,
            ArpRate::PerSecond(n) => tempo as f64 / (2.5 * n as f64),
            ArpRate::PerRow(n) => speed as f64 / n as f64,
            ArpRate::Note(length) => speed as f64 * length.rows(),
        };
        if ticks.is_finite() {
            ticks.clamp(1.0e-3, 1.0e6)
        } else {
            1.0
        }
    }
}

/// The order an arpeggio's notes are stepped through in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArpOrder {
    /// Lowest to highest, round and round.
    #[default]
    Up,
    Down,
    /// Up and then back down.
    UpDown,
    /// In the order the chord lists them.
    Written,
    Random,
    /// Up and then back down, with the top and the bottom note played twice.
    UpDownEnds,
    /// Every note at once on each step: a chord struck again and again.
    Together,
}

/// How an arpeggio moves from one note to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArpStyle {
    /// One voice, as the Commodore 64 played chords: the pitch jumps and
    /// the note's envelope carries on.
    #[default]
    Slide,
    /// Each step is a new note, struck afresh, as a player's arpeggio is.
    Strike,
}

/// How a chord's notes are played one at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arp {
    pub rate: ArpRate,
    pub order: ArpOrder,
    pub style: ArpStyle,
    /// Struck notes: how much of its step each is held, 0.05 to 1.
    pub gate: f32,
    /// How many octaves the pattern runs up through, 1 to 4.
    pub octaves: u8,
    /// How far every second step is put back, 0 to 0.75 of a step.
    pub swing: f32,
    /// How hard each step is struck: four bits a step, the first lowest, 15
    /// for full and 0 for a rest, as long as its last level that is not 0.
    /// 0 for every step at full.
    pub accents: u32,
}

impl Arp {
    /// What a `chord` meant before chords and arpeggios were told apart,
    /// and still the Commodore 64's chord: one voice, a step a tick, upwards.
    pub const FAST: Arp = Arp { rate: ArpRate::PerTick(1.0), order: ArpOrder::Up, style: ArpStyle::Slide, gate: 0.5, octaves: 1, swing: 0.0, accents: 0 };

    /// A player's arpeggio: a new note each sixteenth, upwards, held for
    /// half its step.
    pub const SIXTEENTHS: Arp = Arp { rate: ArpRate::Note(NoteLength { division: 16, feel: Feel::Plain }), style: ArpStyle::Strike, ..Arp::FAST };

    /// Is this one a voice can play by itself, as songs have always had
    /// it: one voice, timed in ticks or seconds, with nothing newer asked?
    /// (Then it is timed at the tempo of the row it begins on.)
    pub fn plain(&self) -> bool {
        self.style == ArpStyle::Slide
            && matches!(self.rate, ArpRate::PerTick(_) | ArpRate::PerSecond(_))
            && (self.order as u8) <= ArpOrder::Random as u8
            && self.octaves <= 1
            && self.swing == 0.0
    }

    /// The accents as a list: each step's level, 0 to 15.
    pub fn accent_list(&self) -> Vec<u8> {
        let len = 8 - self.accents.leading_zeros() as usize / 4;
        (0..len).map(|i| (self.accents >> (4 * i) & 15) as u8).collect()
    }
}

/// A chord: several notes on one note of a pattern, sounding together, or
/// with `arp` one at a time. It belongs to a note and carries on until the
/// channel's next note, or until an event switches it off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chord {
    /// Each note of the chord, in semitones from the note it is on; the note
    /// itself is 0 and has to be listed to be heard.
    pub offsets: [i8; MAX_CHORD],
    /// How many of `offsets` are used. None means "stop the arpeggio".
    pub count: u8,
    /// How the notes are played one at a time; `None` for all together.
    pub arp: Option<Arp>,
}

impl Chord {
    /// The event that stops a channel's arpeggio.
    pub const OFF: Chord = Chord { offsets: [0; MAX_CHORD], count: 0, arp: None };

    /// A chord of these offsets (at most 8 are kept), sounding together.
    pub fn new(offsets: &[i8]) -> Chord {
        let mut chord = Chord::OFF;
        for (slot, o) in chord.offsets.iter_mut().zip(offsets) {
            *slot = *o;
        }
        chord.count = offsets.len().min(MAX_CHORD) as u8;
        chord
    }

    /// A chord of these offsets, played one at a time.
    pub fn arp(offsets: &[i8], arp: Arp) -> Chord {
        Chord { arp: Some(arp), ..Chord::new(offsets) }
    }

    pub fn offsets(&self) -> &[i8] {
        &self.offsets[..(self.count as usize).min(MAX_CHORD)]
    }

    pub fn is_off(&self) -> bool {
        self.count == 0
    }

    /// The arpeggio that the trackers' effect 0xy plays: the note, x
    /// semitones up and y semitones up, a step a tick. For an editor turning
    /// an imported effect into an arpeggio; the replayer plays 0xy itself,
    /// tick by tick, because the old trackers' quirks do not fit a plain one.
    pub fn from_effect(param: u8) -> Chord {
        Chord::arp(&[0, (param >> 4) as i8, (param & 15) as i8], Arp { order: ArpOrder::Written, ..Arp::FAST })
    }

    /// The offsets a chord's name means: "maj", "min", "m7", "sus4" and so
    /// on. A name may begin with its root note ("Cm7", "F#maj7"), which is
    /// given back as a number from 0 (C) to 11 (B).
    pub fn named(name: &str) -> Option<(Option<u8>, &'static [i8])> {
        let name = text::trim(name);
        let b = name.as_bytes();
        let letter = match b.first().map(u8::to_ascii_uppercase) {
            Some(b'C') => Some(0i8),
            Some(b'D') => Some(2),
            Some(b'E') => Some(4),
            Some(b'F') => Some(5),
            Some(b'G') => Some(7),
            Some(b'A') => Some(9),
            Some(b'B') => Some(11),
            _ => None,
        };
        // A root is a capital letter: "dim" and "add9" are kinds, not notes.
        let letter = letter.filter(|_| b[0].is_ascii_uppercase());
        let (root, kind) = match letter {
            None => (None, name),
            Some(l) => match b.get(1) {
                Some(b'#') => (Some((l + 1).rem_euclid(12) as u8), name.get(2..).unwrap_or("")),
                Some(b'b') => (Some((l - 1).rem_euclid(12) as u8), name.get(2..).unwrap_or("")),
                _ => (Some(l as u8), name.get(1..).unwrap_or("")),
            },
        };
        let offsets: &'static [i8] = match kind {
            "" | "maj" | "major" | "M" => &[0, 4, 7],
            "m" | "min" | "minor" => &[0, 3, 7],
            "dim" => &[0, 3, 6],
            "aug" | "+" => &[0, 4, 8],
            "sus2" => &[0, 2, 7],
            "sus4" | "sus" => &[0, 5, 7],
            "5" => &[0, 7],
            "6" => &[0, 4, 7, 9],
            "m6" | "min6" => &[0, 3, 7, 9],
            "7" => &[0, 4, 7, 10],
            "maj7" | "M7" => &[0, 4, 7, 11],
            "m7" | "min7" => &[0, 3, 7, 10],
            "dim7" => &[0, 3, 6, 9],
            "m7b5" => &[0, 3, 6, 10],
            "9" => &[0, 4, 7, 10, 14],
            "maj9" | "M9" => &[0, 4, 7, 11, 14],
            "m9" | "min9" => &[0, 3, 7, 10, 14],
            "add9" => &[0, 4, 7, 14],
            "oct" | "octave" => &[0, 12],
            _ => return None,
        };
        // A bare kind is not allowed to be empty: "" names nothing.
        if root.is_none() && kind.is_empty() {
            return None;
        }
        Some((root, offsets))
    }
}

/// A cell and where it is in its pattern, with the chord or arpeggio it
/// starts or stops, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub row: u16,
    pub channel: u8,
    pub cell: Cell,
    pub chord: Option<Chord>,
}

impl Event {
    /// True if the event says nothing at all.
    pub fn is_empty(&self) -> bool {
        self.cell.is_empty() && self.chord.is_none()
    }
}

/// A pattern: so many rows, and the cells that are not empty, in order of
/// row and then channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pattern {
    pub rows: u16,
    pub events: Vec<Event>,
}

impl Pattern {
    pub fn new(rows: u16) -> Pattern {
        Pattern { rows, events: Vec::new() }
    }

    fn find(&self, row: u16, channel: u8) -> Result<usize, usize> {
        self.events.binary_search_by(|e| (e.row, e.channel).cmp(&(row, channel)))
    }

    /// Puts a cell in the pattern, replacing what was there; an empty cell
    /// removes it. A chord at that place stays.
    pub fn set(&mut self, row: u16, channel: u8, cell: Cell) {
        let chord = self.chord(row, channel);
        self.put(Event { row, channel, cell, chord });
    }

    /// Sets or removes the chord at a place, leaving its cell as it is.
    pub fn set_chord(&mut self, row: u16, channel: u8, chord: Option<Chord>) {
        let cell = self.cell(row, channel);
        self.put(Event { row, channel, cell, chord });
    }

    fn put(&mut self, event: Event) {
        match (self.find(event.row, event.channel), event.is_empty()) {
            (Ok(i), true) => {
                self.events.remove(i);
            }
            (Ok(i), false) => self.events[i] = event,
            (Err(_), true) => {}
            (Err(i), false) => self.events.insert(i, event),
        }
    }

    /// The chord at a row and channel, if one starts or stops there.
    pub fn chord(&self, row: u16, channel: u8) -> Option<Chord> {
        self.find(row, channel).ok().and_then(|i| self.events[i].chord)
    }

    /// The cell at a row and channel; empty if nothing is there.
    pub fn cell(&self, row: u16, channel: u8) -> Cell {
        self.find(row, channel).map(|i| self.events[i].cell).unwrap_or_default()
    }

    /// The events on one row.
    pub fn row(&self, row: u16) -> &[Event] {
        let a = self.events.partition_point(|e| e.row < row);
        let b = a + self.events[a..].partition_point(|e| e.row == row);
        &self.events[a..b]
    }

    /// Sorts the events and drops duplicates and anything outside the
    /// pattern, so the rest of the code can rely on the order.
    pub fn tidy(&mut self, channels: usize) {
        let rows = self.rows;
        self.events.retain(|e| e.row < rows && (e.channel as usize) < channels && !e.is_empty());
        sort_events(&mut self.events);
        self.events.dedup_by_key(|e| (e.row, e.channel));
    }
}

/// Puts events in order of row and then channel. A heap sort, written out
/// here because it is a fraction of the size of the standard library's
/// sort, and a pattern's events are nearly always in order already.
fn sort_events(events: &mut [Event]) {
    let key = |e: &Event| (e.row, e.channel);
    if events.windows(2).all(|w| key(&w[0]) <= key(&w[1])) {
        return;
    }
    fn sink(events: &mut [Event], mut at: usize, end: usize) {
        let key = |e: &Event| (e.row, e.channel);
        loop {
            let mut child = 2 * at + 1;
            if child >= end {
                return;
            }
            if child + 1 < end && key(&events[child]) < key(&events[child + 1]) {
                child += 1;
            }
            if key(&events[at]) >= key(&events[child]) {
                return;
            }
            events.swap(at, child);
            at = child;
        }
    }
    let n = events.len();
    for at in (0..n / 2).rev() {
        sink(events, at, n);
    }
    for end in (1..n).rev() {
        events.swap(0, end);
        sink(events, 0, end);
    }
}

/// How a sample repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopKind {
    #[default]
    Off,
    Forward,
    PingPong,
}

/// A sample's sound: one channel, 8 or 16 bits.
#[derive(Clone, Debug, PartialEq)]
pub enum SampleData {
    Bits8(Vec<i8>),
    Bits16(Vec<i16>),
}

impl Default for SampleData {
    fn default() -> Self {
        SampleData::Bits8(Vec::new())
    }
}

impl SampleData {
    /// How many frames.
    pub fn len(&self) -> usize {
        match self {
            SampleData::Bits8(v) => v.len(),
            SampleData::Bits16(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// One frame as a number from -1 to 1; 0 past the end.
    #[inline]
    pub fn get(&self, i: usize) -> f32 {
        match self {
            SampleData::Bits8(v) => v.get(i).map_or(0.0, |&x| x as f32 / 128.0),
            SampleData::Bits16(v) => v.get(i).map_or(0.0, |&x| x as f32 / 32768.0),
        }
    }

    /// The whole sample as numbers from -1 to 1.
    pub fn to_f32(&self) -> Vec<f32> {
        (0..self.len()).map(|i| self.get(i)).collect()
    }
}

/// A sample and how it is played.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sample {
    pub name: String,
    pub data: SampleData,
    /// The first frame of the loop.
    pub loop_start: u32,
    /// How many frames the loop has.
    pub loop_len: u32,
    pub loop_kind: LoopKind,
    /// 0 to 64.
    pub volume: u8,
    /// -128 to 127; 128 steps make a semitone.
    pub finetune: i8,
    /// Semitones to add to the note.
    pub relative_note: i8,
    /// 0 (left) to 255 (right); `None` to use the channel's own panning.
    pub pan: Option<u8>,
}

impl Sample {
    /// The loop the sample can actually play: `None` if it is switched off,
    /// empty or outside the sound.
    pub fn loop_range(&self) -> Option<(u32, u32)> {
        let len = self.data.len() as u64;
        let end = self.loop_start as u64 + self.loop_len as u64;
        if self.loop_kind == LoopKind::Off || self.loop_len == 0 || end > len {
            None
        } else {
            Some((self.loop_start, end as u32))
        }
    }

    /// Where playing stops or turns back: the end of the loop, or of the sound.
    pub fn play_end(&self) -> u32 {
        self.loop_range().map_or(self.data.len() as u32, |(_, end)| end)
    }
}

/// An envelope: a line through up to 12 points that shapes the volume or
/// the panning over a note's life. A point is (tick, value 0 to 64).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Envelope {
    pub enabled: bool,
    pub points: Vec<(u16, u16)>,
    /// The point the envelope waits at until the key is let go.
    pub sustain: Option<u8>,
    /// The points the envelope repeats between (start, end).
    pub loop_points: Option<(u8, u8)>,
}

/// The most points an envelope can have.
pub const MAX_ENVELOPE_POINTS: usize = 12;

/// An instrument's own slow pitch wobble.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutoVibrato {
    /// 0 sine, 1 square, 2 ramp up, 3 ramp down.
    pub waveform: u8,
    /// How many ticks the wobble takes to reach full depth; 0 for at once.
    pub sweep: u8,
    pub depth: u8,
    pub rate: u8,
}

/// An instrument: which sample each note plays, and how the note is shaped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Instrument {
    pub name: String,
    /// For each note from C-0 (96 entries), an index into the song's samples
    /// or `NO_SAMPLE`. Empty means no samples at all.
    pub keymap: Vec<u16>,
    pub volume_envelope: Envelope,
    pub pan_envelope: Envelope,
    pub vibrato: AutoVibrato,
    /// How fast the note fades after key off; 0 for never. A step of 32768
    /// would silence it in one tick.
    pub fadeout: u16,
    /// Set when the instrument is a synth patch and not samples: the name of
    /// the patch. The replayer hands it to the voices untouched.
    pub patch: Option<String>,
    /// What a host has written about the instrument's synth sound (the
    /// `synth` key of a song's JSON), as JSON text. The engine does not
    /// look inside it: it keeps it, and hands it back when asked.
    pub synth: Option<String>,
}

impl Instrument {
    /// An instrument that plays one sample on every key.
    pub fn of_sample(name: &str, sample: u16) -> Instrument {
        Instrument { name: name.to_string(), keymap: vec![sample; 96], ..Default::default() }
    }

    /// The sample for a note (1 to 96), if it has one.
    pub fn sample_for(&self, note: u8) -> Option<usize> {
        let k = *self.keymap.get(note.wrapping_sub(1) as usize)?;
        (k != NO_SAMPLE).then_some(k as usize)
    }
}

/// Whose rules the song is played by. The two trackers disagree on a great
/// many details, and songs were written to their tracker's behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rules {
    /// ProTracker 2 on the Amiga: 4-channel MOD files.
    ProTracker,
    /// FastTracker 2: XM files, MOD files with more channels, and our own songs.
    #[default]
    FastTracker2,
}

/// How a note becomes a frequency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrequencyTable {
    /// Slides move the pitch evenly, the same amount anywhere on the keyboard.
    #[default]
    Linear,
    /// Slides move the Amiga's period, so they are faster on high notes.
    Amiga,
}

/// A whole song.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub title: String,
    pub rules: Rules,
    pub frequency_table: FrequencyTable,
    /// Ticks in a row, to begin with.
    pub speed: u8,
    /// Beats a minute, to begin with: a tick lasts 2.5 / tempo seconds.
    pub tempo: u16,
    /// The order to go back to when the song runs off its end.
    pub restart: u16,
    /// Each channel's panning to begin with, 0 (left) to 255 (right). Its
    /// length is the number of channels.
    pub channel_pan: Vec<u8>,
    /// The patterns to play, in order.
    pub orders: Vec<u16>,
    pub patterns: Vec<Pattern>,
    /// Instrument 1 in a cell is `instruments[0]`.
    pub instruments: Vec<Instrument>,
    pub samples: Vec<Sample>,
}

impl Default for Song {
    fn default() -> Song {
        Song {
            title: String::new(),
            rules: Rules::FastTracker2,
            frequency_table: FrequencyTable::Linear,
            speed: 6,
            tempo: 125,
            restart: 0,
            channel_pan: vec![128; 4],
            orders: Vec::new(),
            patterns: Vec::new(),
            instruments: Vec::new(),
            samples: Vec::new(),
        }
    }
}

impl Song {
    pub fn channels(&self) -> usize {
        self.channel_pan.len()
    }

    /// The pattern at an order, if the order and the pattern exist.
    pub fn pattern_at(&self, order: usize) -> Option<&Pattern> {
        self.patterns.get(*self.orders.get(order)? as usize)
    }

    /// How many rows the pattern at an order has. A pattern that is missing
    /// plays as 64 empty rows, as the trackers do.
    pub fn rows_at(&self, order: usize) -> u16 {
        self.pattern_at(order).map_or(64, |p| p.rows.max(1))
    }

    /// The instrument with this number (counted from 1).
    pub fn instrument(&self, number: u8) -> Option<&Instrument> {
        self.instruments.get((number as usize).checked_sub(1)?)
    }

    /// Makes the song safe to play whatever it was loaded from: limits in
    /// range, events in order, and nothing pointing outside the song.
    pub fn tidy(&mut self) {
        self.channel_pan.truncate(MAX_CHANNELS);
        if self.channel_pan.is_empty() {
            self.channel_pan.push(128);
        }
        self.patterns.truncate(MAX_PATTERNS);
        self.orders.truncate(MAX_ORDERS);
        self.instruments.truncate(MAX_INSTRUMENTS);
        self.samples.truncate(MAX_SAMPLES);
        if self.orders.is_empty() {
            self.orders.push(0);
        }
        if self.restart as usize >= self.orders.len() {
            self.restart = 0;
        }
        self.tempo = self.tempo.clamp(32, 255);
        self.speed = self.speed.clamp(1, 31);
        let channels = self.channels();
        for p in &mut self.patterns {
            p.rows = p.rows.clamp(1, MAX_ROWS as u16);
            p.tidy(channels);
        }
        let samples = self.samples.len();
        for i in &mut self.instruments {
            for k in &mut i.keymap {
                if *k as usize >= samples {
                    *k = NO_SAMPLE;
                }
            }
            // A map with no sample on any key is the same as no map.
            if i.keymap.iter().all(|&k| k == NO_SAMPLE) {
                i.keymap.clear();
            } else {
                i.keymap.resize(96, NO_SAMPLE);
            }
            for e in [&mut i.volume_envelope, &mut i.pan_envelope] {
                e.points.truncate(MAX_ENVELOPE_POINTS);
                let n = e.points.len() as u8;
                if n == 0 {
                    e.enabled = false;
                }
                if e.sustain.is_some_and(|s| s >= n) {
                    e.sustain = None;
                }
                if e.loop_points.is_some_and(|(a, b)| a >= n || b >= n || a > b) {
                    e.loop_points = None;
                }
            }
        }
        for s in &mut self.samples {
            s.volume = s.volume.min(64);
            if s.loop_range().is_none() {
                s.loop_kind = LoopKind::Off;
                s.loop_start = 0;
                s.loop_len = 0;
            }
            if s.data.is_empty() {
                s.data = SampleData::default();
            }
        }
    }
}

/// Why a song could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The data is not a kind of song this code knows.
    NotRecognised,
    /// The file ends before its header does.
    TooShort,
    /// Something in the file makes no sense; the text says what.
    Invalid(String),
    /// A sample the song names was not supplied.
    MissingSample(String),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::NotRecognised => write!(f, "not a song file this player knows"),
            LoadError::TooShort => write!(f, "the file is too short to be a song"),
            LoadError::Invalid(what) => write!(f, "the song file is damaged: {what}"),
            LoadError::MissingSample(name) => write!(f, "the song needs a sample that was not given: {name}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Loads a tracker module or a packed song, working out which it is: a
/// packed song and an XM say so at the start; anything else is tried as a MOD.
pub fn load_module(data: &[u8]) -> Result<Song, LoadError> {
    if binary::is_binary(data) {
        binary::load(data)
    } else if xm::is_xm(data) {
        xm::load(data)
    } else {
        modfile::load(data)
    }
}

const NOTE_NAMES: [&str; 12] = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];

/// A note's name as trackers write it: "C-4", "F#2", "off", or "" for none.
pub fn note_name(note: u8) -> String {
    match note {
        NOTE_NONE => String::new(),
        NOTE_OFF => "off".to_string(),
        n if n <= NOTE_MAX => text::join(&[NOTE_NAMES[(n as usize - 1) % 12], &text::int(((n - 1) / 12) as i64)]),
        _ => String::new(),
    }
}

/// The note a name means; "C4" and "c-4" work too.
pub fn parse_note(name: &str) -> Option<u8> {
    let s = text::trim(name);
    if s.eq_ignore_ascii_case("off") || s == "===" {
        return Some(NOTE_OFF);
    }
    let b = s.as_bytes();
    let letter = b.first()?.to_ascii_uppercase();
    let mut semitone = match letter {
        b'C' => 0i32,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    let mut rest = &b[1..];
    match rest.first() {
        Some(b'#') => {
            semitone += 1;
            rest = &rest[1..];
        }
        Some(b'b') => {
            semitone -= 1;
            rest = &rest[1..];
        }
        Some(b'-') => rest = &rest[1..],
        _ => {}
    }
    if rest.len() != 1 || !rest[0].is_ascii_digit() {
        return None;
    }
    let n = (rest[0] - b'0') as i32 * 12 + semitone + 1;
    (1..=NOTE_MAX as i32).contains(&n).then_some(n as u8)
}
