//! Chords and arpeggios in songs, played through the engine: a `chord` is
//! its notes together, an `arp` is its notes one at a time, in the song's
//! own time, and a song from before the two were told apart is unchanged.

use synths_core::event::*;
use synths_core::params::*;
use synths_core::song::{binary, native, Song};
use synths_core::song_player::{self, FIRST_PART, STATE_PLAYING, STATE_STOPPED};
use synths_core::{Engine, Event};

const SR: f32 = 44100.0;
/// The part the song's synth patch plays on.
const LEAD: u16 = 10;

/// A song of one pattern of 32 rows and two channels. Instrument 1 is a
/// synth patch and instrument 2 a looped square wave; `top` is any other
/// top-level keys, each with its comma.
fn song(top: &str, events: &str) -> Song {
    let text = format!(
        r#"{{"format": "synths-song", "version": 2, {top} "channels": 2, "order": [0],
        "patterns": [{{"rows": 32, "events": [{events}]}}],
        "instruments": [{{"name": "lead", "patch": "lead"}}, {{"name": "square", "sample": 0}}],
        "samples": [{{"wave": [100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
                              -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100],
                     "loop": {{}}}}]}}"#
    );
    native::load(&text, &|_| None).unwrap()
}

/// An engine with the song loaded and playing from frame 0, its patch a
/// plain sine that starts and stops at once.
fn playing(song: Song) -> Box<Engine> {
    let mut e = Engine::new(SR);
    for (id, value) in [(OSC1_WAVE, 0.0), (AMP_ATTACK, 0.0), (AMP_DECAY, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.0), (VELOCITY_SENS, 0.0)] {
        e.set_param(LEAD, id, value);
    }
    e.set_param(0, LIMITER, 0.0);
    song_player::install(&mut e, song, FIRST_PART, 0, false).unwrap();
    e.push(Event::song(0.0, SONG_PATCH_PART, 1.0, LEAD as f32));
    e.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
    e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    e
}

/// Render a frame at a time and note every frame on which more voices were
/// sounding than the frame before, with how many: the frames notes were
/// struck on, exactly. Also gives the sound (the left side).
fn strikes(e: &mut Engine, frames: usize) -> (Vec<(usize, usize)>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let mut out = Vec::new();
    let mut before = e.active_voices();
    for i in 0..frames {
        e.render(&mut l[i..i + 1], &mut r[i..i + 1]);
        let now = e.active_voices();
        if now > before {
            out.push((i, now));
        }
        before = now;
    }
    (out, l)
}

fn render(e: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        e.render(a, b);
    }
    (l, r)
}

/// The frame each tick begins on, for ticks of these lengths in frames.
fn tick_frames(lengths: impl Iterator<Item = f64>) -> Vec<usize> {
    let (mut at, mut carry) = (0usize, 0.0f64);
    lengths
        .map(|length| {
            let frame = at;
            carry += length;
            at += carry.floor() as usize;
            carry -= carry.floor();
            frame
        })
        .collect()
}

fn tick_length(tempo: f64) -> f64 {
    SR as f64 * 2.5 / tempo
}

/// The semitones above C-4 of the tone in a stretch of sound.
fn semitones(x: &[f32]) -> i32 {
    let rises: Vec<usize> = (1..x.len()).filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0).collect();
    let period = (rises[rises.len() - 1] - rises[0]) as f64 / (rises.len() - 1) as f64;
    (12.0 * (SR as f64 / period / 261.6256).log2()).round() as i32
}

/// How much of `hz` there is in a stretch of sound.
fn amount(x: &[f32], hz: f64) -> f64 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let w = std::f64::consts::TAU * hz * i as f64 / SR as f64;
        let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / x.len() as f64).cos();
        re += v as f64 * window * w.cos();
        im += v as f64 * window * w.sin();
    }
    4.0 * (re * re + im * im).sqrt() / x.len() as f64
}

fn hz(semitones_above_c4: i32) -> f64 {
    261.6256 * (semitones_above_c4 as f64 / 12.0).exp2()
}

#[test]
fn a_chord_is_its_notes_together_on_one_channel() {
    for ins in [1, 2] {
        let mut e = playing(song(
            "",
            &format!(r#"{{"row": 2, "ch": 0, "note": "C-4", "ins": {ins}, "chord": "m7"}}, {{"row": 8, "ch": 0, "vol": 16}}, {{"row": 12, "ch": 0, "note": "off"}}"#),
        ));
        let (struck, x) = strikes(&mut e, 882 * 6 * 14);
        // All four on the row's first frame, and nothing struck after.
        assert_eq!(struck, [(882 * 6 * 2, 4)], "instrument {ins}");
        let (first, quiet) = (&x[882 * 12 + 2000..882 * 48 - 2000], &x[882 * 48 + 2000..882 * 72 - 2000]);
        for note in [0, 3, 7, 10] {
            assert!(amount(first, hz(note)) > 0.02, "instrument {ins}: note {note} sounds");
            // The channel's volume is every note's.
            let ratio = amount(quiet, hz(note)) / amount(first, hz(note));
            assert!((ratio - 0.25).abs() < 0.02, "instrument {ins}, note {note}: {ratio}");
        }
        assert!(amount(first, hz(5)) < 0.004, "instrument {ins}");
        // Key off lets go of them all. (A sample with no envelope is
        // silenced by it and plays on unheard, as a single note's is.)
        assert_eq!(e.active_voices(), if ins == 1 { 0 } else { 4 }, "instrument {ins}");
        assert!(x[882 * 72 + 2000..].iter().all(|v| v.abs() < 1.0e-4), "instrument {ins}");
    }
}

#[test]
fn before_version_2_a_chord_was_one_voice_and_it_still_is() {
    let two = song("", r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "maj"}"#);
    // The same text, marked as version 1.
    let one = native::load(&native::save(&two).json.replace(r#""version": 2,"#, r#""version": 1,"#), &|_| None).unwrap();
    assert!(two.patterns[0].chord(0, 0).unwrap().arp.is_none() && one.patterns[0].chord(0, 0).unwrap().arp.is_some());
    let mut e = playing(one);
    let (_, x) = strikes(&mut e, 882 * 12);
    assert_eq!(e.active_voices(), 1);
    assert_eq!(e.active_arpeggiators(), 0, "a voice steps by itself, as it always did");
    // 50 steps a second: 0, 4, 7 by turns, 882 frames each.
    assert_eq!([semitones(&x[100..800]), semitones(&x[982..1700]), semitones(&x[1864..2600]), semitones(&x[2746..3400])], [0, 4, 7, 0]);
    let mut e = playing(two);
    render(&mut e, 882 * 12, 256);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (3, 1));
}

/// The notes of the first `count` strikes, each measured over `hold` frames.
fn notes(struck: &[(usize, usize)], x: &[f32], hold: usize, count: usize) -> Vec<i32> {
    struck[..count].iter().map(|&(at, _)| semitones(&x[at + 20..at + hold - 20])).collect()
}

#[test]
fn a_struck_arp_lands_on_the_songs_rows_at_any_tempo() {
    for (tempo, speed) in [(125.0, 6), (140.0, 6), (97.0, 6), (125.0, 4), (173.0, 3)] {
        let top = format!(r#""tempo": {tempo}, "speed": {speed},"#);
        let mut e = playing(song(&top, r#"{"row": 1, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "order": "up", "octaves": 2, "gate": 0.5}}"#));
        let ticks = tick_frames((0..400).map(|_| tick_length(tempo)));
        let (struck, x) = strikes(&mut e, ticks[speed * 13]);
        // A sixteenth is a row: one note on the first frame of each row from the note's own.
        let want: Vec<(usize, usize)> = (1..13).map(|row| (ticks[speed * row], 1)).collect();
        assert_eq!(struck, want, "tempo {tempo}, speed {speed}");
        let hold = (speed as f64 * tick_length(tempo) * 0.5) as usize;
        assert_eq!(notes(&struck, &x, hold, 10), [0, 3, 7, 10, 12, 15, 19, 22, 0, 3], "tempo {tempo}, speed {speed}");
        // Each is held for half its row and then silent until the next.
        for &(at, _) in &struck[..11] {
            assert!(x[at + hold + hold / 2..at + 2 * hold - 10].iter().all(|v| v.abs() < 0.01), "tempo {tempo}, speed {speed}");
            assert!(x[at + 10..at + hold - 10].iter().any(|v| v.abs() > 0.1));
        }
    }
}

#[test]
fn every_rate_keeps_the_time_it_says() {
    // At 125 beats a minute a tick is exactly 882 frames, and at speed 6 a row is six of them.
    for (rate, ticks) in [
        (r#""rate": "1/4""#, 24.0),
        (r#""rate": "1/8""#, 12.0),
        (r#""rate": "1/32""#, 3.0),
        (r#""rate": "1/8t""#, 8.0),
        (r#""rate": "1/16t""#, 4.0),
        (r#""rate": "1/16.""#, 9.0),
        (r#""rate": "1/64""#, 1.5),
        (r#""perRow": 4"#, 1.5),
        (r#""perRow": 0.5"#, 12.0),
        (r#""perTick": 0.25, "style": "strike""#, 4.0),
        (r#""hz": 20, "style": "strike""#, 2.5),
    ] {
        let events = format!(r#"{{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {{"chord": [0, 7, 12], {rate}, "gate": 0.3}}}}"#);
        let mut e = playing(song("", &events));
        let (struck, _) = strikes(&mut e, 882 * 100);
        let want: Vec<(usize, usize)> = (0..).map(|k| ((k as f64 * ticks * 882.0) as usize, 1)).take_while(|&(at, _)| at < 882 * 100).collect();
        assert_eq!(struck, want, "{rate}");
    }
    // A tick that is not a whole number of frames: a step between two
    // ticks falls that part of the way through the tick it is in.
    let mut e = playing(song(r#""tempo": 140,"#, r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": [0, 7], "rate": "1/64", "gate": 0.3}}"#));
    let length = tick_length(140.0);
    let ticks = tick_frames((0..40).map(|_| length));
    let (struck, _) = strikes(&mut e, ticks[30]);
    let want: Vec<(usize, usize)> = (0..20).map(|k| (if k % 2 == 0 { ticks[k * 3 / 2] } else { ticks[k * 3 / 2] + (0.5 * length) as usize }, 1)).collect();
    assert_eq!(struck, want);
}

#[test]
fn an_arp_follows_the_song_when_its_tempo_or_its_speed_changes() {
    // The tempo goes from 125 to 150 on row 4, and the speed from 6 to 3 on row 8.
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/16", "gate": 0.4}},
        {"row": 0, "ch": 1, "note": "C-3", "ins": 1, "vol": 0, "arp": {"chord": [0, 12], "hz": 25, "style": "strike", "gate": 0.4}},
        {"row": 4, "ch": 1, "fx": "F96"}, {"row": 8, "ch": 1, "fx": "F03"}"#;
    let ticks = tick_frames((0..200).map(|tick| tick_length(if tick < 24 { 125.0 } else { 150.0 })));
    // Rows 0 to 7 are six ticks each; from row 8 they are three.
    let row = |row: usize| if row < 8 { ticks[6 * row] } else { ticks[48 + 3 * (row - 8)] };
    let mut e = playing(song("", events));
    e.push(Event::song(0.0, SONG_MUTE, 1.0, 1.0));
    let (struck, _) = strikes(&mut e, row(16));
    let on_rows: Vec<usize> = (0..16).map(row).collect();
    let got: Vec<usize> = struck.iter().map(|s| s.0).filter(|at| on_rows.contains(at)).collect();
    assert_eq!(got, on_rows, "a step on every row, whatever the rows' length");

    // The other channel alone: 25 steps a second, whatever the tempo. Each
    // step is 1764 frames, and the change of tempo does not move one.
    let alone = events.split_once('\n').unwrap().1;
    let mut e = playing(song("", alone));
    let (struck, _) = strikes(&mut e, row(12));
    let got: Vec<usize> = struck.iter().map(|s| s.0).collect();
    for (k, at) in got.iter().enumerate() {
        assert!((*at as i64 - 1764 * k as i64).abs() <= 2, "step {k} at {at}");
    }
    assert!(got.len() > 20);
}

#[test]
fn a_chord_change_under_a_running_arp_keeps_its_place() {
    // A minor chord for four rows, F major from row 4 (with no note: the
    // arp carries on), then a new note on row 8 starts it again.
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "min", "rate": "1/16", "gate": 0.5}},
        {"row": 4, "ch": 0, "arp": {"chord": [5, 9, 12], "rate": "1/16", "gate": 0.5}},
        {"row": 8, "ch": 0, "note": "D-4", "ins": 1, "arp": {"chord": [0, 4, 7, 11], "rate": "1/32", "order": "down", "gate": 0.5}}"#;
    let mut e = playing(song("", events));
    let (struck, x) = strikes(&mut e, 882 * 6 * 11);
    let want: Vec<usize> = (0..8).map(|row| 882 * 6 * row).chain((0..6).map(|k| 882 * 6 * 8 + 882 * 3 * k)).collect();
    assert_eq!(struck.iter().map(|s| s.0).collect::<Vec<_>>(), want);
    // Steps 0 to 3 of C minor are 0 3 7 0; steps 4 to 7 of the new chord
    // are its second, third, first and second notes.
    assert_eq!(notes(&struck, &x, 2646, 8), [0, 3, 7, 0, 9, 12, 5, 9]);
    // The new note's arp begins at its own beginning: down from the top, on D.
    assert_eq!(notes(&struck[8..], &x, 1323, 6), [13, 9, 6, 2, 13, 9]);
}

#[test]
fn an_arp_of_one_voice_can_keep_musical_time_too() {
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/16", "style": "slide", "octaves": 2}},
        {"row": 6, "ch": 0, "arp": "off"}"#;
    let mut e = playing(song("", events));
    let (struck, x) = strikes(&mut e, 882 * 6 * 9);
    assert_eq!(struck, [(0, 1)], "one note, struck once");
    let rows: Vec<i32> = (0..9).map(|row| semitones(&x[5292 * row + 30..5292 * (row + 1) - 30])).collect();
    assert_eq!(rows, [0, 4, 7, 12, 16, 19, 0, 0, 0], "the pitch jumps on each row, and goes back to the note when the arp is off");
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (1, 0));
}

#[test]
fn swing_accents_and_stabs_in_a_song() {
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": [0, 7], "rate": "1/16", "gate": 0.25, "swing": 0.5, "accent": [15, 5]}},
        {"row": 8, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "min", "rate": "1/8", "order": "together", "gate": 0.25}}"#;
    let mut e = playing(song("", events));
    let (struck, x) = strikes(&mut e, 5292 * 14);
    // Every second sixteenth is half a row late.
    let want: Vec<(usize, usize)> = (0..8).map(|k| (5292 * k + if k % 2 == 1 { 2646 } else { 0 }, 1)).chain((0..3).map(|k| (5292 * (8 + 2 * k), 3))).collect();
    assert_eq!(struck, want);
    // And a third as loud (velocity moves this patch's loudness not at all,
    // so here it is the same: the accent is the note's velocity).
    let level = |at: usize| x[at + 10..at + 600].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(level(0) > 0.1 && level(5292 + 2646) > 0.1);
    // Stabs: the three notes together on every second row.
    let stab = &x[5292 * 8 + 100..5292 * 8 + 2500];
    assert!([0, 3, 7].iter().all(|&n| amount(stab, hz(n)) > 0.02));
}

#[test]
fn key_off_an_off_and_the_songs_end_all_stop_an_arp() {
    // Key off: the patch's notes are let go and nothing more is struck.
    let mut e = playing(song("", r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/32"}}, {"row": 3, "ch": 0, "note": "off"}"#));
    let (struck, _) = strikes(&mut e, 5292 * 8);
    assert_eq!(struck.len(), 6);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));
    // "arp": "off": the step in hand finishes, and that is all.
    let mut e = playing(song("", r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/32"}}, {"row": 3, "ch": 0, "arp": "off"}"#));
    let (struck, _) = strikes(&mut e, 5292 * 8);
    assert_eq!(struck.len(), 6);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));
    // A plain note on the channel takes the arp's place.
    let mut e = playing(song("", r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/32", "gate": 1}}, {"row": 3, "ch": 0, "note": "G-4", "ins": 1}"#));
    let (_, x) = strikes(&mut e, 5292 * 8);
    assert_eq!(semitones(&x[5292 * 3 + 100..5292 * 8]), 7);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (1, 0));
    // The song's end, with repeat off: on a patch the notes are let go; on
    // a sample the note that is sounding plays out its gate.
    for ins in [1, 2] {
        let events = format!(
            r#"{{"row": 28, "ch": 0, "note": "C-4", "ins": {ins}, "arp": {{"chord": "maj", "rate": "1/16"}}}}, {{"row": 28, "ch": 1, "note": "C-3", "ins": {ins}, "chord": "min"}}"#
        );
        let mut e = playing(song("", &events));
        render(&mut e, 5292 * 32 + 44100, 256);
        assert_eq!(song_player::shared(&e).unwrap().position()[0], STATE_STOPPED);
        assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0), "instrument {ins}");
        assert!(render(&mut e, 4410, 256).0.iter().all(|v| *v == 0.0));
    }
}

#[test]
fn seeking_and_looping_start_an_arp_at_its_first_step_on_its_row() {
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/32", "gate": 0.5}},
        {"row": 4, "ch": 0, "note": "F-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/16t", "gate": 0.5}}"#;
    // Seek into the middle of the first arp, part way through a step: it
    // is not picked up (a seek starts nothing that began before it), and
    // the next begins on its row, from its first note.
    let mut e = playing(song("", events));
    render(&mut e, 5292 + 1000, 256);
    let at = e.time() as usize;
    e.push(Event::song(at as f64, SONG_SEEK, 0.0, 2.0));
    let (struck, x) = strikes(&mut e, 5292 * 5);
    assert!(x[1000..5292 * 2 - 10].iter().all(|v| v.abs() < 1.0e-3), "rows 2 and 3 are silent");
    let want: Vec<usize> = (0..5).map(|k| 5292 * 2 + 882 * 4 * k).collect();
    assert_eq!(struck.iter().map(|s| s.0).collect::<Vec<_>>(), want, "counted from the seek");
    assert_eq!(notes(&struck, &x, 1764, 4), [5, 9, 12, 5]);

    // One pattern round and round: each time round, row 0's arp starts afresh.
    let mut e = Engine::new(SR);
    for (id, value) in [(OSC1_WAVE, 0.0), (AMP_ATTACK, 0.0), (AMP_DECAY, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.0)] {
        e.set_param(LEAD, id, value);
    }
    let mut short = song("", events);
    short.patterns[0].rows = 8;
    song_player::install(&mut e, short, FIRST_PART, 0, false).unwrap();
    e.push(Event::song(0.0, SONG_PATCH_PART, 1.0, LEAD as f32));
    e.push(Event::song(0.0, SONG_LOOP_PATTERN, 1.0, 0.0));
    e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (struck, x) = strikes(&mut e, 5292 * 16);
    let round: Vec<usize> = (0..8).map(|k| 882 * 3 * k).chain((0..6).map(|k| 5292 * 4 + 882 * 4 * k)).collect();
    let want: Vec<usize> = round.iter().copied().chain(round.iter().map(|at| at + 5292 * 8)).collect();
    assert_eq!(struck.iter().map(|s| s.0).collect::<Vec<_>>(), want);
    assert_eq!(notes(&struck[14..], &x, 1323, 8), [0, 3, 7, 10, 0, 3, 7, 10]);

    // The trackers' own pattern loop (E6x) plays rows again: an arp that
    // is running runs on through it, and a note inside it starts again.
    let looped = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "gate": 0.5}},
        {"row": 2, "ch": 1, "fx": "E60"}, {"row": 3, "ch": 1, "fx": "E61"}, {"row": 5, "ch": 0, "note": "off"}"#;
    let mut e = playing(song("", looped));
    let (struck, x) = strikes(&mut e, 5292 * 9);
    assert_eq!(struck.iter().map(|s| s.0).collect::<Vec<_>>(), (0..7).map(|k| 5292 * k).collect::<Vec<_>>());
    assert_eq!(notes(&struck, &x, 2646, 7), [0, 3, 7, 10, 0, 3, 7]);
}

#[test]
fn pausing_holds_an_arp_where_it_is() {
    let mut e = playing(song("", r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "gate": 0.5}}"#));
    let (a, _) = strikes(&mut e, 5292 * 2 + 3000);
    e.push(Event::song(e.time() as f64, SONG_PAUSE, 0.0, 0.0));
    let (b, x) = strikes(&mut e, 20000);
    assert!(b.is_empty() && x[100..].iter().all(|v| v.abs() < 1.0e-3), "nothing is struck and nothing heard while paused");
    e.push(Event::song(e.time() as f64, SONG_PLAY, 0.0, 0.0));
    let (c, x) = strikes(&mut e, 5292 * 3);
    assert_eq!(a.len(), 3);
    // It goes on with its fourth note, then its first.
    assert_eq!(notes(&c, &x, 2646, 2), [10, 0]);
    assert_eq!(c[1].0 - c[0].0, 5292);
}

#[test]
fn an_edited_song_takes_the_place_of_the_playing_one_without_the_arp_missing_a_step() {
    let before = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "gate": 0.5}}"#;
    // The edit: another chord on row 0, and a change of chord on row 6.
    let after = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "rate": "1/16", "gate": 0.5}},
        {"row": 6, "ch": 0, "arp": {"chord": [5, 9], "rate": "1/16", "gate": 0.5}}"#;
    let mut e = playing(song("", before));
    e.push(Event::song(0.0, SONG_REPEAT, 1.0, 0.0));
    let (mut struck, mut x) = strikes(&mut e, 5292 * 3 + 1234);
    assert_eq!(song_player::replace(&mut e, song("", after), 0, false), Ok(true));
    let (more, y) = strikes(&mut e, 5292 * 36 - 1234);
    struck.extend(more.iter().map(|&(at, n)| (at + x.len(), n)));
    x.extend(y);
    // A step on every row, straight through the change.
    assert_eq!(struck.iter().map(|s| s.0).collect::<Vec<_>>(), (0..39).map(|k| 5292 * k).collect::<Vec<_>>());
    // Rows 0 to 5 are the arp as it began (the old row 0 is not read
    // again); row 6 is read from the new song, and keeps the place; the
    // second time round, row 0 is the new chord.
    assert_eq!(notes(&struck, &x, 2646, 9), [0, 3, 7, 10, 0, 3, 5, 9, 5]);
    assert_eq!(notes(&struck[32..], &x, 2646, 7), [0, 4, 7, 0, 4, 7, 5]);
    assert_eq!(song_player::shared(&e).unwrap().position()[0], STATE_PLAYING);
}

/// A song with a block chord, a struck arp, a chip arp of the old kind, a
/// tempo change, a sample and a patch, and the tracker's own arpeggio effect.
fn everything() -> Song {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/songs/chords-and-arps.json")).unwrap();
    native::load(&text, &|_| None).unwrap()
}

#[test]
fn the_example_song_plays_the_same_in_blocks_of_any_size_and_survives_saving() {
    let song = everything();
    let uses = |f: &dyn Fn(&synths_core::song::Chord) -> bool| song.patterns.iter().flat_map(|p| &p.events).filter_map(|e| e.chord).any(|c| f(&c));
    assert!(uses(&|c| c.arp.is_none() && !c.is_off()), "a chord");
    assert!(uses(&|c| c.arp.is_some_and(|a| !a.plain())), "a struck arp");
    assert!(uses(&|c| c.arp.is_some_and(|a| a.plain())), "a chip arp");
    assert_eq!(native::load(&native::save(&song).json, &|_| None).unwrap(), song);
    assert_eq!(binary::load(&binary::save(&song)).unwrap(), song);

    let run = |block: usize| {
        let mut e = Engine::new(SR);
        for (part, wave) in [(LEAD, 2.0), (LEAD + 1, 3.0)] {
            for (id, value) in [(OSC1_WAVE, wave), (AMP_ATTACK, 0.002), (AMP_DECAY, 0.2), (AMP_SUSTAIN, 0.4), (AMP_RELEASE, 0.2), (CUTOFF, 2500.0), (FILTER_TYPE, 1.0)] {
                e.set_param(part, id, value);
            }
        }
        song_player::install(&mut e, song.clone(), FIRST_PART, 0, false).unwrap();
        for (i, ins) in song.instruments.iter().enumerate() {
            if ins.patch.is_some() {
                e.push(Event::song(0.0, SONG_PATCH_PART, i as f32 + 1.0, (LEAD + (i % 2) as u16) as f32));
            }
        }
        e.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
        e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        let mut most = 0;
        let (mut l, mut r) = (vec![0.0f32; 44100 * 12], vec![0.0f32; 44100 * 12]);
        for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
            e.render(a, b);
            most = most.max(e.active_voices());
        }
        assert_eq!(song_player::shared(&e).unwrap().position()[0], STATE_STOPPED);
        assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0), "nothing is left sounding at the end");
        (l, r, most)
    };
    let (l, r, most) = run(128);
    assert!(l.iter().any(|v| v.abs() > 0.05) && most >= 6 && most <= 24, "{most}");
    for block in [1, 64, 333, 4096] {
        let (bl, br, _) = run(block);
        assert!(l.iter().zip(&bl).all(|(a, b)| a.to_bits() == b.to_bits()) && r.iter().zip(&br).all(|(a, b)| a.to_bits() == b.to_bits()), "block {block}");
    }
}
