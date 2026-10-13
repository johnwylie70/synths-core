mod common;
use common::*;
use synths_core::params::*;
use synths_core::Event;

/// A sample whose every frame holds its own index (scaled down), so the
/// output shows exactly which frame is playing.
fn ramp_engine(frames: usize) -> Box<synths_core::Engine> {
    let mut e = plain_engine();
    let data: Vec<f32> = (0..frames).map(|i| i as f32 / 1000.0).collect();
    e.load_sample(3, &data, 1, SR).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, OSC1_SAMPLE, 3.0);
    e
}

fn frames_played(x: &[f32]) -> Vec<i32> {
    x.iter().map(|v| (v * 1000.0).round() as i32).collect()
}

#[test]
fn one_shot_plays_once_and_frees_its_voice() {
    let mut e = ramp_engine(50);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 2000, 128).0;
    let want: Vec<i32> = (0..50).collect();
    assert_eq!(&frames_played(&x[..50]), &want);
    for (i, v) in x[..50].iter().enumerate() {
        assert!((v - i as f32 / 1000.0).abs() < 1e-6, "frame {i} is {v}");
    }
    // Then a very short fade from the last value rather than a click, and silence.
    assert!(x[50] > 0.0 && x[50] < 0.049 && x[60] < x[50]);
    assert!(x[1000..].iter().all(|&v| v == 0.0));
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn forward_loop_goes_round_exactly() {
    let mut e = ramp_engine(50);
    e.configure_sample(3, 60.0, 1, 10, 20).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 200, 64).0;
    let got = frames_played(&x);
    for (i, &g) in got.iter().enumerate() {
        let want = if i < 20 { i } else { 10 + (i - 10) % 10 };
        assert_eq!(g, want as i32, "output sample {i}");
    }
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn ping_pong_loop_goes_there_and_back() {
    let mut e = ramp_engine(50);
    e.configure_sample(3, 60.0, 2, 10, 14).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 40, 7).0;
    let got = frames_played(&x);
    let want = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 13, 12, 11, 10, 10, 11, 12, 13, 13, 12, 11, 10, 10, 11];
    assert_eq!(&got[..want.len()], &want);
}

#[test]
fn start_offset_and_position_events() {
    let mut e = ramp_engine(1000);
    e.set_param(0, OSC1_START, 0.5);
    e.push(Event::note_on(0.0, 0, 7, 60.0, 1.0));
    e.push(Event::voice_position(100.0, 0, 7, 20.0));
    let x = render(&mut e, 900, 128).0;
    let got = frames_played(&x);
    assert_eq!(got[0], 500);
    assert_eq!(got[99], 599);
    // The jump is exact, but for a moment the old sound is still fading under it.
    assert!(got[100] > 20);
    assert_eq!(got[899], 20 + 799);
    // A position past the end of a one-shot ends the note.
    e.push(Event::voice_position(900.0, 0, 7, 5000.0));
    let _ = render(&mut e, 64, 64);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn pitch_follows_the_note_and_the_base_note() {
    // One cycle of a sine in 100 frames: 480 Hz at its own rate.
    let cycle: Vec<f32> = (0..100).map(|i| (std::f32::consts::TAU * i as f32 / 100.0).sin() * 0.5).collect();
    let run = |note: f32, base: f32, rate: f32, keytrack: f32| {
        let mut e = plain_engine();
        e.load_sample(0, &cycle, 1, rate).unwrap();
        e.configure_sample(0, base, 1, 0, 100).unwrap();
        e.set_param(0, OSC1_WAVE, 4.0);
        e.set_param(0, OSC1_KEYTRACK, keytrack);
        note_on(&mut e, 0, 0, note);
        let x = render(&mut e, 48000, 128).0;
        (frequency(&x[1000..], SR), x)
    };
    let close = |got: f32, want: f32| assert!((got / want - 1.0).abs() < 2e-4, "{got} vs {want}");
    close(run(60.0, 60.0, SR, 1.0).0, 480.0);
    close(run(72.0, 60.0, SR, 1.0).0, 960.0);
    close(run(53.0, 60.0, SR, 1.0).0, 480.0 * 2.0f32.powf(-7.0 / 12.0));
    close(run(60.0, 48.0, SR, 1.0).0, 960.0);
    close(run(60.0, 60.0, 24000.0, 1.0).0, 240.0);
    close(run(60.0, 60.5, SR, 1.0).0, 480.0 * 2.0f32.powf(-0.5 / 12.0));
    close(run(72.0, 60.0, SR, 0.0).0, 480.0);
    // Played between frames, the curve through them is still a clean sine.
    let (f, x) = run(67.3, 60.0, SR, 1.0);
    let x = &x[4800..];
    assert!(db(amplitude_at(x, 2.0 * f, SR) / amplitude_at(x, f, SR)) < -55.0);
    assert!((peak(x) - 0.5).abs() < 0.01);
}

#[test]
fn stereo_samples_keep_their_sides() {
    let mut e = plain_engine();
    e.set_param(0, PAN, 0.0);
    let data: Vec<f32> = (0..200).flat_map(|i| [i as f32 / 1000.0, -(i as f32) / 2000.0]).collect();
    e.load_sample(1, &data, 2, SR).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, OSC1_SAMPLE, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let (l, r) = render(&mut e, 200, 128);
    for i in 0..200 {
        assert!((l[i] - i as f32 / 1000.0).abs() < 1e-6, "{i}: {} {}", l[i], r[i]);
        assert!((r[i] + i as f32 / 2000.0).abs() < 1e-6);
    }
    // As a second oscillator a stereo sample is mixed to one channel.
    let mut e = plain_engine();
    e.load_sample(1, &data, 2, SR).unwrap();
    e.set_param(0, OSC1_LEVEL, 0.0);
    e.set_param(0, OSC2_WAVE, 4.0);
    e.set_param(0, OSC2_SAMPLE, 1.0);
    e.set_param(0, OSC2_LEVEL, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let (l, _) = render(&mut e, 200, 128);
    assert!((l[100] - 0.025).abs() < 1e-5, "{}", l[100]);
}

#[test]
fn bad_samples_are_refused_and_missing_ones_are_silent() {
    let mut e = plain_engine();
    assert!(e.load_sample(5000, &[0.0; 4], 1, SR).is_err());
    assert!(e.load_sample(0, &[0.0; 4], 3, SR).is_err());
    assert!(e.load_sample(0, &[], 1, SR).is_err());
    assert!(e.load_sample(0, &[0.0; 4], 1, 0.0).is_err());
    assert!(e.configure_sample(9, 60.0, 1, 0, 4).is_err());
    // Not-a-number in a sample becomes silence, and loop points are kept in range.
    e.load_sample(0, &[f32::NAN, f32::INFINITY, 0.5, 100.0], 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 900, 1000).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 256, 128).0;
    assert!(x.iter().all(|v| v.is_finite()));
    // A slot that is emptied under a playing voice just stops.
    e.configure_sample(0, 60.0, 1, 0, 4).unwrap();
    note_on(&mut e, 256, 0, 62.0);
    let _ = render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), 1);
    e.free_sample(0).unwrap();
    let x = render(&mut e, 2048, 128).0;
    assert_eq!(e.active_voices(), 0);
    assert!(x[1024..].iter().all(|&v| v == 0.0));
}

#[test]
fn a_one_shot_plays_backwards_from_its_last_frame() {
    let mut e = ramp_engine(50);
    e.set_param(0, OSC1_REVERSE, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 2000, 128).0;
    let want: Vec<i32> = (0..50).rev().collect();
    assert_eq!(&frames_played(&x[..50]), &want);
    assert!(x[1000..].iter().all(|&v| v == 0.0));
    assert_eq!(e.active_voices(), 0);
    // `osc1.start` is then measured from the end.
    let mut e = ramp_engine(1000);
    e.set_param(0, OSC1_REVERSE, 1.0);
    e.set_param(0, OSC1_START, 0.5);
    note_on(&mut e, 0, 0, 60.0);
    let got = frames_played(&render(&mut e, 600, 128).0);
    assert_eq!((got[0], got[1], got[499]), (499, 498, 0));
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn loops_play_backwards() {
    // A forward loop goes round the other way, starting at its end.
    let mut e = ramp_engine(50);
    e.configure_sample(3, 60.0, 1, 10, 20).unwrap();
    e.set_param(0, OSC1_REVERSE, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let got = frames_played(&render(&mut e, 200, 64).0);
    for (i, &g) in got.iter().enumerate() {
        assert_eq!(g, 19 - (i % 10) as i32, "output sample {i}");
    }
    // A ping-pong loop starts on its way back.
    let mut e = ramp_engine(50);
    e.configure_sample(3, 60.0, 2, 10, 14).unwrap();
    e.set_param(0, OSC1_REVERSE, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let got = frames_played(&render(&mut e, 40, 7).0);
    let want = [13, 12, 11, 10, 10, 11, 12, 13, 13, 12, 11, 10, 10, 11];
    assert_eq!(&got[..want.len()], &want);
    // Backwards at half speed, the curve between frames still runs true.
    let mut e = ramp_engine(50);
    e.configure_sample(3, 60.0, 1, 10, 30).unwrap();
    e.set_param(0, OSC1_REVERSE, 1.0);
    note_on(&mut e, 0, 0, 48.0);
    let x = render(&mut e, 34, 64).0;
    // (Away from the ends of the loop, where the ramp jumps.)
    for (i, v) in x.iter().enumerate().skip(4) {
        assert!((v - (29.0 - i as f32 * 0.5) / 1000.0).abs() < 1e-5, "output sample {i} is {v}");
    }
}

#[test]
fn a_sounding_sample_can_be_turned_round() {
    let mut e = ramp_engine(1000);
    note_on(&mut e, 0, 0, 60.0);
    e.push(Event::param(96.0, 0, OSC1_REVERSE, 1.0, 0.0));
    e.push(Event::param(128.0, 0, OSC1_REVERSE, 0.0, 0.0));
    let got = frames_played(&render(&mut e, 200, 128).0);
    assert_eq!(&got[94..99], &[94, 95, 96, 95, 94]);
    assert_eq!(&got[126..131], &[66, 65, 64, 65, 66]);
    // The same sound comes out whatever the block size.
    let mut e = ramp_engine(1000);
    note_on(&mut e, 0, 0, 60.0);
    e.push(Event::param(96.0, 0, OSC1_REVERSE, 1.0, 0.0));
    e.push(Event::param(128.0, 0, OSC1_REVERSE, 0.0, 0.0));
    assert_eq!(frames_played(&render(&mut e, 200, 7).0), got);
}

/// A looped sample holding two tones, played `semitones` above its own pitch.
fn two_tones(low: f32, high: f32, semitones: f32, antialias: f32, stereo: bool) -> Vec<f32> {
    let mut e = plain_engine();
    let tone = |i: usize| {
        let t = std::f32::consts::TAU * i as f32 / SR;
        0.4 * (t * low).sin() + 0.4 * (t * high).sin()
    };
    let data: Vec<f32> = if stereo { (0..4800).flat_map(|i| [tone(i), tone(i)]).collect() } else { (0..4800).map(tone).collect() };
    e.load_sample(0, &data, if stereo { 2 } else { 1 }, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 4800).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, SAMPLE_ANTIALIAS, antialias);
    note_on(&mut e, 0, 0, 60.0 + semitones);
    render(&mut e, 48000, 128).0
}

#[test]
fn a_sample_pitched_far_up_is_smoothed() {
    // 1 kHz and 15 kHz, two octaves up: 4 kHz, and 60 kHz, which 48 kHz
    // cannot hold. Raw, it folds down to 12 kHz as loud as it went in.
    for stereo in [false, true] {
        let raw = two_tones(1000.0, 15000.0, 24.0, 0.0, stereo);
        let smooth = two_tones(1000.0, 15000.0, 24.0, 1.0, stereo);
        let raw_alias = db(amplitude_at(&raw[4800..], 12000.0, SR) / 0.4);
        let alias = db(amplitude_at(&smooth[4800..], 12000.0, SR) / 0.4);
        let kept = db(amplitude_at(&smooth[4800..], 4000.0, SR) / 0.4);
        assert!(raw_alias > -3.0, "raw alias {raw_alias} dB");
        assert!(alias < -24.0, "alias {alias} dB (stereo: {stereo})");
        assert!(kept > -0.5 && kept < 0.1, "the 4 kHz tone is at {kept} dB");
    }
    // Three octaves and a bit: 2 kHz becomes 17.6 kHz and is kept; 11 kHz
    // would be 96.9 kHz, and folds to 0.9 kHz.
    let up = 12.0 * 8.81f32.log2();
    let raw = two_tones(2000.0, 11000.0, up, 0.0, false);
    let smooth = two_tones(2000.0, 11000.0, up, 1.0, false);
    let fold = (11000.0 * 8.81f32 - 2.0 * SR).abs();
    let raw_alias = db(amplitude_at(&raw[4800..], fold, SR) / 0.4);
    let alias = db(amplitude_at(&smooth[4800..], fold, SR) / 0.4);
    assert!(raw_alias > -3.0 && alias < raw_alias - 20.0, "alias {alias} dB, raw {raw_alias} dB");
}

#[test]
fn smoothing_leaves_a_sample_at_or_below_its_own_pitch_alone() {
    for semitones in [0.0, -7.0, -24.0] {
        let raw = two_tones(1000.0, 9000.0, semitones, 0.0, false);
        let smooth = two_tones(1000.0, 9000.0, semitones, 1.0, false);
        assert_eq!(raw, smooth, "{semitones} semitones");
    }
    // Sliding up through its own pitch, there is no jump as the smoothing comes in.
    let mut e = plain_engine();
    let data: Vec<f32> = (0..4800).map(|i| 0.5 * (std::f32::consts::TAU * i as f32 * 5000.0 / SR).sin()).collect();
    e.load_sample(0, &data, 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 4800).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e.push(Event::note_on(0.0, 0, 9, 58.0, 1.0));
    e.push(Event::voice_pitch(0.0, 0, 9, 76.0, 1.0));
    let x = render(&mut e, 48000, 128).0;
    // The loudness of the tone falls gently as it rises; it never steps.
    let level: Vec<f32> = x.chunks(480).map(rms).collect();
    for w in level.windows(2) {
        assert!((w[1] / w[0] - 1.0).abs() < 0.08, "{} then {}", w[0], w[1]);
    }
}
