//! Mono and legato mode remember the notes held down: letting go of the
//! sounding note goes back to the latest note still held.

mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::params::*;
use synths_core::Event;

fn mono(mode: f32) -> Box<synths_core::Engine> {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, MODE, mode);
    e
}

/// The pitch sounding around each of the given frames.
fn pitches(x: &[f32], at: &[usize]) -> Vec<f32> {
    at.iter().map(|&a| (12.0 * (frequency(&x[a..a + 4000], SR) / 440.0).log2() + 69.0).round()).collect()
}

#[test]
fn letting_go_returns_to_the_last_note_still_held() {
    for mode in [1.0, 2.0] {
        let mut e = mono(mode);
        note_on(&mut e, 0, 0, 60.0);
        note_on(&mut e, 10000, 0, 64.0);
        note_on(&mut e, 20000, 0, 67.0);
        note_off(&mut e, 30000, 0, 67.0); // back to 64
        note_off(&mut e, 40000, 0, 60.0); // not sounding: nothing changes
        note_off(&mut e, 50000, 0, 64.0); // nothing left held: the note ends
        let x = render(&mut e, 60000, 128).0;
        assert_eq!(pitches(&x, &[2000, 12000, 22000, 32000, 42000]), [60.0, 64.0, 67.0, 64.0, 64.0], "mode {mode}");
        assert!(x[50100..].iter().all(|&v| v == 0.0));
        assert_eq!(e.active_voices(), 0);
    }
}

#[test]
fn the_latest_note_wins_and_a_note_played_twice_is_held_once() {
    let mut e = mono(1.0);
    note_on(&mut e, 0, 0, 60.0);
    note_on(&mut e, 5000, 0, 72.0);
    note_on(&mut e, 10000, 0, 60.0); // 60 again: now the latest, and held once
    note_off(&mut e, 20000, 0, 60.0); // back to 72
    note_off(&mut e, 30000, 0, 72.0); // nothing held
    let x = render(&mut e, 40000, 128).0;
    assert_eq!(pitches(&x, &[1000, 5500, 12000, 22000]), [60.0, 72.0, 60.0, 72.0]);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn mono_restarts_the_envelope_on_the_way_back_and_legato_does_not() {
    let level_after_return = |mode: f32| {
        let mut e = mono(mode);
        e.set_param(0, AMP_ATTACK, 0.1);
        note_on(&mut e, 0, 0, 60.0);
        note_on(&mut e, 20000, 0, 64.0);
        note_off(&mut e, 40000, 0, 64.0);
        let x = render(&mut e, 44000, 128).0;
        (peak(&x[39000..40000]), peak(&x[40000..40200]), peak(&x[43000..44000]))
    };
    // Legato: one unbroken note.
    let (before, just_after, later) = level_after_return(2.0);
    assert!(before > 0.99 && just_after > 0.99 && later > 0.99);
    // Mono: the attack runs again. It starts from where the level is, so there is no click.
    let (before, just_after, later) = level_after_return(1.0);
    assert!(before > 0.99 && just_after > 0.99 && later > 0.99);
    let attack_runs_again = |mode: f32| {
        let mut e = mono(mode);
        e.set_param(0, AMP_DECAY, 0.05);
        e.set_param(0, AMP_SUSTAIN, 0.2);
        note_on(&mut e, 0, 0, 60.0);
        note_on(&mut e, 20000, 0, 64.0);
        note_off(&mut e, 40000, 0, 64.0);
        let x = render(&mut e, 41000, 128).0;
        (peak(&x[39000..40000]), peak(&x[40000..40200]))
    };
    let (sustained, returned) = attack_runs_again(1.0);
    assert!(sustained < 0.21 && returned > 0.9, "mono: {sustained} then {returned}");
    let (sustained, returned) = attack_runs_again(2.0);
    assert!(sustained < 0.21 && returned < 0.21, "legato: {sustained} then {returned}");
}

#[test]
fn held_notes_work_by_handle_and_glide_back() {
    let mut e = mono(2.0);
    e.set_param(0, GLIDE, 0.1);
    e.push(Event::note_on(0.0, 0, 11, 48.0, 1.0));
    e.push(Event::note_on(10000.0, 0, 12, 60.0, 0.5));
    e.push(Event::note_off(30000.0, 0, 12, 0.0));
    let x = render(&mut e, 48000, 128).0;
    assert_eq!(pitches(&x, &[5000, 24000, 42000]), [48.0, 60.0, 48.0]);
    // Half way through the glide back it is half way there.
    let mid = 12.0 * (frequency(&x[32000..32800], SR) / 440.0).log2() + 69.0;
    assert!(mid > 51.0 && mid < 57.0, "{mid}");
    // The handle now steers the note it came back to.
    e.push(Event::voice_kill(48000.0, 0, 12));
    let _ = render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), 1);
    e.push(Event::note_off(48256.0, 0, 11, 0.0));
    let _ = render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn held_notes_are_forgotten_when_they_should_be() {
    // All notes off forgets them.
    let mut e = mono(1.0);
    e.set_param(0, AMP_RELEASE, 0.01);
    note_on(&mut e, 0, 0, 60.0);
    note_on(&mut e, 1000, 0, 64.0);
    e.push(Event::all_notes_off(2000.0, ALL_PARTS, false));
    note_on(&mut e, 10000, 0, 67.0);
    note_off(&mut e, 12000, 0, 67.0);
    let x = render(&mut e, 20000, 128).0;
    assert_eq!(e.active_voices(), 0, "67 was the only note held");
    assert!(x[13000..].iter().all(|&v| v == 0.0));
    // So does a reset, and so does leaving mono mode.
    let mut e = mono(1.0);
    note_on(&mut e, 0, 0, 60.0);
    let _ = render(&mut e, 1000, 128);
    e.reset();
    note_on(&mut e, 1000, 0, 67.0);
    note_off(&mut e, 2000, 0, 67.0);
    let _ = render(&mut e, 2000, 128);
    assert_eq!(e.active_voices(), 0);
    // A note-off to every part reaches a mono part's held notes too.
    let mut e = mono(1.0);
    note_on(&mut e, 0, 0, 60.0);
    note_on(&mut e, 1000, 0, 64.0);
    e.push(Event::note_off(2000.0, ALL_PARTS, 0, 64.0));
    let x = render(&mut e, 12000, 128).0;
    assert_eq!(pitches(&x, &[4000]), [60.0]);
    // More notes than it remembers: the oldest are dropped, and nothing breaks.
    let mut e = mono(1.0);
    for i in 0..40 {
        note_on(&mut e, i * 100, 0, 40.0 + i as f32);
    }
    for i in (0..40).rev() {
        note_off(&mut e, 4000 + (39 - i) * 100, 0, 40.0 + i as f32);
    }
    let _ = render(&mut e, 9000, 128);
    assert_eq!(e.active_voices(), 0);
}
