//! Arpeggio patterns: an arpeggio that follows a list of steps (which note
//! of the chord, a rest, a tie, an octave, an accent, a gate, a slide) in
//! place of one of the fixed orders.

mod common;
use common::*;
use synths_core::arp::*;
use synths_core::event::ALL_PARTS;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::pattern_abi::*;
use synths_core::{Engine, Event};

fn sine_engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.set_param(ALL_PARTS, OSC1_WAVE, 0.0);
    e.set_param(ALL_PARTS, AMP_RELEASE, 0.02);
    e
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x.to_bits() != y.to_bits())
}

/// Start a note with an arpeggio that follows pattern 1, four steps to a beat.
fn start(e: &mut Engine, pattern: &str, chord: &[i8], gate: f32, octaves: u8, tempo: f32) {
    e.set_arp_pattern(1, &Pattern::parse(pattern).unwrap()).unwrap();
    e.set_param(0, TEMPO, tempo);
    e.push(Event::arp_notes(0.0, 0, 0, 0.0, ORDER_UP, chord));
    e.push(Event::arp_style(0.0, 0, 0, gate, octaves, 0.0, 0));
    e.push(Event::arp_beat(0.0, 0, 0, 4.0));
    e.push(Event::arp_pattern(0.0, 0, 0, 1));
    e.push(Event::note_on(0.0, 0, 3, 60.0, 0.9));
}

#[test]
fn a_pattern_is_played_as_the_notes_it_says_to_the_frame() {
    // Rests, a tie, an octave up, accents and a step with a gate of its own,
    // over C minor seventh, at three tempos. By hand: the same notes.
    for tempo in [120.0f32, 138.0, 97.0] {
        let step = SR as f64 * 60.0 / (tempo as f64 * 4.0);
        let frame = |k: f64| (k * step) as u64 as f64;
        let mut e = sine_engine();
        start(&mut e, "1^ - 3 2 _ 1' 4^:2 -", &[0, 3, 7, 10], 0.5, 1, tempo);
        let frames = (step * 16.0) as usize;
        let got = render(&mut e, frames, 128).0;

        let mut by_hand = sine_engine();
        let (soft, loud) = (0.9 * 10.0 / 15.0, 0.9);
        for round in [0.0, 8.0] {
            // (step, note, velocity, how long it is held in steps)
            for (k, note, velocity, held) in [(0.0, 60.0, loud, 0.5), (2.0, 67.0, soft, 0.5), (3.0, 63.0, soft, 1.5), (5.0, 72.0, soft, 0.5), (6.0, 70.0, loud, 0.2)] {
                let at = frame(round + k);
                by_hand.push(Event::note_on(at, 0, 3, note, velocity));
                // A tied note is let go half way through the step it is tied over.
                let off = if held > 1.0 { frame(round + k + 1.0) + (0.5 * step) as u64 as f64 } else { at + (held * step) as u64 as f64 };
                by_hand.push(Event::note_off(off, 0, 3, 0.0));
            }
        }
        let want = render(&mut by_hand, frames, 128).0;
        assert_eq!(first_difference(&got, &want), None, "tempo {tempo}");
        assert!(rms(&got) > 0.1);
    }
}

/// The semitones above C heard in each step of 6000 frames.
fn heard(x: &[f32], count: usize) -> Vec<i32> {
    (0..count)
        .map(|n| {
            let part = &x[n * 6000 + 300..(n + 1) * 6000 - 300];
            if rms(part) < 0.01 {
                return -99;
            }
            (12.0 * (frequency(part, SR) / hz(60.0)).log2()).round() as i32
        })
        .collect()
}

#[test]
fn notes_are_counted_up_the_chord_and_on_into_the_octaves() {
    let run = |pattern: &str, chord: &[i8], octaves: u8, count: usize| {
        let mut e = sine_engine();
        e.set_param(0, AMP_RELEASE, 0.0);
        start(&mut e, pattern, chord, 1.0, octaves, 120.0);
        heard(&render(&mut e, 6000 * count, 128).0, count)
    };
    // Past the chord's last note the count carries on an octave up; the
    // pattern is its own length, whatever the chord's.
    assert_eq!(run("1 2 3 4 5 6 7", &[0, 4, 7], 1, 8), [0, 4, 7, 12, 16, 19, 24, 0]);
    assert_eq!(run("1 2 3 4 5 6 7", &[7, 0], 1, 7), [0, 7, 12, 19, 24, 31, 36]);
    // Octave marks, and a rest.
    assert_eq!(run("1 1' 1'' 3, - 2", &[0, 4, 7], 1, 6), [0, 12, 24, -5, -99, 4]);
    // With octaves, each time round is an octave higher.
    assert_eq!(run("1 3", &[0, 4, 7], 3, 8), [0, 7, 12, 19, 24, 31, 0, 7]);
    // The named patterns are patterns.
    assert_eq!(run("bounce", &[0, 4, 7], 1, 5), [0, 4, 7, 4, 0]);
    assert_eq!(run("root-fifth", &[0, 4, 7], 1, 4), [0, 7, 12, 7]);
    for (name, text) in NAMED_PATTERNS {
        assert_eq!(Pattern::parse(name), Pattern::parse(text), "{name}");
    }
    // Steps given as numbers can count down below the chord too.
    let below = Pattern::new(&[PatternStep { note: -1, ..Default::default() }, PatternStep { note: -3, ..Default::default() }, PatternStep { note: 1, octave: -1, ..Default::default() }]);
    let mut e = sine_engine();
    start(&mut e, "1", &[0, 4, 7], 1.0, 1, 120.0);
    e.set_arp_pattern(1, &below).unwrap();
    assert_eq!(heard(&render(&mut e, 18000, 128).0, 3), [-5, -12, -12]);
}

#[test]
fn a_slide_moves_the_sounding_note_and_a_change_of_chord_keeps_the_place() {
    // 303-style: the second step is not struck; the first note slides to it.
    let mut e = sine_engine();
    e.set_param(0, AMP_ATTACK, 0.2);
    e.set_param(0, AMP_CURVE, 0.0);
    start(&mut e, "1 ~3:9 2 -", &[0, 4, 7], 0.5, 1, 120.0);
    let x = render(&mut e, 24000, 128).0;
    assert_eq!(e.active_arpeggiators(), 1);
    let note = |from: usize, to: usize| 12.0 * (frequency(&x[from..to], SR) / hz(60.0)).log2();
    assert!(note(1000, 5900).abs() < 0.02);
    let part_way = note(6000 + 1200, 6000 + 1700);
    assert!(part_way > 2.0 && part_way < 5.5, "sliding: {part_way}");
    assert!((note(6000 + 3100, 11300) - 7.0).abs() < 0.05, "arrived within 60 ms");
    // One attack for the two: the level keeps rising across the slide.
    assert!(peak(&x[6000..6600]) > 0.55 && peak(&x[12400..12600]) < 0.1, "then the third step is struck afresh");

    // A new chord under a running pattern: the next step is the pattern's
    // next step, in the new chord.
    let mut e = sine_engine();
    start(&mut e, "1 2 3 2", &[0, 4, 7], 1.0, 1, 120.0);
    e.push(Event::arp_notes(9000.0, 0, 3, 0.0, ORDER_UP, &[2, 5, 9]));
    assert_eq!(heard(&render(&mut e, 36000, 128).0, 6), [0, 4, 9, 5, 2, 5]);
    // And a new pattern under a running arpeggio, or none.
    let mut e = sine_engine();
    start(&mut e, "1 2 3 2", &[0, 4, 7], 1.0, 1, 120.0);
    let text = "3 3 1";
    assert_eq!(unsafe { synths_arp_pattern_text(&mut *e, 2, text.as_ptr(), text.len() as u32) }, 3);
    e.push(Event::arp_pattern(9000.0, 0, 3, 2));
    e.push(Event::arp_pattern(27000.0, 0, 3, 0));
    // Steps 2 to 4 are the new pattern's third, first and second; then the plain order, up.
    assert_eq!(heard(&render(&mut e, 48000, 128).0, 8), [0, 4, 0, 7, 7, 7, 0, 4]);
}

#[test]
fn one_voice_follows_a_pattern_too() {
    // The other style: one note whose pitch jumps. A rest and a tie leave it where it is.
    let mut e = sine_engine();
    start(&mut e, "1 3 - 2 _ 1'", &[0, 4, 7], 0.0, 1, 120.0);
    let x = render(&mut e, 36000, 128).0;
    assert_eq!(heard(&x, 6), [0, 7, 7, 4, 4, 12]);
    assert_eq!(e.active_voices(), 1);
}

#[test]
fn pattern_text_reads_back_and_bad_text_is_refused_in_words() {
    for text in ["1 - 3 2 _ 1 3 5^", "1", "~3 1'' 2,,^:4 - _ 32", "1^ 1^", "1:9 2:1"] {
        let p = Pattern::parse(text).unwrap();
        assert_eq!(p.to_text().as_deref(), Some(text), "{text}");
        assert_eq!(Pattern::parse(&p.to_text().unwrap()), Ok(p));
        // And as bytes, four a step.
        let bytes: Vec<u8> = p.steps().iter().flat_map(|s| [s.note as u8, s.flags, s.octave as u8, s.shape]).collect();
        assert_eq!(Pattern::from_bytes(&bytes), p);
    }
    assert_eq!(Pattern::parse("1 . 3").unwrap(), Pattern::parse("1 - 3").unwrap());
    let p = Pattern::parse("1^ 2 - _").unwrap();
    assert_eq!(p.steps().iter().map(|s| s.shape & 15).collect::<Vec<_>>(), [15, PLAIN_LEVEL, 0, 0]);
    assert!(p.steps[2].is_rest() && p.steps[3].carries() && !p.steps[3].is_rest());
    for bad in ["", "   ", "0", "33", "x", "1 two", "1^^:", "1:0", "1:55", "-1", "1''''", "~", "~-", "banana", &"1 ".repeat(33)] {
        assert_eq!(Pattern::parse(bad), Err(PATTERN_HELP), "{bad:?}");
    }
    assert_eq!(Pattern::parse(&"1 ".repeat(32)).unwrap().len, 32);
    // Through the C interface: bad text and bad places are refused, and nothing changes.
    let mut e = Engine::new(SR);
    unsafe {
        assert_eq!(synths_arp_pattern_text(&mut *e, 1, "1 2".as_ptr(), 3), 2);
        assert_eq!(synths_arp_pattern_text(&mut *e, 1, "what".as_ptr(), 4), -2);
        assert_eq!(synths_arp_pattern_text(&mut *e, 0, "1".as_ptr(), 1), -1);
        assert_eq!(synths_arp_pattern_text(&mut *e, 65, "1".as_ptr(), 1), -1);
        assert_eq!(e.arp_pattern(1).unwrap().len, 2);
        let steps = [3u8, 0, 0, 0, 0, STEP_TIE, 0, 0];
        assert_eq!(synths_arp_pattern_set(&mut *e, 64, steps.as_ptr(), 2), 0);
        assert_eq!(e.arp_pattern(64).unwrap().to_text().as_deref(), Some("3 _"));
        assert_eq!(synths_arp_pattern_set(&mut *e, 64, steps.as_ptr(), 33), -2);
        assert_eq!(synths_arp_pattern_set(core::ptr::null_mut(), 1, steps.as_ptr(), 2), -4);
    }
    // Any text at all: an answer, never a panic.
    let mut rng = Rng::new(5);
    let letters = b"123 -_~^:',.x9 0\t4";
    for _ in 0..20000 {
        let text: String = (0..rng.next_u32() % 24).map(|_| letters[rng.next_u32() as usize % letters.len()] as char).collect();
        if let Ok(p) = Pattern::parse(&text) {
            assert!(p.len >= 1 && p.steps().iter().all(|s| (0..=32).contains(&s.note) && s.octave.abs() <= 3));
        }
    }
}

#[test]
fn any_pattern_at_all_leaves_the_engine_sound() {
    let mut rng = Rng::new(9);
    let mut e = Engine::new(SR);
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    for round in 0..600 {
        let bytes: Vec<u8> = (0..4 * (rng.next_u32() % 33)).map(|_| if rng.next_u32() % 3 == 0 { 0 } else { rng.next_u32() as u8 }).collect();
        e.set_arp_pattern(1 + (rng.next_u32() % 4) as usize, &Pattern::from_bytes(&bytes)).unwrap();
        let now = e.time() as f64;
        let handle = 1 + rng.next_u32() % 3;
        if round % 5 == 0 {
            e.push(Event::arp_notes(now, 0, 0, [-1.0, 30.0, 200.0][rng.next_u32() as usize % 3], 0, &[0, 3, 7, 10][..1 + (rng.next_u32() % 4) as usize]));
            e.push(Event::arp_style(now, 0, 0, [0.0, 0.3, 1.0][rng.next_u32() as usize % 3], 1 + (rng.next_u32() % 4) as u8, 0.2, 0));
            e.push(Event::arp_pattern(now, 0, 0, rng.next_u32() % 6));
            e.push(Event::note_on(now, 0, handle, 50.0, 0.5));
        }
        e.push(Event::arp_pattern(now + 100.0, 0, handle, rng.next_u32() % 6));
        e.push(Event::arp_step(now + 200.0, 0, handle, 300.0));
        if round % 7 == 0 {
            e.push(Event::note_off(now + 300.0, 0, handle, 0.0));
        }
        e.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0));
        assert!(e.active_voices() <= 64);
    }
    e.push(Event::all_notes_off(0.0, ALL_PARTS, true));
    e.render(&mut l, &mut r);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));
}
