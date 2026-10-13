//! The live input and the recorder through the C interface.

mod common;
use common::*;
use synths_core::abi::*;
use synths_core::event::RECORD_INPUT;
use synths_core::input_abi::*;
use synths_core::params::*;
use synths_core::{Engine, Event};

/// The same patch for an engine reached either way.
fn events() -> Vec<Event> {
    vec![
        Event::param(0.0, 31, FX_PITCH, -4.0, 0.0),
        Event::param(0.0, 31, SEND_REVERB, 0.4, 0.0),
        Event::param(0.0, 31, PAN, -0.4, 0.0),
        Event::record_start(500.0, 9, RECORD_INPUT, 0),
        Event::record_stop(20500.0),
    ]
}

#[test]
fn rendering_with_an_input_through_the_c_functions_is_the_same_as_from_rust() {
    let xl = speech::speech(SR, 0.5);
    let xr = hiss(8, 0.2, xl.len());
    for stereo in [false, true] {
        let mut e = Engine::new(SR);
        e.record_buffer(9, 24000, 2).unwrap();
        for ev in events() {
            e.push(ev);
        }
        let (wl, wr) = render_io(&mut e, &xl, if stereo { Some(&xr) } else { None }, 128);
        unsafe {
            // The engine's own buffers, as a WebAssembly host uses them.
            let engine = synths_create(SR);
            assert_eq!(synths_record_buffer(engine, 9, 24000, 2), 0);
            let buffer = synths_event_buffer(engine) as *mut Event;
            for (i, ev) in events().into_iter().enumerate() {
                buffer.add(i).write(ev);
            }
            assert_eq!(synths_push_events(engine, 5), 5);
            let (mut l, mut r) = (Vec::new(), Vec::new());
            for at in (0..xl.len()).step_by(128) {
                let n = 128.min(xl.len() - at);
                core::ptr::copy_nonoverlapping(xl[at..].as_ptr(), synths_input_left(engine), n);
                if stereo {
                    core::ptr::copy_nonoverlapping(xr[at..].as_ptr(), synths_input_right(engine), n);
                }
                assert_eq!(synths_render_io(engine, n as u32, 1 + stereo as u32), n as u32);
                l.extend_from_slice(core::slice::from_raw_parts(synths_output_left(engine), n));
                r.extend_from_slice(core::slice::from_raw_parts(synths_output_right(engine), n));
            }
            assert!(l == wl && r == wr, "stereo {stereo}: the engine's buffers");
            // What was recorded can be read back, all at once or a piece at a time.
            assert_eq!(synths_sample_frames(engine, 9), 20000);
            let mut whole = vec![0.0f32; 40000];
            assert_eq!(synths_sample_read(engine, 9, 0, whole.as_mut_ptr(), 20000), 20000);
            assert_eq!(whole.len(), e.sample_frames(9) * 2);
            let mut same = vec![0.0f32; 40000];
            e.read_sample(9, 0, &mut same);
            assert!(whole == same && whole[2000] == xl[1500] && whole[2001] == if stereo { xr[1500] } else { xl[1500] });
            let mut piece = [9.0f32; 20];
            assert_eq!(synths_sample_read(engine, 9, 19995, piece.as_mut_ptr(), 10), 5);
            assert!(piece[..10] == whole[39990..] && piece[10] == 9.0);
            assert_eq!(synths_sample_read(engine, 9, 20000, piece.as_mut_ptr(), 10), 0);
            assert_eq!(synths_sample_read(engine, 10, 0, piece.as_mut_ptr(), 10), 0);
            assert_eq!(synths_sample_read(engine, 9, 0, core::ptr::null_mut(), 10), 0);
            let mut words = [7u32; 4];
            assert_eq!(synths_record_read(engine, words.as_mut_ptr()), 4);
            assert_eq!(words, [0, 9, 20000, 24000]);
            synths_destroy(engine);

            // The host's own buffers, as a native app uses them.
            let engine = synths_create(SR);
            synths_record_buffer(engine, 9, 24000, 2);
            let buffer = synths_event_buffer(engine) as *mut Event;
            for (i, ev) in events().into_iter().enumerate() {
                buffer.add(i).write(ev);
            }
            synths_push_events(engine, 5);
            let (mut l, mut r) = (vec![0.0f32; xl.len()], vec![0.0f32; xl.len()]);
            let right = if stereo { xr.as_ptr() } else { core::ptr::null() };
            synths_render_io_into(engine, xl.as_ptr(), right, l.as_mut_ptr(), r.as_mut_ptr(), xl.len() as u32);
            assert!(l == wl && r == wr, "stereo {stereo}: the host's buffers");
            synths_destroy(engine);
        }
    }
}

#[test]
fn the_new_functions_take_nulls_and_nonsense_without_harm() {
    unsafe {
        let null: *mut Engine = core::ptr::null_mut();
        assert!(synths_input_left(null).is_null() && synths_input_right(null).is_null());
        assert_eq!(synths_render_io(null, 128, 1), 0);
        synths_render_io_into(null, core::ptr::null(), core::ptr::null(), core::ptr::null_mut(), core::ptr::null_mut(), 128);
        assert_eq!(synths_record_buffer(null, 0, 100, 1), -4);
        assert_eq!(synths_record_read(null, core::ptr::null_mut()), 0);
        assert_eq!(synths_sample_frames(null, 0), 0);
        assert_eq!(synths_sample_read(null, 0, 0, core::ptr::null_mut(), 1), 0);

        let engine = synths_create(SR);
        assert_eq!(synths_record_buffer(engine, 1024, 100, 1), -1);
        assert_eq!(synths_record_buffer(engine, 0, 0, 1), -2);
        assert_eq!(synths_record_buffer(engine, 0, 100, 0), -2);
        assert_eq!(synths_record_buffer(engine, 0, u32::MAX, 2), -2);
        assert_eq!(synths_record_read(engine, core::ptr::null_mut()), 0);
        // With no input (0 channels, or no left buffer) these are plain renders.
        let (mut l, mut r) = ([1.0f32; 64], [1.0f32; 64]);
        synths_render_io_into(engine, core::ptr::null(), core::ptr::null(), l.as_mut_ptr(), r.as_mut_ptr(), 64);
        assert!(l == [0.0; 64] && r == [0.0; 64]);
        synths_render_io_into(engine, l.as_ptr(), core::ptr::null(), core::ptr::null_mut(), r.as_mut_ptr(), 64);
        assert_eq!(synths_render_io(engine, 64, 0), 64);
        // More frames than the engine's buffers hold: only that many are rendered.
        assert_eq!(synths_render_io(engine, 100_000, 2), synths_output_capacity());
        assert_eq!(synths_time(engine), 64.0 + 64.0 + 4096.0);
        synths_destroy(engine);
    }
}
