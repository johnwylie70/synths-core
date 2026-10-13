mod common;
use common::*;
use synths_core::params::*;
use synths_core::taps::MASTER;
use synths_core::{Engine, Event};

#[test]
fn the_scope_holds_the_latest_sound() {
    let mut e = Engine::new(SR);
    e.set_param(0, OSC1_WAVE, 2.0);
    e.set_param(1, PAN, 0.6);
    note_on(&mut e, 0, 0, 50.0);
    note_on(&mut e, 0, 1, 67.0);
    let (l, r) = render(&mut e, 10_000, 128);
    let mut seen = vec![0.0f32; 4096];
    assert_eq!(e.taps().read_scope(MASTER, &mut seen), 4096);
    for (i, &v) in seen.iter().enumerate() {
        let at = 10_000 - 4096 + i;
        assert_eq!(v, 0.5 * (l[at] + r[at]), "frame {at}");
    }
    // A shorter read is the newest end of the same thing.
    let mut short = vec![0.0f32; 300];
    assert_eq!(e.taps().read_scope(MASTER, &mut short), 300);
    assert_eq!(&short[..], &seen[4096 - 300..]);
    // Each part has its own, 1024 frames long; together they make the master.
    let mut a = vec![0.0f32; 2000];
    let mut b = vec![0.0f32; 2000];
    assert_eq!(e.taps().read_scope(0, &mut a), 1024);
    assert_eq!(e.taps().read_scope(1, &mut b), 1024);
    assert!(rms(&a[..1024]) > 0.1 && rms(&b[..1024]) > 0.1);
    for i in 0..1024 {
        let sum = (a[i] + b[i]) * 0.35;
        assert!((sum - seen[4096 - 1024 + i]).abs() < 1e-6);
    }
    assert_eq!(e.taps().read_scope(2, &mut a), 1024);
    assert!(a[..1024].iter().all(|&v| v == 0.0), "a part that has not played is flat");
    assert_eq!(e.taps().read_scope(64, &mut a), 0);
}

#[test]
fn the_scope_shows_silence_after_the_sound_stops() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    note_on(&mut e, 0, 0, 60.0);
    note_off(&mut e, 600, 0, 60.0);
    let _ = render(&mut e, 1000, 128);
    let mut seen = vec![0.0f32; 1024];
    e.taps().read_scope(0, &mut seen);
    // 1000 frames have gone by: 24 from before the engine began, 600 of note, 400 of silence.
    assert!(seen[..24].iter().all(|&v| v == 0.0));
    assert!(seen[24..624].iter().all(|&v| v != 0.0));
    assert!(seen[624..].iter().all(|&v| v == 0.0));
    // The part stops being rendered, but its scope keeps moving on.
    let _ = render(&mut e, 300, 128);
    e.taps().read_scope(0, &mut seen);
    assert!(seen[..324].iter().all(|&v| v != 0.0) && seen[324..].iter().all(|&v| v == 0.0));
    // Long after, with the engine asleep, both scopes are flat; then a new note shows.
    let _ = render(&mut e, 48000 * 4, 128);
    let mut all = vec![1.0f32; 4096];
    e.taps().read_scope(MASTER, &mut all);
    e.taps().read_scope(0, &mut seen);
    assert!(all.iter().chain(seen.iter()).all(|&v| v == 0.0));
    let at = e.time() + 500;
    note_on(&mut e, at, 0, 60.0);
    let _ = render(&mut e, 600, 128);
    e.taps().read_scope(MASTER, &mut all);
    assert!(all[..4096 - 100].iter().all(|&v| v == 0.0) && all[4096 - 100..].iter().all(|&v| v != 0.0));
}

#[test]
fn the_meters_follow_the_sound_and_fall_back() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 3.0);
    e.set_param(0, VOLUME, 0.5);
    e.set_param(0, MASTER_VOLUME, 0.5);
    note_on(&mut e, 0, 0, 57.0);
    note_off(&mut e, 48000, 0, 57.0);
    let _ = render(&mut e, 48000, 128);
    let near = |got: f32, want: f32| assert!((got - want).abs() < 0.02 * want.max(0.05), "{got} vs {want}");
    // A square wave of height 0.5, all on the left: peak and RMS are both 0.5.
    let [pl, pr, rl, rr] = e.taps().read_levels(0).unwrap();
    near(pl, 0.5);
    near(rl, 0.5);
    assert!(pr < 1e-5 && rr < 1e-5);
    let [pl, _, rl, _] = e.taps().read_levels(MASTER).unwrap();
    near(pl, 0.25);
    near(rl, 0.25);
    assert_eq!(e.taps().read_levels(1).unwrap(), [0.0; 4]);
    assert!(e.taps().read_levels(64).is_none());
    // Half a second after the sound stops the peak has fallen to about a third, the RMS further.
    let _ = render(&mut e, 24000, 128);
    let [pl, _, rl, _] = e.taps().read_levels(0).unwrap();
    near(pl, 0.5 * (-1.0f32).exp());
    assert!(rl < 0.3 * pl && rl > 0.0);
    let [pl, _, _, _] = e.taps().read_levels(MASTER).unwrap();
    near(pl, 0.25 * (-1.0f32).exp());
    // And in the end, nothing, even with the engine asleep.
    let _ = render(&mut e, 48000 * 12, 128);
    assert_eq!(e.taps().read_levels(0).unwrap(), [0.0; 4]);
    assert_eq!(e.taps().read_levels(MASTER).unwrap(), [0.0; 4]);
    // A sine reads 0.707 of its peak.
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    note_on(&mut e, 0, 0, 69.0);
    let _ = render(&mut e, 48000, 128);
    let [pl, _, rl, _] = e.taps().read_levels(0).unwrap();
    near(pl, 1.0);
    near(rl, std::f32::consts::FRAC_1_SQRT_2);
}

#[test]
fn the_notes_sounding_can_be_read_for_a_keyboard() {
    let held = |e: &Engine, part: u32| -> Vec<usize> {
        let bits = e.taps().read_notes(part).unwrap();
        (0..128).filter(|n| bits[n / 32] >> (n % 32) & 1 == 1).collect()
    };
    let sounding = |e: &Engine, part: u32| -> Vec<usize> {
        let bits = e.taps().read_notes(part).unwrap();
        (0..128).filter(|n| bits[4 + n / 32] >> (n % 32) & 1 == 1).collect()
    };
    let mut e = plain_engine();
    e.set_param(0, AMP_RELEASE, 0.5);
    for note in [60.0, 64.0, 67.2, 0.0, 127.0, 140.0] {
        note_on(&mut e, 0, 0, note);
    }
    e.push(Event::note_on(0.0, 5, 9, 31.0, 1.0));
    let _ = render(&mut e, 512, 128);
    assert_eq!(held(&e, 0), [0, 60, 64, 67, 127]);
    assert_eq!(held(&e, 5), [31]);
    assert_eq!(held(&e, 1), [] as [usize; 0]);
    assert_eq!(held(&e, MASTER), [0, 31, 60, 64, 67, 127]);
    assert!(e.taps().read_notes(64).is_none());
    // Let go, a note is no longer held but still sounds until its release is over.
    note_off(&mut e, 512, 0, 64.0);
    e.push(Event::voice_pitch(512.0, 5, 9, 43.0, 0.0));
    let _ = render(&mut e, 512, 128);
    assert_eq!(held(&e, 0), [0, 60, 67, 127]);
    assert_eq!(sounding(&e, 0), [0, 60, 64, 67, 127]);
    assert_eq!(held(&e, 5), [43], "a voice moved to a new pitch lights the new key");
    let _ = render(&mut e, 48000, 128);
    assert_eq!(sounding(&e, 0), [0, 60, 67, 127]);
    // Reset clears the keys at once.
    e.reset();
    assert_eq!(sounding(&e, MASTER), [] as [usize; 0]);
}

#[test]
fn reading_the_taps_does_not_change_the_sound() {
    let run = |read: bool| {
        let mut e = Engine::new(SR);
        e.set_param(0, UNISON, 3.0);
        e.set_param(0, SEND_REVERB, 0.5);
        note_on(&mut e, 0, 0, 60.0);
        let mut out = Vec::new();
        let mut scratch = vec![0.0f32; 4096];
        for _ in 0..100 {
            out.extend(render(&mut e, 128, 128).0);
            if read {
                e.taps().read_scope(MASTER, &mut scratch);
                e.taps().read_scope(0, &mut scratch);
                e.taps().read_levels(0);
                e.taps().read_notes(0);
            }
        }
        out
    };
    assert!(run(true) == run(false));
}
