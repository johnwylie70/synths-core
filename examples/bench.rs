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
}
