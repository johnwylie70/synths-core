mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;

fn tone(wave: f32, note: f32, seconds: f32) -> Vec<f32> {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, wave);
    note_on(&mut e, 0, 0, note);
    render(&mut e, (SR * seconds) as usize, 128).0
}

#[test]
fn every_wave_plays_in_tune() {
    for (wave, name) in [(0.0, "sine"), (1.0, "triangle"), (2.0, "saw"), (3.0, "pulse")] {
        for note in [21.0, 33.0, 45.5, 57.0, 69.0, 81.0, 93.0, 105.0, 108.0] {
            let x = tone(wave, note, 2.0);
            let got = frequency(&x[4800..], SR);
            let want = hz(note);
            let cents = 1200.0 * (got / want).log2();
            assert!(cents.abs() < 0.1, "{name} note {note}: {got} Hz, wanted {want} Hz ({cents} cents)");
        }
    }
}

#[test]
fn waves_have_the_right_size_and_shape() {
    for wave in [0.0, 1.0, 2.0, 3.0] {
        let x = tone(wave, 45.0, 0.5);
        let p = peak(&x);
        assert!(p > 0.95 && p < 1.3, "wave {wave} peaks at {p}");
    }
    // A sine is only its fundamental; a saw's second harmonic is half as strong.
    let sine = tone(0.0, 57.0, 1.0);
    let saw = tone(2.0, 57.0, 1.0);
    let f = hz(57.0);
    assert!(db(amplitude_at(&sine, 2.0 * f, SR) / amplitude_at(&sine, f, SR)) < -80.0);
    let ratio = amplitude_at(&saw, 2.0 * f, SR) / amplitude_at(&saw, f, SR);
    assert!((ratio - 0.5).abs() < 0.02, "saw second harmonic is {ratio}");
}

#[test]
fn high_notes_do_not_alias_badly() {
    // A saw at note 100 (2637 Hz): its 17th harmonic is past the top of what
    // 48 kHz can hold and would fold down to 3171 Hz. A plain saw puts it
    // 25 dB below the fundamental; the rounded-off edges must do much better.
    let f = hz(100.0);
    for wave in [2.0, 3.0] {
        let x = tone(wave, 100.0, 1.0);
        let alias = (17.0 * f - SR).abs();
        let level = db(amplitude_at(&x[4800..], alias, SR) / amplitude_at(&x[4800..], f, SR));
        assert!(level < -50.0, "wave {wave}: alias at {alias} Hz is {level} dB");
    }
    let x = tone(1.0, 100.0, 1.0);
    let alias = (19.0 * f - SR).abs();
    let level = db(amplitude_at(&x[4800..], alias, SR) / amplitude_at(&x[4800..], f, SR));
    assert!(level < -70.0, "triangle: alias at {alias} Hz is {level} dB");
}

#[test]
fn tuning_parameters_move_the_pitch() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, TRANSPOSE, 12.0);
    e.set_param(0, OSC1_COARSE, -5.0);
    e.set_param(0, OSC1_FINE, 50.0);
    e.set_param(0, BEND, 0.5);
    e.set_param(0, BEND_RANGE, 4.0);
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 48000, 128).0;
    let want = hz(60.0 + 12.0 - 5.0 + 0.5 + 2.0);
    let got = frequency(&x, SR);
    assert!((got / want - 1.0).abs() < 1e-4, "{got} vs {want}");
}

#[test]
fn sub_oscillator_is_an_octave_or_two_down() {
    for (octave, ratio) in [(-1.0, 0.5), (-2.0, 0.25)] {
        let mut e = plain_engine();
        e.set_param(0, OSC1_LEVEL, 0.0);
        e.set_param(0, SUB_LEVEL, 1.0);
        e.set_param(0, SUB_OCTAVE, octave);
        note_on(&mut e, 0, 0, 69.0);
        let x = render(&mut e, 48000, 128).0;
        let got = frequency(&x, SR);
        assert!((got / (440.0 * ratio) - 1.0).abs() < 1e-4, "{got}");
    }
}

#[test]
fn second_oscillator_ring_and_sync() {
    // Two sines a fifth apart, ring modulated: the sum and difference tones.
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, OSC1_LEVEL, 0.0);
    e.set_param(0, OSC2_WAVE, 0.0);
    e.set_param(0, OSC2_LEVEL, 1.0);
    e.set_param(0, OSC2_COARSE, 12.0);
    e.set_param(0, OSC2_RING, 1.0);
    note_on(&mut e, 0, 0, 69.0);
    let x = render(&mut e, 48000, 128).0;
    assert!(amplitude_at(&x, 440.0, SR) > 0.45);
    assert!(amplitude_at(&x, 1320.0, SR) > 0.45);
    assert!(amplitude_at(&x, 880.0, SR) < 0.01);

    // Hard sync: whatever oscillator 2 is tuned to, the pitch heard is oscillator 1's.
    let mut e = plain_engine();
    e.set_param(0, OSC1_LEVEL, 0.0);
    e.set_param(0, OSC2_WAVE, 2.0);
    e.set_param(0, OSC2_LEVEL, 1.0);
    e.set_param(0, OSC2_COARSE, 7.3);
    e.set_param(0, OSC2_SYNC, 1.0);
    note_on(&mut e, 0, 0, 57.0);
    let x = render(&mut e, 48000, 128).0;
    let f = hz(57.0);
    // Every part of the sound sits on a multiple of oscillator 1's frequency.
    let on = amplitude_at(&x, f, SR) + amplitude_at(&x, 2.0 * f, SR) + amplitude_at(&x, 3.0 * f, SR);
    let off = amplitude_at(&x, 1.5 * f, SR) + amplitude_at(&x, hz(57.0 + 7.3), SR);
    assert!(on > 0.5 && off < 0.01, "on {on} off {off}");
    assert!(peak(&x) < 1.5);
}

#[test]
fn noises_are_noise_and_the_chip_noise_follows_the_note() {
    for kind in [0.0, 1.0, 2.0, 3.0] {
        let mut e = plain_engine();
        e.set_param(0, OSC1_LEVEL, 0.0);
        e.set_param(0, NOISE_LEVEL, 1.0);
        e.set_param(0, NOISE_TYPE, kind);
        note_on(&mut e, 0, 0, 60.0);
        let x = render(&mut e, 48000, 128).0;
        let r = rms(&x);
        assert!(r > 0.1 && r < 1.01 && peak(&x) <= 1.3, "noise {kind}: rms {r}");
    }
    // Pink noise has more bass than treble; white does not.
    let band = |kind: f32, lo: bool| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_LEVEL, 0.0);
        e.set_param(0, NOISE_LEVEL, 1.0);
        e.set_param(0, NOISE_TYPE, kind);
        e.set_param(0, FILTER_TYPE, if lo { 1.0 } else { 2.0 });
        e.set_param(0, FILTER_SLOPE, 1.0);
        e.set_param(0, RESONANCE, 0.0);
        e.set_param(0, CUTOFF, if lo { 300.0 } else { 6000.0 });
        note_on(&mut e, 0, 0, 60.0);
        rms(&render(&mut e, 96000, 128).0)
    };
    assert!(band(1.0, true) > 1.5 * band(1.0, false));
    assert!(band(0.0, true) < 0.5 * band(0.0, false));
    // The chip noise is brighter on a high note than a low one.
    let bright = |note: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_LEVEL, 0.0);
        e.set_param(0, NOISE_LEVEL, 1.0);
        e.set_param(0, NOISE_TYPE, 2.0);
        e.set_param(0, FILTER_TYPE, 2.0);
        e.set_param(0, CUTOFF, 4000.0);
        note_on(&mut e, 0, 0, note);
        rms(&render(&mut e, 48000, 128).0)
    };
    assert!(bright(84.0) > 3.0 * bright(36.0));
    let _ = ALL_PARTS;
}
