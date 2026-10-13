mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;
use synths_core::{Engine, Event};

const UP: u8 = 0;
const DOWN: u8 = 1;
const UP_DOWN: u8 = 2;
const AS_GIVEN: u8 = 3;
const RANDOM: u8 = 4;

fn sine_engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e
}

/// The semitones above `note` heard in each window of `step` frames.
fn heard(x: &[f32], note: f32, step: usize, count: usize) -> Vec<i32> {
    (0..count)
        .map(|n| {
            let f = frequency(&x[n * step + 40..(n + 1) * step - 10], SR);
            (12.0 * (f / hz(note)).log2()).round() as i32
        })
        .collect()
}

/// The first frame at which two renders differ.
fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x != y)
}

#[test]
fn one_event_turns_a_note_into_a_chord() {
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, 0.0, UP, &[0, 4, 7]));
    let x = render(&mut e, 960 * 9, 128).0;
    assert_eq!(e.active_voices(), 1, "it is one voice");
    assert_eq!(heard(&x, 72.0, 960, 9), [0, 4, 7, 0, 4, 7, 0, 4, 7]);
}

#[test]
fn steps_land_on_exactly_the_right_frame() {
    let plain = |start: f64, frames: usize| {
        let mut e = sine_engine();
        e.push(Event::note_on(start, 0, 1, 72.0, 1.0));
        render(&mut e, frames, 128).0
    };
    // 50 steps a second at 48 kHz: a step every 960 frames from the note's start.
    for start in [0u64, 1, 777, 4095] {
        for block in [128, 1, 333] {
            let mut e = sine_engine();
            e.push(Event::note_on(start as f64, 0, 1, 72.0, 1.0));
            e.push(Event::arp(start as f64, 0, 1, 50.0, AS_GIVEN, &[0, 7, 0, 0]));
            let x = render(&mut e, start as usize + 4000, block).0;
            let reference = plain(start as f64, start as usize + 4000);
            assert_eq!(first_difference(&x, &reference), Some(start as usize + 960), "start {start}, block {block}");
            // And back to the note exactly one step later: from there on the
            // wave has the note's pitch again.
            let after = &x[start as usize + 1920..start as usize + 2880];
            assert!((frequency(after, SR) / hz(72.0) - 1.0).abs() < 2e-3);
        }
    }
    // A rate that does not divide the sample rate: steps at 1088.4, 2176.9, 3265.3 frames.
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, 44.1, AS_GIVEN, &[0, 0, 0, 5]));
    let x = render(&mut e, 6000, 128).0;
    assert_eq!(first_difference(&x, &plain(0.0, 6000)), Some(3265));
    // The pitch jumps on that frame; it does not slide there.
    let jumped = frequency(&x[3265..4353], SR);
    assert!((jumped / hz(77.0) - 1.0).abs() < 2e-3, "{jumped}");
}

#[test]
fn the_orders() {
    let run = |order: u8, offsets: &[i8], count: usize| {
        let mut e = sine_engine();
        e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
        e.push(Event::arp(0.0, 0, 1, 50.0, order, offsets));
        heard(&render(&mut e, 960 * count, 128).0, 72.0, 960, count)
    };
    assert_eq!(run(UP, &[7, 0, 4], 6), [0, 4, 7, 0, 4, 7]);
    assert_eq!(run(DOWN, &[7, 0, 4], 6), [7, 4, 0, 7, 4, 0]);
    assert_eq!(run(UP_DOWN, &[0, 12, 4, 7], 9), [0, 4, 7, 12, 7, 4, 0, 4, 7]);
    assert_eq!(run(UP_DOWN, &[0, 5], 4), [0, 5, 0, 5]);
    assert_eq!(run(AS_GIVEN, &[7, 0, 4], 6), [7, 0, 4, 7, 0, 4]);
    assert_eq!(run(AS_GIVEN, &[-12, 0, 12], 3), [-12, 0, 12]);
    assert_eq!(run(UP, &[3], 3), [3, 3, 3]);
    // Random: only the notes given, not all the same, and the same every time.
    let a = run(RANDOM, &[0, 4, 7, 12], 24);
    assert!(a.iter().all(|n| [0, 4, 7, 12].contains(n)));
    assert!((0..4).all(|i| a.contains(&[0, 4, 7, 12][i])), "{a:?}");
    assert_eq!(a, run(RANDOM, &[0, 4, 7, 12], 24));
}

#[test]
fn eight_offsets_with_a_second_event() {
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 60.0, 1.0));
    let mut first = Event::arp(0.0, 0, 1, 50.0, AS_GIVEN, &[0, 3, 7, 10]);
    first.z = 8.0;
    e.push(first);
    e.push(Event::arp_more(0.0, 0, 1, &[12, 15, 19, 22]));
    let x = render(&mut e, 960 * 10, 128).0;
    assert_eq!(heard(&x, 60.0, 960, 10), [0, 3, 7, 10, 12, 15, 19, 22, 0, 3]);
}

#[test]
fn rate_and_tracker_ticks() {
    // 100 steps a second.
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, 100.0, UP, &[0, 12]));
    let x = render(&mut e, 480 * 6, 128).0;
    assert_eq!(heard(&x, 72.0, 480, 6), [0, 12, 0, 12, 0, 12]);
    // A rate below 0: one step on each tick event, whenever those come.
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, -1.0, UP, &[0, 4, 7]));
    for t in [1234.0, 2500.0, 6000.0] {
        e.push(Event::macro_tick(t, ALL_PARTS));
    }
    let x = render(&mut e, 8000, 128).0;
    let mut plain = sine_engine();
    plain.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    assert_eq!(first_difference(&x, &render(&mut plain, 8000, 128).0), Some(1234));
    let pitch = |from: usize, to: usize| (12.0 * (frequency(&x[from..to], SR) / hz(72.0)).log2()).round() as i32;
    assert_eq!([pitch(100, 1200), pitch(1300, 2450), pitch(2550, 5950), pitch(6050, 7950)], [0, 4, 7, 0]);
}

#[test]
fn an_arpeggio_can_wait_for_the_next_note_of_a_part() {
    let mut e = sine_engine();
    // Handle 0: for the part's next note, whatever handle that has, and only that one.
    e.push(Event::arp(0.0, 0, 0, 50.0, UP, &[0, 4, 7]));
    e.push(Event::note_on(100.0, 0, 0, 72.0, 1.0));
    e.push(Event::note_off(4000.0, 0, 0, 72.0));
    e.push(Event::note_on(5000.0, 0, 0, 72.0, 1.0));
    let x = render(&mut e, 5000 + 2880, 128).0;
    assert_eq!(heard(&x[100..], 72.0, 960, 3), [0, 4, 7]);
    assert_eq!(heard(&x[5000..], 72.0, 960, 3), [0, 0, 0]);
    // It does not leak to another part.
    let mut e = sine_engine();
    e.set_param(1, OSC1_WAVE, 0.0);
    e.push(Event::arp(0.0, 0, 0, 50.0, UP, &[0, 7]));
    e.push(Event::note_on(0.0, 1, 0, 72.0, 1.0));
    let x = render(&mut e, 2880, 128).0;
    assert_eq!(heard(&x, 72.0, 960, 3), [0, 0, 0]);
}

#[test]
fn a_live_note_can_change_chord_without_starting_again() {
    let mut e = sine_engine();
    e.set_param(0, AMP_ATTACK, 1.0);
    e.set_param(0, AMP_CURVE, 0.0);
    e.push(Event::note_on(0.0, 0, 5, 60.0, 1.0));
    e.push(Event::arp(0.0, 0, 5, 50.0, UP, &[0, 4, 7]));
    // Major to minor at 0.25 s, then no arpeggio at all at 0.5 s.
    e.push(Event::arp(12000.0, 0, 5, 50.0, UP, &[0, 3, 7]));
    e.push(Event::arp(24000.0, 0, 5, 50.0, UP, &[]));
    let x = render(&mut e, 36000, 128).0;
    assert_eq!(heard(&x[2880..], 60.0, 960, 3), [0, 4, 7]);
    assert_eq!(heard(&x[12000..], 60.0, 960, 6), [0, 3, 7, 0, 3, 7], "the new chord starts from its first note on the frame of the event");
    assert_eq!(heard(&x[24000..], 60.0, 960, 3), [0, 0, 0]);
    // The envelope carried straight on: still the one slow attack.
    let level = |at: usize| peak(&x[at - 480..at + 480]);
    for at in [6000, 11500, 12500, 18000, 23500, 24500, 30000] {
        assert!((level(at) - at as f32 / 48000.0).abs() < 0.02, "at {at}: {}", level(at));
    }
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn the_arpeggio_adds_to_the_patch_and_its_macro() {
    let mut e = sine_engine();
    e.set_param(0, TRANSPOSE, -12.0);
    // A macro that lifts every second step an octave.
    e.set_macro(0, &[-1.0, 0.0, -1.0, 0.0, 1.0, -1.0, 12.0, -1.0, 0.0, 1.0], 0, 2, 255, 0).unwrap();
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, 25.0, UP, &[0, 7]));
    let x = render(&mut e, 960 * 8, 128).0;
    // Macro: 0 12 0 12 ... at 50 a second; arpeggio: 0 0 7 7 ... at 25 a second.
    assert_eq!(heard(&x, 60.0, 960, 8), [0, 12, 7, 19, 0, 12, 7, 19]);
    // Unison voices each run it, and handles on other parts are left alone.
    let mut e = sine_engine();
    e.set_param(0, UNISON, 3.0);
    e.set_param(0, UNISON_DETUNE, 0.0);
    e.push(Event::note_on(0.0, 0, 1, 72.0, 1.0));
    e.push(Event::note_on(0.0, 1, 1, 30.0, 0.0));
    e.push(Event::arp(0.0, 0, 1, 50.0, UP, &[0, 12]));
    let _ = render(&mut e, 1000, 128);
    assert_eq!(e.active_voices(), 4);
}
