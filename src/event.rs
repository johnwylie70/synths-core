//! Events: the timestamped commands a host sends the engine, and the queue
//! that holds them until their moment.
//!
//! An event is a fixed 32-byte record; `docs/engine-api.md` gives its layout
//! byte by byte.

/// The size of one event record in bytes.
pub const EVENT_SIZE: usize = 32;

pub const EV_NOTE_ON: u16 = 1;
pub const EV_NOTE_OFF: u16 = 2;
pub const EV_ALL_NOTES_OFF: u16 = 3;
pub const EV_PARAM: u16 = 4;
pub const EV_PITCH_BEND: u16 = 5;
pub const EV_VOICE_PITCH: u16 = 6;
pub const EV_VOICE_FREQ: u16 = 7;
pub const EV_VOICE_VOLUME: u16 = 8;
pub const EV_VOICE_PAN: u16 = 9;
pub const EV_VOICE_POSITION: u16 = 10;
pub const EV_VOICE_KILL: u16 = 11;
pub const EV_MACRO_TICK: u16 = 12;

/// The part number that means "every part".
pub const ALL_PARTS: u16 = 0xFFFF;

/// Every event type: number, name, and what its fields mean.
pub const EVENT_TYPES: &[(u16, &str, &str)] = &[
    (EV_NOTE_ON, "note_on", "part; a: handle (0 for none); x: note; y: velocity 0..1"),
    (EV_NOTE_OFF, "note_off", "part; a: handle, or 0 to match by note; x: note"),
    (EV_ALL_NOTES_OFF, "all_notes_off", "part (65535: every part); a: 0 release, 1 cut at once"),
    (EV_PARAM, "param", "part (65535: every part); a: parameter id; x: value; y: glide seconds"),
    (EV_PITCH_BEND, "pitch_bend", "part; x: bend -1..1; y: glide seconds"),
    (EV_VOICE_PITCH, "voice_pitch", "part; a: handle; x: note; y: glide seconds"),
    (EV_VOICE_FREQ, "voice_freq", "part; a: handle; x: frequency in Hz; y: glide seconds"),
    (EV_VOICE_VOLUME, "voice_volume", "part; a: handle; x: volume 0..4"),
    (EV_VOICE_PAN, "voice_pan", "part; a: handle; x: pan -1..1, added to the part's pan"),
    (EV_VOICE_POSITION, "voice_position", "part; a: handle; x: sample position in frames"),
    (EV_VOICE_KILL, "voice_kill", "part; a: handle"),
    (EV_MACRO_TICK, "macro_tick", "part (65535: every part): step the macros of parts with macro.sync on"),
];

/// One event, exactly as it is laid out in memory (little-endian).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Event {
    /// When: frames since the engine was created. Anything in the past
    /// happens at the start of the next render.
    pub time: f64,
    /// Which sort of event (`EV_...`).
    pub kind: u16,
    /// Which part, from 0; `ALL_PARTS` where the event allows it.
    pub part: u16,
    /// A whole-number argument: a voice handle or a parameter id.
    pub a: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Not used yet; write 0.
    pub b: u32,
}

const _: () = assert!(core::mem::size_of::<Event>() == EVENT_SIZE);

impl Event {
    fn new(time: f64, kind: u16, part: u16, a: u32, x: f32, y: f32) -> Event {
        Event { time, kind, part, a, x, y, z: 0.0, b: 0 }
    }
    pub fn note_on(time: f64, part: u16, handle: u32, note: f32, velocity: f32) -> Event {
        Event::new(time, EV_NOTE_ON, part, handle, note, velocity)
    }
    pub fn note_off(time: f64, part: u16, handle: u32, note: f32) -> Event {
        Event::new(time, EV_NOTE_OFF, part, handle, note, 0.0)
    }
    pub fn all_notes_off(time: f64, part: u16, cut: bool) -> Event {
        Event::new(time, EV_ALL_NOTES_OFF, part, cut as u32, 0.0, 0.0)
    }
    pub fn param(time: f64, part: u16, id: usize, value: f32, glide: f32) -> Event {
        Event::new(time, EV_PARAM, part, id as u32, value, glide)
    }
    pub fn pitch_bend(time: f64, part: u16, bend: f32) -> Event {
        Event::new(time, EV_PITCH_BEND, part, 0, bend, 0.0)
    }
    pub fn voice_pitch(time: f64, part: u16, handle: u32, note: f32, glide: f32) -> Event {
        Event::new(time, EV_VOICE_PITCH, part, handle, note, glide)
    }
    pub fn voice_freq(time: f64, part: u16, handle: u32, hz: f32, glide: f32) -> Event {
        Event::new(time, EV_VOICE_FREQ, part, handle, hz, glide)
    }
    pub fn voice_volume(time: f64, part: u16, handle: u32, volume: f32) -> Event {
        Event::new(time, EV_VOICE_VOLUME, part, handle, volume, 0.0)
    }
    pub fn voice_pan(time: f64, part: u16, handle: u32, pan: f32) -> Event {
        Event::new(time, EV_VOICE_PAN, part, handle, pan, 0.0)
    }
    pub fn voice_position(time: f64, part: u16, handle: u32, frames: f32) -> Event {
        Event::new(time, EV_VOICE_POSITION, part, handle, frames, 0.0)
    }
    pub fn voice_kill(time: f64, part: u16, handle: u32) -> Event {
        Event::new(time, EV_VOICE_KILL, part, handle, 0.0, 0.0)
    }
    pub fn macro_tick(time: f64, part: u16) -> Event {
        Event::new(time, EV_MACRO_TICK, part, 0, 0.0, 0.0)
    }
}

#[derive(Clone, Copy)]
struct Entry {
    time: u64,
    order: u64,
    event: Event,
}

impl Entry {
    #[inline]
    fn before(&self, other: &Entry) -> bool {
        (self.time, self.order) < (other.time, other.order)
    }
}

/// The queue: a fixed-size heap that gives events back in time order, and
/// events with the same time in the order they were pushed.
pub struct Queue {
    items: Box<[Entry]>,
    len: usize,
    order: u64,
}

impl Queue {
    pub fn new(capacity: usize) -> Queue {
        let blank = Entry { time: 0, order: 0, event: Event::default() };
        Queue { items: vec![blank; capacity].into_boxed_slice(), len: 0, order: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Add an event. False if the queue is full.
    pub fn push(&mut self, event: Event) -> bool {
        if self.len >= self.items.len() {
            return false;
        }
        let time = if event.time.is_finite() && event.time > 0.0 { event.time as u64 } else { 0 };
        let entry = Entry { time, order: self.order, event };
        self.order += 1;
        let mut i = self.len;
        self.len += 1;
        while i > 0 {
            let parent = (i - 1) / 2;
            if entry.before(&self.items[parent]) {
                self.items[i] = self.items[parent];
                i = parent;
            } else {
                break;
            }
        }
        self.items[i] = entry;
        true
    }

    /// The time of the earliest event.
    #[inline]
    pub fn next_time(&self) -> Option<u64> {
        if self.len > 0 {
            Some(self.items[0].time)
        } else {
            None
        }
    }

    /// Take the earliest event.
    pub fn pop(&mut self) -> Option<Event> {
        if self.len == 0 {
            return None;
        }
        let top = self.items[0].event;
        self.len -= 1;
        let last = self.items[self.len];
        let mut i = 0;
        loop {
            let mut child = 2 * i + 1;
            if child >= self.len {
                break;
            }
            if child + 1 < self.len && self.items[child + 1].before(&self.items[child]) {
                child += 1;
            }
            if self.items[child].before(&last) {
                self.items[i] = self.items[child];
                i = child;
            } else {
                break;
            }
        }
        self.items[i] = last;
        Some(top)
    }
}
