//! The C interface to the parts' sample maps. `docs/engine-api.md` is the
//! reference, and gives the zone record byte by byte.

use crate::engine::{Engine, SampleError, NUM_PARTS};
use crate::event::ALL_PARTS;
use crate::zones::{Zone, ZONES_PER_PART};

/// Give a part its sample map: `count` zone records of 48 bytes each,
/// replacing any it had. `flags` 1: a note no zone holds is silent. A count
/// of 0 takes the map away. Returns 0, or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null; `zones` must point to `count`
/// records (it may be null when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn synths_zones_set(engine: *mut Engine, part: u32, zones: *const Zone, count: u32, flags: u32) -> i32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return SampleError::Null as i32,
    };
    if part as usize >= NUM_PARTS {
        return SampleError::BadSlot as i32;
    }
    if count as usize > ZONES_PER_PART {
        return SampleError::BadFormat as i32;
    }
    if count > 0 && zones.is_null() {
        return SampleError::Null as i32;
    }
    // The host's records may sit anywhere in its memory, so each is read as bytes.
    match e.set_zones_with(part as usize, count as usize, flags, |i| zones.add(i).read_unaligned()) {
        Ok(()) => 0,
        Err(err) => err as i32,
    }
}

/// Take a part's sample map away (part 65535: every part's). Returns 0, or
/// a negative number.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_zones_clear(engine: *mut Engine, part: u32) -> i32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return SampleError::Null as i32,
    };
    if part == ALL_PARTS as u32 {
        for i in 0..NUM_PARTS {
            let _ = e.set_zones(i, &[], 0);
        }
        return 0;
    }
    match e.set_zones(part as usize, &[], 0) {
        Ok(()) => 0,
        Err(err) => err as i32,
    }
}

/// How many zones a part's sample map has.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_zones_count(engine: *const Engine, part: u32) -> u32 {
    engine.as_ref().map(|e| e.zones(part as usize).len() as u32).unwrap_or(0)
}

/// The flags a part's sample map was set with.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_zones_flags(engine: *const Engine, part: u32) -> u32 {
    engine.as_ref().map(|e| e.zone_flags(part as usize)).unwrap_or(0)
}

/// Copy a part's zones into `out`, at most `capacity` records, as the
/// engine holds them. Returns how many were copied.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for
/// `capacity` records.
#[no_mangle]
pub unsafe extern "C" fn synths_zones_read(engine: *const Engine, part: u32, out: *mut Zone, capacity: u32) -> u32 {
    let e = match engine.as_ref() {
        Some(e) if !out.is_null() => e,
        _ => return 0,
    };
    let zones = e.zones(part as usize);
    let n = zones.len().min(capacity as usize);
    for (i, z) in zones[..n].iter().enumerate() {
        out.add(i).write_unaligned(*z);
    }
    n as u32
}
