//! Renders chords and arpeggios to WAV files in target/demo, to listen to.
//!
//!     cargo run --release --example arps
//!
//! `arp-c64.wav`: the fast arpeggio of a sound chip, one voice stepping 50
//! times a second. `arp-trance.wav`: a held note arpeggiated in sixteenths
//! over two octaves while the chord changes under it, each note struck
//! afresh on a saw with a filter envelope, with an echo. `arp-pop.wav`:
//! eighths up and down on a soft pluck. `chord-together.wav`: the same
//! progression as block chords. `song-chords-and-arps.wav`: the song in
//! `examples/songs/chords-and-arps.json`, which has all three, played by
//! the song player. `arp-pattern.wav`: the trance progression following a
//! syncopated pattern with rests, accents, a tie and a slide.
//! `trance-gate.wav`: a pad holding the progression, chopped in sixteenths,
//! with the echo and the reverb ringing through the gaps.
//! `arp-pattern-and-gate.wav`: the two together. `bends.wav`: a lead line
//! from a song with a glide, a scoop, a fall, a delayed vibrato and a siren.
//! `song-mix.wav`: `examples/songs/mix.json`, a song with its own faders,
//! pans, sends, reverb, echo and loop, with two faders moved as it plays.

use std::fs;
use synths_core::arp::*;
use synths_core::event::*;
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
    fs::write(path, out).unwrap();
}

/// Render `seconds` of the engine to a file and say how loud it was and
/// how many voices it took.
fn save(name: &str, e: &mut Engine, seconds: f32) {
    let n = (seconds * SR) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    let mut most = 0;
    for (cl, cr) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(cl, cr);
        most = most.max(e.active_voices());
    }
    let peak = l.iter().chain(r.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    let path = format!("target/demo/{name}.wav");
    write_wav(&path, &l, &r);
    println!("{path}: {seconds} s, peak {peak:.3}, {most} voices at most");
}

fn set(e: &mut Engine, part: u16, values: &[(usize, f32)]) {
    for &(id, v) in values {
        e.set_param(part, id, v);
    }
}

/// A minor, F, C, G: each chord as semitones from the A the arpeggios are
/// held on, and as a root with its own offsets for the block chords.
const FROM_A: [[i8; 3]; 4] = [[0, 3, 7], [-4, 0, 3], [-9, -5, -2], [-2, 2, 5]];
const BLOCK: [(f32, [i8; 4]); 4] = [(57.0, [0, 3, 7, 12]), (53.0, [0, 4, 7, 12]), (48.0, [0, 4, 7, 12]), (55.0, [0, 4, 7, 10])];

fn main() {
    fs::create_dir_all("target/demo").unwrap();

    // The Commodore 64's chord: one pulse voice, 50 steps a second.
    let mut e = Engine::new(SR);
    set(&mut e, 0, &[(OSC1_WAVE, 3.0), (OSC1_PW, 0.3), (AMP_ATTACK, 0.002), (AMP_DECAY, 0.3), (AMP_SUSTAIN, 0.6), (AMP_RELEASE, 0.1), (VOLUME, 0.6)]);
    for bar in 0..8 {
        let at = (bar as f32 * 0.9 * SR) as f64;
        let (root, chord) = BLOCK[bar % 4];
        e.push(Event::arp(at, 0, 0, 50.0, ORDER_UP, &chord[..3]));
        e.push(Event::note_on(at, 0, 1 + bar as u32, root + 12.0, 0.8));
        e.push(Event::note_off(at + (0.8 * SR) as f64, 0, 1 + bar as u32, 0.0));
    }
    save("arp-c64", &mut e, 7.6);

    // Trance: a saw through a filter that snaps shut on every note, in
    // sixteenths at 138 beats a minute, up through two octaves, each note
    // held for half its step, with an echo three sixteenths long. One note
    // is held throughout; the chord changes under it every bar and the
    // arpeggio carries on in step.
    let mut e = Engine::new(SR);
    let bar = 60.0 / 138.0 * 4.0;
    e.set_param(0, TEMPO, 138.0);
    set(&mut e, 0, &[(DELAY_TIME, bar / 16.0 * 3.0), (DELAY_FEEDBACK, 0.45), (DELAY_PINGPONG, 1.0)]);
    set(
        &mut e,
        0,
        &[
            (OSC1_WAVE, 2.0),
            (OSC2_WAVE, 2.0),
            (OSC2_LEVEL, 0.7),
            (OSC2_FINE, 9.0),
            (FILTER_TYPE, 1.0),
            (CUTOFF, 500.0),
            (RESONANCE, 0.35),
            (FILTER_ENV, 48.0),
            (FENV_ATTACK, 0.0),
            (FENV_DECAY, 0.12),
            (FENV_SUSTAIN, 0.1),
            (AMP_ATTACK, 0.001),
            (AMP_DECAY, 0.2),
            (AMP_SUSTAIN, 0.7),
            (AMP_RELEASE, 0.12),
            (SEND_DELAY, 0.35),
            (VOLUME, 0.5),
        ],
    );
    e.push(Event::arp_notes(0.0, 0, 0, 0.0, ORDER_UP, &FROM_A[0]));
    e.push(Event::arp_style(0.0, 0, 0, 0.5, 2, 0.0, 0));
    e.push(Event::arp_beat(0.0, 0, 0, 4.0));
    e.push(Event::note_on(0.0, 0, 1, 57.0, 0.9));
    for bars in 1..8 {
        e.push(Event::arp_notes((bars as f32 * bar * SR) as f64, 0, 1, 0.0, ORDER_UP, &FROM_A[bars % 4]));
    }
    e.push(Event::note_off((8.0 * bar * SR) as f64, 0, 1, 0.0));
    save("arp-trance", &mut e, 8.0 * bar + 2.0);

    // Pop: eighths at 100 beats a minute, up and down, on a soft pluck that
    // rings on under the next note, in a room.
    let mut e = Engine::new(SR);
    let bar = 60.0 / 100.0 * 4.0;
    e.set_param(0, TEMPO, 100.0);
    set(
        &mut e,
        0,
        &[
            (OSC1_WAVE, 1.0),
            (OSC2_WAVE, 0.0),
            (OSC2_LEVEL, 0.5),
            (OSC2_COARSE, 12.0),
            (AMP_ATTACK, 0.003),
            (AMP_DECAY, 0.5),
            (AMP_SUSTAIN, 0.15),
            (AMP_RELEASE, 0.5),
            (SEND_REVERB, 0.3),
            (VOLUME, 0.7),
        ],
    );
    for bars in 0..8 {
        let at = (bars as f32 * bar * SR) as f64;
        let (root, chord) = BLOCK[bars % 4];
        e.push(Event::arp_notes(at, 0, 0, 0.0, ORDER_UP_DOWN, &chord));
        // (Every other eighth a little softer.)
        e.push(Event::arp_style(at, 0, 0, 0.9, 1, 0.0, 0xAF));
        e.push(Event::arp_beat(at, 0, 0, 2.0));
        e.push(Event::note_on(at, 0, 1 + bars as u32, root, 0.8));
        e.push(Event::note_off(at + (bar * SR) as f64, 0, 1 + bars as u32, 0.0));
    }
    save("arp-pop", &mut e, 8.0 * bar + 2.0);

    // The same progression as block chords: every note of each on one
    // frame, under one handle, on a soft pad.
    let mut e = Engine::new(SR);
    set(
        &mut e,
        0,
        &[
            (OSC1_WAVE, 2.0),
            (OSC2_WAVE, 2.0),
            (OSC2_LEVEL, 0.8),
            (OSC2_FINE, 7.0),
            (FILTER_TYPE, 1.0),
            (CUTOFF, 1400.0),
            (AMP_ATTACK, 0.08),
            (AMP_DECAY, 0.5),
            (AMP_SUSTAIN, 0.7),
            (AMP_RELEASE, 0.5),
            (SEND_REVERB, 0.25),
            (VOLUME, 0.35),
        ],
    );
    for bars in 0..8 {
        let at = (bars as f32 * bar * SR) as f64;
        let (root, chord) = BLOCK[bars % 4];
        e.push(Event::arp(at, 0, 0, -1.0, ORDER_TOGETHER, &chord));
        e.push(Event::note_on(at, 0, 1 + bars as u32, root, 0.7));
        e.push(Event::note_off(at + (0.95 * bar * SR) as f64, 0, 1 + bars as u32, 0.0));
    }
    save("chord-together", &mut e, 8.0 * bar + 2.0);

    // The trance lead again, following a pattern: accents on the root, rests,
    // a tie and a slide; and a pad chopped by a gate; and both at once.
    let bar = 60.0 / 138.0 * 4.0;
    let lead = |e: &mut Engine| {
        e.set_arp_pattern(1, &Pattern::parse("1^ - 3 2 _ 1^ ~3 - 2 1' - 3^ 2 - 1 ~4").unwrap()).unwrap();
        set(e, 0, &[(DELAY_TIME, bar / 16.0 * 3.0), (DELAY_FEEDBACK, 0.45), (DELAY_PINGPONG, 1.0)]);
        set(
            e,
            0,
            &[
                (OSC1_WAVE, 2.0),
                (OSC2_WAVE, 2.0),
                (OSC2_LEVEL, 0.7),
                (OSC2_FINE, 9.0),
                (FILTER_TYPE, 1.0),
                (CUTOFF, 500.0),
                (RESONANCE, 0.35),
                (FILTER_ENV, 48.0),
                (FENV_DECAY, 0.12),
                (FENV_SUSTAIN, 0.1),
                (FILTER_VELOCITY, 0.6),
                (AMP_ATTACK, 0.001),
                (AMP_DECAY, 0.2),
                (AMP_SUSTAIN, 0.7),
                (AMP_RELEASE, 0.12),
                (SEND_DELAY, 0.3),
                (VOLUME, 0.5),
            ],
        );
        e.push(Event::arp_notes(0.0, 0, 0, 0.0, ORDER_UP, &FROM_A[0]));
        e.push(Event::arp_style(0.0, 0, 0, 0.5, 1, 0.0, 0));
        e.push(Event::arp_beat(0.0, 0, 0, 4.0));
        e.push(Event::arp_pattern(0.0, 0, 0, 1));
        e.push(Event::note_on(0.0, 0, 1, 57.0, 0.9));
        for bars in 1..8 {
            e.push(Event::arp_notes((bars as f32 * bar * SR) as f64, 0, 1, 0.0, ORDER_UP, &FROM_A[bars % 4]));
        }
        e.push(Event::note_off((8.0 * bar * SR) as f64, 0, 1, 0.0));
    };
    let pad = |e: &mut Engine| {
        set(e, 0, &[(DELAY_TIME, bar / 16.0 * 3.0), (DELAY_FEEDBACK, 0.4), (DELAY_PINGPONG, 1.0), (REVERB_TYPE, 1.0)]);
        set(
            e,
            1,
            &[
                (OSC1_WAVE, 2.0),
                (OSC2_WAVE, 2.0),
                (OSC2_LEVEL, 0.8),
                (OSC2_FINE, 8.0),
                (FILTER_TYPE, 1.0),
                (CUTOFF, 2200.0),
                (AMP_ATTACK, 0.05),
                (AMP_SUSTAIN, 0.9),
                (AMP_RELEASE, 0.3),
                (SEND_DELAY, 0.35),
                (SEND_REVERB, 0.3),
                (VOLUME, 0.3),
                (GATE_DEPTH, 1.0),
                (GATE_RATE, 4.0),
                (GATE_SMOOTH, 0.004),
            ],
        );
        let (levels, n) = synths_core::gate::parse("x.x.xx.xx.x6xx.x").unwrap();
        e.set_gate(1, &levels[..n]).unwrap();
        for bars in 0..8 {
            let at = (bars as f32 * bar * SR) as f64;
            let (root, chord) = BLOCK[bars % 4];
            e.push(Event::arp(at, 1, 0, -1.0, ORDER_TOGETHER, &chord));
            e.push(Event::note_on(at, 1, 10 + bars as u32, root, 0.7));
            e.push(Event::note_off(at + (bar * SR) as f64, 1, 10 + bars as u32, 0.0));
        }
    };
    for (name, with_lead, with_pad) in [("arp-pattern", true, false), ("trance-gate", false, true), ("arp-pattern-and-gate", true, true)] {
        let mut e = Engine::new(SR);
        e.set_param(0, TEMPO, 138.0);
        if with_lead {
            lead(&mut e);
        }
        if with_pad {
            pad(&mut e);
        }
        save(name, &mut e, 8.0 * bar + 2.5);
    }

    // A song with all three, through the song player.
    #[cfg(feature = "json")]
    {
        use synths_core::song::native;
        use synths_core::song_player;
        let text = fs::read_to_string("examples/songs/chords-and-arps.json").unwrap();
        let song = native::load(&text, &|_| None).unwrap();
        let mut e = Engine::new(SR);
        // The lead on part 0 and the pad on part 1.
        set(
            &mut e,
            0,
            &[
                (OSC1_WAVE, 2.0),
                (FILTER_TYPE, 1.0),
                (CUTOFF, 600.0),
                (RESONANCE, 0.3),
                (FILTER_ENV, 42.0),
                (FENV_DECAY, 0.12),
                (FENV_SUSTAIN, 0.1),
                (AMP_ATTACK, 0.001),
                (AMP_DECAY, 0.2),
                (AMP_SUSTAIN, 0.7),
                (AMP_RELEASE, 0.15),
                (SEND_DELAY, 0.3),
                (VOLUME, 0.45),
            ],
        );
        set(&mut e, 1, &[(OSC1_WAVE, 2.0), (FILTER_TYPE, 1.0), (CUTOFF, 1200.0), (AMP_ATTACK, 0.05), (AMP_SUSTAIN, 0.8), (AMP_RELEASE, 0.4), (SEND_REVERB, 0.3), (VOLUME, 0.3)]);
        set(&mut e, 0, &[(DELAY_TIME, 0.326), (DELAY_FEEDBACK, 0.35)]);
        song_player::install(&mut e, song, song_player::FIRST_PART, 0, false).unwrap();
        e.push(Event::song(0.0, SONG_PATCH_PART, 1.0, 0.0));
        e.push(Event::song(0.0, SONG_PATCH_PART, 2.0, 1.0));
        e.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
        e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        save("song-chords-and-arps", &mut e, 9.5);

        // A lead line with a glide between notes, a scoop, a fall, a
        // delayed vibrato on a long note and a siren slide.
        let text = fs::read_to_string("examples/songs/bends.json").unwrap();
        let song = native::load(&text, &|_| None).unwrap();
        let mut e = Engine::new(SR);
        set(
            &mut e,
            0,
            &[(OSC1_WAVE, 3.0), (OSC1_PW, 0.25), (FILTER_TYPE, 1.0), (CUTOFF, 3500.0), (AMP_ATTACK, 0.005), (AMP_DECAY, 0.3), (AMP_SUSTAIN, 0.7), (AMP_RELEASE, 0.15), (SEND_DELAY, 0.2), (VOLUME, 0.4)],
        );
        song_player::install(&mut e, song, song_player::FIRST_PART, 0, false).unwrap();
        e.push(Event::song(0.0, SONG_PATCH_PART, 1.0, 0.0));
        e.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
        e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        save("bends", &mut e, 9.0);

        // A song with a mix of its own: faders, pans, sends, a hall and a
        // dotted-eighth echo, going round its loop.
        let text = fs::read_to_string("examples/songs/mix.json").unwrap();
        let song = native::load(&text, &|_| None).unwrap();
        let mut e = Engine::new(SR);
        set(
            &mut e,
            0,
            &[(OSC1_WAVE, 2.0), (FILTER_TYPE, 1.0), (CUTOFF, 700.0), (RESONANCE, 0.3), (FILTER_ENV, 40.0), (FENV_DECAY, 0.12), (FENV_SUSTAIN, 0.1), (AMP_ATTACK, 0.001), (AMP_SUSTAIN, 0.7), (AMP_RELEASE, 0.12), (VOLUME, 0.5)],
        );
        set(&mut e, 1, &[(OSC1_WAVE, 2.0), (FILTER_TYPE, 1.0), (CUTOFF, 1500.0), (AMP_ATTACK, 0.05), (AMP_SUSTAIN, 0.8), (AMP_RELEASE, 0.4), (VOLUME, 0.3)]);
        song_player::install(&mut e, song, song_player::FIRST_PART, 0, false).unwrap();
        e.push(Event::song(0.0, SONG_PATCH_PART, 1.0, 0.0));
        e.push(Event::song(0.0, SONG_PATCH_PART, 2.0, 1.0));
        e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        // Half way through, the pad's fader comes down and the lead's echo goes up.
        e.push(Event::song((5.0 * SR) as f64, SONG_CHANNEL_VOLUME, 1.0, 0.2));
        e.push(Event::song((5.0 * SR) as f64, SONG_CHANNEL_DELAY, 0.0, 0.8));
        save("song-mix", &mut e, 10.0);
    }
}
