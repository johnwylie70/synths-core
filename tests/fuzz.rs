mod common;
use common::*;
use synths_core::engine::{MAX_SAMPLES, MAX_VOICES, NUM_PARTS};
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::{Engine, Event};

/// A number that is sometimes sensible and sometimes not at all.
fn wild(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
    match rng.next_u32() % 40 {
        0 => f32::NAN,
        1 => f32::INFINITY,
        2 => f32::NEG_INFINITY,
        3 => 1.0e30,
        4 => -1.0e30,
        5 => 1.0e-40,
        6 => 0.0,
        _ => lo + (hi - lo) * rng.unipolar(),
    }
}

fn storm(seed: u32, seconds: usize, sr: f32) {
    let mut rng = Rng::new(seed);
    let mut e = Engine::new(sr);
    // Some samples to play, including awkward ones.
    for slot in 0..8 {
        let frames = 1 + (rng.next_u32() % 3000) as usize;
        let channels = 1 + (slot % 2) as u32;
        let data: Vec<f32> = (0..frames * channels as usize).map(|_| wild(&mut rng, -1.0, 1.0)).collect();
        e.load_sample(slot, &data, channels, 8000.0 + 40000.0 * rng.unipolar()).unwrap();
        let _ = e.configure_sample(slot, wild(&mut rng, 0.0, 127.0), rng.next_u32() % 4, rng.next_u32() % 4000, rng.next_u32() % 4000);
    }
    let total = seconds * sr as usize;
    let mut done = 0;
    let mut l = vec![0.0f32; 4096];
    let mut r = vec![0.0f32; 4096];
    let (mut loud, mut most) = (0usize, 0usize);
    while done < total {
        let now = e.time() as f64;
        for _ in 0..(rng.next_u32() % 24) {
            let time = match rng.next_u32() % 8 {
                0 => f64::NAN,
                1 => -1.0e12,
                2 => now + 1.0e15,
                _ => now + (rng.next_u32() % 3000) as f64,
            };
            let part = match rng.next_u32() % 20 {
                0 => 0xFFFF,
                1 => 1000,
                _ => (rng.next_u32() % 6) as u16,
            };
            let kind = if rng.next_u32() % 2 == 0 { 1 } else { (rng.next_u32() % 17) as u16 };
            let mut ev = Event::note_on(time, part, rng.next_u32() % 5, wild(&mut rng, 0.0, 127.0), wild(&mut rng, 0.0, 1.0));
            ev.kind = kind;
            if kind == 4 {
                // A parameter: any id, in range or not, and any value.
                let info = PARAMS[rng.next_u32() as usize % PARAMS.len()];
                ev.a = if rng.next_u32() % 10 == 0 { rng.next_u32() % 300 } else { info.id as u32 };
                let span = info.max - info.min;
                ev.x = wild(&mut rng, info.min - span, info.max + span);
                ev.y = wild(&mut rng, -1.0, 2.0);
            }
            if kind == 7 {
                ev.x = wild(&mut rng, -100.0, 30000.0);
            }
            if kind == 10 {
                ev.x = wild(&mut rng, -100.0, 5000.0);
            }
            ev.z = wild(&mut rng, -1.0, 9.0);
            ev.b = rng.next_u32();
            if kind == 13 {
                ev.x = wild(&mut rng, -10.0, 2000.0);
                ev.y = wild(&mut rng, -1.0, 6.0);
            }
            e.push(ev);
        }
        if rng.next_u32() % 50 == 0 {
            let slot = (rng.next_u32() % 8) as usize;
            let _ = e.free_sample(slot);
            if rng.next_u32() % 2 == 0 {
                let data: Vec<f32> = (0..500).map(|_| rng.bipolar()).collect();
                e.load_sample(slot, &data, 1, sr).unwrap();
            }
        }
        if rng.next_u32() % 400 == 0 {
            e.reset();
        }
        if rng.next_u32() % 30 == 0 {
            // A step macro of any length, full of anything.
            let count = (rng.next_u32() % 65) as usize;
            let numbers: Vec<f32> = (0..count * 5).map(|_| wild(&mut rng, -5.0, 5.0)).collect();
            let part = (rng.next_u32() % 6) as u16;
            e.set_macro(part, &numbers, rng.next_u32() % 70, rng.next_u32() % 70, rng.next_u32() % 70, rng.next_u32() % 2).unwrap();
        }
        if rng.next_u32() % 20 == 0 {
            // The taps are read as a display would.
            let mut scope = [0.0f32; 512];
            e.taps().read_scope(rng.next_u32() % 70, &mut scope);
            e.taps().read_scope(synths_core::taps::MASTER, &mut scope);
            assert!(scope.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
            if let Some(levels) = e.taps().read_levels(rng.next_u32() % 64) {
                assert!(levels.iter().all(|v| v.is_finite() && *v >= 0.0));
            }
        }
        if rng.next_u32() % 8 == 0 {
            // Back to many voices, however the storm has left things.
            e.set_param(0, VOICES, 64.0);
            e.set_param(0, MASTER_VOLUME, 0.5);
            e.set_param((rng.next_u32() % 6) as u16, MODE, 0.0);
            e.set_param((rng.next_u32() % 6) as u16, AMP_SUSTAIN, 0.7);
        }
        let n = 1 + (rng.next_u32() % 700) as usize;
        e.render(&mut l[..n], &mut r[..n]);
        for i in 0..n {
            assert!(l[i].is_finite() && r[i].is_finite(), "seed {seed}: not a number at frame {}", done + i);
            assert!(l[i].abs() <= 1.0 && r[i].abs() <= 1.0, "seed {seed}: {} {} at frame {}", l[i], r[i], done + i);
        }
        loud += l[..n].iter().filter(|v| v.abs() > 0.01).count();
        most = most.max(e.active_voices());
        assert!(e.active_voices() <= MAX_VOICES);
        done += n;
    }
    // The storm really did make a noise.
    assert!(loud > total / 10 && most == MAX_VOICES, "seed {seed}: {loud} loud frames of {total}, {most} voices; master {} vol0 {} voices {}", e.param(0, MASTER_VOLUME), e.param(0, VOLUME), e.param(0, VOICES));
}

#[test]
fn random_events_never_break_the_output() {
    for seed in 1..=6 {
        storm(seed, 8, SR);
    }
    storm(77, 4, 8000.0);
    storm(78, 4, 44100.0);
    storm(79, 4, 192_000.0);
}

#[test]
fn the_parameter_table_is_sound() {
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for p in PARAMS {
        assert!(ids.insert(p.id), "id {} is used twice", p.id);
        assert!(names.insert(p.name), "name {} is used twice", p.name);
        assert!(p.min < p.max && p.default >= p.min && p.default <= p.max, "{}", p.name);
        assert!((p.id as usize) < PART_PARAMS || ((p.id as usize) >= GLOBAL_BASE && (p.id as usize) < GLOBAL_BASE + GLOBAL_PARAMS));
        assert_eq!(p.kind == Kind::Choice, !p.choices.is_empty(), "{}", p.name);
        if p.kind == Kind::Choice {
            assert_eq!(p.choices.split('|').count() as f32, p.max - p.min + 1.0, "{}", p.name);
        }
        assert!(!p.help.is_empty());
        assert_eq!(info(p.id as usize).unwrap().name, p.name);
        assert_eq!(by_name(p.name).unwrap().id, p.id);
    }
    assert!(info(15).is_none() && info(5000).is_none());
    assert_eq!(OSC2_PW - OSC1_PW, OSC_STRIDE);
    assert_eq!(LFO2_PAN - LFO1_PAN, LFO_STRIDE);
    // A new engine holds the defaults.
    let e = Engine::new(SR);
    for p in PARAMS {
        assert_eq!(e.param(NUM_PARTS - 1, p.id as usize), p.default, "{}", p.name);
    }
    assert_eq!(by_name("osc1.sample").unwrap().max as usize, MAX_SAMPLES - 1);
    assert_eq!(by_name("voices").unwrap().max as usize, MAX_VOICES);
}
