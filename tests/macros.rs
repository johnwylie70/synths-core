mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::macros::MACRO_SMOOTH;
use synths_core::params::*;
use synths_core::{Engine, Event};

/// Steps from (wave, pitch, pulse width, cutoff, volume).
fn steps(s: &[(f32, f32, f32, f32, f32)]) -> Vec<f32> {
    s.iter().flat_map(|&(a, b, c, d, e)| [a, b, c, d, e]).collect()
}

/// Steps that only set the volume.
fn volumes(v: &[f32]) -> Vec<f32> {
    v.iter().flat_map(|&x| [-1.0, 0.0, -1.0, 0.0, x]).collect()
}

/// An engine whose part 0 plays a steady 1.0, so the output is the volume.
fn dc_engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.load_sample(0, &[1.0; 64], 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 64).unwrap();
    e.set_param(0, OSC1_WAVE, 4.0);
    e
}

/// Equal, give or take the rounding of a slide.
fn same(got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len());
    for (g, w) in got.iter().zip(want.iter()) {
        assert!((g - w).abs() < 1e-4, "{got:?} is not {want:?}");
    }
}

/// The level in the middle of step `n` (steps are 960 frames at 50 a second).
fn mid(x: &[f32], n: usize) -> f32 {
    x[n * 960 + 480]
}

#[test]
fn steps_go_by_at_the_rate_set_and_the_last_one_holds() {
    let mut e = dc_engine();
    e.set_macro(0, &volumes(&[1.0, 0.5, 0.25, 0.75]), 0, 0, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 960 * 8, 128).0;
    let got: Vec<f32> = (0..8).map(|n| mid(&x, n)).collect();
    same(&got, &[1.0, 0.5, 0.25, 0.75, 0.75, 0.75, 0.75, 0.75]);
    // A step changes within a control period of its time, sliding, not jumping.
    assert_eq!(x[959], 1.0);
    assert!((x[960 + 32] - 0.5).abs() < 1e-4 && x[960 + 8] < 1.0 && x[960 + 8] > 0.5);

    // Twice the rate, half the time.
    let mut e = dc_engine();
    e.set_param(0, MACRO_RATE, 100.0);
    e.set_macro(0, &volumes(&[1.0, 0.5, 0.25, 0.75]), 0, 0, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 960 * 2, 128).0;
    same(&[x[240], x[720], x[1200], x[1680]], &[1.0, 0.5, 0.25, 0.75]);
}

#[test]
fn the_loop_goes_round_until_the_note_is_let_go() {
    let mut e = dc_engine();
    e.set_param(0, AMP_RELEASE, 20.0);
    e.set_param(0, AMP_CURVE, 0.0);
    // Steps 1 and 2 loop; letting go jumps to step 3 and plays on to the end.
    e.set_macro(0, &volumes(&[1.0, 0.5, 0.25, 0.8, 0.1]), 1, 3, 3, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let off = 960 * 7 + 300;
    note_off(&mut e, off as u64, 0, 60.0);
    let x = render(&mut e, 960 * 12, 128).0;
    let got: Vec<f32> = (0..7).map(|n| mid(&x, n)).collect();
    same(&got, &[1.0, 0.5, 0.25, 0.5, 0.25, 0.5, 0.25]);
    same(&[x[off - 1]], &[0.5]);
    let near = |got: f32, want: f32| assert!((got - want).abs() < 0.02, "{got} vs {want}");
    near(x[off + 100], 0.8);
    near(x[off + 960 + 100], 0.1);
    near(x[off + 3000], 0.1);

    // With no release point the loop carries on through the release.
    let mut e = dc_engine();
    e.set_param(0, AMP_RELEASE, 20.0);
    e.set_param(0, AMP_CURVE, 0.0);
    e.set_macro(0, &volumes(&[1.0, 0.5]), 0, 2, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    note_off(&mut e, 1000, 0, 60.0);
    let x = render(&mut e, 960 * 4, 128).0;
    near(mid(&x, 2), 1.0);
    near(mid(&x, 3), 0.5);
}

#[test]
fn an_arpeggio_makes_one_voice_sound_like_a_chord() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    let arp = steps(&[(-1.0, 0.0, -1.0, 0.0, 1.0), (-1.0, 4.0, -1.0, 0.0, 1.0), (-1.0, 7.0, -1.0, 0.0, 1.0)]);
    e.set_macro(0, &arp, 0, 3, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 72.0);
    let x = render(&mut e, 960 * 9, 128).0;
    assert_eq!(e.active_voices(), 1);
    for n in 0..9 {
        let got = frequency(&x[n * 960 + 60..(n + 1) * 960 - 20], SR);
        let want = hz(72.0 + [0.0, 4.0, 7.0][n % 3]);
        assert!((got / want - 1.0).abs() < 0.004, "step {n}: {got} vs {want}");
    }
}

#[test]
fn a_step_of_noise_then_a_falling_pulse_is_a_drum() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    e.set_param(0, NOISE_TYPE, 2.0);
    let mut drum = vec![(4.0, 12.0, -1.0, 0.0, 1.0)];
    for i in 0..6 {
        drum.push((3.0, -4.0 * i as f32, 0.5, 0.0, 1.0 - i as f32 * 0.15));
    }
    e.set_macro(0, &steps(&drum), 0, 0, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 960 * 7, 128).0;
    // The first step is noise: plenty of sound, but no steady tone at the note.
    let first = &x[..960];
    assert!(rms(first) > 0.3);
    assert!(amplitude_at(first, hz(60.0), SR) < 0.5 * rms(first));
    assert!(amplitude_at(&x[1020..1900], hz(60.0), SR) > 1.2 * rms(&x[1020..1900]), "and the next step is a tone");
    // Then a pulse wave whose pitch and level fall step by step.
    for i in 0..6 {
        let part = &x[(i + 1) * 960 + 60..(i + 2) * 960 - 20];
        let got = frequency(part, SR);
        let want = hz(60.0 - 4.0 * i as f32);
        assert!((got / want - 1.0).abs() < 0.02, "step {i}: {got} vs {want}");
        assert!((rms(part) - (1.0 - i as f32 * 0.15)).abs() < 0.03, "step {i}: {}", rms(part));
    }
}

#[test]
fn pulse_width_and_cutoff_steps_and_sliding_between_them() {
    // The second harmonic of a pulse wave is missing at a width of a half and strong at a quarter.
    let second = |x: &[f32]| 2.0 * amplitude_at(x, 2.0 * hz(48.0), SR);
    let widths = steps(&[(3.0, 0.0, 0.5, 0.0, 1.0), (3.0, 0.0, 0.25, 0.0, 1.0)]);
    let run = |flags: u32| {
        let mut e = plain_engine();
        e.set_param(0, VOLUME, 0.5);
        e.set_param(0, MACRO_RATE, 1.0);
        e.set_macro(0, &widths, 0, 0, 255, flags).unwrap();
        note_on(&mut e, 0, 0, 48.0);
        render(&mut e, 96000, 128).0
    };
    let stepped = run(0);
    assert!(second(&stepped[4800..43200]) < 0.01);
    assert!(second(&stepped[52800..91200]) > 0.5, "{}", second(&stepped[52800..91200]));
    // Sliding: half way through the first step the width is already half way.
    let slid = run(MACRO_SMOOTH);
    assert!(second(&slid[..4800]) < 0.06);
    let half = second(&slid[21600..26400]);
    assert!(half > 0.2 && half < 0.45, "{half}");
    assert!(second(&slid[52800..91200]) > 0.5);

    // Cutoff steps open and close the filter.
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 2.0);
    e.set_param(0, FILTER_TYPE, 1.0);
    e.set_param(0, FILTER_SLOPE, 1.0);
    e.set_param(0, CUTOFF, 100.0);
    e.set_macro(0, &steps(&[(-1.0, 0.0, -1.0, 72.0, 1.0), (-1.0, 0.0, -1.0, 0.0, 1.0)]), 0, 0, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 72.0);
    let x = render(&mut e, 2880, 128).0;
    // (The second window starts late: a filter closed this far takes a moment to settle.)
    assert!(rms(&x[100..900]) > 100.0 * rms(&x[2000..2800]));
}

#[test]
fn a_macro_can_step_on_tick_events_instead_of_the_clock() {
    let mut e = dc_engine();
    e.set_param(0, MACRO_SYNC, 1.0);
    e.set_macro(0, &volumes(&[1.0, 0.5, 0.25]), 0, 3, 255, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    // Uneven ticks, as a tracker changing speed would send them.
    for t in [1000.0, 1300.0, 5000.0, 5001.0] {
        e.push(Event::macro_tick(t, ALL_PARTS));
    }
    let x = render(&mut e, 6000, 128).0;
    same(&[x[999], x[1016], x[1299], x[1316], x[4999], x[5020]], &[1.0, 0.5, 0.5, 0.25, 0.25, 0.5]);
    assert!(x[1001] < 1.0, "the step starts on the very frame of the tick");
}

#[test]
fn macros_are_checked_and_can_be_taken_away() {
    let mut e = dc_engine();
    assert!(e.set_macro(64, &volumes(&[1.0]), 0, 0, 255, 0).is_err());
    assert!(e.set_macro(0, &volumes(&[1.0; 65]), 0, 0, 255, 0).is_err());
    // Nonsense numbers are made safe; loop points are kept inside the macro.
    e.set_macro(0, &[f32::NAN, f32::INFINITY, 9.0, -1.0e9, 7.0], 50, 90, 2, 0).unwrap();
    note_on(&mut e, 0, 0, 60.0);
    let x = render(&mut e, 4800, 128).0;
    assert!(x.iter().all(|v| v.is_finite()));
    // Taking the macro away mid-note leaves the plain patch.
    e.set_macro(0, &volumes(&[0.25]), 0, 0, 255, 0).unwrap();
    let x = render(&mut e, 480, 128).0;
    same(&[x[400]], &[0.25]);
    e.set_macro(ALL_PARTS, &[], 0, 0, 0, 0).unwrap();
    let x = render(&mut e, 480, 128).0;
    same(&[x[400]], &[1.0]);
}

#[test]
fn pulse_width_works_across_its_whole_range() {
    let run = |width: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 3.0);
        e.set_param(0, OSC1_PW, width);
        // Half volume: a narrow pulse with no steady offset stands nearly twice as tall.
        e.set_param(0, VOLUME, 0.5);
        note_on(&mut e, 0, 0, 57.0);
        render(&mut e, 24000, 128).0.iter().map(|v| v * 2.0).collect::<Vec<f32>>()
    };
    // At the two ends there is nothing left of the wave, and no thump either.
    assert!(peak(&run(0.0)) < 1e-6 && peak(&run(1.0)) < 1e-6);
    // Narrow pulses are quieter, the same from either end, and have no steady offset.
    let (a, b, mid) = (run(0.05), run(0.95), run(0.5));
    assert!((rms(&a) - rms(&b)).abs() < 0.01 && rms(&a) < 0.5 * rms(&mid) && rms(&a) > 0.3, "{} {} {}", rms(&a), rms(&b), rms(&mid));
    for x in [&a, &b, &mid] {
        let mean = x.iter().sum::<f32>() / x.len() as f32;
        assert!(mean.abs() < 0.01, "{mean}");
    }
    // An LFO sweeping the width from end to end and back, on a high note:
    // nothing wild, and what folds back down stays well under the note. (A
    // pulse a sample or two wide is the hardest wave there is to keep clean;
    // a plain pulse wave would fold back at -13 dB here.)
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    e.set_param(0, VOLUME, 0.4);
    e.set_param(0, LFO1_PW, 1.0);
    e.set_param(0, LFO1_RATE, 3.0);
    note_on(&mut e, 0, 0, 93.0);
    let x = render(&mut e, 48000, 128).0;
    assert!(peak(&x) < 0.4 * 2.1);
    let f = hz(93.0);
    let alias = (15.0 * f - SR).abs();
    let level = db(amplitude_at(&x, alias, SR) / amplitude_at(&x, f, SR));
    assert!(level < -28.0, "{level}");
    // At widths that are not extreme it is cleaner still. (This alias lands at
    // 20 kHz, just under the top of the range, where rounding the edges helps least.)
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    e.set_param(0, VOLUME, 0.4);
    e.set_param(0, LFO1_PW, 0.6);
    e.set_param(0, LFO1_RATE, 3.0);
    note_on(&mut e, 0, 0, 93.0);
    let x = render(&mut e, 48000, 128).0;
    let level = db(amplitude_at(&x, alias, SR) / amplitude_at(&x, f, SR));
    assert!(level < -34.0, "{level}");
}

#[test]
fn filter_outputs_can_be_added_together() {
    // How loud a sine at `hz_in` comes out of a filter at 1 kHz, in dB.
    let gain = |kind: f32, hz_in: f32| {
        let mut e = plain_engine();
        e.set_param(0, OSC1_WAVE, 0.0);
        e.set_param(0, VOLUME, 0.1);
        e.set_param(0, FILTER_TYPE, kind);
        e.set_param(0, CUTOFF, 1000.0);
        e.set_param(0, RESONANCE, 0.3);
        note_on(&mut e, 0, 0, 69.0 + 12.0 * (hz_in / 440.0).log2());
        db(rms(&render(&mut e, 24000, 128).0[12000..]) * std::f32::consts::SQRT_2) + 20.0
    };
    let (low, mid, high) = (62.5, 1000.0, 16000.0);
    // low + band: the bass passes, the treble does not.
    assert!(gain(5.0, low).abs() < 1.0 && gain(5.0, high) < -20.0);
    // band + high: the other way round.
    assert!(gain(6.0, high).abs() < 1.5 && gain(6.0, low) < -20.0);
    // low + high is the notch.
    assert!((gain(4.0, mid) - gain(4.0, mid)).abs() < 1e-6 && gain(4.0, mid) < -30.0);
    // All three: everything passes, with a lift at the cutoff.
    assert!(gain(7.0, low).abs() < 1.0 && gain(7.0, high).abs() < 1.5 && gain(7.0, mid) > 3.0);
}
