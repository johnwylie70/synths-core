mod common;
use common::*;
use synths_core::params::*;
use synths_core::Event;

/// How much louder or quieter (in dB) a sine at `hz_in` comes out of the filter.
fn gain(kind: f32, slope: f32, cutoff: f32, res: f32, hz_in: f32) -> f32 {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    // Quiet, so that a resonant peak has room; made up for below.
    e.set_param(0, VOLUME, 0.01);
    e.set_param(0, FILTER_TYPE, kind);
    e.set_param(0, FILTER_SLOPE, slope);
    e.set_param(0, CUTOFF, cutoff);
    e.set_param(0, RESONANCE, res);
    let note = 69.0 + 12.0 * (hz_in / 440.0).log2();
    note_on(&mut e, 0, 0, note);
    let x = render(&mut e, 48000, 128).0;
    db(rms(&x[24000..]) * std::f32::consts::SQRT_2) + 40.0
}

#[test]
fn low_pass_cuts_treble_at_12_and_24_db_an_octave() {
    // Three octaves above the cutoff: 36 dB down, or 72 dB with the steeper slope.
    assert!(gain(1.0, 0.0, 1000.0, 0.0, 100.0).abs() < 0.5);
    let g12 = gain(1.0, 0.0, 1000.0, 0.0, 8000.0);
    let g24 = gain(1.0, 1.0, 1000.0, 0.0, 8000.0);
    assert!((g12 + 36.0).abs() < 4.0, "12 dB slope: {g12}");
    assert!((g24 + 72.0).abs() < 6.0, "24 dB slope: {g24}");
    // One more octave is 12 (or 24) dB more.
    let step12 = gain(1.0, 0.0, 500.0, 0.0, 4000.0) - gain(1.0, 0.0, 500.0, 0.0, 8000.0);
    let step24 = gain(1.0, 1.0, 500.0, 0.0, 2000.0) - gain(1.0, 1.0, 500.0, 0.0, 4000.0);
    assert!((step12 - 12.0).abs() < 2.5, "{step12}");
    assert!((step24 - 24.0).abs() < 3.0, "{step24}");
}

#[test]
fn high_pass_cuts_bass() {
    assert!(gain(2.0, 0.0, 1000.0, 0.0, 10000.0).abs() < 0.5);
    let g12 = gain(2.0, 0.0, 1000.0, 0.0, 125.0);
    let g24 = gain(2.0, 1.0, 1000.0, 0.0, 125.0);
    assert!((g12 + 36.0).abs() < 3.0, "{g12}");
    assert!((g24 + 72.0).abs() < 5.0, "{g24}");
}

#[test]
fn band_pass_and_notch() {
    let at = gain(3.0, 0.0, 1000.0, 0.5, 1000.0);
    assert!(gain(3.0, 0.0, 1000.0, 0.5, 100.0) < at - 15.0);
    assert!(gain(3.0, 0.0, 1000.0, 0.5, 10000.0) < at - 15.0);
    assert!(gain(4.0, 0.0, 1000.0, 0.3, 1000.0) < -30.0);
    assert!(gain(4.0, 0.0, 1000.0, 0.3, 100.0).abs() < 1.0);
    assert!(gain(4.0, 0.0, 1000.0, 0.3, 10000.0).abs() < 1.0);
}

#[test]
fn resonance_lifts_the_cutoff_frequency() {
    let flat = gain(1.0, 0.0, 1000.0, 0.0, 1000.0);
    let peaky = gain(1.0, 0.0, 1000.0, 0.7, 1000.0);
    assert!(flat < -5.0 && flat > -7.0, "no resonance: {flat} dB at the cutoff");
    assert!(peaky > flat + 15.0, "{peaky}");
}

#[test]
fn key_tracking_moves_the_cutoff_with_the_note() {
    // With full tracking, a note two octaves up is filtered two octaves higher,
    // so its fundamental gets through just the same.
    let run = |track: f32, note: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 0.0);
        e.set_param(0, FILTER_TYPE, 1.0);
        e.set_param(0, FILTER_SLOPE, 1.0);
        e.set_param(0, CUTOFF, 200.0);
        e.set_param(0, FILTER_KEYTRACK, track);
        note_on(&mut e, 0, 0, note);
        rms(&render(&mut e, 24000, 128).0[12000..])
    };
    assert!((run(1.0, 84.0) / run(1.0, 60.0) - 1.0).abs() < 0.05);
    assert!(run(0.0, 84.0) < 0.1 * run(0.0, 60.0));
}

#[test]
fn the_filter_stays_stable_when_the_cutoff_jumps_about() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 2.0);
    e.set_param(0, FILTER_TYPE, 1.0);
    e.set_param(0, FILTER_SLOPE, 1.0);
    e.set_param(0, RESONANCE, 1.0);
    e.set_param(0, LFO1_CUTOFF, 96.0);
    e.set_param(0, LFO1_RATE, 50.0);
    e.set_param(0, LFO1_WAVE, 4.0);
    note_on(&mut e, 0, 0, 40.0);
    let mut seed = 12345u32;
    let mut loudest = 0.0f32;
    for i in 0..3000u64 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let cutoff = 20.0 * 1000.0f32.powf((seed >> 8) as f32 / 16_777_216.0);
        let kind = 1.0 + ((seed >> 4) & 3) as f32;
        e.push(Event::param((i * 37) as f64, 0, CUTOFF, cutoff, 0.0));
        e.push(Event::param((i * 37) as f64, 0, FILTER_TYPE, kind, 0.0));
        let x = render(&mut e, 37, 37).0;
        assert!(x.iter().all(|v| v.is_finite()));
        loudest = loudest.max(peak(&x));
    }
    // It rings (that is what full resonance does) but it does not run away.
    assert!(loudest < 64.0, "{loudest}");
}
