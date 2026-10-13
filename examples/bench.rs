//! How many voices can this machine render in real time?
//!
//!     cargo run --release --example bench

use std::time::Instant;
use synths_core::engine::MAX_VOICES;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;
use synths_core::{Engine, Event};

const SR: f32 = 48000.0;
const SECONDS: usize = 20;

/// Render `SECONDS` of `voices` held notes spread over 16 parts and return
/// how long it took, in seconds.
fn run(voices: usize, patch: &[(usize, f32)]) -> f64 {
    let mut e = Engine::new(SR);
    e.set_param(0, MASTER_VOLUME, 0.05);
    for &(id, v) in patch {
        e.set_param(ALL_PARTS, id, v);
    }
    for i in 0..voices {
        e.push(Event::note_on(0.0, (i % 16) as u16, 0, 36.0 + (i * 7 % 60) as f32, 0.8));
    }
    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    // Let it settle before the clock starts.
    for _ in 0..100 {
        e.render(&mut l, &mut r);
    }
    assert_eq!(e.active_voices(), voices);
    let blocks = SECONDS * SR as usize / 128;
    let mut sum = 0.0f32;
    let start = Instant::now();
    for _ in 0..blocks {
        e.render(&mut l, &mut r);
        sum += l[0] + r[127];
    }
    let took = start.elapsed().as_secs_f64();
    assert!(sum.is_finite());
    assert_eq!(e.active_voices(), voices);
    took
}

/// Render `SECONDS` with a live input playing through part 31 with `patch`,
/// and return how long it took, in seconds. The input is a tone and hiss,
/// so that every effect has something to chew on; `level` 0 is silence.
fn run_input(level: f32, patch: &[(usize, f32)], globals: &[(usize, f32)]) -> f64 {
    let mut e = Engine::new(SR);
    e.set_param(0, MASTER_VOLUME, 0.05);
    for &(id, v) in patch {
        e.set_param(31, id, v);
    }
    for &(id, v) in globals {
        e.set_param(0, id, v);
    }
    let mut rng = synths_core::math::Rng::new(1);
    let input: Vec<f32> = (0..128 * 64)
        .map(|i| level * (0.5 * (i as f32 * 0.02).sin() + 0.2 * (i as f32 * 0.171).sin() + 0.1 * rng.bipolar()))
        .collect();
    let mut l = [0.0f32; 128];
    let mut r = [0.0f32; 128];
    let blocks = SECONDS * SR as usize / 128;
    let mut sum = 0.0f32;
    let start = Instant::now();
    for i in 0..blocks {
        let at = (i % 64) * 128;
        e.render_io(&input[at..at + 128], None, &mut l, &mut r);
        sum += l[0] + r[127];
    }
    let took = start.elapsed().as_secs_f64();
    // Loud enough to be heard, or silence (or hiss under the gate) and nothing out.
    assert!(sum.is_finite() && (sum != 0.0) == (level > 0.5));
    took
}

/// Time the reverb by itself: `SECONDS` of a tone and hiss through one
/// type, in the 16-frame pieces the engine gives it. Milliseconds per second of sound.
fn run_reverb(kind: usize) -> f64 {
    use synths_core::reverb::{Reverb, Settings};
    let mut reverb = Reverb::new(SR);
    let p = Settings { kind, ..Settings::default() };
    let mut rng = synths_core::math::Rng::new(1);
    let input: Vec<f32> = (0..16 * 512).map(|i| 0.3 * (i as f32 * 0.02).sin() + 0.1 * rng.bipolar()).collect();
    let (mut l, mut r) = ([0.0f32; 16], [0.0f32; 16]);
    let pieces = SECONDS * SR as usize / 16;
    let mut sum = 0.0f32;
    let start = Instant::now();
    for i in 0..pieces {
        let at = (i % 512) * 16;
        reverb.process(&input[at..at + 16], &input[at..at + 16], &mut l, &mut r, &p);
        sum += l[0] + r[15];
        l = [0.0; 16];
        r = [0.0; 16];
    }
    let took = start.elapsed().as_secs_f64();
    assert!(sum.is_finite() && sum != 0.0);
    took / SECONDS as f64 * 1000.0
}

fn main() {
    let simple: &[(usize, f32)] = &[(OSC1_WAVE, 2.0)];
    let full: &[(usize, f32)] = &[
        (OSC1_WAVE, 2.0), (OSC2_WAVE, 3.0), (OSC2_LEVEL, 0.8), (OSC2_FINE, 7.0),
        (SUB_LEVEL, 0.5), (NOISE_LEVEL, 0.1), (NOISE_TYPE, 1.0),
        (FILTER_TYPE, 1.0), (FILTER_SLOPE, 1.0), (CUTOFF, 1500.0), (RESONANCE, 0.4), (FILTER_ENV, 24.0),
        (LFO1_PITCH, 0.2), (LFO1_PW, 0.4), (LFO2_CUTOFF, 12.0), (LFO2_PAN, 0.5),
        (DRIVE, 0.2), (CRUSH_BITS, 12.0),
        (SEND_DELAY, 0.3), (SEND_REVERB, 0.3), (SEND_CHORUS, 0.3),
    ];
    println!("{SECONDS} s of sound at {SR} Hz in 128-frame blocks, one thread");
    for (name, patch) in [("one saw, no filter", simple), ("everything on", full)] {
        println!("{name}:");
        let one = run(1, patch);
        for voices in [1, 16, 32, MAX_VOICES] {
            let took = if voices == 1 { one } else { run(voices, patch) };
            let load = took / SECONDS as f64;
            println!(
                "  {voices:3} voices: {:6.2} ms per second of sound ({:5.2}% of one core), {:.0} times faster than real time",
                load * 1000.0,
                load * 100.0,
                1.0 / load
            );
            if voices == MAX_VOICES {
                // The first voice also pays for the effects and the mix, which run whenever anything sounds.
                let each = (took - one) / (voices - 1) as f64 / SECONDS as f64;
                let fixed = one / SECONDS as f64 - each;
                println!(
                    "  each voice costs {:.0} microseconds per second and the effects and mix {:.2} ms: about {:.0} voices would fill one core",
                    each * 1.0e6,
                    fixed * 1000.0,
                    (1.0 - fixed) / each
                );
            }
        }
    }

    // The reverb by itself, which every part shares.
    let reverb = synths_core::reverb::Reverb::new(SR);
    println!("the reverb by itself ({} KB of memory at {SR} Hz):", reverb.bytes() / 1024);
    for (kind, k) in synths_core::reverb::KINDS.iter().enumerate() {
        let ms = run_reverb(kind);
        println!("  {:10} {ms:6.3} ms per second of sound ({:.3}% of one core)", k.name, ms / 10.0);
    }
    let idle = {
        let mut reverb = synths_core::reverb::Reverb::new(SR);
        let p = synths_core::reverb::Settings::default();
        let (mut l, mut r) = ([0.0f32; 16], [0.0f32; 16]);
        let start = Instant::now();
        for _ in 0..SECONDS * SR as usize / 16 {
            reverb.process(&[0.0; 16], &[0.0; 16], &mut l, &mut r, &p);
        }
        start.elapsed().as_secs_f64() / SECONDS as f64 * 1000.0
    };
    println!("  {:10} {idle:6.3} ms", "idle");

    // The live input, and what each voice effect adds for the one part that uses it.
    println!("a live input (mono), through one part:");
    let ms = |took: f64| took / SECONDS as f64 * 1000.0;
    let asleep = {
        let mut e = Engine::new(SR);
        let (mut l, mut r) = ([0.0f32; 128], [0.0f32; 128]);
        let start = Instant::now();
        for _ in 0..SECONDS * SR as usize / 128 {
            e.render(&mut l, &mut r);
        }
        ms(start.elapsed().as_secs_f64())
    };
    let silent = ms(run_input(0.0, &[], &[]));
    println!("  no input at all, engine asleep: {asleep:6.3} ms per second of sound");
    let gated = ms(run_input(0.01, &[], &[(INPUT_GATE, -20.0)]));
    let plain = ms(run_input(1.0, &[], &[]));
    println!("  silence coming in:              {silent:6.3} ms");
    println!("  hiss the gate keeps out:        {gated:6.3} ms");
    println!("  sound, no effects:              {plain:6.3} ms (the mix and the shared effects run, as for one voice)");
    let extra: &[(&str, &[(usize, f32)], &[(usize, f32)])] = &[
        ("the gate, open", &[], &[(INPUT_GATE, -60.0)]),
        ("the part's filter", &[(FILTER_TYPE, 3.0), (FILTER_SLOPE, 1.0)], &[]),
        ("pitch shifter, +3 semitones", &[(FX_PITCH, 3.0)], &[]),
        ("pitch shifter, +7 semitones", &[(FX_PITCH, 7.0)], &[]),
        ("pitch shifter, +12 semitones", &[(FX_PITCH, 12.0)], &[]),
        ("pitch shifter, -7 semitones", &[(FX_PITCH, -7.0)], &[]),
        ("pitch shifter, -12 semitones", &[(FX_PITCH, -12.0)], &[]),
        ("ring modulator", &[(FX_RING, 1.0)], &[]),
        ("ring modulator, noise", &[(FX_RING, 1.0), (FX_RING_WAVE, 1.0)], &[]),
        ("comb", &[(FX_COMB, 0.5)], &[]),
        ("comb, swept", &[(FX_COMB, 0.5), (FX_COMB_DEPTH, 0.5)], &[]),
        ("distortion and bit crusher", &[(DRIVE, 0.3), (CRUSH_BITS, 8.0), (CRUSH_RATE, 4.0)], &[]),
        (
            "all of them at once",
            &[(FILTER_TYPE, 3.0), (FX_PITCH, 7.0), (FX_RING, 1.0), (FX_COMB, 0.5), (FX_COMB_DEPTH, 0.5), (DRIVE, 0.3), (CRUSH_BITS, 8.0)],
            &[(INPUT_GATE, -60.0)],
        ),
    ];
    for (name, patch, globals) in extra {
        let took = ms(run_input(1.0, patch, globals));
        println!("  + {name:28} {:6.3} ms more ({:.3}% of one core)", took - plain, (took - plain) / 10.0);
    }
}
