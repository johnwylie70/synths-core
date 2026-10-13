//! Arpeggiators: arpeggios whose notes are struck afresh at each step, kept
//! in time with a tempo, run through octaves, swung and accented, and
//! chords whose notes sound together. Most of these tests say what such an
//! arpeggio is by playing the same notes by hand, as a player at a keyboard
//! would, and asking that the two sound exactly alike, sample for sample.

mod common;
use common::*;
use synths_core::arp::*;
use synths_core::event::ALL_PARTS;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::sequencer::{Sequencer, Voices};
use synths_core::{Engine, Event};

/// A sine on part 0 with a short release, so that the notes of one step die
/// away under the next.
fn sine_engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.set_param(ALL_PARTS, OSC1_WAVE, 0.0);
    e.set_param(ALL_PARTS, AMP_RELEASE, 0.03);
    e
}

/// What to ask of an arpeggio.
#[derive(Clone)]
struct Ask {
    offsets: Vec<i8>,
    order: u8,
    /// Steps a second, or with `beat` above 0 steps to a beat.
    rate: f32,
    beat: f32,
    gate: f32,
    octaves: u8,
    swing: f32,
    levels: u32,
}

impl Ask {
    fn new(offsets: &[i8]) -> Ask {
        Ask { offsets: offsets.to_vec(), order: ORDER_UP, rate: 50.0, beat: 0.0, gate: 0.5, octaves: 1, swing: 0.0, levels: 0 }
    }

    /// The events that give the arpeggio to a handle (or, with 0, to the part's next note).
    fn events(&self, time: f64, part: u16, handle: u32) -> Vec<Event> {
        let mut out = vec![Event::arp_notes(time, part, handle, self.rate, self.order, &self.offsets)];
        if self.offsets.len() > 4 {
            out.push(Event::arp_more(time, part, handle, &self.offsets[4..]));
        }
        out.push(Event::arp_style(time, part, handle, self.gate, self.octaves, self.swing, self.levels));
        if self.beat > 0.0 {
            out.push(Event::arp_beat(time, part, handle, self.beat));
        }
        out
    }

    /// Start a note with this arpeggio.
    fn start(&self, e: &mut Engine, time: f64, part: u16, handle: u32, note: f32, velocity: f32) {
        for ev in self.events(time, part, 0) {
            assert!(e.push(ev));
        }
        assert!(e.push(Event::note_on(time, part, handle, note, velocity)));
    }
}

/// The semitones above `note` heard in each window of `step` frames.
fn heard(x: &[f32], note: f32, step: usize, count: usize) -> Vec<i32> {
    (0..count)
        .map(|n| {
            let f = frequency(&x[n * step + 40..(n + 1) * step - 10], SR);
            (12.0 * (f / hz(note)).log2()).round() as i32
        })
        .collect()
}

fn first_difference(a: &[f32], b: &[f32]) -> Option<usize> {
    a.iter().zip(b.iter()).position(|(x, y)| x.to_bits() != y.to_bits())
}

/// The frame of each of the first `count` steps of a clock that starts at
/// `start` and whose steps are `period` frames long.
fn step_frames(start: u64, period: f64, count: usize) -> Vec<u64> {
    let mut due = start as f64;
    (0..count)
        .map(|_| {
            let frame = due as u64;
            due += period;
            frame
        })
        .collect()
}

#[test]
fn a_struck_arpeggio_is_the_notes_a_player_would_strike_to_the_frame() {
    // Sixteenths (four steps to a beat) at several tempos, each note held
    // for half its step.
    for tempo in [120.0f32, 125.0, 140.0, 97.3, 174.0] {
        let period = SR as f64 * 60.0 / (tempo as f64 * 4.0);
        for start in [0u64, 777] {
            let frames = start as usize + (period * 9.5) as usize;
            let chord = [0i8, 3, 7];
            let mut e = sine_engine();
            e.set_param(0, TEMPO, tempo);
            let ask = Ask { beat: 4.0, ..Ask::new(&chord) };
            ask.start(&mut e, start as f64, 0, 7, 60.0, 0.8);
            let got = render(&mut e, frames, 128).0;

            let mut by_hand = sine_engine();
            for (k, &at) in step_frames(start, period, 10).iter().enumerate() {
                by_hand.push(Event::note_on(at as f64, 0, 7, 60.0 + chord[k % 3] as f32, 0.8));
                by_hand.push(Event::note_off((at + (0.5 * period) as u64) as f64, 0, 7, 0.0));
            }
            let want = render(&mut by_hand, frames, 128).0;
            assert_eq!(first_difference(&got, &want), None, "tempo {tempo}, start {start}");
            assert!(rms(&got[start as usize..]) > 0.1);
        }
    }
    // At 120 beats a minute and 48 kHz a sixteenth is exactly 6000 frames.
    assert_eq!(step_frames(0, SR as f64 * 60.0 / (120.0 * 4.0), 4), [0, 6000, 12000, 18000]);
}

#[test]
fn each_step_starts_its_envelopes_again() {
    // A slow attack: every step swells from nothing.
    let mut e = sine_engine();
    e.set_param(0, AMP_ATTACK, 0.05);
    e.set_param(0, AMP_CURVE, 0.0);
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { rate: 10.0, gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    let x = render(&mut e, 4800 * 4, 128).0;
    for step in 0..4 {
        let at = step * 4800;
        assert!(peak(&x[at + 20..at + 200]) < 0.12, "step {step} starts quietly");
        assert!(peak(&x[at + 2600..at + 4700]) > 0.95, "step {step} reaches full");
    }
    // The other style does not: one note, one attack, and the pitch jumps.
    let mut e = sine_engine();
    e.set_param(0, AMP_ATTACK, 0.05);
    e.set_param(0, AMP_CURVE, 0.0);
    Ask { rate: 10.0, gate: 0.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    let x = render(&mut e, 4800 * 4, 128).0;
    assert!(peak(&x[4800 + 20..4800 + 200]) > 0.95);
    assert_eq!(heard(&x, 60.0, 4800, 4), [0, 4, 7, 0]);
    assert_eq!(e.active_voices(), 1);
}

/// The notes of the first `count` steps: struck (`gate` 1) or as one voice (`gate` 0).
fn steps_of(ask: &Ask, count: usize) -> Vec<i32> {
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { rate: 50.0, ..ask.clone() }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    heard(&render(&mut e, 960 * count, 128).0, 60.0, 960, count)
}

#[test]
fn the_orders_and_the_octaves() {
    for gate in [1.0, 0.0] {
        let run = |order: u8, octaves: u8, offsets: &[i8], count: usize| steps_of(&Ask { order, octaves, gate, ..Ask::new(offsets) }, count);
        assert_eq!(run(ORDER_UP, 1, &[7, 0, 4], 5), [0, 4, 7, 0, 4]);
        assert_eq!(run(ORDER_UP, 2, &[7, 0, 4], 8), [0, 4, 7, 12, 16, 19, 0, 4]);
        assert_eq!(run(ORDER_UP, 4, &[0, 7], 9), [0, 7, 12, 19, 24, 31, 36, 43, 0]);
        assert_eq!(run(ORDER_DOWN, 1, &[7, 0, 4], 5), [7, 4, 0, 7, 4]);
        assert_eq!(run(ORDER_DOWN, 2, &[0, 4, 7], 8), [19, 16, 12, 7, 4, 0, 19, 16]);
        assert_eq!(run(ORDER_UP_DOWN, 1, &[0, 4, 7], 6), [0, 4, 7, 4, 0, 4]);
        assert_eq!(run(ORDER_UP_DOWN, 2, &[0, 7], 8), [0, 7, 12, 19, 12, 7, 0, 7]);
        assert_eq!(run(ORDER_UP_DOWN_ENDS, 1, &[0, 4, 7], 8), [0, 4, 7, 7, 4, 0, 0, 4]);
        assert_eq!(run(ORDER_UP_DOWN, 1, &[5], 3), [5, 5, 5]);
        // As given: the order written, each octave in that order too.
        assert_eq!(run(ORDER_AS_GIVEN, 2, &[7, 0, 4], 7), [7, 0, 4, 19, 12, 16, 7]);
        // Eight notes, some below the note.
        assert_eq!(run(ORDER_UP, 1, &[0, 3, 7, 10, 12, 15, -5, -12], 9), [-12, -5, 0, 3, 7, 10, 12, 15, -12]);
        // Random: only the chord's notes, in both octaves, the same every time.
        let a = run(ORDER_RANDOM, 2, &[0, 4, 7], 40);
        assert!(a.iter().all(|n| [0, 4, 7, 12, 16, 19].contains(n)), "{a:?}");
        assert!([0, 4, 7, 12, 16, 19].iter().all(|n| a.contains(n)), "{a:?}");
        assert_eq!(a, run(ORDER_RANDOM, 2, &[0, 4, 7], 40));
    }
}

#[test]
fn all_together_is_a_stab_at_every_step() {
    let chord = [0i8, 4, 7];
    let mut e = sine_engine();
    Ask { order: ORDER_TOGETHER, rate: 10.0, gate: 0.5, octaves: 2, ..Ask::new(&chord) }.start(&mut e, 0.0, 0, 3, 60.0, 0.6);
    let got = render(&mut e, 4800 * 4, 128).0;
    // The same by hand: three notes at once, an octave up at every second step.
    let mut by_hand = sine_engine();
    for k in 0..4u64 {
        for o in chord {
            by_hand.push(Event::note_on((k * 4800) as f64, 0, 3, 60.0 + o as f32 + 12.0 * (k % 2) as f32, 0.6));
        }
        by_hand.push(Event::note_off((k * 4800 + 2400) as f64, 0, 3, 0.0));
    }
    assert_eq!(first_difference(&got, &render(&mut by_hand, 4800 * 4, 128).0), None);
    for o in chord {
        assert!(amplitude_at(&got[200..2200], hz(60.0 + o as f32), SR) > 0.3);
    }
}

#[test]
fn the_gate_is_how_much_of_a_step_a_note_is_held() {
    for gate in [0.05f32, 0.25, 0.8, 1.0] {
        let mut e = sine_engine();
        e.set_param(0, AMP_RELEASE, 0.0);
        Ask { rate: 10.0, gate, ..Ask::new(&[0, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
        let x = render(&mut e, 4800 * 3, 128).0;
        for step in 0..3 {
            let at = step * 4800;
            let open = (gate * 4800.0) as usize;
            assert!(rms(&x[at + 16..at + open - 16]) > 0.6, "gate {gate}, step {step}: sounding");
            if open + 64 < 4800 {
                assert_eq!(peak(&x[at + open + 64..at + 4800]), 0.0, "gate {gate}, step {step}: silent after it");
            }
        }
    }
    // A gate out of range is brought into it; none at all is a pitch that jumps.
    let mut spec = ArpSpec::NONE;
    spec.set_style(0.001, 9.0, 3.0, 0);
    assert_eq!((spec.gate, spec.octaves, spec.swing), (MIN_GATE, MAX_OCTAVES, MAX_SWING));
    spec.set_style(f32::NAN, f32::NAN, f32::NAN, 0);
    assert_eq!((spec.gate, spec.octaves, spec.swing), (0.0, 1, 0.0));
}

#[test]
fn a_change_of_tempo_is_taken_up_at_the_next_step() {
    let (slow, fast) = (100.0f64, 150.0f64);
    let (long, short) = (SR as f64 * 60.0 / (slow * 4.0), SR as f64 * 60.0 / (fast * 4.0));
    // The tempo changes in the middle of step 2: that step keeps its
    // length, and every step after it has the new one.
    let change = (2.5 * long) as u64;
    let mut e = sine_engine();
    e.set_param(0, TEMPO, slow as f32);
    Ask { beat: 4.0, ..Ask::new(&[0, 3, 7, 10]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    e.push(Event::param(change as f64, 0, TEMPO, fast as f32, 0.0));
    let frames = (3.0 * long + 4.5 * short) as usize;
    let got = render(&mut e, frames, 128).0;

    let mut by_hand = sine_engine();
    let mut due = 0.0f64;
    for k in 0..8 {
        let length = if k < 3 { long } else { short };
        by_hand.push(Event::note_on(due as u64 as f64, 0, 1, 60.0 + [0.0, 3.0, 7.0, 10.0][k % 4], 1.0));
        by_hand.push(Event::note_off((due as u64 + (0.5 * length) as u64) as f64, 0, 1, 0.0));
        due += length;
    }
    assert_eq!(first_difference(&got, &render(&mut by_hand, frames, 128).0), None);
}

#[test]
fn a_change_of_chord_rate_or_style_comes_at_the_next_step_and_keeps_the_place() {
    // Up through C major; half way through the second step it becomes a
    // minor seventh. The third step is the new chord's third note.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 9, 60.0, 1.0);
    e.push(Event::arp_notes(1400.0, 0, 9, 0.0, ORDER_UP, &[0, 3, 7, 10]));
    let x = render(&mut e, 960 * 7, 128).0;
    assert_eq!(heard(&x, 60.0, 960, 7), [0, 4, 7, 10, 0, 3, 7]);

    // The order and the octaves change the same way.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 9, 60.0, 1.0);
    e.push(Event::arp_style(1400.0, 0, 9, 1.0, 2, 0.0, 0));
    e.push(Event::arp_notes(4000.0, 0, 9, 0.0, ORDER_DOWN, &[0, 4, 7]));
    let x = render(&mut e, 960 * 8, 128).0;
    // Up: 0 4, then with two octaves 7 12 16; then down, which is 19 16 12 7 4 0
    // and round again: the sixth step is its sixth note.
    assert_eq!(heard(&x, 60.0, 960, 8), [0, 4, 7, 12, 16, 0, 19, 16]);

    // The rate: the step in hand finishes at the old one.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 9, 60.0, 1.0);
    e.push(Event::arp_notes(1400.0, 0, 9, 100.0, ORDER_UP, &[0, 4, 7]));
    let x = render(&mut e, 1920 + 480 * 6, 128).0;
    assert_eq!(heard(&x, 60.0, 960, 2), [0, 4]);
    assert_eq!(heard(&x[1920..], 60.0, 480, 6), [7, 0, 4, 7, 0, 4]);

    // From notes struck to a pitch that jumps, and back.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 9, 60.0, 1.0);
    e.push(Event::arp_style(1400.0, 0, 9, 0.0, 1, 0.0, 0));
    e.push(Event::arp_style(4300.0, 0, 9, 0.5, 1, 0.0, 0));
    let x = render(&mut e, 960 * 7, 128).0;
    assert_eq!(heard(&x[..960 * 5], 60.0, 960, 5), [0, 4, 7, 0, 4]);
    assert_eq!(e.active_voices(), 0, "half way through a step with a gate of a half and no release");
    assert!(peak(&x[960 * 5 + 500..960 * 6]) == 0.0 && rms(&x[960 * 5..960 * 5 + 400]) > 0.5);
}

#[test]
fn swing_puts_every_second_step_back() {
    let swing = 0.25f32;
    let period = 4800.0f64;
    let mut e = sine_engine();
    Ask { rate: 10.0, gate: 0.5, swing, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    let got = render(&mut e, 4800 * 6, 128).0;
    let mut by_hand = sine_engine();
    for k in 0..6u64 {
        // An odd step is a quarter of a step late and so a quarter shorter;
        // the even step before it is that much longer.
        let (late, length) = if k % 2 == 1 { (swing as f64 * period, period * (1.0 - swing as f64)) } else { (0.0, period * (1.0 + swing as f64)) };
        let at = k * 4800 + late as u64;
        by_hand.push(Event::note_on(at as f64, 0, 1, 60.0 + [0.0, 4.0, 7.0][k as usize % 3], 1.0));
        by_hand.push(Event::note_off((at + (0.5 * length) as u64) as f64, 0, 1, 0.0));
    }
    assert_eq!(first_difference(&got, &render(&mut by_hand, 4800 * 6, 128).0), None);
}

#[test]
fn levels_accent_some_steps_and_rest_on_others() {
    // Full, about half, a rest, two thirds: a pattern of four over a chord of three.
    let levels = 15 | 8 << 4 | 0 << 8 | 10 << 12;
    let mut e = sine_engine();
    Ask { rate: 10.0, gate: 0.5, levels, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 0.9);
    let got = render(&mut e, 4800 * 8, 128).0;
    let mut by_hand = sine_engine();
    for k in 0..8u64 {
        let level = [15.0f32, 8.0, 0.0, 10.0][k as usize % 4] / 15.0;
        if level > 0.0 {
            by_hand.push(Event::note_on((k * 4800) as f64, 0, 1, 60.0 + [0.0, 4.0, 7.0][k as usize % 3], 0.9 * level));
            by_hand.push(Event::note_off((k * 4800 + 2400) as f64, 0, 1, 0.0));
        }
    }
    assert_eq!(first_difference(&got, &render(&mut by_hand, 4800 * 8, 128).0), None);
    assert_eq!(peak(&got[4800 * 2 + 200..4800 * 3]), 0.0, "the rest");
    let spec = |levels: u32| ArpSpec { levels, ..ArpSpec::NONE };
    assert_eq!(spec(0).level(5), 1.0);
    assert_eq!((0..5).map(|k| spec(0xF03).level(k)).collect::<Vec<_>>(), [0.2, 0.0, 1.0, 0.2, 0.0]);
}

#[test]
fn a_chord_is_its_notes_on_one_frame_under_one_handle() {
    let chord = [0i8, 3, 7, 10];
    let mut e = sine_engine();
    e.push(Event::arp(500.0, 0, 0, -1.0, ORDER_TOGETHER, &chord));
    e.push(Event::note_on(500.0, 0, 4, 57.0, 0.7));
    e.push(Event::note_off(9000.0, 0, 4, 0.0));
    let mut by_hand = sine_engine();
    for o in chord {
        by_hand.push(Event::note_on(500.0, 0, 4, 57.0 + o as f32, 0.7));
    }
    by_hand.push(Event::note_off(9000.0, 0, 4, 0.0));
    let (got, want) = (render(&mut e, 4000, 128).0, render(&mut by_hand, 4000, 128).0);
    assert_eq!(first_difference(&got, &want), None);
    assert_eq!(got[..500].iter().position(|&x| x != 0.0), None);
    assert_eq!(e.active_voices(), 4, "a voice for each note");
    assert_eq!(e.active_arpeggiators(), 1);
    // The notes are held: nothing steps, however long it goes on.
    let (got, want) = (render(&mut e, 12000, 128).0, render(&mut by_hand, 12000, 128).0);
    assert_eq!(first_difference(&got, &want), None, "and they are let go together");
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));

    // The handle is the chord: one event moves, turns down or cuts all of it.
    let mut e = sine_engine();
    e.push(Event::arp(0.0, 0, 0, -1.0, ORDER_TOGETHER, &[0, 4, 7]));
    e.push(Event::note_on(0.0, 0, 4, 60.0, 0.25));
    e.push(Event::voice_pitch(4800.0, 0, 4, 62.0, 0.0));
    e.push(Event::voice_volume(9600.0, 0, 4, 0.5));
    e.push(Event::voice_kill(14400.0, 0, 4));
    let x = render(&mut e, 16000, 128).0;
    for (o, was) in [(0.0, 60.0), (4.0, 60.0), (7.0, 60.0)].map(|(o, root)| (o, root + o)) {
        assert!(amplitude_at(&x[1000..4000], hz(was), SR) > 0.1);
        assert!(amplitude_at(&x[5800..8800], hz(62.0 + o), SR) > 0.1, "every note moved, keeping its place in the chord");
    }
    let (full, half) = (rms(&x[5800..8800]), rms(&x[10600..13600]));
    assert!((half / full - 0.5).abs() < 0.02, "{half} {full}");
    assert_eq!(e.active_voices(), 0);
    assert_eq!(peak(&x[15000..]), 0.0);
}

/// A sequencer that plays a chord, changes it, and lets it go.
struct ChordPlayer(u32);

impl Sequencer for ChordPlayer {
    fn advance(&mut self, v: &mut Voices<'_>) -> u32 {
        self.0 += 1;
        match self.0 {
            1 => v.chord(0, 2, 60.0, 0.25, &[0, 4, 7]),
            2 => {
                // The same handle and note, another chord: struck on this frame.
                v.arp_notes(0, 2, 0.0, ORDER_TOGETHER, &[0, 5, 9, 12, 17]);
                v.arp_step(0, 2, 0.0);
            }
            3 => v.note_off(0, 2),
            _ => {}
        }
        4800
    }
}

#[test]
fn a_sequencer_plays_a_chord_in_one_call_and_changes_it_in_two() {
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    e.set_sequencer(Some(Box::new(ChordPlayer(0))));
    let x = render(&mut e, 4800, 128).0;
    assert_eq!(e.active_voices(), 3);
    assert!(amplitude_at(&x[1000..4000], hz(64.0), SR) > 0.1);
    let x = render(&mut e, 4800, 128).0;
    assert_eq!(e.active_voices(), 5);
    assert!(amplitude_at(&x[1000..4000], hz(65.0), SR) > 0.1 && amplitude_at(&x[1000..4000], hz(77.0), SR) > 0.1);
    assert!(amplitude_at(&x[1000..4000], hz(67.0), SR) < 0.02);
    render(&mut e, 4800, 128);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));
}

#[test]
fn an_arpeggio_with_no_clock_steps_when_it_is_told_and_as_long_as_it_is_told() {
    let mut e = sine_engine();
    Ask { rate: -1.0, gate: 0.5, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 100.0, 0, 6, 60.0, 1.0);
    // The first step sounds with the note; a step event on the same frame
    // says how long it is, and each one after begins the next.
    let times = [(100u64, 2000.0f32), (2100, 1000.0), (3100, 3000.0), (6100, 0.0)];
    for (at, length) in times {
        e.push(Event::arp_step(at as f64, 0, 6, length));
    }
    // Ticks do not step it: it is the step events' alone.
    e.push(Event::macro_tick(1500.0, ALL_PARTS));
    let got = render(&mut e, 10000, 128).0;
    let mut by_hand = sine_engine();
    for (k, (at, length)) in times.into_iter().enumerate() {
        // (A length of 0 is as long as the last.)
        let length = if length > 0.0 { length } else { 3000.0 };
        by_hand.push(Event::note_on(at as f64, 0, 6, 60.0 + [0.0, 4.0, 7.0][k % 3], 1.0));
        by_hand.push(Event::note_off((at + (length * 0.5) as u64) as f64, 0, 6, 0.0));
    }
    assert_eq!(first_difference(&got, &render(&mut by_hand, 10000, 128).0), None);
}

#[test]
fn letting_go_or_cutting_the_note_ends_the_arpeggio() {
    for how in 0..4 {
        let mut e = sine_engine();
        Ask { gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 5, 60.0, 1.0);
        e.push(match how {
            0 => Event::note_off(3000.0, 0, 5, 0.0),
            1 => Event::voice_kill(3000.0, 0, 5),
            2 => Event::all_notes_off(3000.0, 0, false),
            _ => Event::all_notes_off(3000.0, ALL_PARTS, true),
        });
        let x = render(&mut e, 12000, 128).0;
        assert!(rms(&x[..3000]) > 0.5);
        assert_eq!(e.active_arpeggiators(), 0, "{how}");
        assert_eq!(e.active_voices(), 0, "{how}");
        assert_eq!(peak(&x[3000 + 1700..]), 0.0, "{how}: no step after it, and the release is over");
    }
    // A note with no handle is let go by its note, as any other.
    let mut e = sine_engine();
    Ask { gate: 0.5, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 0, 60.0, 1.0);
    e.push(Event::note_off(3000.0, 0, 0, 60.0));
    let x = render(&mut e, 12000, 128).0;
    assert_eq!((e.active_arpeggiators(), e.active_voices()), (0, 0));
    assert_eq!(peak(&x[3000 + 1700..]), 0.0);
    // An engine that is reset has none.
    let mut e = sine_engine();
    Ask::new(&[0, 4, 7]).start(&mut e, 0.0, 0, 5, 60.0, 1.0);
    render(&mut e, 3000, 128);
    e.reset();
    assert_eq!(e.active_arpeggiators(), 0);
    assert_eq!(peak(&render(&mut e, 4000, 128).0), 0.0);
}

#[test]
fn taking_the_arpeggio_away_lets_the_step_in_hand_finish() {
    // Struck, with a gate: the note that is sounding is held for its gate.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { rate: 10.0, gate: 0.5, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 5, 60.0, 1.0);
    e.push(Event::arp(5000.0, 0, 5, 0.0, 0, &[]));
    let x = render(&mut e, 20000, 128).0;
    assert!(rms(&x[5000..7100]) > 0.6 && peak(&x[7300..]) == 0.0);
    assert_eq!((e.active_arpeggiators(), e.active_voices()), (0, 0));
    // A pitch that jumps: the note stays, on its own pitch.
    let mut e = sine_engine();
    Ask { rate: 10.0, gate: 0.0, octaves: 2, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 5, 60.0, 1.0);
    e.push(Event::arp_notes(9620.0, 0, 5, 0.0, 0, &[]));
    let x = render(&mut e, 20000, 128).0;
    assert_eq!(heard(&x, 60.0, 4800, 4), [0, 4, 0, 0]);
    assert_eq!((e.active_arpeggiators(), e.active_voices()), (0, 1));
    // The plain `arp` event puts a voice's own arpeggio in an arpeggiator's place.
    let mut e = sine_engine();
    Ask { rate: 10.0, gate: 0.0, octaves: 2, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 5, 60.0, 1.0);
    e.push(Event::arp(9600.0, 0, 5, 50.0, ORDER_UP, &[0, 12]));
    let x = render(&mut e, 9600 + 960 * 4, 128).0;
    assert_eq!(heard(&x[9600..], 60.0, 960, 4), [0, 12, 0, 12]);
    assert_eq!((e.active_arpeggiators(), e.active_voices()), (0, 1));
}

#[test]
fn what_is_done_to_the_handle_reaches_the_notes_still_to_come() {
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    e.set_param(0, PAN, 0.0);
    Ask { rate: 10.0, gate: 0.5, ..Ask::new(&[0, 7]) }.start(&mut e, 0.0, 0, 8, 60.0, 1.0);
    // Up a tone during the second step, quieter during the fourth, and to
    // the right during the sixth.
    e.push(Event::voice_pitch(6000.0, 0, 8, 62.0, 0.0));
    e.push(Event::voice_volume(16000.0, 0, 8, 0.25));
    e.push(Event::voice_pan(26000.0, 0, 8, 1.0));
    let (l, r) = render(&mut e, 4800 * 8, 128);
    let note = |step: usize, from: usize, to: usize| 12.0 * (frequency(&l[step * 4800 + from..step * 4800 + to], SR) / hz(60.0)).log2();
    assert!((note(0, 40, 2300)).abs() < 0.02);
    assert!((note(1, 40, 1150) - 7.0).abs() < 0.02 && (note(1, 1250, 2300) - 9.0).abs() < 0.02, "the sounding note moves with it");
    assert!((note(2, 40, 2300) - 2.0).abs() < 0.02 && (note(3, 40, 2300) - 9.0).abs() < 0.02);
    let level = |x: &[f32], step: usize| rms(&x[step * 4800 + 2000..step * 4800 + 2300]);
    assert!((level(&l, 4) / level(&l, 2) - 0.25).abs() < 0.01);
    assert!(level(&l, 5) > 0.01 && level(&l, 6) < 1.0e-4 && level(&r, 6) > 0.01, "the next note starts on the right: {} {} {}", level(&l, 5), level(&l, 6), level(&r, 6));
}

#[test]
fn an_arpeggio_can_be_given_to_a_note_that_is_sounding() {
    // Struck: the plain note is let go as the first step is struck, at once.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    e.push(Event::note_on(0.0, 0, 3, 60.0, 0.5));
    for ev in (Ask { gate: 1.0, ..Ask::new(&[4, 7, 12]) }).events(3000.0, 0, 3) {
        e.push(ev);
    }
    let x = render(&mut e, 3000 + 960 * 4, 128).0;
    assert!((frequency(&x[100..2900], SR) / hz(60.0) - 1.0).abs() < 1e-3);
    assert_eq!(heard(&x[3000..], 60.0, 960, 4), [4, 7, 12, 4]);
    assert_eq!(e.active_voices(), 1);
    // It is struck as hard as the note was.
    assert!((peak(&x[3100..3900]) / peak(&x[100..2900]) - 1.0).abs() < 0.01);

    // A pitch that jumps: the note itself, with its envelope carrying on.
    let mut e = sine_engine();
    e.set_param(0, AMP_ATTACK, 1.0);
    e.set_param(0, AMP_CURVE, 0.0);
    e.push(Event::note_on(0.0, 0, 3, 60.0, 1.0));
    for ev in (Ask { gate: 0.0, octaves: 2, ..Ask::new(&[0, 7]) }).events(4800.0, 0, 3) {
        e.push(ev);
    }
    let x = render(&mut e, 4800 + 960 * 5, 128).0;
    assert_eq!(heard(&x[4800..], 60.0, 960, 5), [0, 7, 12, 19, 0]);
    assert!((peak(&x[9000..9600]) - 0.2).abs() < 0.02, "one slow attack, still going");

    // Given to a handle nothing has, it does nothing.
    let mut e = sine_engine();
    for ev in Ask::new(&[0, 7]).events(0.0, 0, 3) {
        e.push(ev);
    }
    assert_eq!(peak(&render(&mut e, 2000, 128).0), 0.0);
    assert_eq!(e.active_arpeggiators(), 0);
}

#[test]
fn a_voices_own_arpeggio_carries_on_from_its_place_when_it_is_given_more() {
    // The old event: 0 4 7 at 50 a second. In the middle of its second step
    // it is given two octaves, and goes on from where it was, on the beat.
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 3, 60.0, 1.0));
    e.push(Event::arp(0.0, 0, 3, 50.0, ORDER_UP, &[0, 4, 7]));
    e.push(Event::arp_style(1400.0, 0, 3, 0.0, 2, 0.0, 0));
    let x = render(&mut e, 960 * 8, 128).0;
    assert_eq!(heard(&x, 60.0, 960, 8), [0, 4, 7, 12, 16, 19, 0, 4]);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (1, 1));
}

#[test]
fn an_arpeggio_costs_no_more_voices_than_it_says() {
    // A release far longer than a step: the notes pile up, to eight and no further.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 3.0);
    Ask { rate: 40.0, gate: 0.5, octaves: 3, ..Ask::new(&[0, 3, 7, 10]) }.start(&mut e, 0.0, 0, 1, 48.0, 1.0);
    let mut most = 0;
    for _ in 0..400 {
        render(&mut e, 240, 240);
        most = most.max(e.active_voices());
        assert!(e.active_voices() <= ARPEGGIATOR_VOICES);
    }
    assert_eq!(most, ARPEGGIATOR_VOICES);
    // Stabs of four: the four that sound and the four before them.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 3.0);
    Ask { order: ORDER_TOGETHER, rate: 20.0, gate: 0.5, ..Ask::new(&[0, 3, 7, 10]) }.start(&mut e, 0.0, 0, 1, 48.0, 1.0);
    for _ in 0..200 {
        render(&mut e, 240, 240);
        assert!(e.active_voices() <= ARPEGGIATOR_VOICES);
    }
    assert_eq!(e.active_voices(), 8);
    // A chord of eight with unison takes what it takes; nothing of the step in hand is cut.
    let mut e = sine_engine();
    e.set_param(0, UNISON, 2.0);
    e.push(Event::arp(0.0, 0, 0, -1.0, ORDER_TOGETHER, &[0, 3, 7, 10]));
    e.push(Event::arp_more(0.0, 0, 0, &[12, 15, 19, 22]));
    let mut ev = Event::arp(0.0, 0, 0, -1.0, ORDER_TOGETHER, &[0, 3, 7, 10]);
    ev.z = 8.0;
    e.push(ev);
    e.push(Event::arp_more(0.0, 0, 0, &[12, 15, 19, 22]));
    e.push(Event::note_on(0.0, 0, 1, 48.0, 1.0));
    render(&mut e, 480, 128);
    assert_eq!(e.active_voices(), 16);
    // In mono mode a struck arpeggio is one voice, started again at each step.
    let mut e = sine_engine();
    e.set_param(0, MODE, 1.0);
    e.set_param(0, AMP_RELEASE, 3.0);
    Ask { gate: 0.5, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    let x = render(&mut e, 960 * 6, 128).0;
    assert_eq!(e.active_voices(), 1);
    assert_eq!(heard(&x, 60.0, 960, 6), [0, 4, 7, 0, 4, 7]);
    // And when its notes are let go, nothing is left.
    e.push(Event::note_off(960.0 * 6.0, 0, 1, 0.0));
    e.set_param(0, AMP_RELEASE, 0.01);
    render(&mut e, 48000, 128);
    assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0));
}

#[test]
fn there_is_room_for_thirty_two_and_the_oldest_makes_way() {
    let mut e = sine_engine();
    for i in 0..MAX_ARPEGGIATORS as u32 + 3 {
        Ask { rate: 1.0, gate: 1.0, ..Ask::new(&[0, 7]) }.start(&mut e, i as f64 * 10.0, (i % 4) as u16, 100 + i, 40.0 + i as f32, 0.2);
    }
    render(&mut e, 1000, 128);
    assert_eq!(e.active_arpeggiators(), MAX_ARPEGGIATORS);
    // The three that made way had their notes let go.
    assert_eq!(e.active_voices(), MAX_ARPEGGIATORS + 3);
    render(&mut e, 4800, 128);
    assert_eq!(e.active_voices(), MAX_ARPEGGIATORS);
}

#[test]
fn the_notes_a_display_reads_are_the_notes_that_sound() {
    let sounding = |e: &Engine| {
        let bits = e.taps().read_notes(0).unwrap();
        (0..128).filter(|n| bits[4 + n / 32] & (1 << (n % 32)) != 0).collect::<Vec<_>>()
    };
    // One voice stepping its own pitch, as the `arp` event has always made:
    // a second in, on its third note.
    let mut e = sine_engine();
    e.push(Event::note_on(0.0, 0, 1, 60.0, 1.0));
    e.push(Event::arp(0.0, 0, 1, 1.0, ORDER_UP, &[0, 4, 7]));
    render(&mut e, 48000 * 2 + 4800, 128);
    assert_eq!(sounding(&e), [67]);
    // A chord.
    let mut e = sine_engine();
    e.push(Event::arp(0.0, 0, 0, -1.0, ORDER_TOGETHER, &[0, 3, 7]));
    e.push(Event::note_on(0.0, 0, 1, 60.0, 1.0));
    render(&mut e, 4800, 128);
    assert_eq!(sounding(&e), [60, 63, 67]);
    // A struck arpeggio, on its second note.
    let mut e = sine_engine();
    e.set_param(0, AMP_RELEASE, 0.0);
    Ask { rate: 1.0, gate: 1.0, ..Ask::new(&[0, 4, 7]) }.start(&mut e, 0.0, 0, 1, 60.0, 1.0);
    render(&mut e, 48000 + 4800, 128);
    assert_eq!(sounding(&e), [64]);
}

/// A phrase with a bit of everything in it.
fn phrase(e: &mut Engine) {
    e.set_param(0, TEMPO, 133.0);
    e.set_param(1, UNISON, 2.0);
    Ask { beat: 4.0, gate: 0.6, octaves: 2, swing: 0.2, levels: 0xF8F, ..Ask::new(&[0, 3, 7, 10]) }.start(e, 37.0, 0, 1, 50.0, 0.8);
    Ask { order: ORDER_TOGETHER, rate: 7.3, gate: 0.4, ..Ask::new(&[0, 4, 7]) }.start(e, 1001.0, 1, 2, 62.0, 0.5);
    Ask { order: ORDER_RANDOM, rate: 31.0, gate: 0.0, octaves: 3, ..Ask::new(&[0, 5, 7]) }.start(e, 2000.0, 2, 3, 70.0, 0.5);
    e.push(Event::arp(3000.0, 3, 0, -1.0, ORDER_TOGETHER, &[0, 4, 7, 11]));
    e.push(Event::note_on(3000.0, 3, 4, 45.0, 0.5));
    e.push(Event::param(9000.0, 0, TEMPO, 180.0, 0.0));
    e.push(Event::arp_notes(15000.0, 0, 1, 0.0, ORDER_UP_DOWN, &[0, 4, 7]));
    e.push(Event::voice_pitch(20000.0, 3, 4, 47.0, 0.1));
    e.push(Event::note_off(30000.0, 0, 2, 0.0));
    e.push(Event::note_off(36000.0, 1, 2, 0.0));
    e.push(Event::note_off(40000.0, 2, 3, 0.0));
    e.push(Event::note_off(41000.0, 3, 4, 0.0));
    e.push(Event::note_off(42000.0, 0, 1, 0.0));
}

#[test]
fn the_sound_does_not_depend_on_the_size_of_the_blocks() {
    let run = |block: usize| {
        let mut e = sine_engine();
        phrase(&mut e);
        let out = render(&mut e, 52000, block);
        assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0), "nothing is left sounding");
        out
    };
    let (l, r) = run(128);
    assert!(rms(&l[..40000]) > 0.05);
    for block in [1, 64, 333, 4096] {
        let (bl, br) = run(block);
        assert_eq!(first_difference(&l, &bl), None, "block {block}");
        assert_eq!(first_difference(&r, &br), None, "block {block}");
    }
}

#[test]
fn any_arpeggio_events_at_all_leave_the_engine_sound() {
    let wild = |rng: &mut Rng, lo: f32, hi: f32| match rng.next_u32() % 30 {
        0 => f32::NAN,
        1 => f32::INFINITY,
        2 => f32::NEG_INFINITY,
        3 => 1.0e30,
        4 => -1.0e30,
        5 => 0.0,
        _ => lo + (hi - lo) * rng.unipolar(),
    };
    for seed in 1..=4 {
        let mut rng = Rng::new(seed);
        let mut e = Engine::new(SR);
        e.set_param(0, MASTER_VOLUME, 0.3);
        let (mut l, mut r) = (vec![0.0f32; 700], vec![0.0f32; 700]);
        let mut loud = 0;
        for round in 0..1500 {
            let now = e.time() as f64;
            for _ in 0..rng.next_u32() % 10 {
                let kind = [1, 1, 2, 3, 4, 6, 8, 11, 12, 13, 13, 14, 17, 17, 18, 19, 19, 20, 20][rng.next_u32() as usize % 19];
                let part = if rng.next_u32() % 30 == 0 { [0xFFFF, 1000][rng.next_u32() as usize % 2] } else { (rng.next_u32() % 4) as u16 };
                let mut ev = Event::note_on(now + (rng.next_u32() % 1500) as f64, part, rng.next_u32() % 4, wild(&mut rng, 20.0, 100.0), wild(&mut rng, 0.0, 1.0));
                ev.kind = kind;
                ev.z = wild(&mut rng, -1.0, 9.0);
                ev.b = rng.next_u32();
                match kind {
                    3 => ev.a = rng.next_u32() % 2,
                    4 => {
                        ev.a = [TEMPO, MODE, UNISON, AMP_RELEASE, VOICES][rng.next_u32() as usize % 5] as u32;
                        ev.x = wild(&mut rng, -10.0, 1200.0);
                        ev.y = 0.0;
                    }
                    13 | 19 => {
                        ev.x = wild(&mut rng, -10.0, 1500.0);
                        ev.y = wild(&mut rng, -1.0, 8.0);
                    }
                    17 => {
                        ev.x = wild(&mut rng, -0.5, 1.5);
                        ev.y = wild(&mut rng, -1.0, 6.0);
                        ev.z = wild(&mut rng, -0.5, 1.5);
                    }
                    18 => ev.x = wild(&mut rng, -1.0, 80.0),
                    20 => ev.x = wild(&mut rng, -100.0, 20000.0),
                    _ => {}
                }
                e.push(ev);
            }
            if round % 400 == 399 {
                e.reset();
            }
            let n = 1 + (rng.next_u32() % 700) as usize;
            e.render(&mut l[..n], &mut r[..n]);
            assert!(l[..n].iter().chain(&r[..n]).all(|x| x.is_finite() && x.abs() <= 1.0), "seed {seed}");
            assert!(e.active_voices() <= 64 && e.active_arpeggiators() <= MAX_ARPEGGIATORS);
            loud += l[..n].iter().filter(|x| x.abs() > 0.01).count();
        }
        assert!(loud > 100_000, "seed {seed}: {loud}");
        // Everything let go and cut, once the last of the events has
        // come: silence, and nothing left running.
        e.render(&mut l, &mut r);
        e.render(&mut l, &mut r);
        e.render(&mut l, &mut r);
        e.push(Event::all_notes_off(0.0, ALL_PARTS, true));
        e.set_param(0, VOICES, 64.0);
        for _ in 0..80 {
            e.render(&mut l, &mut r);
        }
        assert_eq!((e.active_voices(), e.active_arpeggiators()), (0, 0), "seed {seed}");
    }
}

#[test]
fn the_tempo_is_a_parameter_of_the_engine() {
    let e = Engine::new(SR);
    assert_eq!(e.param(0, TEMPO), 120.0);
    let info = by_name("tempo").unwrap();
    assert!(info.is_global() && info.min == 20.0 && info.max == 999.0);
    // Steps to a beat: 4 is sixteenths, 2 eighths, 3 eighth-note triplets.
    let spec = |beat: f32| {
        let mut s = ArpSpec::NONE;
        s.set_beat(beat);
        s.period(48000.0, 120.0)
    };
    assert_eq!([spec(1.0), spec(2.0), spec(3.0), spec(4.0), spec(8.0)], [24000.0, 12000.0, 8000.0, 6000.0, 3000.0]);
}
