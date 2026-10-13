mod common;
use common::*;
use synths_core::engine::MAX_VOICES;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;
use synths_core::{Engine, Event};

/// A very short blip on part 0, 100 samples long.
fn blip(e: &mut Engine) {
    e.set_param(0, OSC1_WAVE, 3.0);
    e.set_param(0, PAN, 0.0);
    note_on(e, 0, 0, 72.0);
    note_off(e, 100, 0, 72.0);
}

#[test]
fn delay_repeats_at_the_time_set_and_fades() {
    let mut e = plain_engine();
    e.set_param(0, DELAY_TIME, 0.25);
    e.set_param(0, DELAY_FEEDBACK, 0.5);
    e.set_param(0, DELAY_TONE, 20000.0);
    e.set_param(0, SEND_DELAY, 1.0);
    blip(&mut e);
    let (l, r) = render(&mut e, 48000, 128);
    assert!(peak(&l[200..11900]) < 1e-3, "silence between the note and its echo");
    let first = rms(&l[12000..12100]);
    let second = rms(&l[24000..24100]);
    let third = rms(&l[36000..36100]);
    let dry = rms(&l[0..100]);
    assert!((first / dry - 1.0).abs() < 0.15, "{first} vs {dry}");
    assert!((second / first - 0.5).abs() < 0.08 && (third / second - 0.5).abs() < 0.08);
    assert!((rms(&r[12000..12100]) / first - 1.0).abs() < 0.01);
    // Ping-pong: the echoes take turns, left then right.
    let mut e = plain_engine();
    e.set_param(0, DELAY_TIME, 0.25);
    e.set_param(0, DELAY_PINGPONG, 1.0);
    e.set_param(0, DELAY_FEEDBACK, 0.7);
    e.set_param(0, SEND_DELAY, 1.0);
    blip(&mut e);
    let (l, r) = render(&mut e, 48000, 128);
    assert!(rms(&l[12000..12100]) > 10.0 * rms(&r[12000..12100]));
    assert!(rms(&r[24000..24100]) > 10.0 * rms(&l[24000..24100]));
    // A darker tone setting takes the edge off each echo.
    let bright = first;
    let mut e = plain_engine();
    e.set_param(0, DELAY_TIME, 0.25);
    e.set_param(0, DELAY_TONE, 300.0);
    e.set_param(0, SEND_DELAY, 1.0);
    blip(&mut e);
    let (l, _) = render(&mut e, 24000, 128);
    assert!(rms(&l[12000..12200]) < 0.6 * bright);
}

#[test]
fn reverb_rings_on_and_dies_away() {
    let tail = |size: f32| {
        let mut e = plain_engine();
        e.set_param(0, REVERB_SIZE, size);
        e.set_param(0, SEND_REVERB, 1.0);
        blip(&mut e);
        let (l, r) = render(&mut e, 96000, 128);
        assert!(l != r, "the reverb is in stereo");
        (rms(&l[2400..7200]), rms(&l[24000..28800]), rms(&l[86400..96000]))
    };
    let (early, mid, late) = tail(0.5);
    assert!(early > 0.005, "{early}");
    assert!(mid < early && late < 0.1 * mid);
    // A bigger room rings for longer.
    assert!(tail(1.0).1 > 2.0 * mid);
}

#[test]
fn chorus_adds_a_moving_copy() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, PAN, 0.0);
    e.set_param(0, SEND_CHORUS, 1.0);
    note_on(&mut e, 0, 0, 69.0);
    let (l, r) = render(&mut e, 96000, 128);
    // The copy drifts in and out of step with the note, so the level wavers.
    let levels: Vec<f32> = l[4800..].chunks(2400).map(rms).collect();
    let hi = levels.iter().cloned().fold(0.0, f32::max);
    let lo = levels.iter().cloned().fold(9.0, f32::min);
    assert!(hi > 1.1 * lo, "{lo}..{hi}");
    assert!(l[48000..] != r[48000..]);
}

#[test]
fn distortion_and_bit_crusher() {
    let run = |id: usize, value: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 0.0);
        e.set_param(0, id, value);
        e.push(Event::note_on(0.0, 0, 0, 57.0, 0.5));
        render(&mut e, 48000, 128).0
    };
    let third = |x: &[f32]| db(amplitude_at(x, 660.0, SR) / amplitude_at(x, 220.0, SR));
    assert!(third(&run(DRIVE, 0.0)) < -80.0);
    let driven = run(DRIVE, 0.6);
    assert!(third(&driven) > -25.0, "distortion adds harmonics");
    assert!(peak(&driven) <= 1.0);
    // Two bits leave only a few levels.
    let crushed = run(CRUSH_BITS, 2.0);
    let mut levels: Vec<i32> = crushed[1000..].iter().map(|v| (v * 1000.0).round() as i32).collect();
    levels.sort();
    levels.dedup();
    assert!(levels.len() <= 3, "{levels:?}");
    // Keeping one sample in eight holds each value for eight samples.
    let held = run(CRUSH_RATE, 8.0);
    let changes = held[1000..9000].windows(2).filter(|w| w[0] != w[1]).count();
    assert!(changes <= 1001 && changes > 900, "{changes}");
}

#[test]
fn the_limiter_keeps_everything_under_the_ceiling() {
    for limiter in [1.0, 0.0] {
        let mut e = Engine::new(SR);
        e.set_param(0, MASTER_VOLUME, 2.0);
        e.set_param(0, LIMITER, limiter);
        e.set_param(ALL_PARTS, OSC1_WAVE, 3.0);
        e.set_param(ALL_PARTS, VOLUME, 2.0);
        for i in 0..MAX_VOICES as u16 {
            e.push(Event::note_on(i as f64 * 50.0, i % 8, 0, 36.0 + i as f32, 1.0));
        }
        let (l, r) = render(&mut e, 48000, 128);
        let top = peak(&l).max(peak(&r));
        if limiter > 0.0 {
            assert!(top <= 0.98 + 1e-6 && top > 0.9, "{top}");
        } else {
            assert!(top <= 1.0, "{top}");
        }
    }
    // Quiet sound passes through the limiter untouched.
    let run = |limiter: f32| {
        let mut e = plain_engine();
        e.set_param(0, LIMITER, limiter);
        e.set_param(0, MASTER_VOLUME, 0.5);
        note_on(&mut e, 0, 0, 60.0);
        render(&mut e, 4800, 128)
    };
    assert!(run(1.0) == run(0.0));
}

#[test]
fn volume_and_pan_place_the_sound() {
    let run = |volume: f32, pan: f32, master: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 3.0);
        e.set_param(0, VOLUME, volume);
        e.set_param(0, PAN, pan);
        e.set_param(0, MASTER_VOLUME, master);
        note_on(&mut e, 0, 0, 45.0);
        let (l, r) = render(&mut e, 4800, 128);
        (rms(&l[2400..]), rms(&r[2400..]))
    };
    let near = |got: f32, want: f32| assert!((got - want).abs() < 4e-3, "{got} vs {want}");
    let (l, r) = run(1.0, 0.0, 1.0);
    near(l, std::f32::consts::FRAC_1_SQRT_2);
    near(r, std::f32::consts::FRAC_1_SQRT_2);
    let (l, r) = run(1.0, -1.0, 1.0);
    near(l, 1.0);
    near(r, 0.0);
    let (l, r) = run(0.5, 1.0, 0.5);
    near(l, 0.0);
    near(r, 0.25);
    // Velocity is loudness in a straight line, as much as the patch says.
    let vel = |sens: f32, velocity: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 3.0);
        e.set_param(0, VELOCITY_SENS, sens);
        e.push(Event::note_on(0.0, 0, 0, 45.0, velocity));
        rms(&render(&mut e, 4800, 128).0[2400..])
    };
    near(vel(1.0, 0.25), 0.25);
    near(vel(0.0, 0.25), 1.0);
    near(vel(0.5, 0.5), 0.75);
    // A change of volume while a note sounds is a quick slide, not a step.
    let mut e = plain_engine();
    e.load_sample(0, &[0.5; 16], 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 16).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    note_on(&mut e, 0, 0, 60.0);
    e.push(Event::param(1000.0, 0, VOLUME, 0.0, 0.0));
    let x = render(&mut e, 8000, 128).0;
    assert!(x[1000] > 0.49 && x[1100] < 0.4 && x[1100] > 0.2 && x[7000] == 0.0);
}

#[test]
fn the_engine_falls_silent_and_wakes_again() {
    let mut e = Engine::new(SR);
    e.set_param(0, SEND_REVERB, 1.0);
    e.set_param(0, SEND_DELAY, 1.0);
    e.set_param(0, DELAY_FEEDBACK, 0.3);
    let (l, r) = render(&mut e, 1000, 128);
    assert!(l.iter().chain(r.iter()).all(|&v| v == 0.0), "silent before anything is played");
    note_on(&mut e, 1000, 0, 60.0);
    note_off(&mut e, 5000, 0, 60.0);
    let (l, _) = render(&mut e, 48000 * 30, 128);
    assert!(peak(&l[..48000]) > 0.05);
    assert!(l[48000 * 25..].iter().all(|&v| v == 0.0), "exactly silent once the tails have gone");
    let at = e.time() + 77;
    note_on(&mut e, at, 0, 64.0);
    let (l, _) = render(&mut e, 4800, 128);
    assert_eq!(l.iter().position(|&v| v != 0.0), Some(77));
}
