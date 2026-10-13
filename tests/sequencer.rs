//! The sequencer hook: something inside the engine that is called at exact
//! frames from `render`, and plays voices directly.

mod common;
use common::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use synths_core::event::ALL_PARTS;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::sequencer::{Sequencer, Trigger, Voices};
use synths_core::{Engine, Event};

/// A sequencer made from a closure, which is given the number of the call.
struct Script<F: FnMut(&mut Voices, u32) -> u32 + Send> {
    run: F,
    calls: u32,
    resets: Arc<AtomicU32>,
}

impl<F: FnMut(&mut Voices, u32) -> u32 + Send> Sequencer for Script<F> {
    fn advance(&mut self, voices: &mut Voices) -> u32 {
        let n = self.calls;
        self.calls += 1;
        (self.run)(voices, n)
    }
    fn reset(&mut self) {
        self.resets.fetch_add(1, Ordering::Relaxed);
    }
}

fn script<F: FnMut(&mut Voices, u32) -> u32 + Send + 'static>(run: F) -> Option<Box<dyn Sequencer>> {
    Some(Box::new(Script { run, calls: 0, resets: Arc::new(AtomicU32::new(0)) }))
}

fn hit(slot: u16) -> Trigger {
    Trigger { slot, position: 0.0, rate: SR, volume: 1.0, pan: 0.0 }
}

/// An engine with a few samples: slot 0 a steady 1.0 that loops, slot 1 a
/// short click of 0.5, slot 2 a ramp whose every frame holds its own index
/// over 1000, slot 3 one cycle of a sine in 100 frames, looped.
fn engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.load_sample(0, &[1.0; 64], 1, SR).unwrap();
    e.configure_sample(0, 60.0, 1, 0, 64).unwrap();
    e.load_sample(1, &[0.5; 8], 1, SR).unwrap();
    let ramp: Vec<f32> = (0..500).map(|i| i as f32 / 1000.0).collect();
    e.load_sample(2, &ramp, 1, SR).unwrap();
    let cycle: Vec<f32> = (0..100).map(|i| (std::f32::consts::TAU * i as f32 / 100.0).sin() * 0.5).collect();
    e.load_sample(3, &cycle, 1, SR).unwrap();
    e.configure_sample(3, 60.0, 1, 0, 100).unwrap();
    e
}

/// A tracker's clock: ticks 1000.5 frames long, a click on each.
fn ticking() -> Box<Engine> {
    let mut e = engine();
    let mut owed = 0.0f64;
    e.set_sequencer(script(move |v, _| {
        assert!(v.trigger(0, 1, &hit(1)));
        owed += 1000.5;
        let whole = owed.floor();
        owed -= whole;
        whole as u32
    }));
    e
}

#[test]
fn ticks_land_on_exact_frames_wherever_they_fall_in_a_block() {
    let mut e = ticking();
    let x = render(&mut e, 6000, 128).0;
    let on = |i: usize| (x[i] - 0.5).abs() < 1e-4;
    let starts: Vec<usize> = (1..x.len()).filter(|&i| on(i) && !on(i - 1)).collect();
    assert!(on(0) && on(7) && x[8] < 0.5);
    assert_eq!(starts, [1000, 2001, 3001, 4002, 5002]);
    // The same sound whatever the size of the blocks.
    for block in [1, 7, 16, 1000, 4096] {
        let mut e = ticking();
        let y = render(&mut e, 6000, block).0;
        let differs = (0..x.len()).find(|&i| x[i] != y[i]);
        assert_eq!(differs.map(|i| (i, x[i], y[i])), None, "blocks of {block}");
    }
}

#[test]
fn a_trigger_plays_its_own_sample_whatever_the_patch_says() {
    let mut e = engine();
    // The patch is a saw; the triggers play samples all the same, a different one each.
    e.set_param(0, OSC1_WAVE, 2.0);
    e.set_sequencer(script(|v, n| {
        if n == 0 {
            assert!(v.trigger(0, 1, &Trigger { volume: 0.25, ..hit(0) }));
            assert!(v.trigger(0, 2, &Trigger { position: 20.0, ..hit(2) }));
            assert!(!v.trigger(0, 3, &hit(900)), "an empty slot");
            assert!(!v.trigger(64, 3, &hit(0)), "no such part");
            assert!(!v.trigger(0, 3, &Trigger { rate: f32::NAN, ..hit(0) }));
        }
        100_000
    }));
    let x = render(&mut e, 256, 128).0;
    assert_eq!(e.active_voices(), 2);
    for (i, v) in x.iter().enumerate().take(200) {
        assert!((v - 0.25 - (20 + i) as f32 / 1000.0).abs() < 1e-5, "frame {i} is {v}");
    }
}

#[test]
fn a_handle_has_one_voice_and_steers_it() {
    let mut e = engine();
    e.set_param(0, PAN, 0.0);
    e.set_sequencer(script(|v, n| {
        match n {
            // A quarter speed: the ramp climbs a quarter as fast.
            0 => assert!(v.trigger(0, 7, &Trigger { rate: SR * 0.25, ..hit(2) })),
            // The same handle again: the first voice is cut on this frame.
            1 => {
                assert!(v.sounding(0, 7) && !v.sounding(0, 8) && !v.sounding(1, 7));
                assert!(v.trigger(0, 7, &Trigger { pan: 1.0, ..hit(0) }));
            }
            2 => v.set_volume(0, 7, 0.5),
            3 => v.set_pan(0, 7, -1.0),
            4 => {
                v.kill(0, 7);
                assert!(!v.sounding(0, 7));
            }
            _ => {}
        }
        1000
    }));
    let (l, r) = render(&mut e, 6000, 128);
    let centre = std::f32::consts::FRAC_1_SQRT_2;
    assert!((l[400] - 0.1 * centre).abs() < 1e-4, "frame 400 of the ramp at a quarter speed is {}", l[400]);
    assert!(r[1500] > 0.999 && l[1500].abs() < 1e-3, "panned right: {} {}", l[1500], r[1500]);
    assert!((r[2500] - 0.5).abs() < 1e-3);
    assert!((l[3500] - 0.5).abs() < 1e-3 && r[3500].abs() < 1e-3);
    assert_eq!(e.active_voices(), 0);
    assert!(l[4010] > 0.3 && l[4010] < 0.5, "the cut is not a click: {}", l[4010]);
    assert!(l[5500..].iter().all(|&v| v == 0.0));
}

#[test]
fn a_rate_is_frames_of_the_sample_a_second() {
    let mut e = engine();
    e.set_sequencer(script(|v, n| {
        match n {
            0 => assert!(v.trigger(0, 1, &Trigger { rate: 24000.0, ..hit(3) })),
            1 => v.set_rate(0, 1, 60000.0, 0.0),
            2 => v.set_rate(0, 1, 30000.0, 0.5),
            _ => {}
        }
        48000
    }));
    let x = render(&mut e, 48000 * 3, 128).0;
    // One cycle is 100 frames: 240 Hz, then 600 Hz, then a slide down to 300 Hz.
    let close = |got: f32, want: f32| assert!((got / want - 1.0).abs() < 1e-3, "{got} vs {want}");
    close(frequency(&x[4800..48000], SR), 240.0);
    close(frequency(&x[52800..96000], SR), 600.0);
    let sliding = frequency(&x[96000 + 9600..96000 + 14400], SR);
    assert!(sliding > 320.0 && sliding < 580.0, "{sliding}");
    close(frequency(&x[96000 + 28800..], SR), 300.0);
}

#[test]
fn a_fade_falls_in_a_straight_line_and_ends_the_voice() {
    let mut e = engine();
    e.set_sequencer(script(|v, n| {
        match n {
            0 => assert!(v.trigger(0, 1, &hit(0))),
            1 => v.fade(0, 1, 0.1),
            _ => {}
        }
        1000
    }));
    let x = render(&mut e, 8000, 128).0;
    assert_eq!(x[999], 1.0);
    for (at, want) in [(1000, 1.0), (2200, 0.75), (3400, 0.5), (4600, 0.25), (5700, 0.0208)] {
        assert!((x[at] - want).abs() < 0.008, "frame {at} is {}, wanted {want}", x[at]);
    }
    assert!(x[1001] < 1.0, "the fade starts on the very frame");
    assert!(x[5800 + 32..].iter().all(|&v| v == 0.0));
    assert!(x[1000..5800].windows(2).all(|w| w[1] <= w[0]));
    assert_eq!(e.active_voices(), 0);
    // A fade of no time at all is a cut.
    let mut e = engine();
    e.set_sequencer(script(|v, n| {
        match n {
            0 => assert!(v.trigger(0, 1, &hit(0))),
            1 => v.fade(0, 1, 0.0),
            _ => {}
        }
        1000
    }));
    let _ = render(&mut e, 1001, 128);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn a_sequencer_steps_macros_on_its_own_ticks() {
    let mut e = engine();
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, MACRO_SYNC, 1.0);
    // Volume steps: 1, a half, a quarter, round and round.
    let steps: Vec<f32> = [1.0, 0.5, 0.25].iter().flat_map(|&v| [-1.0, 0.0, -1.0, 0.0, v]).collect();
    e.set_macro(0, &steps, 0, 3, 255, 0).unwrap();
    e.set_sequencer(script(|v, n| {
        if n == 0 {
            v.note_on(0, 5, 60.0, 1.0);
            1001
        } else {
            v.macro_tick(ALL_PARTS);
            [299, 3700, 1][(n as usize - 1) % 3]
        }
    }));
    let x = render(&mut e, 6000, 100).0;
    let near = |at: usize, want: f32| assert!((x[at] - want).abs() < 1e-4, "frame {at} is {}, wanted {want}", x[at]);
    near(1000, 1.0);
    assert!(x[1002] < 1.0, "the step starts on the very frame of the tick");
    near(1017, 0.5);
    near(1299, 0.5);
    near(1317, 0.25);
    near(4999, 0.25);
    near(5020, 0.5);
}

#[test]
fn a_sequencer_wakes_a_sleeping_engine_and_a_host_has_the_last_word() {
    let mut e = engine();
    e.set_sequencer(script(|v, n| {
        if n == 1 {
            assert_eq!(v.time(), 200_000);
            assert_eq!(v.sample_rate(), SR);
            assert!(v.trigger(0, 1, &Trigger { volume: 0.5, ..hit(0) }));
        }
        200_000
    }));
    // The host's own event for the same frame comes after the sequencer's.
    e.push(Event::voice_volume(200_000.0, 0, 1, 0.25));
    let x = render(&mut e, 200_100, 4096).0;
    assert!(x[..200_000].iter().all(|&v| v == 0.0));
    assert!((x[200_000] - 0.25).abs() < 1e-4 && (x[200_099] - 0.25).abs() < 1e-4, "{}", x[200_000]);
}

#[test]
fn a_sequencer_can_be_reset_and_taken_away() {
    let mut e = engine();
    assert!(!e.has_sequencer());
    let resets = Arc::new(AtomicU32::new(0));
    let calls = Arc::new(AtomicU32::new(0));
    let counted = calls.clone();
    let run = move |v: &mut Voices, _n: u32| {
        counted.fetch_add(1, Ordering::Relaxed);
        v.trigger(0, 1, &hit(1));
        500
    };
    assert!(e.set_sequencer(Some(Box::new(Script { run, calls: 0, resets: resets.clone() }))).is_none());
    assert!(e.has_sequencer());
    let _ = render(&mut e, 1200, 128);
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    // After a reset it is told, and called again on the very next frame.
    e.reset();
    assert_eq!(resets.load(Ordering::Relaxed), 1);
    assert_eq!(e.active_voices(), 0);
    let x = render(&mut e, 10, 128).0;
    assert_eq!(calls.load(Ordering::Relaxed), 4);
    assert!((x[0] - 0.5).abs() < 1e-4);
    // Taken away, it is called no more, and comes back to be dropped.
    assert!(e.set_sequencer(None).is_some());
    let _ = render(&mut e, 5000, 128);
    assert_eq!(calls.load(Ordering::Relaxed), 4);
}

#[test]
fn a_wild_sequencer_cannot_break_the_output() {
    for seed in 1..=4u32 {
        let mut e = engine();
        e.set_param(0, MASTER_VOLUME, 0.35);
        e.set_param(0, LIMITER, 1.0);
        let mut rng = Rng::new(seed);
        e.set_sequencer(script(move |v, _| {
            let part = (rng.next_u32() % 5) as u16;
            let handle = rng.next_u32() % 6;
            let wild = [f32::NAN, f32::INFINITY, -1.0, 0.0, 1.0e9, 440.0, 20000.0][(rng.next_u32() % 7) as usize];
            let x = if rng.next_u32() % 3 == 0 { wild } else { rng.unipolar() * 60000.0 };
            match rng.next_u32() % 12 {
                0..=3 => {
                    let t = Trigger { slot: (rng.next_u32() % 5) as u16, position: x as f64 * 0.01, rate: x, volume: rng.unipolar() * 5.0, pan: rng.bipolar() * 2.0 };
                    v.trigger(part, handle, &t);
                }
                4 => v.set_rate(part, handle, x, rng.unipolar()),
                5 => v.set_volume(part, handle, x),
                6 => v.set_position(part, handle, x),
                7 => v.fade(part, handle, x * 1.0e-5),
                8 => v.kill(ALL_PARTS, handle),
                9 => v.note_on(part, handle, 30.0 + rng.unipolar() * 60.0, 1.0),
                10 => v.note_off(part, handle),
                _ => v.set_freq(part, handle, x, 0.0),
            }
            // Sometimes 0, which counts as 1.
            rng.next_u32() % 300
        }));
        let (l, r) = render(&mut e, 48000 * 4, 333);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0), "seed {seed}");
        assert!(rms(&l) > 0.001, "seed {seed} made no sound");
    }
}
