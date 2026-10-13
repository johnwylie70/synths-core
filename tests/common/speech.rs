//! A voice to test with, since a test has no microphone: a train of pulses
//! like those of the vocal cords, through resonances like those of the
//! mouth, with a moving pitch, changing vowels and bursts of hiss.
#![allow(dead_code)]

use std::f64::consts::{PI, TAU};

/// The first three resonances of five vowels, in Hz: a, i, u, e, o.
const VOWELS: [[f64; 3]; 5] = [[730.0, 1090.0, 2440.0], [270.0, 2290.0, 3010.0], [300.0, 870.0, 2240.0], [530.0, 1840.0, 2480.0], [570.0, 840.0, 2410.0]];
const WIDTHS: [f64; 3] = [80.0, 100.0, 140.0];

struct Resonance {
    y1: f64,
    y2: f64,
}

impl Resonance {
    fn run(&mut self, x: f64, hz: f64, width: f64, sr: f64) -> f64 {
        let r = (-PI * width / sr).exp();
        let a1 = -2.0 * r * (TAU * hz / sr).cos();
        let a2 = r * r;
        let y = (1.0 + a1 + a2) * x - a1 * self.y1 - a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as i32) as f64 / 2_147_483_648.0
    }
}

/// One pulse of the vocal cords, as the air pressure it makes: `phase` is 0..1 through the cycle.
fn pulse(phase: f64) -> f64 {
    // The cords open slowly and snap shut: the snap is what excites the mouth.
    if phase < 0.6 {
        let t = phase / 0.6;
        (PI * t).sin() * (PI * t * 0.5).sin() * 0.6
    } else if phase < 0.7 {
        -1.0 + (phase - 0.6) / 0.1
    } else {
        0.0
    }
}

/// A steady vowel ("a") at `f0` Hz: `seconds` of it, peaking near 0.5.
pub fn vowel(sr: f32, f0: f32, seconds: f32) -> Vec<f32> {
    voice(sr, seconds, |_| (f0 as f64, 0, 0, 0.0, 1.0, 0.0))
}

/// Speech-like sound: syllables on a moving pitch, with a new vowel each
/// time and a hiss ("s", "sh", "t") between some of them.
pub fn speech(sr: f32, seconds: f32) -> Vec<f32> {
    voice(sr, seconds, |t| {
        // A syllable every 0.3 seconds: 0.2 of voice, then a gap that is sometimes a hiss.
        let syllable = (t / 0.3) as usize;
        let within = t - syllable as f64 * 0.3;
        // The pitch falls through each phrase of five syllables and wobbles a little.
        let phrase = (syllable % 5) as f64 + within / 0.3;
        let f0 = 150.0 - 12.0 * phrase + 3.0 * (TAU * 5.5 * t).sin() + if syllable % 7 == 3 { 25.0 } else { 0.0 };
        let (from, to) = (syllable % 5, (syllable * 3 + 1) % 5);
        let glide = (within / 0.2).min(1.0);
        let level = if within < 0.2 { (PI * within / 0.2).sin().powf(0.4) } else { 0.0 };
        let hiss = if within >= 0.21 && within < 0.28 && syllable % 3 != 2 { (PI * (within - 0.21) / 0.07).sin() } else { 0.0 };
        (f0, from, to, glide, level, hiss)
    })
}

/// The voice at one moment: its pitch in Hz, two vowels and how far from
/// the first to the second (0..1), how loud the voice is and how loud the hiss.
type Moment = (f64, usize, usize, f64, f64, f64);

/// `shape` gives the voice at a time in seconds.
fn voice(sr: f32, seconds: f32, shape: impl Fn(f64) -> Moment) -> Vec<f32> {
    let sr = sr as f64;
    let n = (seconds as f64 * sr) as usize;
    let mut out = Vec::with_capacity(n);
    let mut mouth = [Resonance { y1: 0.0, y2: 0.0 }, Resonance { y1: 0.0, y2: 0.0 }, Resonance { y1: 0.0, y2: 0.0 }];
    let mut noise = Noise(0x1234_5678);
    let (mut phase, mut last, mut hp) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let t = i as f64 / sr;
        let (f0, from, to, glide, level, hiss) = shape(t);
        phase += f0 / sr;
        if phase >= 1.0 {
            phase -= 1.0;
        }
        let flow = pulse(phase) * level;
        // What reaches the mouth is the change in flow, with a little breath.
        let mut x = (flow - last) * sr / (f0 * 12.0) + 0.004 * level * noise.next();
        last = flow;
        for (k, m) in mouth.iter_mut().enumerate() {
            let hz = VOWELS[from][k] + (VOWELS[to][k] - VOWELS[from][k]) * glide;
            x = m.run(x, hz, WIDTHS[k], sr);
        }
        // The hiss is noise with its low end taken off.
        let white = noise.next();
        hp += 0.35 * (white - hp);
        out.push((0.5 * x + 0.12 * hiss * (white - hp)) as f32);
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        for v in out.iter_mut() {
            *v *= 0.5 / peak;
        }
    }
    out
}
