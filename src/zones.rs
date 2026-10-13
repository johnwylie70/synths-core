//! Sample maps: an instrument made of several recordings, each covering a
//! range of keys and of loudness, so that a piano is not one note stretched
//! across the whole keyboard.
//!
//! A part can have a list of zones. When its oscillator 1 plays a sample, a
//! new note looks the list up (here, with no memory asked for) and its voice
//! plays the zone's sample in the place of `osc1.sample`. All the zones of
//! all the parts live in one pool made with the engine.
//!
//! `docs/engine-api.md` gives the record byte by byte and the rule in words.

use crate::engine::{MAX_SAMPLES, NUM_PARTS};

/// The size of one zone record in bytes.
pub const ZONE_SIZE: usize = 48;
/// The most zones one part can have.
pub const ZONES_PER_PART: usize = 512;
/// The most zones all the parts can have between them.
pub const ZONES_TOTAL: usize = 2048;
/// The most zones one note plays at once.
pub const MAX_LAYERS: usize = 4;
/// How many zones that match a note are looked at.
pub const MAX_MATCHES: usize = 32;
/// Where a key's count of round-robin hits starts again: every number of
/// takes up to 10 divides it, so the turn never stumbles.
pub const ROUND_ROBIN_WRAP: u16 = 5040;
/// The highest key and the highest velocity of a zone's ranges.
pub const TOP: u8 = 127;

/// `Zone::flags`: the pitch does not follow the key (a drum kit).
pub const ZONE_FIXED_PITCH: u8 = 1;
/// `Zone::flags`: the sample plays to its end whatever the key does.
pub const ZONE_ONE_SHOT: u8 = 2;
/// `Zone::flags`: the zone's fades are straight lines, not equal power.
pub const ZONE_LINEAR_FADE: u8 = 4;
const ZONE_FLAGS: u8 = ZONE_FIXED_PITCH | ZONE_ONE_SHOT | ZONE_LINEAR_FADE;

/// `Zone::loop_mode`: the loop of the sample's slot.
pub const ZONE_LOOP_OWN: u8 = 0;
pub const ZONE_LOOP_NONE: u8 = 1;
pub const ZONE_LOOP_FORWARD: u8 = 2;
pub const ZONE_LOOP_PINGPONG: u8 = 3;

/// A map's flags: a note that no zone holds is silent (a drum kit), where
/// otherwise it plays the nearest zone.
pub const MAP_SILENT_OUTSIDE: u32 = 1;
const MAP_FLAGS: u32 = MAP_SILENT_OUTSIDE;

/// One zone, exactly as it is laid out in memory (little-endian).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    /// The sample slot the zone plays.
    pub slot: u16,
    /// The lowest and highest key, 0..127, both included.
    pub key_lo: u8,
    pub key_hi: u8,
    /// The lowest and highest velocity, 0..127, both included.
    pub vel_lo: u8,
    pub vel_hi: u8,
    /// Round robin: of the zones with the same number here that match a
    /// note, one plays, and the next hit of that key plays the next. 0: none.
    pub group: u8,
    /// `ZONE_FIXED_PITCH`, `ZONE_ONE_SHOT`, `ZONE_LINEAR_FADE`.
    pub flags: u8,
    /// The key the recording was played at. Not a number: the base note of
    /// the sample's slot.
    pub root: f32,
    /// Tuning in cents.
    pub tune: f32,
    /// How loud, as a multiplier (1 is unchanged).
    pub volume: f32,
    /// Pan, -1..1, added to the part's.
    pub pan: f32,
    /// How long the note takes to die away, in seconds, in the place of the
    /// part's `amp.release`. Below 0: the part's.
    pub release: f32,
    /// How many keys at the bottom and at the top of the range the zone
    /// fades in over, and how many velocities.
    pub key_fade_lo: u8,
    pub key_fade_hi: u8,
    pub vel_fade_lo: u8,
    pub vel_fade_hi: u8,
    /// The loop, when `loop_mode` is not `ZONE_LOOP_OWN`.
    pub loop_start: u32,
    pub loop_end: u32,
    pub loop_mode: u8,
    /// Not used yet: 0.
    pub reserved: [u8; 3],
    /// Where in the sample a note starts, in frames.
    pub start: u32,
}

const _: () = assert!(core::mem::size_of::<Zone>() == ZONE_SIZE);
const _: () = assert!(core::mem::align_of::<Zone>() == 4);

impl Zone {
    /// A zone that plays `slot` over the keys `key_lo..=key_hi` at every
    /// velocity, recorded at `root`, with nothing else changed.
    pub fn new(slot: u16, key_lo: u8, key_hi: u8, root: f32) -> Zone {
        Zone {
            slot,
            key_lo,
            key_hi,
            vel_lo: 0,
            vel_hi: TOP,
            group: 0,
            flags: 0,
            root,
            tune: 0.0,
            volume: 1.0,
            pan: 0.0,
            release: -1.0,
            key_fade_lo: 0,
            key_fade_hi: 0,
            vel_fade_lo: 0,
            vel_fade_hi: 0,
            loop_start: 0,
            loop_end: 0,
            loop_mode: ZONE_LOOP_OWN,
            reserved: [0; 3],
            start: 0,
        }
    }

    /// The zone with every number brought to something that can be played.
    /// Nothing here refuses a zone; `ZoneMaps::set` refuses a slot that
    /// does not exist.
    pub fn clean(&self) -> Zone {
        let mut z = *self;
        let number = |x: f32, lo: f32, hi: f32, or: f32| if x.is_finite() { x.clamp(lo, hi) } else { or };
        let range = |a: u8, b: u8| (a.min(b).min(TOP), a.max(b).min(TOP));
        (z.key_lo, z.key_hi) = range(z.key_lo, z.key_hi);
        (z.vel_lo, z.vel_hi) = range(z.vel_lo, z.vel_hi);
        z.flags &= ZONE_FLAGS;
        z.root = if z.root.is_nan() { f32::NAN } else { z.root.clamp(-60.0, 200.0) };
        z.tune = number(z.tune, -4800.0, 4800.0, 0.0);
        z.volume = number(z.volume, 0.0, 4.0, 1.0);
        z.pan = number(z.pan, -1.0, 1.0, 0.0);
        z.release = if z.release >= 0.0 { z.release.min(20.0) } else { -1.0 };
        let keys = z.key_hi - z.key_lo + 1;
        let vels = z.vel_hi - z.vel_lo + 1;
        z.key_fade_lo = z.key_fade_lo.min(keys);
        z.key_fade_hi = z.key_fade_hi.min(keys);
        z.vel_fade_lo = z.vel_fade_lo.min(vels);
        z.vel_fade_hi = z.vel_fade_hi.min(vels);
        if z.loop_mode > ZONE_LOOP_PINGPONG {
            z.loop_mode = ZONE_LOOP_OWN;
        }
        z.reserved = [0; 3];
        z
    }

    /// How far a key and a velocity are outside the zone: 0 and 0 inside.
    #[inline]
    fn distance(&self, key: i32, vel: i32) -> (i32, i32) {
        let out = |x: i32, lo: u8, hi: u8| (lo as i32 - x).max(x - hi as i32).max(0);
        (out(key, self.key_lo, self.key_hi), out(vel, self.vel_lo, self.vel_hi))
    }

    /// How much of the zone a key and a velocity inside it get, from its
    /// fades: 1 away from the edges.
    pub fn fade(&self, key: i32, vel: i32) -> f32 {
        // `width` steps of fade: nothing one step outside the edge, all of
        // it `width` steps inside.
        let ramp = |inside: i32, width: u8| if width == 0 { 1.0 } else { ((inside + 1) as f32 / (width as f32 + 1.0)).clamp(0.0, 1.0) };
        let t = [
            ramp(key - self.key_lo as i32, self.key_fade_lo),
            ramp(self.key_hi as i32 - key, self.key_fade_hi),
            ramp(vel - self.vel_lo as i32, self.vel_fade_lo),
            ramp(self.vel_hi as i32 - vel, self.vel_fade_hi),
        ];
        let linear = self.flags & ZONE_LINEAR_FADE != 0;
        t.iter().map(|&x| if linear { x } else { x.sqrt() }).product()
    }
}

/// The zones a note plays: where each is in the pool, and how much of it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Picks {
    pub count: usize,
    pub index: [u16; MAX_LAYERS],
    pub gain: [f32; MAX_LAYERS],
}

/// One part's stretch of the pool.
#[derive(Clone, Copy, Default)]
struct Map {
    start: u16,
    count: u16,
    flags: u32,
}

/// Why a part's zones could not be set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoneError {
    /// The part does not exist.
    BadPart,
    /// More zones than a part can have, or than there is room for in all.
    TooMany,
    /// A zone names a sample slot that does not exist.
    BadSlot,
}

/// Every part's zones.
pub struct ZoneMaps {
    pool: Box<[Zone]>,
    used: usize,
    maps: Box<[Map]>,
    /// For each key of each part, how many round-robin hits it has had.
    turns: Box<[u16]>,
}

impl ZoneMaps {
    pub fn new() -> ZoneMaps {
        ZoneMaps {
            pool: vec![Zone::new(0, 0, TOP, f32::NAN); ZONES_TOTAL].into_boxed_slice(),
            used: 0,
            maps: vec![Map::default(); NUM_PARTS].into_boxed_slice(),
            turns: vec![0; NUM_PARTS * (TOP as usize + 1)].into_boxed_slice(),
        }
    }

    /// A part's zones, in the order they were given.
    pub fn zones(&self, part: usize) -> &[Zone] {
        match self.maps.get(part) {
            Some(m) => &self.pool[m.start as usize..m.start as usize + m.count as usize],
            None => &[],
        }
    }

    /// A part's map flags.
    pub fn flags(&self, part: usize) -> u32 {
        self.maps.get(part).map(|m| m.flags).unwrap_or(0)
    }

    /// How many zones every part has between them.
    pub fn used(&self) -> usize {
        self.used
    }

    /// A zone by its place in the pool, as `Picks` names it.
    #[inline]
    pub fn zone(&self, index: u16) -> Zone {
        self.pool.get(index as usize).copied().unwrap_or(Zone::new(0, 0, TOP, f32::NAN))
    }

    /// Give a part its zones, replacing any it had; none takes them away.
    /// Either all of them are taken or nothing changes. It asks for no
    /// memory: the zones are copied into the pool.
    pub fn set(&mut self, part: usize, zones: &[Zone], flags: u32) -> Result<(), ZoneError> {
        self.set_with(part, zones.len(), flags, |i| zones[i])
    }

    /// `set`, with the zones read one at a time from wherever they are:
    /// `zone(i)` is zone `i` of `len`.
    pub fn set_with(&mut self, part: usize, len: usize, flags: u32, zone: impl Fn(usize) -> Zone) -> Result<(), ZoneError> {
        let old = match self.maps.get(part) {
            Some(m) => *m,
            None => return Err(ZoneError::BadPart),
        };
        if len > ZONES_PER_PART || self.used - old.count as usize + len > ZONES_TOTAL {
            return Err(ZoneError::TooMany);
        }
        if (0..len).any(|i| zone(i).slot as usize >= MAX_SAMPLES) {
            return Err(ZoneError::BadSlot);
        }
        // Close the gap the part's old zones leave, then put the new ones at the end.
        let (start, count) = (old.start as usize, old.count as usize);
        self.pool.copy_within(start + count..self.used, start);
        self.used -= count;
        for m in self.maps.iter_mut() {
            if m.start as usize > start {
                m.start -= count as u16;
            }
        }
        for i in 0..len {
            self.pool[self.used + i] = zone(i).clean();
        }
        self.maps[part] = Map { start: self.used as u16, count: len as u16, flags: if len == 0 { 0 } else { flags & MAP_FLAGS } };
        self.used += len;
        self.rewind(part);
        Ok(())
    }

    /// Start a part's round robins again from their first takes.
    fn rewind(&mut self, part: usize) {
        let keys = TOP as usize + 1;
        if let Some(t) = self.turns.get_mut(part * keys..(part + 1) * keys) {
            t.fill(0);
        }
    }

    /// Start every round robin again from its first take.
    pub fn rewind_all(&mut self) {
        self.turns.fill(0);
    }

    /// The zones a new note plays, and how much of each. This is the whole
    /// rule; it changes nothing but the key's round-robin turn.
    pub fn pick(&mut self, part: usize, note: f32, velocity: f32) -> Picks {
        let mut picks = Picks::default();
        let map = match self.maps.get(part) {
            Some(m) if m.count > 0 => *m,
            _ => return picks,
        };
        let zones = &self.pool[map.start as usize..map.start as usize + map.count as usize];
        let key = (note + 0.5).floor().clamp(0.0, TOP as f32) as i32;
        let vel = (velocity.clamp(0.0, 1.0) * TOP as f32 + 0.5) as i32;

        // The nearest zone: by key first, then by velocity, and of two as
        // near the one given first. A note outside every zone is played as
        // the nearest key and velocity inside that zone would be.
        let mut best = (i32::MAX, i32::MAX);
        let (mut near_key, mut near_vel) = (key, vel);
        for z in zones {
            let d = z.distance(key, vel);
            if d < best {
                best = d;
                near_key = key.clamp(z.key_lo as i32, z.key_hi as i32);
                near_vel = vel.clamp(z.vel_lo as i32, z.vel_hi as i32);
            }
        }
        if best != (0, 0) && map.flags & MAP_SILENT_OUTSIDE != 0 {
            return picks;
        }

        // The zones that hold it, in the order they were given.
        let mut matches = [0u16; MAX_MATCHES];
        let mut found = 0;
        for (i, z) in zones.iter().enumerate() {
            if z.distance(near_key, near_vel) == (0, 0) {
                matches[found] = i as u16;
                found += 1;
                if found == MAX_MATCHES {
                    break;
                }
            }
        }
        let matches = &matches[..found];

        // Of the zones in one round-robin group, the key's turn picks one.
        let turn = &mut self.turns[part * (TOP as usize + 1) + key as usize];
        let mut took_a_turn = false;
        for (at, &i) in matches.iter().enumerate() {
            let z = &zones[i as usize];
            if z.group != 0 {
                took_a_turn = true;
                let same = |&&j: &&u16| zones[j as usize].group == z.group;
                let before = matches[..at].iter().filter(same).count();
                let takes = matches.iter().filter(same).count();
                if before != *turn as usize % takes {
                    continue;
                }
            }
            if picks.count < MAX_LAYERS {
                picks.index[picks.count] = map.start + i;
                picks.gain[picks.count] = z.fade(near_key, near_vel);
                picks.count += 1;
            }
        }
        if took_a_turn {
            *turn = (*turn + 1) % ROUND_ROBIN_WRAP;
        }
        picks
    }
}

impl Default for ZoneMaps {
    fn default() -> ZoneMaps {
        ZoneMaps::new()
    }
}
