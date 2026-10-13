//! Songs: one in-memory form that tracker modules (MOD, XM) and our own song
//! format all load into, and that the replayer plays.
//!
//! A song is an order list of patterns. A pattern is a grid of rows and
//! channels, stored sparsely: only the cells that hold something. A cell
//! names a note, an instrument, a volume command and an effect, in the
//! FastTracker 2 numbering, which is a superset of ProTracker's.
//!
//! `docs/song-format.md` describes the JSON form of this model.

pub mod bytes;
pub mod json;
pub mod modfile;
pub mod native;
pub mod sha256;
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

/// A cell and where it is in its pattern.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub row: u16,
    pub channel: u8,
    pub cell: Cell,
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
    /// removes it.
    pub fn set(&mut self, row: u16, channel: u8, cell: Cell) {
        match (self.find(row, channel), cell.is_empty()) {
            (Ok(i), true) => {
                self.events.remove(i);
            }
            (Ok(i), false) => self.events[i].cell = cell,
            (Err(_), true) => {}
            (Err(i), false) => self.events.insert(i, Event { row, channel, cell }),
        }
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
        self.events.retain(|e| e.row < rows && (e.channel as usize) < channels && !e.cell.is_empty());
        self.events.sort_by_key(|e| (e.row, e.channel));
        self.events.dedup_by_key(|e| (e.row, e.channel));
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

/// Loads a tracker module, working out which kind it is: XM first (it says
/// so at the start), then MOD.
pub fn load_module(data: &[u8]) -> Result<Song, LoadError> {
    if xm::is_xm(data) {
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
        n if n <= NOTE_MAX => format!("{}{}", NOTE_NAMES[(n as usize - 1) % 12], (n - 1) / 12),
        _ => String::new(),
    }
}

/// The note a name means; "C4" and "c-4" work too.
pub fn parse_note(name: &str) -> Option<u8> {
    let s = name.trim();
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
