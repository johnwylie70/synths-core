mod common;
use common::*;
use synths_core::params::*;
use synths_core::Event;

/// An engine whose part 0 plays a steady 1.0, so the output is the envelope.
fn dc_engine() -> Box<synths_core::Engine> {
    let mut e = plain_engine();
    e.load_sample(0, &[1.0; 64], 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 64).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e
}

#[test]
fn envelope_stages_last_exactly_as_long_as_asked() {
    let mut e = dc_engine();
    e.set_param(0, AMP_ATTACK, 0.1);
    e.set_param(0, AMP_DECAY, 0.2);
    e.set_param(0, AMP_SUSTAIN, 0.5);
    e.set_param(0, AMP_RELEASE, 0.25);
    e.set_param(0, AMP_CURVE, 0.0);
    note_on(&mut e, 100, 0, 60.0);
    note_off(&mut e, 30000, 0, 60.0);
    let x = render(&mut e, 48000, 128).0;
    let full = x[100 + 4800];
    assert!((full - 1.0).abs() < 1e-4, "full level is {full}");
    assert_eq!(x[99], 0.0);
    assert_eq!(x[100], 0.0);
    assert!(x[101] > 0.0);
    // Straight lines when the curve is 0.
    assert!((x[100 + 2400] - 0.5).abs() < 1e-3);
    assert!(x[100 + 4799] < full && x[100 + 4801] < full, "the peak is at exactly 0.1 s");
    assert!((x[100 + 4800 + 4800] - 0.75).abs() < 1e-3);
    assert!((x[100 + 4800 + 9600] - 0.5).abs() < 1e-4);
    assert!((x[29999] - 0.5).abs() < 1e-4);
    // Release: 0.25 s is 12000 samples, and then nothing at all.
    assert!((x[30000] - 0.5).abs() < 1e-4);
    assert!((x[30000 + 6000] - 0.25).abs() < 1e-3);
    assert!(x[30000 + 11999] > 0.0);
    assert_eq!(x[30000 + 12000], 0.0);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn the_curve_bends_the_stages() {
    let mut e = dc_engine();
    e.set_param(0, AMP_ATTACK, 0.1);
    e.set_param(0, AMP_DECAY, 0.1);
    e.set_param(0, AMP_SUSTAIN, 0.0);
    e.set_param(0, AMP_CURVE, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 12000, 128).0;
    // Quick at first, then slow: well past half way at the half-way point.
    assert!(x[2400] > 0.85, "{}", x[2400]);
    assert!(x[4800 + 2400] < 0.15);
    assert!((x[4800] - 1.0).abs() < 1e-4);
    // A sustain of nothing ends the voice without a note-off.
    assert_eq!(x[9600], 0.0);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn filter_envelope_opens_and_closes_the_filter() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 2.0);
    e.set_param(0, FILTER_TYPE, 1.0);
    e.set_param(0, FILTER_SLOPE, 1.0);
    e.set_param(0, CUTOFF, 30.0);
    e.set_param(0, FILTER_ENV, 84.0);
    e.set_param(0, FENV_ATTACK, 0.0);
    e.set_param(0, FENV_DECAY, 0.5);
    e.set_param(0, FENV_SUSTAIN, 0.0);
    e.set_param(0, FENV_CURVE, 0.0);
    note_on(&mut e, 0, 0, 45.0);
    let x = render(&mut e, 48000, 128).0;
    let early = rms(&x[0..4800]);
    let late = rms(&x[36000..48000]);
    assert!(early > 5.0 * late, "bright {early} then dull {late}");
}

#[test]
fn modulation_envelope_bends_the_pitch() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, MENV_PITCH, 12.0);
    e.set_param(0, MENV_ATTACK, 0.0);
    e.set_param(0, MENV_DECAY, 0.0);
    e.set_param(0, MENV_SUSTAIN, 1.0);
    note_on(&mut e, 0, 0, 57.0);
    let x = render(&mut e, 24000, 128).0;
    assert!((frequency(&x, SR) / 440.0 - 1.0).abs() < 1e-3);
}

#[test]
fn lfo_vibrato_tremolo_and_delay() {
    // Tremolo with a square LFO at 10 Hz and full depth: on for 50 ms, off for 50 ms.
    let mut e = dc_engine();
    e.set_param(0, LFO1_WAVE, 3.0);
    e.set_param(0, LFO1_RATE, 10.0);
    e.set_param(0, LFO1_AMP, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 9600, 128).0;
    assert!(x[1200] > 0.99 && x[3600] < 0.01 && x[6000] > 0.99 && x[8400] < 0.01);

    // Vibrato: the pitch swings a semitone either side, 5 times a second.
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, LFO1_PITCH, 1.0);
    e.set_param(0, LFO1_RATE, 5.0);
    e.set_param(0, LFO1_DELAY, 0.5);
    note_on(&mut e, 0, 0, 69.0);
    let x = render(&mut e, 48000, 128).0;
    // Before the delay is over the note is steady.
    assert!((frequency(&x[..24000], SR) / 440.0 - 1.0).abs() < 1e-4);
    // After it, the top of the swing (a twentieth of a second in) is sharp.
    let top = frequency(&x[24000 + 2400 - 480..24000 + 2400 + 480], SR);
    let want = hz(70.0);
    assert!((top / want - 1.0).abs() < 0.01, "{top} vs {want}");

    // A shared (not key-synced) LFO gives two notes the same swing.
    let mut e = dc_engine();
    e.set_param(0, LFO1_SYNC, 0.0);
    e.set_param(0, LFO1_WAVE, 2.0);
    e.set_param(0, LFO1_RATE, 2.0);
    e.set_param(0, LFO1_AMP, 1.0);
    note_on(&mut e, 0, 0, 60.0);
    let a = render(&mut e, 5000, 128).0;
    e.push(Event::all_notes_off(5000.0, 0, true));
    note_on(&mut e, 5000, 0, 60.0);
    let b = render(&mut e, 5000, 128).0;
    // The saw keeps falling across the two notes instead of starting again.
    assert!(a[100] > a[4900] && a[4900] > b[1000] && b[1000] > b[4900], "{} {} {} {}", a[100], a[4900], b[1000], b[4900]);
}

#[test]
fn glide_slides_from_the_last_note() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, MODE, 2.0);
    e.set_param(0, GLIDE, 1.0);
    note_on(&mut e, 0, 0, 57.0);
    note_on(&mut e, 4800, 0, 69.0);
    let x = render(&mut e, 4800 + 72000, 128).0;
    assert_eq!(e.active_voices(), 1);
    // Half way through the slide the pitch is half way, in semitones.
    let mid = frequency(&x[4800 + 24000 - 960..4800 + 24000 + 960], SR);
    assert!((mid / hz(63.0) - 1.0).abs() < 0.01, "{mid}");
    let end = frequency(&x[4800 + 50000..], SR);
    assert!((end / 440.0 - 1.0).abs() < 1e-4);
}

#[test]
fn mono_mode_keeps_one_voice() {
    for mode in [1.0, 2.0] {
        let mut e = plain_engine();
        e.set_param(0, MODE, mode);
        e.set_param(0, AMP_RELEASE, 0.5);
        for (i, note) in [60.0, 64.0, 67.0, 72.0].into_iter().enumerate() {
            note_on(&mut e, i as u64 * 1000, 0, note);
        }
        note_off(&mut e, 5000, 0, 60.0);
        let _ = render(&mut e, 6000, 128);
        assert_eq!(e.active_voices(), 1, "an old note's note-off does not stop the new note");
        note_off(&mut e, 6000, 0, 72.0);
        note_on(&mut e, 7000, 0, 48.0);
        let x = render(&mut e, 48000, 128).0;
        assert_eq!(e.active_voices(), 1);
        assert!((frequency(&x[4800..], SR) / hz(48.0) - 1.0).abs() < 1e-4);
    }
}

#[test]
fn unison_stacks_detuned_voices() {
    let mut e = plain_engine();
    e.set_param(0, UNISON, 5.0);
    e.set_param(0, PAN, 0.0);
    note_on(&mut e, 0, 0, 60.0);
    let (l, r) = render(&mut e, 48000, 128);
    assert_eq!(e.active_voices(), 5);
    assert!(rms(&l) > 0.1 && rms(&r) > 0.1);
    note_off(&mut e, 48000, 0, 60.0);
    let _ = render(&mut e, 4800, 128);
    assert_eq!(e.active_voices(), 0);
}
