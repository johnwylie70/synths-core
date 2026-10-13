//! Renders a made-up piano two ways, and a small drum kit, to WAV files in
//! target/demo, to listen to:
//!
//!     cargo run --release --example zones
//!
//! The piano is five recordings, of C2, C3, C4, C5 and C6: each a decaying
//! tone with its own brightness and its own length, as the strings of a
//! piano have. `zones-single.wav` plays a two-octave scale and a chord the
//! old way, from the one recording of middle C stretched over every key;
//! `zones-multi.wav` plays the same from a sample map of all five.
//! `zones-kit.wav` is a kit on a map whose keys each play their own sound
//! as it was recorded, with two takes of the hi-hat taking turns.

use std::fs;
use std::io::Write;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::zones::*;
use synths_core::{Engine, Event};

const SR: f32 = 48000.0;

fn write_wav(path: &str, l: &[f32], r: &[f32]) {
    let bytes = (l.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&(SR as u32).to_le_bytes());
    out.extend_from_slice(&(SR as u32 * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for i in 0..l.len() {
        for v in [l[i], r[i]] {
            out.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    fs::File::create(path).unwrap().write_all(&out).unwrap();
}

/// Render `seconds` of the engine to a file and say how loud it was.
fn save(name: &str, e: &mut Engine, seconds: f32) {
    let n = (seconds * SR) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    let mut most = 0;
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(cl, cr);
        most = most.max(e.active_voices());
    }
    let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
    fs::create_dir_all("target/demo").unwrap();
    let path = format!("target/demo/{name}.wav");
    write_wav(&path, &l, &r);
    println!("{path}: {seconds:.1} s, peak {peak:.2}, {most} voices at most");
}

/// A piano-like tone at `note`: harmonics that die away, the high ones
/// soonest. A low string is bright and rings for seconds; a high one is
/// nearly a pure tone and is gone in one.
fn string(note: f32) -> Vec<f32> {
    let hz = 440.0 * 2.0f32.powf((note - 69.0) / 12.0);
    let height = (note - 36.0) / 48.0; // 0 at C2, 1 at C6
    let seconds = 4.0 - 2.8 * height;
    let harmonics = (((0.4 * SR) / hz) as usize).min((24.0 - 20.0 * height) as usize).max(1);
    let frames = (seconds * SR) as usize;
    let mut out = vec![0.0f32; frames];
    for h in 1..=harmonics {
        // A stiff string's harmonics are a little sharp, and the higher the weaker.
        let f = hz * h as f32 * (1.0 + 0.0004 * (h * h) as f32).sqrt();
        let level = 1.0 / (h as f32).powf(1.0 + 1.2 * height);
        let decay = (1.2 + 3.0 * height) * (1.0 + 0.35 * (h - 1) as f32);
        for (i, x) in out.iter_mut().enumerate() {
            let t = i as f32 / SR;
            *x += level * (-decay * t).exp() * (std::f32::consts::TAU * f * t).sin();
        }
    }
    // The hammer: a moment to reach full, and a fade at the very end.
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    for (i, x) in out.iter_mut().enumerate() {
        let attack = (i as f32 / (0.002 * SR)).min(1.0);
        let end = ((frames - i) as f32 / (0.05 * SR)).min(1.0);
        *x *= 0.8 / peak * attack * end;
    }
    out
}

const ROOTS: [u8; 5] = [36, 48, 60, 72, 84];

/// An engine with the five recordings in slots 0 to 4 and part 0 set to play samples.
fn piano() -> Box<Engine> {
    let mut e = Engine::new(SR);
    for (slot, &root) in ROOTS.iter().enumerate() {
        e.load_sample(slot, &string(root as f32), 1, SR).unwrap();
        e.configure_sample(slot, root as f32, 0, 0, 0).unwrap();
    }
    for (id, v) in [(OSC1_WAVE, 4.0), (AMP_ATTACK, 0.0), (AMP_DECAY, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.25), (VELOCITY_SENS, 0.6), (SEND_REVERB, 0.12)] {
        e.set_param(0, id, v);
    }
    e.set_param(0, MASTER_VOLUME, 0.35);
    e
}

/// The five recordings as a map: each covers the keys nearest its own, so
/// the ranges meet halfway between neighbours, and the low strings ring on
/// longer than the high ones once the key is let go.
fn five_zones() -> Vec<Zone> {
    (0..ROOTS.len())
        .map(|i| {
            let lo = if i == 0 { 0 } else { (ROOTS[i - 1] + ROOTS[i]).div_ceil(2) };
            let hi = if i + 1 == ROOTS.len() { 127 } else { (ROOTS[i] + ROOTS[i + 1]).div_ceil(2) - 1 };
            Zone { release: 0.5 - 0.09 * i as f32, ..Zone::new(i as u16, lo, hi, ROOTS[i] as f32) }
        })
        .collect()
}

/// A scale of two octaves up from C3, then a chord across four.
fn phrase(e: &mut Engine) -> f32 {
    let scale = [48, 50, 52, 53, 55, 57, 59, 60, 62, 64, 65, 67, 69, 71, 72];
    let mut at = 0.1;
    for &note in &scale {
        e.push(Event::note_on((at * SR) as f64, 0, 0, note as f32, 0.8));
        e.push(Event::note_off(((at + 0.3) * SR) as f64, 0, 0, note as f32));
        at += 0.32;
    }
    at += 0.4;
    for (i, &note) in [36, 48, 55, 64, 72, 79, 84].iter().enumerate() {
        let on = at + 0.03 * i as f32;
        e.push(Event::note_on((on * SR) as f64, 0, 0, note as f32, 0.7));
        e.push(Event::note_off(((at + 2.5) * SR) as f64, 0, 0, note as f32));
    }
    at + 4.5
}

/// A burst of noise through a one-pole filter, dying away: `tone` 0 is dull, 1 is raw hiss.
fn hiss(seed: u32, seconds: f32, decay: f32, tone: f32, high: bool) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut low = 0.0f32;
    (0..(seconds * SR) as usize)
        .map(|i| {
            let x = rng.bipolar();
            low += (x - low) * tone;
            (if high { x - low } else { low }) * (-decay * i as f32 / SR).exp() * 0.8
        })
        .collect()
}

fn kit() -> Box<Engine> {
    let mut e = Engine::new(SR);
    // A kick: a tone that falls from 160 to 45 Hz.
    let kick: Vec<f32> = {
        let mut phase = 0.0f32;
        (0..(0.4 * SR) as usize)
            .map(|i| {
                let t = i as f32 / SR;
                phase += (45.0 + 115.0 * (-30.0 * t).exp()) / SR;
                0.9 * (-9.0 * t).exp() * (std::f32::consts::TAU * phase).sin()
            })
            .collect()
    };
    // A snare: a short tone under a burst of noise.
    let snare: Vec<f32> = hiss(3, 0.3, 22.0, 0.6, false)
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let t = i as f32 / SR;
            0.7 * n + 0.4 * (-30.0 * t).exp() * (std::f32::consts::TAU * 190.0 * t).sin()
        })
        .collect();
    let sounds = [kick, snare, hiss(5, 0.08, 70.0, 0.35, true), hiss(6, 0.08, 55.0, 0.25, true), hiss(7, 0.6, 7.0, 0.3, true)];
    for (slot, data) in sounds.iter().enumerate() {
        e.load_sample(slot, data, 1, SR).unwrap();
    }
    // Each key is one sound, at the pitch it was recorded at, played to its
    // end whatever the key does. The two closed hats share a key and take turns.
    let drum = |slot: u16, key: u8| Zone { flags: ZONE_FIXED_PITCH | ZONE_ONE_SHOT, ..Zone::new(slot, key, key, 60.0) };
    let zones = [
        drum(0, 36),
        drum(1, 38),
        Zone { group: 1, pan: 0.3, volume: 0.7, ..drum(2, 42) },
        Zone { group: 1, pan: 0.3, volume: 0.7, ..drum(3, 42) },
        Zone { pan: 0.3, volume: 0.6, ..drum(4, 46) },
    ];
    e.set_zones(9, &zones, MAP_SILENT_OUTSIDE).unwrap();
    for (id, v) in [(OSC1_WAVE, 4.0), (AMP_ATTACK, 0.0), (AMP_SUSTAIN, 1.0), (VELOCITY_SENS, 0.8), (SEND_REVERB, 0.08)] {
        e.set_param(9, id, v);
    }
    e.set_param(0, MASTER_VOLUME, 0.6);
    e
}

fn main() {
    // The old way: middle C alone, stretched over every key.
    let mut e = piano();
    e.set_param(0, OSC1_SAMPLE, 2.0);
    let seconds = phrase(&mut e);
    save("zones-single", &mut e, seconds);

    // The same notes from all five recordings.
    let mut e = piano();
    e.set_zones(0, &five_zones(), 0).unwrap();
    for z in e.zones(0) {
        println!("  slot {}: keys {} to {}, recorded at {}, release {:.2} s", z.slot, z.key_lo, z.key_hi, z.root, z.release);
    }
    let seconds = phrase(&mut e);
    save("zones-multi", &mut e, seconds);

    // Two bars of a kit; key 40 is not a drum and is not heard.
    let mut e = kit();
    let step = 0.125 * SR as f64;
    for bar in 0..2 {
        for i in 0..16 {
            let at = (bar * 16 + i) as f64 * step + 4800.0;
            let mut hit = |key: f32, velocity: f32| {
                e.push(Event::note_on(at, 9, 0, key, velocity));
                e.push(Event::note_off(at + 600.0, 9, 0, key));
            };
            if i % 8 == 0 || i == 10 {
                hit(36.0, 1.0);
            }
            if i % 8 == 4 {
                hit(38.0, 0.9);
            }
            if i == 14 {
                hit(46.0, 0.8);
            } else {
                hit(42.0, if i % 2 == 0 { 0.8 } else { 0.5 });
            }
            hit(40.0, 1.0);
        }
    }
    save("zones-kit", &mut e, 4.6);
}
