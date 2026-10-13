//! Measurements of a reverb's tail, for `tests/reverb.rs` and `examples/reverb.rs`.
#![allow(dead_code)]

use std::f64::consts::{PI, TAU};

/// The Fourier transform of `re` and `im`, in place. Their length is a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let (wr, wi) = ((TAU / len as f64).cos(), -(TAU / len as f64).sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

/// `x` with only what lies between `low` and `high` Hz left in (a
/// fourth-order band-pass: two second-order filters, one after the other).
pub fn band(x: &[f32], low: f32, high: f32, sr: f32) -> Vec<f64> {
    let mut y: Vec<f64> = x.iter().map(|&v| v as f64).collect();
    let centre = (low as f64 * high as f64).sqrt();
    let q = centre / (high as f64 - low as f64);
    for _ in 0..2 {
        let w = TAU * centre / sr as f64;
        let alpha = w.sin() / (2.0 * q);
        let (b0, b2, a0, a1, a2) = (alpha, -alpha, 1.0 + alpha, -2.0 * w.cos(), 1.0 - alpha);
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in y.iter_mut() {
            let out = (b0 * *v + b2 * x2 - a1 * y1 - a2 * y2) / a0;
            (x2, x1) = (x1, *v);
            (y2, y1) = (y1, out);
            *v = out;
        }
    }
    y
}

/// How long an impulse response takes to fall by 60 dB, in seconds, by
/// Schroeder's method: the energy still to come at each moment falls in a
/// straight line (in dB), and the slope of that line between 5 and 35 dB
/// down says how long 60 dB takes.
pub fn rt60(x: &[f64], sr: f32) -> f32 {
    let mut left = vec![0.0f64; x.len() + 1];
    for i in (0..x.len()).rev() {
        left[i] = left[i + 1] + x[i] * x[i];
    }
    let total = left[0].max(1.0e-300);
    let down = |db: f64| left.iter().position(|&e| 10.0 * (e / total).log10() <= db).unwrap_or(x.len());
    let (a, b) = (down(-5.0), down(-35.0));
    assert!(b < x.len(), "the recording is too short to fall by 35 dB");
    // The line of best fit through the curve between the two.
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for i in (a..b).step_by(((b - a) / 2000).max(1)) {
        let (t, db) = (i as f64 / sr as f64, 10.0 * (left[i] / total).log10());
        sx += t;
        sy += db;
        sxx += t * t;
        sxy += t * db;
        n += 1.0;
    }
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    (-60.0 / slope) as f32
}

/// How like noise `x` is around each moment, every 5 ms: of the frames in
/// the 20 ms around it, the share that lie further from nothing than one
/// standard deviation, against the share that would in noise (Abel and
/// Huang's echo density, 2006). A few separate echoes give a number near 0;
/// a wash too thick to tell echoes apart gives 1.
pub fn echo_density(x: &[f32], sr: f32) -> Vec<(f32, f32)> {
    let half = (0.01 * sr) as usize;
    let hop = (0.005 * sr) as usize;
    let mut out = Vec::new();
    let mut at = half;
    while at + half < x.len() {
        let win = &x[at - half..at + half];
        let sd = (win.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / win.len() as f64).sqrt();
        if sd > 0.0 {
            let outside = win.iter().filter(|&&v| (v as f64).abs() > sd).count() as f64 / win.len() as f64;
            out.push((at as f32 / sr, (outside / 0.3173) as f32));
        }
        at += hop;
    }
    out
}

/// What the late tail is like: how flat its spectrum is and how far it repeats.
#[derive(Debug, Clone, Copy)]
pub struct Tail {
    /// The geometric mean of the power spectrum over its mean, from 200 Hz
    /// to 6 kHz: 0.56 for noise, less the more it is a few ringing pitches.
    pub flatness: f32,
    /// The strongest pitch against the average, in dB (about 10 for noise).
    pub peak_db: f32,
    /// How like itself the tail is when moved along by 2 to 100 ms, at the
    /// worst: 0 for noise, towards 1 for something that repeats.
    pub repeats: f32,
    /// How far along that is, in seconds.
    pub repeats_at: f32,
}

/// Measure `frames` frames of a tail, a power of two, starting at `from`.
/// The tail is first made level (its decay of `t60` seconds undone).
pub fn tail(x: &[f32], from: usize, frames: usize, t60: f32, sr: f32) -> Tail {
    let n = frames;
    let mut re: Vec<f64> = (0..n)
        .map(|i| {
            let level = (6.907_755 * i as f64 / (t60 as f64 * sr as f64)).exp();
            let hann = 0.5 - 0.5 * (TAU * i as f64 / n as f64).cos();
            x[from + i] as f64 * level * hann
        })
        .collect();
    let mut im = vec![0.0f64; n];
    fft(&mut re, &mut im);
    let mut power: Vec<f64> = (0..n).map(|i| re[i] * re[i] + im[i] * im[i]).collect();
    let bin = |hz: f32| (hz * n as f32 / sr) as usize;
    let part = &power[bin(200.0)..bin(6000.0)];
    let mean = part.iter().sum::<f64>() / part.len() as f64;
    let log_mean = part.iter().map(|&p| p.max(1.0e-300).ln()).sum::<f64>() / part.len() as f64;
    let peak = part.iter().cloned().fold(0.0f64, f64::max);
    // Only that band, and its autocorrelation is the transform of its power.
    for (i, p) in power.iter_mut().enumerate() {
        let k = i.min(n - i);
        if k < bin(200.0) || k >= bin(6000.0) {
            *p = 0.0;
        }
    }
    let mut zero = vec![0.0f64; n];
    fft(&mut power, &mut zero);
    let (a, b) = ((0.002 * sr) as usize, (0.1 * sr) as usize);
    let (repeats, lag) = (a..b).map(|lag| ((power[lag] / power[0]).abs(), lag)).fold((0.0f64, 0), |m, v| if v.0 > m.0 { v } else { m });
    Tail {
        flatness: (log_mean.exp() / mean) as f32,
        peak_db: (10.0 * (peak / mean).log10()) as f32,
        repeats: repeats as f32,
        repeats_at: lag as f32 / sr,
    }
}

/// How alike two signals are: 1 the same, 0 nothing in common, -1 opposite.
pub fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let dot = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(&p, &q)| p as f64 * q as f64).sum::<f64>();
    (dot(a, b) / (dot(a, a) * dot(b, b)).sqrt().max(1.0e-300)) as f32
}

pub fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| (v as f64) * (v as f64)).sum()
}

/// When the energy of `x` between `low` and `high` Hz arrives, on average, in seconds.
pub fn arrival(x: &[f32], low: f32, high: f32, sr: f32) -> f32 {
    let y = band(x, low, high, sr);
    let total: f64 = y.iter().map(|v| v * v).sum();
    (y.iter().enumerate().map(|(i, v)| i as f64 * v * v).sum::<f64>() / total.max(1.0e-300) / sr as f64) as f32
}

const _: f64 = PI;
