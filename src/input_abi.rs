//! The C interface to the live input and the recorder. `docs/engine-api.md`
//! is the reference; the rest of the interface is in `abi`.

use crate::engine::{Engine, SampleError, OUTPUT_FRAMES};
use crate::input::RECORD_SIZE;

/// Where to put the left (or only) channel of the live input before `synths_render_io`.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_input_left(engine: *mut Engine) -> *mut f32 {
    engine.as_mut().map(|e| e.input.in_l.as_mut_ptr()).unwrap_or(core::ptr::null_mut())
}

/// Where to put the right channel of a stereo live input.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_input_right(engine: *mut Engine) -> *mut f32 {
    engine.as_mut().map(|e| e.input.in_r.as_mut_ptr()).unwrap_or(core::ptr::null_mut())
}

/// `synths_render`, with the live input the host has put in the engine's
/// input buffers: `channels` is 1 (the left buffer only), 2 or 0 (none).
/// Returns how many frames were rendered.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_render_io(engine: *mut Engine, frames: u32, channels: u32) -> u32 {
    let e = match engine.as_mut() {
        Some(e) => e,
        None => return 0,
    };
    let n = (frames as usize).min(OUTPUT_FRAMES);
    // The buffers are taken out while they are used, and put straight back.
    let mut l = core::mem::take(&mut e.out_l);
    let mut r = core::mem::take(&mut e.out_r);
    let in_l = core::mem::take(&mut e.input.in_l);
    let in_r = core::mem::take(&mut e.input.in_r);
    match channels {
        0 => e.render(&mut l[..n], &mut r[..n]),
        1 => e.render_io(&in_l[..n], None, &mut l[..n], &mut r[..n]),
        _ => e.render_io(&in_l[..n], Some(&in_r[..n]), &mut l[..n], &mut r[..n]),
    }
    e.out_l = l;
    e.out_r = r;
    e.input.in_l = in_l;
    e.input.in_r = in_r;
    n as u32
}

/// `synths_render_into`, with a live input from the caller's own buffers.
/// `in_right` is null for a mono input; `in_left` null is no input at all.
///
/// # Safety
/// `engine` must be a live engine or null; each buffer that is not null
/// must hold `frames` numbers, and the output buffers must not overlap
/// each other or the input.
#[no_mangle]
pub unsafe extern "C" fn synths_render_io_into(
    engine: *mut Engine,
    in_left: *const f32,
    in_right: *const f32,
    left: *mut f32,
    right: *mut f32,
    frames: u32,
) {
    if let Some(e) = engine.as_mut() {
        if !left.is_null() && !right.is_null() {
            let n = frames as usize;
            let l = core::slice::from_raw_parts_mut(left, n);
            let r = core::slice::from_raw_parts_mut(right, n);
            if in_left.is_null() {
                e.render(l, r);
            } else {
                let in_r = if in_right.is_null() { None } else { Some(core::slice::from_raw_parts(in_right, n)) };
                e.render_io(core::slice::from_raw_parts(in_left, n), in_r, l, r);
            }
        }
    }
}

/// Make a sample slot to record into, with room for `frames` frames of 1
/// or 2 channels. Returns 0, or a negative number for an error.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_record_buffer(engine: *mut Engine, slot: u32, frames: u32, channels: u32) -> i32 {
    match engine.as_mut() {
        Some(e) => match e.record_buffer(slot as usize, frames as usize, channels) {
            Ok(()) => 0,
            Err(err) => err as i32,
        },
        None => SampleError::Null as i32,
    }
}

/// Write the state of the recorder into `out`, four numbers: recording or
/// not, the slot, the frames recorded, the frames there is room for.
/// Returns 4. It only reads, and may be called while another thread renders.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for 4 words.
#[no_mangle]
pub unsafe extern "C" fn synths_record_read(engine: *const Engine, out: *mut u32) -> u32 {
    if engine.is_null() || out.is_null() {
        return 0;
    }
    // Reached without touching the rest of the engine, as the taps are.
    let v = crate::input::Recorder::read_shared(&(*engine).recorder.shared);
    core::ptr::copy_nonoverlapping(v.as_ptr(), out, RECORD_SIZE);
    RECORD_SIZE as u32
}

/// How many frames the sample in a slot has.
///
/// # Safety
/// `engine` must be a live engine or null.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_frames(engine: *const Engine, slot: u32) -> u32 {
    engine.as_ref().map(|e| e.sample_frames(slot as usize) as u32).unwrap_or(0)
}

/// Copy `frames` frames of the sample in a slot, from frame `start` on,
/// into `out`. Returns how many frames were copied.
///
/// # Safety
/// `engine` must be a live engine or null; `out` must have room for
/// `frames` numbers, or twice that for a stereo sample.
#[no_mangle]
pub unsafe extern "C" fn synths_sample_read(engine: *const Engine, slot: u32, start: u32, out: *mut f32, frames: u32) -> u32 {
    let e = match engine.as_ref() {
        Some(e) if !out.is_null() => e,
        _ => return 0,
    };
    let channels = e.sample_channels(slot as usize);
    if channels == 0 {
        return 0;
    }
    let n = (frames as usize).min(e.sample_frames(slot as usize).saturating_sub(start as usize));
    e.read_sample(slot as usize, start as usize, core::slice::from_raw_parts_mut(out, n * channels)) as u32
}
