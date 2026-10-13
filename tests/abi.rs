//! The C interface, used the way a host uses it: through pointers and bytes.

use synths_core::abi::*;
use synths_core::params::*;

/// Write one event record, byte by byte, as docs/engine-api.md lays it out.
#[allow(clippy::too_many_arguments)]
unsafe fn write_event(buf: *mut u8, index: usize, time: f64, kind: u16, part: u16, a: u32, x: f32, y: f32) {
    let mut rec = [0u8; 32];
    rec[0..8].copy_from_slice(&time.to_le_bytes());
    rec[8..10].copy_from_slice(&kind.to_le_bytes());
    rec[10..12].copy_from_slice(&part.to_le_bytes());
    rec[12..16].copy_from_slice(&a.to_le_bytes());
    rec[16..20].copy_from_slice(&x.to_le_bytes());
    rec[20..24].copy_from_slice(&y.to_le_bytes());
    std::ptr::copy_nonoverlapping(rec.as_ptr(), buf.add(index * 32), 32);
}

unsafe fn c_str(p: *const u8) -> String {
    std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_str().unwrap().to_string()
}

#[test]
fn a_host_can_drive_the_engine_through_the_c_interface() {
    unsafe {
        assert_eq!(synths_abi_version(), 1);
        assert_eq!(c_str(synths_version()), env!("CARGO_PKG_VERSION"));
        assert!(synths_create(f32::NAN).is_null());
        assert!(synths_create(0.0).is_null());

        let e = synths_create(48000.0);
        assert!(!e.is_null());
        assert_eq!(synths_sample_rate(e), 48000.0);
        assert_eq!(synths_time(e), 0.0);
        assert_eq!(synths_event_capacity(), 256);
        assert_eq!(synths_output_capacity(), 4096);
        assert_eq!(synths_event_space(e), 4096);

        // A square wave on part 2 from frame 300 to frame 1000, panned hard right.
        let buf = synths_event_buffer(e);
        assert_eq!(buf as usize % 8, 0);
        write_event(buf, 0, 0.0, 4, 2, OSC1_WAVE as u32, 3.0, 0.0);
        write_event(buf, 1, 0.0, 4, 2, PAN as u32, 1.0, 0.0);
        write_event(buf, 2, 0.0, 4, 2, AMP_ATTACK as u32, 0.0, 0.0);
        write_event(buf, 3, 0.0, 4, 2, AMP_RELEASE as u32, 0.0, 0.0);
        write_event(buf, 4, 300.0, 1, 2, 0, 60.0, 1.0);
        write_event(buf, 5, 1000.0, 2, 2, 0, 60.0, 0.0);
        assert_eq!(synths_push_events(e, 6), 6);
        assert_eq!(synths_event_space(e), 4090);

        let mut right = Vec::new();
        for _ in 0..10 {
            assert_eq!(synths_render(e, 128), 128);
            let l = std::slice::from_raw_parts(synths_output_left(e), 128);
            let r = std::slice::from_raw_parts(synths_output_right(e), 128);
            assert!(l.iter().all(|&v| v.abs() < 1e-5));
            right.extend_from_slice(r);
        }
        assert_eq!(synths_time(e), 1280.0);
        assert_eq!(right.iter().position(|&v| v != 0.0), Some(300));
        assert_eq!(right.iter().rposition(|&v| v != 0.0), Some(999));
        assert_eq!(synths_get_param(e, 2, PAN as u32), 1.0);
        assert_eq!(synths_get_param(e, 0, MASTER_VOLUME as u32), 0.35);
        assert_eq!(synths_active_voices(e), 0);

        // The engine's own buffers hold 4096 frames; a host's can be any size.
        assert_eq!(synths_render(e, 100_000), 4096);
        let mut l = vec![1.0f32; 10_000];
        let mut r = vec![1.0f32; 10_000];
        synths_render_into(e, l.as_mut_ptr(), r.as_mut_ptr(), 10_000);
        assert!(l.iter().chain(r.iter()).all(|&v| v == 0.0));
        assert_eq!(synths_time(e), 1280.0 + 4096.0 + 10_000.0);

        // A sample: get memory, fill it, load it (the engine copies), free it.
        let frames = 1000u32;
        let mem = synths_alloc(frames * 4) as *mut f32;
        assert!(!mem.is_null() && mem as usize % 16 == 0);
        for i in 0..frames as usize {
            mem.add(i).write(0.25);
        }
        assert_eq!(synths_sample_load(e, 9, mem, frames, 1, 48000.0), 0);
        assert_eq!(synths_sample_load(e, 5000, mem, frames, 1, 48000.0), -1);
        assert_eq!(synths_sample_load(e, 9, mem, frames, 3, 48000.0), -2);
        assert_eq!(synths_sample_load(e, 9, std::ptr::null(), frames, 1, 48000.0), -4);
        synths_free(mem as *mut u8);
        assert_eq!(synths_sample_config(e, 9, 60.0, 1, 0, 1000), 0);
        assert_eq!(synths_sample_config(e, 10, 60.0, 1, 0, 1000), -3);
        let now = synths_time(e);
        write_event(buf, 0, now, 4, 2, OSC1_WAVE as u32, 4.0, 0.0);
        write_event(buf, 1, now, 4, 2, OSC1_SAMPLE as u32, 9.0, 0.0);
        write_event(buf, 2, now, 4, 0, MASTER_VOLUME as u32, 1.0, 0.0);
        write_event(buf, 3, now, 4, 2, AMP_SUSTAIN as u32, 1.0, 0.0);
        write_event(buf, 4, now, 1, 2, 77, 60.0, 1.0);
        assert_eq!(synths_push_events(e, 5), 5);
        synths_render(e, 256);
        assert_eq!(synths_active_voices(e), 1);
        let r = std::slice::from_raw_parts(synths_output_right(e), 256);
        assert!((r[200] - 0.25).abs() < 1e-5, "{}", r[200]);
        assert_eq!(synths_sample_free(e, 9), 0);

        // What a display reads: the scope, the meters, the notes.
        let scope = synths_alloc(4096 * 4) as *mut f32;
        assert_eq!(synths_scope_read(e, 0xFFFF, scope, 100_000), 4096);
        assert_eq!(synths_scope_read(e, 2, scope, 100_000), 1024);
        assert_eq!(synths_scope_read(e, 64, scope, 16), 0);
        assert_eq!(synths_scope_read(e, 2, scope, 256), 256);
        let seen = std::slice::from_raw_parts(scope, 256);
        for i in 0..256 {
            assert_eq!(seen[i], 0.5 * r[i], "the scope holds the mean of left and right");
        }
        let mut levels = [0.0f32; 12];
        assert_eq!(synths_levels_read(e, 1, 3, levels.as_mut_ptr()), 3);
        assert!(levels[0..4] == [0.0; 4] && levels[8..12] == [0.0; 4]);
        // (The peak still remembers the louder square wave from before.)
        assert!(levels[4] == 0.0 && levels[5] >= 0.2499 && levels[5] < 1.0 && levels[7] > 0.01, "{levels:?}");
        assert_eq!(synths_levels_read(e, 0xFFFF, 3, levels.as_mut_ptr()), 1);
        assert!(levels[1] >= 0.2499 && levels[1] < 0.36);
        assert_eq!(synths_levels_read(e, 62, 5, levels.as_mut_ptr()), 2);
        let mut notes = [0u32; 8];
        assert_eq!(synths_notes_read(e, 2, notes.as_mut_ptr()), 8);
        assert_eq!(notes, [0, 1 << 28, 0, 0, 0, 1 << 28, 0, 0], "note 60 is held and sounding");
        assert_eq!(synths_notes_read(e, 3, notes.as_mut_ptr()), 8);
        assert_eq!(notes, [0; 8]);
        assert_eq!(synths_notes_read(e, 0xFFFF, notes.as_mut_ptr()), 8);
        assert_eq!(notes[1], 1 << 28);
        assert_eq!(synths_notes_read(e, 64, notes.as_mut_ptr()), 0);
        synths_free(scope as *mut u8);

        // A step macro: an arpeggio of three steps on part 2.
        let steps: [f32; 15] = [-1.0, 0.0, -1.0, 0.0, 1.0, -1.0, 4.0, -1.0, 0.0, 1.0, -1.0, 7.0, -1.0, 0.0, 1.0];
        assert_eq!(synths_macro_set(e, 2, steps.as_ptr(), 3, 0, 3, 255, 0), 0);
        assert_eq!(synths_macro_set(e, 0xFFFF, steps.as_ptr(), 3, 0, 3, 255, 0), 0);
        assert_eq!(synths_macro_set(e, 64, steps.as_ptr(), 3, 0, 3, 255, 0), -1);
        assert_eq!(synths_macro_set(e, 2, steps.as_ptr(), 65, 0, 3, 255, 0), -2);
        assert_eq!(synths_macro_set(e, 2, std::ptr::null(), 3, 0, 3, 255, 0), -4);
        assert_eq!(synths_macro_set(e, 2, std::ptr::null(), 0, 0, 0, 0, 0), 0);

        // Filling the queue: the count taken says where it stopped.
        synths_reset(e);
        assert_eq!(synths_active_voices(e), 0);
        for i in 0..256 {
            write_event(buf, i, 1.0e12, 4, 0, CUTOFF as u32, 500.0, 0.0);
        }
        for _ in 0..16 {
            assert_eq!(synths_push_events(e, 256), 256);
        }
        assert_eq!(synths_push_events(e, 256), 0);
        assert_eq!(synths_push_events(e, 100_000), 0);
        synths_destroy(e);

        // Null is safe everywhere.
        let none = std::ptr::null_mut();
        synths_destroy(none);
        synths_reset(none);
        synths_free(none as *mut u8);
        assert_eq!(synths_render(none, 128), 0);
        assert!(synths_output_left(none).is_null() && synths_event_buffer(none).is_null());
        assert_eq!(synths_push_events(none, 1), 0);
        assert_eq!(synths_sample_free(none, 0), -4);
        assert_eq!(synths_macro_set(none, 0, std::ptr::null(), 0, 0, 0, 0, 0), -4);
        let mut word = [0u32; 8];
        let mut number = [0.0f32; 4];
        assert_eq!(synths_scope_read(none, 0, number.as_mut_ptr(), 4), 0);
        assert_eq!(synths_levels_read(none, 0, 1, number.as_mut_ptr()), 0);
        assert_eq!(synths_notes_read(none, 0, word.as_mut_ptr()), 0);
    }
}

#[test]
fn the_parameter_table_is_json_a_host_can_read() {
    unsafe {
        let text = c_str(synths_param_table());
        assert_eq!(text.len() as u32, synths_param_table_len());
        assert!(text.starts_with("{\"version\":\"") && text.ends_with("]}"));
        assert!(text.contains("\"abi\":1"));
        assert!(text.contains("\"limits\":{\"voices\":64,\"parts\":64,\"samples\":1024,\"event_size\":32,\"event_buffer\":256,\"event_queue\":4096,\"output_frames\":4096,\"macro_steps\":64,\"scope_master\":4096,\"scope_part\":1024}"));
        assert!(text.contains("{\"type\":1,\"name\":\"note_on\""));
        assert!(text.contains("{\"id\":50,\"name\":\"filter.cutoff\",\"scope\":\"part\",\"kind\":\"float\",\"min\":20,\"max\":20000,\"default\":2000,\"unit\":\"hz\","));
        assert!(text.contains("{\"id\":16,\"name\":\"osc1.wave\",\"scope\":\"part\",\"kind\":\"choice\",\"min\":0,\"max\":4,\"default\":1,\"unit\":\"\",\"choices\":[\"sine\",\"triangle\",\"saw\",\"pulse\",\"sample\"],"));
        assert!(text.contains("{\"id\":200,\"name\":\"master.volume\",\"scope\":\"engine\""));
        // Every parameter is there once, and the brackets balance.
        for p in PARAMS {
            assert_eq!(text.matches(&format!("{{\"id\":{},\"name\":\"{}\"", p.id, p.name)).count(), 1);
        }
        let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
        for c in text.chars() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else {
                match c {
                    '"' => in_string = true,
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    _ => {}
                }
                assert!(depth >= 0);
            }
        }
        assert!(depth == 0 && !in_string);
    }
}
