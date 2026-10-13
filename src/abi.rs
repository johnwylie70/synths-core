//! The C interface: the same functions whether the engine is a WebAssembly
//! module or a static library. `docs/engine-api.md` is the reference.
//!
//! Every function taking an engine must be called from one thread at a time.

use crate::engine::{Engine, SampleError, EVENT_BUFFER, OUTPUT_FRAMES};
use crate::event::Event;
use crate::macros::STEP_SIZE;
use crate::taps::{Taps, LEVEL_SIZE, NOTES_SIZE};
use std::alloc::{alloc, dealloc, Layout};
use std::sync::OnceLock;

static VERSION_Z: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
static TABLE: OnceLock<String> = OnceLock::new();

fn table() -> &'static str {
    TABLE.get_or_init(|| {
        let mut s = crate::params::table_json();
        s.push('\0');
        s
    })
}

/// The version of this interface (1).
#[no_mangle]
pub extern "C" fn synths_abi_version() -> u32 {
    crate::ABI_VERSION
}

/// The engine's version as text ending in a zero byte.
#[no_mangle]
pub extern "C" fn synths_version() -> *const u8 {
    VERSION_Z.as_ptr()
}

/// The parameter table as JSON text ending in a zero byte.
#[no_mangle]
pub extern "C" fn synths_param_table() -> *const u8 {
    table().as_ptr()
}

/// The length of the parameter table in bytes, not counting the zero byte.
#[no_mangle]
pub extern "C" fn synths_param_table_len() -> u32 {
    (table().len() - 1) as u32
}

/// Make an engine for a sample rate (8000 to 192000). Null if the rate is not a number.
#[no_mangle]
pub extern "C" fn synths_create(sample_rate: f32) -> *mut Engine {
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return core::ptr::null_mut();
    }
    Box::into_raw(Engine::new(sample_rate))
}

/// Free an engine.
///
/// # Safety
/// `engine` must have come from `synths_create` and not been destroyed.
#[no_mangle]
pub unsafe extern "C" fn synths_destroy(engine: *mut Engine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// Stop every sound and forget every waiting event.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_reset(engine: *mut Engine) {
    if let Some(e) = engine.as_mut() {
        e.reset();
    }
}

/// The sample rate the engine was made with.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_rate(engine: *const Engine) -> f32 {
    engine.as_ref().map(|e| e.sample_rate()).unwrap_or(0.0)
}

/// Frames rendered so far: the clock events are timed by.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_time(engine: *const Engine) -> f64 {
    engine.as_ref().map(|e| e.time() as f64).unwrap_or(0.0)
}

/// How many voices are sounding.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_active_voices(engine: *const Engine) -> u32 {
    engine.as_ref().map(|e| e.active_voices() as u32).unwrap_or(0)
}

/// A parameter's value now.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_get_param(engine: *const Engine, part: u32, id: u32) -> f32 {
    engine.as_ref().map(|e| e.param(part as usize, id as usize)).unwrap_or(0.0)
}

/// Render up to `synths_output_capacity()` frames into the engine's own
/// buffers. Returns how many were rendered.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_render(engine: *mut Engine, frames: u32) -> u32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return 0,
    };
    let n = (frames as usize).min(OUTPUT_FRAMES);
    // The buffers are taken out while they are written to, and put straight back.
    let mut l = core::mem::take(&mut e.out_l);
    let mut r = core::mem::take(&mut e.out_r);
    e.render(&mut l[..n], &mut r[..n]);
    e.out_l = l;
    e.out_r = r;
    n as u32
}

/// The left channel that `synths_render` wrote.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_output_left(engine: *const Engine) -> *const f32 {
    engine.as_ref().map(|e| e.out_l.as_ptr()).unwrap_or(core::ptr::null())
}

/// The right channel that `synths_render` wrote.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_output_right(engine: *const Engine) -> *const f32 {
    engine.as_ref().map(|e| e.out_r.as_ptr()).unwrap_or(core::ptr::null())
}

/// How many frames `synths_render` can render in one call.
#[no_mangle]
pub extern "C" fn synths_output_capacity() -> u32 {
    OUTPUT_FRAMES as u32
}

/// Render any number of frames into the caller's own buffers.
///
/// # Safety
/// `engine` must be a live engine or null; `left` and `right` must each have
/// room for `frames` numbers and must not overlap.
#[no_mangle]
pub unsafe extern "C" fn synths_render_into(engine: *mut Engine, left: *mut f32, right: *mut f32, frames: u32) {
    if let Some(e) = engine.as_mut() {
        if !left.is_null() && !right.is_null() {
            let l = core::slice::from_raw_parts_mut(left, frames as usize);
            let r = core::slice::from_raw_parts_mut(right, frames as usize);
            e.render(l, r);
        }
    }
}

/// The buffer to write event records into before `synths_push_events`.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_event_buffer(engine: *mut Engine) -> *mut u8 {
    engine.as_mut().map(|e| e.event_buf.as_mut_ptr() as *mut u8).unwrap_or(core::ptr::null_mut())
}

/// How many event records the event buffer holds.
#[no_mangle]
pub extern "C" fn synths_event_capacity() -> u32 {
    EVENT_BUFFER as u32
}

/// Queue the first `count` records of the event buffer. Returns how many
/// were taken; fewer than `count` means the queue is full.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_push_events(engine: *mut Engine, count: u32) -> u32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return 0,
    };
    let n = (count as usize).min(EVENT_BUFFER);
    for i in 0..n {
        let event: Event = e.event_buf[i];
        if !e.push(event) {
            return i as u32;
        }
    }
    n as u32
}

/// How many more events the queue has room for.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_event_space(engine: *const Engine) -> u32 {
    engine
        .as_ref()
        .map(|e| (crate::engine::QUEUE_CAPACITY - e.queued_events()) as u32)
        .unwrap_or(0)
}

const HEADER: usize = 16;

/// Get `bytes` of memory (aligned to 16), for handing sample data to
/// `synths_sample_load`. Null if there is none to be had.
#[no_mangle]
pub extern "C" fn synths_alloc(bytes: u32) -> *mut u8 {
    let total = match (bytes as usize).checked_add(HEADER) {
        Some(t) => t,
        None => return core::ptr::null_mut(),
    };
    let layout = match Layout::from_size_align(total, HEADER) {
        Ok(l) => l,
        Err(_) => return core::ptr::null_mut(),
    };
    // The size is kept just in front of the memory handed out, so that
    // `synths_free` needs only the pointer.
    unsafe {
        let p = alloc(layout);
        if p.is_null() {
            return p;
        }
        (p as *mut usize).write(total);
        p.add(HEADER)
    }
}

/// Give back memory from `synths_alloc`.
///
/// # Safety
/// `ptr` must have come from `synths_alloc` and not been freed, or be null.
#[no_mangle]
pub unsafe extern "C" fn synths_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let p = ptr.sub(HEADER);
    let total = (p as *const usize).read();
    if let Ok(layout) = Layout::from_size_align(total, HEADER) {
        dealloc(p, layout);
    }
}

fn code(r: Result<(), SampleError>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

/// Copy a sample into a slot. `data` holds `frames * channels` numbers.
/// Returns 0, or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null; `data` must point to
/// `frames * channels` numbers.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_load(
    engine: *mut Engine,
    slot: u32,
    data: *const f32,
    frames: u32,
    channels: u32,
    sample_rate: f32,
) -> i32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return SampleError::Null as i32,
    };
    if data.is_null() {
        return SampleError::Null as i32;
    }
    if !(channels == 1 || channels == 2) {
        return SampleError::BadFormat as i32;
    }
    let len = match (frames as usize).checked_mul(channels as usize) {
        Some(l) => l,
        None => return SampleError::BadFormat as i32,
    };
    code(e.load_sample(slot as usize, core::slice::from_raw_parts(data, len), channels, sample_rate))
}

/// Set a sample's base note and loop. Returns 0, or a negative number.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_config(
    engine: *mut Engine,
    slot: u32,
    base_note: f32,
    loop_mode: u32,
    loop_start: u32,
    loop_end: u32,
) -> i32 {
    match engine.as_mut() {
        Some(e) => code(e.configure_sample(slot as usize, base_note, loop_mode, loop_start, loop_end)),
        None => SampleError::Null as i32,
    }
}

/// Empty a sample slot. Returns 0, or a negative number.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_free(engine: *mut Engine, slot: u32) -> i32 {
    match engine.as_mut() {
        Some(e) => code(e.free_sample(slot as usize)),
        None => SampleError::Null as i32,
    }
}

/// Give a part its step macro: `count` steps of five numbers each (wave,
/// pitch, pulse width, cutoff, volume). A count of 0 takes the macro away.
/// Returns 0, or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null; `steps` must point to `count * 5`
/// numbers (it may be null when `count` is 0).
#[no_mangle]
pub unsafe extern "C" fn synths_macro_set(
    engine: *mut Engine,
    part: u32,
    steps: *const f32,
    count: u32,
    loop_start: u32,
    loop_end: u32,
    release: u32,
    flags: u32,
) -> i32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return SampleError::Null as i32,
    };
    if count as usize > crate::macros::MAX_STEPS {
        return SampleError::BadFormat as i32;
    }
    if part > u16::MAX as u32 {
        return SampleError::BadSlot as i32;
    }
    let numbers: &[f32] = if count == 0 {
        &[]
    } else if steps.is_null() {
        return SampleError::Null as i32;
    } else {
        core::slice::from_raw_parts(steps, count as usize * STEP_SIZE)
    };
    code(e.set_macro(part as u16, numbers, loop_start, loop_end, release, flags))
}

/// The engine's taps, reached without touching the rest of the engine, so
/// that a display may read them while another thread renders.
unsafe fn taps<'a>(engine: *const Engine) -> Option<&'a Taps> {
    if engine.is_null() {
        None
    } else {
        Some(&*(*engine).taps)
    }
}

/// Copy the most recent `frames` frames of a part's sound (or, for source
/// 65535, of the master output) into `out`, oldest first. Returns how many
/// were written.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for `frames` numbers.
#[no_mangle]
pub unsafe extern "C" fn synths_scope_read(engine: *const Engine, source: u32, out: *mut f32, frames: u32) -> u32 {
    match taps(engine) {
        Some(t) if !out.is_null() => {
            let frames = (frames as usize).min(crate::taps::SCOPE_MASTER);
            t.read_scope(source, core::slice::from_raw_parts_mut(out, frames)) as u32
        }
        _ => 0,
    }
}

/// Write the levels of `count` parts starting at part `source` (or, for
/// source 65535, of the master output alone) into `out`: four numbers each,
/// peak left, peak right, RMS left, RMS right. Returns how many sources were written.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for `count * 4` numbers.
#[no_mangle]
pub unsafe extern "C" fn synths_levels_read(engine: *const Engine, source: u32, count: u32, out: *mut f32) -> u32 {
    let t = match taps(engine) {
        Some(t) if !out.is_null() => t,
        _ => return 0,
    };
    let mut done = 0;
    while done < count {
        match source.checked_add(done).and_then(|s| t.read_levels(s)) {
            Some(v) => core::ptr::copy_nonoverlapping(v.as_ptr(), out.add(done as usize * LEVEL_SIZE), LEVEL_SIZE),
            None => break,
        }
        done += 1;
        if source == crate::taps::MASTER {
            break;
        }
    }
    done
}

/// Write which notes a part is playing (or, for part 65535, every part
/// together) into `out`: eight 32-bit words, 128 bits of notes held and then
/// 128 of notes sounding. Returns 8, or 0 if the part does not exist.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for 8 words.
#[no_mangle]
pub unsafe extern "C" fn synths_notes_read(engine: *const Engine, part: u32, out: *mut u32) -> u32 {
    match taps(engine).and_then(|t| t.read_notes(part)) {
        Some(v) if !out.is_null() => {
            core::ptr::copy_nonoverlapping(v.as_ptr(), out, NOTES_SIZE);
            NOTES_SIZE as u32
        }
        _ => 0,
    }
}
