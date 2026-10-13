//! The C interface to arpeggio patterns and gate patterns: both are short
//! lists of steps a host hands over as data, or as the same text a song
//! file has them in. `docs/engine-api.md` is the reference.

use crate::arp::{Pattern, MAX_PATTERN, PATTERN_STEP_SIZE};
use crate::engine::{Engine, SampleError};
use crate::gate::{self, MAX_GATE_STEPS};

unsafe fn text<'a>(ptr: *const u8, len: u32) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    core::str::from_utf8(core::slice::from_raw_parts(ptr, len as usize)).ok()
}

/// Put a pattern of `count` steps, four bytes each, in place `slot` (1 to
/// 64) for arpeggios to follow. A count of 0 empties the place. Returns 0,
/// or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null; `steps` must point to `count * 4`
/// bytes (it may be null when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn synths_arp_pattern_set(engine: *mut Engine, slot: u32, steps: *const u8, count: u32) -> i32 {
    let Some(e) = engine.as_mut() else { return SampleError::Null as i32 };
    if count as usize > MAX_PATTERN {
        return SampleError::BadFormat as i32;
    }
    if count > 0 && steps.is_null() {
        return SampleError::Null as i32;
    }
    let bytes: &[u8] = if count == 0 { &[] } else { core::slice::from_raw_parts(steps, count as usize * PATTERN_STEP_SIZE) };
    e.set_arp_pattern(slot as usize, &Pattern::from_bytes(bytes)).map_or_else(|err| err as i32, |()| 0)
}

/// The same from the pattern's text (`"1 - 3 2 _ 1 3 5^"`, or the name of
/// a built-in pattern), `len` bytes of UTF-8. Returns how many steps it
/// has, or a negative number: -2 if the text is not a pattern.
///
/// # Safety
/// `engine` must be a live engine or null; `pattern` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_arp_pattern_text(engine: *mut Engine, slot: u32, pattern: *const u8, len: u32) -> i32 {
    let Some(e) = engine.as_mut() else { return SampleError::Null as i32 };
    let Some(Ok(p)) = text(pattern, len).map(Pattern::parse) else { return SampleError::BadFormat as i32 };
    e.set_arp_pattern(slot as usize, &p).map_or_else(|err| err as i32, |()| p.len as i32)
}

/// Give a part's gate its pattern: `count` levels, 0 (closed) to 1 (open),
/// a step each (part 65535: every part's). A count of 0 leaves it with open
/// and closed by turns. Returns 0, or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null; `levels` must point to `count`
/// numbers (it may be null when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn synths_gate_set(engine: *mut Engine, part: u32, levels: *const f32, count: u32) -> i32 {
    let Some(e) = engine.as_mut() else { return SampleError::Null as i32 };
    if count as usize > MAX_GATE_STEPS || part > u16::MAX as u32 {
        return SampleError::BadFormat as i32;
    }
    if count > 0 && levels.is_null() {
        return SampleError::Null as i32;
    }
    let mut steps = [0.0f32; MAX_GATE_STEPS];
    for (i, slot) in steps.iter_mut().enumerate().take(count as usize) {
        *slot = levels.add(i).read_unaligned();
    }
    e.set_gate(part as u16, &steps[..count as usize]).map_or_else(|err| err as i32, |()| 0)
}

/// The same from the pattern's text (`"x.x.xx.x"`, or the name of a
/// built-in pattern), `len` bytes of UTF-8. Returns how many steps it has,
/// or a negative number: -2 if the text is not a pattern.
///
/// # Safety
/// `engine` must be a live engine or null; `pattern` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn synths_gate_text(engine: *mut Engine, part: u32, pattern: *const u8, len: u32) -> i32 {
    let Some(e) = engine.as_mut() else { return SampleError::Null as i32 };
    if part > u16::MAX as u32 {
        return SampleError::BadFormat as i32;
    }
    let Some(Ok((levels, n))) = text(pattern, len).map(gate::parse) else { return SampleError::BadFormat as i32 };
    e.set_gate(part as u16, &levels[..n]).map_or_else(|err| err as i32, |()| n as i32)
}
