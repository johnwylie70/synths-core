//! Renders three sounds through every type of the reverb, and through the
//! reverb the engine used to have, to WAV files in target/demo, to listen to:
//!
//!     cargo run --release --example reverb
//!
//! `reverb-dry-click.wav`, `reverb-room-click.wav`, ... `reverb-old-chord.wav`:
//! a snare-like click, a short stab and a held chord, each dry, through
//! room, hall, plate, cathedral and spring as they are to begin with, and
//! through the old reverb as it was to begin with.

use std::fs;
use std::io::Write;
use synths_core::params::*;
use synths_core::reverb::{rt60, Reverb, Settings, KINDS};
use synths_core::{Engine, Event};

#[path = "../tests/common/old_reverb.rs"]
mod old_reverb;

const SR: f32 = 48000.0;
/// How much of the sound is sent to the reverb, and the master volume.
const SEND: f32 = 0.5;
const MASTER: f32 = 0.35;

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

/// An engine that hands on exactly what its parts make.
fn engine(patch: &[(usize, f32)]) -> Box<Engine> {
    let mut e = Engine::new(SR);
    e.set_param(0, MASTER_VOLUME, 1.0);
    e.set_param(0, LIMITER, 0.0);
    for &(id, v) in patch {
        e.set_param(0, id, v);
    }
    e
}

fn notes(e: &mut Engine, at: f32, hold: f32, notes: &[f32]) {
    for &note in notes {
        e.push(Event::note_on((at * SR) as f64, 0, 0, note, 0.9));
        e.push(Event::note_off(((at + hold) * SR) as f64, 0, 0, note));
    }
}

fn render(e: &mut Engine, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (seconds * SR) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(cl, cr);
    }
    (l, r)
}

fn main() {
    fs::create_dir_all("target/demo").unwrap();
    const LONG: f32 = 14.0;

    // A snare-like click: a burst of noise over a tone that drops, three times.
    let mut e = engine(&[
        (OSC1_WAVE, 0.0), (OSC1_LEVEL, 0.7), (NOISE_LEVEL, 0.9), (MENV_PITCH, 12.0), (MENV_DECAY, 0.04),
        (AMP_ATTACK, 0.0), (AMP_DECAY, 0.11), (AMP_SUSTAIN, 0.0), (AMP_RELEASE, 0.05),
        (FILTER_TYPE, 1.0), (CUTOFF, 6000.0),
    ]);
    for at in [0.1, 0.85, 1.225] {
        notes(&mut e, at, 0.1, &[50.0]);
    }
    let click = render(&mut e, LONG);

    // A short stab: a chord of saws through a filter that snaps shut, twice.
    let mut e = engine(&[
        (OSC1_WAVE, 2.0), (OSC2_WAVE, 2.0), (OSC2_LEVEL, 0.7), (OSC2_FINE, 9.0), (FILTER_TYPE, 1.0), (CUTOFF, 500.0),
        (FILTER_ENV, 40.0), (FENV_DECAY, 0.12), (FENV_SUSTAIN, 0.0), (AMP_ATTACK, 0.0), (AMP_RELEASE, 0.05), (VOLUME, 0.5),
    ]);
    notes(&mut e, 0.1, 0.12, &[48.0, 60.0, 63.0, 67.0]);
    notes(&mut e, 1.1, 0.12, &[46.0, 58.0, 62.0, 65.0]);
    let stab = render(&mut e, LONG);

    // A held chord: a soft pad for three seconds.
    let mut e = engine(&[
        (OSC1_WAVE, 2.0), (UNISON, 3.0), (UNISON_DETUNE, 12.0), (UNISON_SPREAD, 0.6), (FILTER_TYPE, 1.0), (CUTOFF, 1400.0),
        (AMP_ATTACK, 0.4), (AMP_SUSTAIN, 0.9), (AMP_RELEASE, 0.4), (VOLUME, 0.35),
    ]);
    notes(&mut e, 0.1, 3.0, &[52.0, 59.0, 64.0, 68.0]);
    let chord = render(&mut e, LONG);

    for (what, (dl, dr)) in [("click", &click), ("stab", &stab), ("chord", &chord)] {
        let sent: [Vec<f32>; 2] = [dl.iter().map(|v| v * SEND).collect(), dr.iter().map(|v| v * SEND).collect()];
        let save = |name: &str, seconds: f32, wet: &dyn Fn(&[f32], &[f32], &mut [f32], &mut [f32])| {
            let n = (seconds * SR) as usize;
            let (mut l, mut r) = (dl[..n].to_vec(), dr[..n].to_vec());
            // In the 16-frame pieces the engine gives its reverb.
            wet(&sent[0][..n], &sent[1][..n], &mut l, &mut r);
            for v in l.iter_mut().chain(r.iter_mut()) {
                *v *= MASTER;
            }
            let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
            let rms = (l.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
            let path = format!("target/demo/reverb-{name}-{what}.wav");
            write_wav(&path, &l, &r);
            println!("{path}: {seconds:.1} s, peak {peak:.3}, rms {rms:.4}");
        };
        save("dry", 4.5, &|_, _, _, _| {});
        save("old", 6.5, &|il, ir, ol, or| old_reverb::OldReverb::new(SR).process(il, ir, ol, or, 0.6, 0.4));
        for (kind, k) in KINDS.iter().enumerate() {
            let p = Settings { kind, ..Settings::default() };
            let seconds = (4.5 + 1.2 * rt60(kind, p.size, p.decay)).min(LONG);
            save(k.name, seconds, &|il, ir, ol, or| {
                let mut reverb = Reverb::new(SR);
                for at in (0..il.len()).step_by(16) {
                    let to = (at + 16).min(il.len());
                    reverb.process(&il[at..to], &ir[at..to], &mut ol[at..to], &mut or[at..to], &p);
                }
            });
        }
    }
}
