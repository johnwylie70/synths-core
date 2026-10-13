//! Very small numbers (denormals) are slow to work with on some processors.
//! Anything that rings on after its input stops (a filter, the echo, the
//! reverb, the limiter) must fall to exactly nothing rather than through them.

mod common;
use common::*;
use synths_core::params::*;

/// The smallest ordinary `f32`; anything nearer nothing (but not nothing) is a denormal.
fn count_denormals(x: &[f32]) -> usize {
    x.iter().filter(|v| v.is_subnormal()).count()
}

/// Play a short note through `patch` and render `seconds` of what follows.
fn tail(patch: &[(usize, f32)], seconds: f32, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut e = plain_engine();
    e.set_param(0, PAN, 0.0);
    e.set_param(0, OSC1_WAVE, 2.0);
    for &(id, v) in patch {
        e.set_param(0, id, v);
    }
    note_on(&mut e, 0, 0, 57.0);
    note_off(&mut e, 2400, 0, 57.0);
    render(&mut e, (SR * seconds) as usize, block)
}

#[test]
fn a_short_echo_dies_away_to_exactly_nothing() {
    // One millisecond between echoes, each half the last: it passes the
    // smallest ordinary number about a tenth of a second after the note.
    let (l, r) = tail(&[(SEND_DELAY, 1.0), (DELAY_TIME, 0.001), (DELAY_FEEDBACK, 0.5), (DELAY_TONE, 20000.0)], 1.5, 128);
    assert!(peak(&l[..4800]) > 0.1);
    assert_eq!(count_denormals(&l) + count_denormals(&r), 0);
    assert!(l[48000..].iter().chain(r[48000..].iter()).all(|&v| v == 0.0), "the echo never reached nothing");
}

#[test]
fn a_dull_echo_dies_away_to_exactly_nothing() {
    // The same with the echo's tone filter low, so its own memory decays too.
    let (l, r) = tail(&[(SEND_DELAY, 1.0), (DELAY_TIME, 0.001), (DELAY_FEEDBACK, 0.3), (DELAY_TONE, 200.0)], 3.0, 61);
    assert_eq!(count_denormals(&l) + count_denormals(&r), 0);
    assert!(l[96000..].iter().chain(r[96000..].iter()).all(|&v| v == 0.0));
}

#[test]
fn the_reverb_and_chorus_die_away_to_exactly_nothing() {
    let patch = [(SEND_REVERB, 1.0), (SEND_CHORUS, 1.0), (REVERB_SIZE, 0.0), (REVERB_DAMP, 0.9)];
    let (l, r) = tail(&patch, 12.0, 128);
    assert!(peak(&l[..4800]) > 0.1);
    assert_eq!(count_denormals(&l) + count_denormals(&r), 0);
    let from = l.len() - 4800;
    assert!(l[from..].iter().chain(r[from..].iter()).all(|&v| v == 0.0), "the reverb never reached nothing");
}

#[test]
fn a_ringing_filter_and_a_long_release_do_not_leave_denormals() {
    // A resonant filter rings on a curved release that creeps towards
    // nothing, through the limiter, the distortion and the bit crusher.
    for slope in [0.0, 1.0] {
        let patch = [
            (FILTER_TYPE, 1.0),
            (FILTER_SLOPE, slope),
            (CUTOFF, 300.0),
            (RESONANCE, 1.0),
            (AMP_RELEASE, 4.0),
            (AMP_CURVE, 1.0),
            (DRIVE, 0.3),
            (LIMITER, 1.0),
            (SUB_LEVEL, 0.5),
            (NOISE_LEVEL, 0.2),
            (NOISE_TYPE, 1.0),
        ];
        let (l, r) = tail(&patch, 8.0, 128);
        assert!(peak(&l[..4800]) > 0.05);
        assert_eq!(count_denormals(&l) + count_denormals(&r), 0, "slope {slope}");
        let from = l.len() - 4800;
        assert!(l[from..].iter().chain(r[from..].iter()).all(|&v| v == 0.0));
    }
}

#[test]
fn an_engine_left_alone_goes_to_sleep() {
    // Everything ringing at once, then silence: the engine must reach exact
    // zeros and stay there, which is what lets it stop working.
    let patch = [(SEND_DELAY, 0.8), (SEND_REVERB, 0.8), (SEND_CHORUS, 0.8), (DELAY_TIME, 0.05), (DELAY_FEEDBACK, 0.6)];
    let (l, r) = tail(&patch, 20.0, 128);
    assert_eq!(count_denormals(&l) + count_denormals(&r), 0);
    let from = l.len() - 48000;
    assert!(l[from..].iter().chain(r[from..].iter()).all(|&v| v == 0.0));
}
