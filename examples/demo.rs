//! Renders a few short phrases to WAV files in target/demo, to listen to.
//!
//!     cargo run --release --example demo

use std::fs;
use std::io::Write;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;
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
            out.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes());
        }
    }
    fs::File::create(path).unwrap().write_all(&out).unwrap();
}

/// Render `seconds` of the engine to a file and say how loud it was.
fn save(name: &str, e: &mut Engine, seconds: f32) {
    let n = (seconds * SR) as usize;
    let mut l = vec![0.0; n];
    let mut r = vec![0.0; n];
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(cl, cr);
    }
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    let rms = (l.iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
    let path = format!("target/demo/{name}.wav");
    write_wav(&path, &l, &r);
    println!("{path}: {seconds} s, peak {peak:.3}, rms {rms:.3}");
}

fn set(e: &mut Engine, part: u16, values: &[(usize, f32)]) {
    for &(id, v) in values {
        e.set_param(part, id, v);
    }
}

/// Play `notes` one after another, each `step` seconds long and held for `hold` of that.
fn line(e: &mut Engine, part: u16, start: f32, step: f32, hold: f32, notes: &[f32]) {
    for (i, &note) in notes.iter().enumerate() {
        if note <= 0.0 {
            continue;
        }
        let t = (start + i as f32 * step) * SR;
        e.push(Event::note_on(t as f64, part, 0, note, 0.9));
        e.push(Event::note_off((t + step * hold * SR) as f64, part, 0, note));
    }
}

fn main() {
    fs::create_dir_all("target/demo").unwrap();

    // 1. The four plain waves, one scale each.
    let mut e = Engine::new(SR);
    let scale = [60.0, 62.0, 64.0, 65.0, 67.0, 69.0, 71.0, 72.0];
    for wave in 0..4 {
        set(&mut e, wave, &[(OSC1_WAVE, wave as f32)]);
        line(&mut e, wave, wave as f32 * 2.0, 0.25, 0.8, &scale);
    }
    save("1-waves", &mut e, 8.5);

    // 2. A filtered bass line: two saws and a sub, a filter that closes on
    // each note, a little drive.
    let mut e = Engine::new(SR);
    set(&mut e, 0, &[
        (OSC1_WAVE, 2.0), (OSC2_WAVE, 2.0), (OSC2_LEVEL, 0.8), (OSC2_FINE, 12.0),
        (SUB_LEVEL, 0.7), (FILTER_TYPE, 1.0), (FILTER_SLOPE, 1.0), (CUTOFF, 120.0),
        (RESONANCE, 0.55), (FILTER_ENV, 48.0), (FENV_DECAY, 0.18), (FENV_SUSTAIN, 0.1),
        (AMP_RELEASE, 0.08), (MODE, 1.0), (GLIDE, 0.04), (DRIVE, 0.25),
    ]);
    let bass = [36.0, 36.0, 48.0, 36.0, 39.0, 36.0, 46.0, 48.0, 36.0, 36.0, 48.0, 36.0, 43.0, 41.0, 39.0, 34.0];
    line(&mut e, 0, 0.0, 0.2, 0.7, &bass);
    line(&mut e, 0, 3.2, 0.2, 0.7, &bass);
    save("2-bass", &mut e, 7.0);

    // 3. A pad: unison saws, slow attack, a wandering filter, chorus and reverb.
    let mut e = Engine::new(SR);
    set(&mut e, 0, &[
        (OSC1_WAVE, 2.0), (UNISON, 5.0), (UNISON_DETUNE, 18.0), (UNISON_SPREAD, 0.9),
        (OSC2_WAVE, 3.0), (OSC2_LEVEL, 0.4), (OSC2_COARSE, 12.0), (LFO1_PW, 0.6), (LFO1_RATE, 0.4),
        (FILTER_TYPE, 1.0), (CUTOFF, 900.0), (RESONANCE, 0.3), (LFO2_CUTOFF, 18.0), (LFO2_RATE, 0.15),
        (AMP_ATTACK, 0.8), (AMP_DECAY, 1.0), (AMP_SUSTAIN, 0.8), (AMP_RELEASE, 1.8),
        (SEND_CHORUS, 0.5), (SEND_REVERB, 0.6),
    ]);
    set(&mut e, 0, &[(REVERB_SIZE, 0.85), (MASTER_VOLUME, 0.2)]);
    for (i, chord) in [[48.0, 55.0, 60.0, 64.0], [45.0, 52.0, 60.0, 64.0], [41.0, 53.0, 57.0, 60.0], [43.0, 55.0, 59.0, 62.0]].iter().enumerate() {
        for &note in chord {
            let t = i as f32 * 3.0 * SR;
            e.push(Event::note_on(t as f64, 0, 0, note, 0.7));
            e.push(Event::note_off((t + 2.6 * SR) as f64, 0, 0, note));
        }
    }
    save("3-pad", &mut e, 16.0);

    // 4. A lead with vibrato that comes in late, pitch bend, sync sweep and echo.
    let mut e = Engine::new(SR);
    set(&mut e, 0, &[
        (OSC1_WAVE, 2.0), (OSC1_LEVEL, 0.0), (OSC2_WAVE, 2.0), (OSC2_LEVEL, 1.0), (OSC2_SYNC, 1.0),
        (OSC2_COARSE, 7.0), (MENV_OSC2_PITCH, 24.0), (MENV_DECAY, 0.5), (MENV_SUSTAIN, 0.2),
        (LFO1_PITCH, 0.25), (LFO1_RATE, 5.5), (LFO1_DELAY, 0.25), (LFO1_FADE, 0.3),
        (AMP_RELEASE, 0.3), (MODE, 2.0), (GLIDE, 0.06), (SEND_DELAY, 0.35), (SEND_REVERB, 0.2),
    ]);
    set(&mut e, 0, &[(DELAY_TIME, 0.375), (DELAY_FEEDBACK, 0.45), (DELAY_PINGPONG, 1.0)]);
    line(&mut e, 0, 0.0, 0.375, 0.95, &[69.0, 72.0, 76.0, 74.0, 72.0, 0.0, 67.0, 69.0]);
    line(&mut e, 0, 3.0, 0.75, 0.9, &[76.0, 79.0, 81.0]);
    e.push(Event::param((4.6 * SR) as f64, 0, BEND, 1.0, 0.4));
    e.push(Event::param((5.2 * SR) as f64, 0, BEND, 0.0, 0.1));
    save("4-lead", &mut e, 8.0);

    // 5. Chip tune: pulse waves with moving width, a triangle bass and noise drums.
    let mut e = Engine::new(SR);
    set(&mut e, ALL_PARTS, &[(AMP_ATTACK, 0.0), (AMP_RELEASE, 0.02)]);
    set(&mut e, 0, &[(OSC1_WAVE, 3.0), (OSC1_PW, 0.25), (LFO1_PW, 0.3), (LFO1_RATE, 3.0), (VOLUME, 0.6), (PAN, -0.3), (CRUSH_BITS, 6.0)]);
    set(&mut e, 1, &[(OSC1_WAVE, 1.0), (VOLUME, 0.9), (CRUSH_BITS, 4.0)]);
    set(&mut e, 2, &[(OSC1_LEVEL, 0.0), (NOISE_LEVEL, 1.0), (NOISE_TYPE, 2.0), (AMP_DECAY, 0.06), (AMP_SUSTAIN, 0.0), (VOLUME, 0.5)]);
    set(&mut e, 3, &[(OSC1_LEVEL, 0.0), (NOISE_LEVEL, 1.0), (NOISE_TYPE, 3.0), (AMP_DECAY, 0.03), (AMP_SUSTAIN, 0.0), (VOLUME, 0.3), (PAN, 0.4)]);
    let tune = [72.0, 76.0, 79.0, 84.0, 79.0, 76.0, 74.0, 77.0, 81.0, 86.0, 81.0, 77.0, 71.0, 74.0, 79.0, 83.0];
    for bar in 0..2 {
        let t = bar as f32 * 2.4;
        line(&mut e, 0, t, 0.15, 0.6, &tune);
        line(&mut e, 1, t, 0.3, 0.9, &[36.0, 36.0, 43.0, 36.0, 41.0, 41.0, 43.0, 47.0]);
        line(&mut e, 2, t, 0.3, 0.5, &[40.0, 90.0, 40.0, 90.0, 40.0, 90.0, 40.0, 90.0]);
        line(&mut e, 3, t, 0.15, 0.5, &[100.0; 16]);
    }
    save("5-chip", &mut e, 5.5);

    // 6. Samples: a plucked string made here in code, played as a one-shot
    // across two octaves, then a looped tone, the way a tracker would drive it.
    let mut e = Engine::new(SR);
    let mut pluck = vec![0.0f32; 24000];
    let period = 100;
    let mut seed = 22222u32;
    for v in pluck.iter_mut().take(period) {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        *v = (seed >> 8) as f32 / 8_388_608.0 - 1.0;
    }
    for i in period..pluck.len() {
        pluck[i] = 0.498 * (pluck[i - period] + pluck[i - period + 1]);
    }
    e.load_sample(0, &pluck, 1, SR).unwrap();
    // The string sounds at 48000 / 100.5 Hz, a little below B4.
    e.configure_sample(0, 69.0 + 12.0 * (SR / 100.5 / 440.0).log2(), 0, 0, 0).unwrap();
    set(&mut e, 0, &[(OSC1_WAVE, 4.0), (OSC1_SAMPLE, 0.0), (AMP_ATTACK, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.3), (SEND_REVERB, 0.3)]);
    line(&mut e, 0, 0.0, 0.2, 2.0, &[52.0, 59.0, 64.0, 68.0, 71.0, 76.0, 71.0, 68.0, 64.0, 59.0, 52.0, 40.0]);
    let wave: Vec<f32> = (0..256).map(|i| {
        let t = i as f32 / 256.0 * std::f32::consts::TAU;
        0.6 * t.sin() + 0.25 * (2.0 * t).sin() + 0.15 * (5.0 * t).sin()
    }).collect();
    e.load_sample(1, &wave, 1, 8363.0).unwrap();
    e.configure_sample(1, 60.0, 2, 0, 256).unwrap();
    set(&mut e, 1, &[(OSC1_WAVE, 4.0), (OSC1_SAMPLE, 1.0), (AMP_ATTACK, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.0)]);
    // A tracker channel: one note, then slides, a volume fade and a pan sweep by handle.
    let t0 = 3.0 * SR;
    e.push(Event::note_on(t0 as f64, 1, 1, 84.0, 1.0));
    for tick in 0..100 {
        let t = (t0 + tick as f32 * 0.02 * SR) as f64;
        let arp = [0.0, 4.0, 7.0][tick % 3];
        e.push(Event::voice_pitch(t, 1, 1, 84.0 + arp + (tick / 25) as f32 * 2.0, 0.0));
        e.push(Event::voice_volume(t, 1, 1, 1.0 - tick as f32 / 100.0));
        e.push(Event::voice_pan(t, 1, 1, (tick as f32 / 16.0).sin()));
    }
    e.push(Event::voice_kill((t0 + 2.0 * SR) as f64, 1, 1));
    save("6-samples", &mut e, 6.0);

    // 7. Everything at once, far too loud, to hear the limiter hold it.
    let mut e = Engine::new(SR);
    set(&mut e, 0, &[(OSC1_WAVE, 2.0), (MASTER_VOLUME, 1.5)]);
    for i in 0..48 {
        e.push(Event::note_on((i as f32 * 0.05 * SR) as f64, 0, 0, 36.0 + (i * 5 % 48) as f32, 1.0));
    }
    e.push(Event::all_notes_off((3.0 * SR) as f64, ALL_PARTS, false));
    save("7-limiter", &mut e, 4.0);
}
