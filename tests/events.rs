mod common;
use common::*;
use synths_core::engine::{MAX_VOICES, QUEUE_CAPACITY};
use synths_core::event::*;
use synths_core::params::*;
use synths_core::{Engine, Event};

#[test]
fn a_note_starts_on_exactly_the_sample_asked_for() {
    for time in [0u64, 1, 15, 16, 17, 127, 128, 1000, 1237, 4095, 4096, 99_999] {
        for block in [1usize, 7, 128, 4096] {
            let mut e = plain_engine();
            e.set_param(0, OSC1_WAVE, 3.0);
            note_on(&mut e, time, 0, 60.0);
            let x = render(&mut e, time as usize + 300, block).0;
            let first = x.iter().position(|&v| v != 0.0);
            assert_eq!(first, Some(time as usize), "time {time}, block {block}");
        }
    }
}

#[test]
fn a_note_stops_on_exactly_the_sample_asked_for() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    note_on(&mut e, 10, 0, 60.0);
    note_off(&mut e, 2345, 0, 60.0);
    let x = render(&mut e, 4000, 128).0;
    assert!(x[2344] != 0.0);
    assert!(x[2345..].iter().all(|&v| v == 0.0));
}

#[test]
fn events_at_the_same_time_happen_in_the_order_pushed() {
    let mut e = plain_engine();
    for i in 0..50 {
        e.push(Event::param(500.0, 0, CUTOFF, 100.0 + i as f32, 0.0));
    }
    // Pushed late and out of order, events still happen in time order.
    e.push(Event::param(100.0, 0, CUTOFF, 5000.0, 0.0));
    let _ = render(&mut e, 200, 64);
    assert_eq!(e.param(0, CUTOFF), 5000.0);
    let _ = render(&mut e, 400, 64);
    assert_eq!(e.param(0, CUTOFF), 149.0);
}

#[test]
fn events_in_the_past_happen_at_once() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    let _ = render(&mut e, 1000, 128);
    note_on(&mut e, 10, 0, 60.0);
    e.push(Event::note_on(f64::NAN, 1, 0, 60.0, 1.0));
    e.push(Event::note_on(-5.0, 2, 0, 60.0, 1.0));
    let x = render(&mut e, 10, 10).0;
    assert!(x[0] != 0.0);
    assert_eq!(e.active_voices(), 3);
}

#[test]
fn the_queue_says_when_it_is_full() {
    let mut e = plain_engine();
    for i in 0..QUEUE_CAPACITY {
        assert!(e.push(Event::param(1.0e9 + i as f64, 0, CUTOFF, 100.0, 0.0)));
    }
    assert!(!e.push(Event::note_on(0.0, 0, 0, 60.0, 1.0)));
    assert_eq!(e.queued_events(), QUEUE_CAPACITY);
    e.reset();
    assert_eq!(e.queued_events(), 0);
}

#[test]
fn parameters_glide_and_stay_in_range() {
    let mut e = plain_engine();
    e.push(Event::param(0.0, 0, CUTOFF, 1000.0, 0.0));
    e.push(Event::param(4800.0, 0, CUTOFF, 3000.0, 0.5));
    let _ = render(&mut e, 4800 + 12000, 128);
    let half = e.param(0, CUTOFF);
    assert!((half - 2000.0).abs() < 5.0, "{half}");
    let _ = render(&mut e, 12000, 128);
    assert_eq!(e.param(0, CUTOFF), 3000.0);
    // Out of range values are brought in; nonsense is ignored; choices are whole numbers.
    e.set_param(0, CUTOFF, 1.0e9);
    assert_eq!(e.param(0, CUTOFF), 20000.0);
    e.set_param(0, CUTOFF, f32::NAN);
    assert_eq!(e.param(0, CUTOFF), 20000.0);
    e.set_param(0, OSC1_WAVE, 2.4);
    assert_eq!(e.param(0, OSC1_WAVE), 2.0);
    e.set_param(0, 999, 1.0);
    e.set_param(900, CUTOFF, 1.0);
    // Every part, and an engine-wide parameter whatever the part.
    e.set_param(ALL_PARTS, PAN, 0.25);
    assert_eq!(e.param(63, PAN), 0.25);
    e.set_param(17, DELAY_TIME, 0.5);
    assert_eq!(e.param(0, DELAY_TIME), 0.5);
    // Pitch bend is the bend parameter.
    e.push(Event::pitch_bend(0.0, 3, -0.5));
    let _ = render(&mut e, 16, 16);
    assert_eq!(e.param(3, BEND), -0.5);
}

#[test]
fn new_notes_take_over_the_oldest_voices() {
    let mut e = plain_engine();
    e.set_param(0, VOICES, 4.0);
    for i in 0..6u64 {
        note_on(&mut e, i * 100, 0, 60.0 + i as f32);
    }
    let _ = render(&mut e, 1000, 128);
    assert_eq!(e.active_voices(), 4);
    // The two oldest notes are the ones that went: letting them go changes nothing.
    note_off(&mut e, 1000, 0, 60.0);
    note_off(&mut e, 1000, 0, 61.0);
    let _ = render(&mut e, 100, 128);
    assert_eq!(e.active_voices(), 4);
    for i in 2..6 {
        note_off(&mut e, 1100, 0, 60.0 + i as f32);
    }
    let _ = render(&mut e, 100, 128);
    assert_eq!(e.active_voices(), 0);

    // A voice that has been let go is taken before one still held.
    let mut e = plain_engine();
    e.set_param(0, VOICES, 2.0);
    e.set_param(0, AMP_RELEASE, 5.0);
    note_on(&mut e, 0, 0, 60.0);
    note_on(&mut e, 10, 0, 64.0);
    note_off(&mut e, 20, 0, 64.0);
    note_on(&mut e, 30, 0, 67.0);
    let _ = render(&mut e, 100, 128);
    note_off(&mut e, 100, 0, 60.0);
    note_off(&mut e, 100, 0, 67.0);
    e.set_param(0, AMP_RELEASE, 0.0);
    let _ = render(&mut e, 100, 128);
    assert_eq!(e.active_voices(), 0, "60 and 67 were the two voices left");

    // The whole pool.
    let mut e = plain_engine();
    for i in 0..(MAX_VOICES as u16 + 10) {
        e.push(Event::note_on(0.0, i % 16, 0, 40.0 + i as f32, 0.5));
    }
    let _ = render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), MAX_VOICES);
}

#[test]
fn taking_over_a_voice_does_not_click() {
    // One voice, a low sine, cut off mid-wave by a new note far quieter:
    // the output must fall away smoothly, not drop in one sample.
    let mut e = plain_engine();
    e.set_param(0, VOICES, 1.0);
    e.set_param(0, OSC1_WAVE, 0.0);
    note_on(&mut e, 0, 0, 45.0);
    e.push(Event::note_on(1000.0, 0, 0, 45.0, 0.0));
    let x = render(&mut e, 2000, 128).0;
    assert!(x[999].abs() > 0.5);
    for i in 1000..2000 {
        assert!((x[i] - x[i - 1]).abs() < 0.03, "a jump of {} at {i}", x[i] - x[i - 1]);
    }
    assert!(x[1999].abs() < 1e-3);
}

#[test]
fn live_voices_can_be_steered_by_handle() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, PAN, 0.0);
    e.push(Event::note_on(0.0, 0, 1, 69.0, 1.0));
    e.push(Event::note_on(0.0, 1, 1, 81.0, 1.0));
    e.push(Event::voice_volume(0.0, 1, 1, 0.0));
    e.push(Event::voice_freq(24000.0, 0, 1, 300.0, 0.0));
    e.push(Event::voice_pitch(48000.0, 0, 1, 57.0, 0.0));
    e.push(Event::voice_volume(72000.0, 0, 1, 0.5));
    e.push(Event::voice_pan(72000.0, 0, 1, 1.0));
    let (l, r) = render(&mut e, 96000, 128);
    let near = |got: f32, want: f32| assert!((got / want - 1.0).abs() < 1e-3, "{got} vs {want}");
    near(frequency(&l[2000..24000], SR), 440.0);
    near(frequency(&l[24100..48000], SR), 300.0);
    near(frequency(&l[48100..72000], SR), 220.0);
    near(peak(&l[60000..72000]), std::f32::consts::FRAC_1_SQRT_2);
    near(peak(&r[80000..]), 0.5);
    assert!(peak(&l[80000..]) < 1e-4);
    // The change of volume is a short slide, not a step.
    for i in 71990..72200 {
        assert!((r[i] - r[i - 1]).abs() < 0.04 && (l[i] - l[i - 1]).abs() < 0.04);
    }
    // A slide of pitch by handle.
    e.push(Event::voice_pitch(96000.0, 0, 1, 69.0, 1.0));
    let (_, r) = render(&mut e, 96000, 128);
    near(frequency(&r[24000 - 480..24000 + 480], SR), hz(63.0));
    near(frequency(&r[60000..], SR), 440.0);
    // Note-off by handle, and a cut by handle.
    e.push(Event::note_off(192000.0, 0, 1, 0.0));
    e.push(Event::voice_kill(192000.0, 1, 1));
    let _ = render(&mut e, 4800, 128);
    assert_eq!(e.active_voices(), 0);
    // Handle 0 addresses nothing.
    e.push(Event::note_on(196800.0, 0, 0, 60.0, 1.0));
    e.push(Event::voice_kill(196800.0, 0, 0));
    let _ = render(&mut e, 128, 128);
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn all_notes_off_releases_or_cuts() {
    let mut e = plain_engine();
    e.set_param(ALL_PARTS, AMP_RELEASE, 1.0);
    for part in 0..8 {
        note_on(&mut e, 0, part, 60.0);
    }
    e.push(Event::all_notes_off(100.0, 3, false));
    let _ = render(&mut e, 200, 128);
    assert_eq!(e.active_voices(), 8, "released, still dying away");
    e.push(Event::all_notes_off(200.0, 3, true));
    let _ = render(&mut e, 100, 128);
    assert_eq!(e.active_voices(), 7);
    e.push(Event::all_notes_off(300.0, ALL_PARTS, true));
    let _ = render(&mut e, 100, 128);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn the_same_commands_always_make_the_same_sound() {
    fn song(e: &mut Engine) {
        e.set_param(0, OSC1_WAVE, 2.0);
        e.set_param(0, OSC2_LEVEL, 0.7);
        e.set_param(0, OSC2_FINE, 9.0);
        e.set_param(0, NOISE_LEVEL, 0.2);
        e.set_param(0, FILTER_TYPE, 1.0);
        e.set_param(0, FILTER_ENV, 36.0);
        e.set_param(0, LFO1_PITCH, 0.3);
        e.set_param(0, LFO2_CUTOFF, 12.0);
        e.set_param(0, LFO2_WAVE, 4.0);
        e.set_param(0, UNISON, 3.0);
        e.set_param(0, SEND_REVERB, 0.4);
        e.set_param(0, SEND_DELAY, 0.3);
        e.set_param(0, SEND_CHORUS, 0.3);
        e.set_param(0, DRIVE, 0.3);
        e.set_param(1, NOISE_LEVEL, 1.0);
        e.set_param(1, NOISE_TYPE, 1.0);
        e.set_param(1, OSC1_LEVEL, 0.0);
        e.set_param(1, AMP_SUSTAIN, 0.0);
        for i in 0..40u64 {
            let t = i * 2399;
            e.push(Event::note_on(t as f64, 0, 0, 40.0 + (i * 7 % 24) as f32, 0.8));
            e.push(Event::note_off((t + 1800) as f64, 0, 0, 40.0 + (i * 7 % 24) as f32));
            e.push(Event::note_on((t + 601) as f64, 1, 0, 60.0, 1.0));
            e.push(Event::param(t as f64, 0, CUTOFF, 300.0 + 50.0 * i as f32, 0.02));
        }
    }
    let run = |block: usize| {
        let mut e = Engine::new(SR);
        song(&mut e);
        render(&mut e, 120_000, block)
    };
    let a = run(128);
    assert!(rms(&a.0) > 0.01 && rms(&a.1) > 0.01);
    assert!(a.0 != a.1, "it is in stereo");
    // Bit for bit the same, however the host slices time.
    for block in [128, 1, 64, 333, 4096] {
        let b = run(block);
        assert!(a.0 == b.0 && a.1 == b.1, "block size {block} gave a different sound");
    }
}
