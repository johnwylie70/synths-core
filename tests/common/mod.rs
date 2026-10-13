//! Helpers shared by the tests.
#![allow(dead_code)]

use synths_core::event::ALL_PARTS;
use synths_core::params::*;
use synths_core::{Engine, Event};

pub const SR: f32 = 48000.0;

/// An engine set up so that what a voice makes arrives at the left output
/// unchanged: full volume, panned hard left, no attack, full sustain.
pub fn plain_engine() -> Box<Engine> {
    let mut e = Engine::new(SR);
    e.set_param(0, MASTER_VOLUME, 1.0);
    e.set_param(0, LIMITER, 0.0);
    e.set_param(ALL_PARTS, PAN, -1.0);
    e.set_param(ALL_PARTS, AMP_ATTACK, 0.0);
    e.set_param(ALL_PARTS, AMP_DECAY, 0.0);
    e.set_param(ALL_PARTS, AMP_SUSTAIN, 1.0);
    e.set_param(ALL_PARTS, AMP_RELEASE, 0.0);
    e
}

/// Render `frames` frames in blocks of `block`.
pub fn render(e: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    let mut at = 0;
    while at < frames {
        let n = block.min(frames - at);
        e.render(&mut l[at..at + n], &mut r[at..at + n]);
        at += n;
    }
    (l, r)
}

pub fn note_on(e: &mut Engine, time: u64, part: u16, note: f32) {
    assert!(e.push(Event::note_on(time as f64, part, 0, note, 1.0)));
}

pub fn note_off(e: &mut Engine, time: u64, part: u16, note: f32) {
    assert!(e.push(Event::note_off(time as f64, part, 0, note)));
}

/// The frequency of a steady tone, from the time between its first and last
/// upward crossings of zero.
pub fn frequency(x: &[f32], sr: f32) -> f32 {
    let mut first = None;
    let mut last = 0.0f64;
    let mut count = 0u32;
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            let t = (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64;
            if first.is_none() {
                first = Some(t);
            } else {
                count += 1;
            }
            last = t;
        }
    }
    match first {
        Some(f) if count > 0 => (count as f64 * sr as f64 / (last - f)) as f32,
        _ => 0.0,
    }
}

pub fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

pub fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
}

/// The amplitude of the part of `x` at exactly `hz`.
pub fn amplitude_at(x: &[f32], hz: f32, sr: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    let w = std::f64::consts::TAU * hz as f64 / sr as f64;
    let n = x.len() as f64;
    for (i, &v) in x.iter().enumerate() {
        // A Hann window keeps other frequencies from leaking in.
        let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        re += v as f64 * win * (w * i as f64).cos();
        im += v as f64 * win * (w * i as f64).sin();
    }
    (4.0 * (re * re + im * im).sqrt() / n) as f32
}

pub fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1.0e-12).log10()
}

pub fn hz(note: f32) -> f32 {
    440.0 * 2.0f32.powf((note - 69.0) / 12.0)
}
