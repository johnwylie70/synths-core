//! Renders a voice through every voice changer to WAV files in target/demo,
//! to listen to. There is no microphone here, so the voice is a made-up one
//! (pulses like the vocal cords', through resonances like the mouth's).
//! Each file also has a recording, made inside the engine while the voice
//! went by, played back an octave down at the end.
//!
//!     cargo run --release --example voice

#[path = "../tests/common/speech.rs"]
mod speech;

use std::fs;
use std::io::Write;
use synths_core::event::RECORD_PART;
use synths_core::params::*;
use synths_core::presets::{apply_voice_preset, VOICE_PRESETS};
use synths_core::{Engine, Event};

const SR: f32 = 48000.0;
/// The part the voice plays through, and the one its recording is played on.
const VOICE: u16 = 31;
const PLAYER: u16 = 0;

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

fn main() {
    fs::create_dir_all("target/demo").unwrap();
    let seconds = 4.5;
    let mut voice = speech::speech(SR, seconds);
    let spoken = voice.len();
    // Then a second for echoes to die away, and three for the recording played slowly.
    voice.resize(spoken + 4 * SR as usize, 0.0);
    let presets = std::iter::once(None).chain(VOICE_PRESETS.iter().map(Some));
    for preset in presets {
        let name = preset.map_or("dry", |p| p.name);
        let mut e = Engine::new(SR);
        e.set_param(0, MASTER_VOLUME, 0.8);
        // A real microphone hisses: a gate at -45 dB keeps that out between words.
        e.set_param(0, INPUT_GATE, -45.0);
        apply_voice_preset(&mut e, VOICE, preset);
        // Record the changed voice (the first second and a half of it), as
        // a host does for "record what I say".
        e.record_buffer(0, (1.5 * SR) as usize, 1).unwrap();
        e.push(Event::record_start(0.0, 0, RECORD_PART, VOICE));
        // And play the recording back an octave down, as any sample is played.
        for (id, v) in [(OSC1_WAVE, 4.0), (OSC1_SAMPLE, 0.0), (AMP_ATTACK, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.05)] {
            e.set_param(PLAYER, id, v);
        }
        e.push(Event::note_on((spoken + SR as usize) as f64, PLAYER, 1, 48.0, 1.0));
        let (mut l, mut r) = (vec![0.0; voice.len()], vec![0.0; voice.len()]);
        let start = std::time::Instant::now();
        for ((x, cl), cr) in voice.chunks(128).zip(l.chunks_mut(128)).zip(r.chunks_mut(128)) {
            e.render_io(x, None, cl, cr);
        }
        let took = start.elapsed().as_secs_f32();
        let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
        let rms = (l[..spoken].iter().map(|v| v * v).sum::<f32>() / spoken as f32).sqrt();
        let path = format!("target/demo/voice-{name}.wav");
        write_wav(&path, &l, &r);
        println!(
            "{path}: peak {peak:.3}, rms {rms:.3}, {:.2} ms to render each second of sound{}",
            took * 1000.0 / (voice.len() as f32 / SR),
            preset.map_or(String::new(), |p| format!("  ({})", p.help))
        );
    }
}
