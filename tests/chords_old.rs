//! Songs written before chords and arpeggios were told apart: a `chord` in
//! them is the fast one-voice arpeggio, and they must go on sounding exactly
//! as they did. `fixtures/old-chords.json` is such a song, `old-chords.sysg`
//! is its packed form as the engine of that time packed it, and
//! `old-chords.f32` is what that engine played: left then right, 32-bit
//! floats.
//!
//! The two files beside the JSON were written by the engine as it was
//! before the change, with `SYNTHS_WRITE_FIXTURES=1 cargo test --test
//! chords_old`. They are not to be written again: a difference is a bug.

use synths_core::event::*;
use synths_core::params::*;
use synths_core::song::{binary, native};
use synths_core::song_player;
use synths_core::{Engine, Event};

const SR: f32 = 16000.0;
const FRAMES: usize = 12000;

fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn play(song: synths_core::song::Song) -> Vec<f32> {
    let mut e = Engine::new(SR);
    // The synth patch's part: a pulse with a little release, off centre.
    for (id, value) in [(OSC1_WAVE, 3.0), (AMP_RELEASE, 0.05), (PAN, 0.3), (CUTOFF, 3000.0), (FILTER_TYPE, 1.0)] {
        e.set_param(10, id, value);
    }
    song_player::install(&mut e, song, song_player::FIRST_PART, 0, false).unwrap();
    e.push(Event::song(0.0, SONG_PATCH_PART, 2.0, 10.0));
    e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (mut l, mut r) = (vec![0.0; FRAMES], vec![0.0; FRAMES]);
    for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        e.render(a, b);
    }
    l.extend(r);
    l
}

fn song() -> synths_core::song::Song {
    let text = std::fs::read_to_string(path("old-chords.json")).unwrap();
    native::load(&text, &|_| None).unwrap()
}

#[test]
fn an_old_song_with_chords_sounds_exactly_as_it_did() {
    let heard = play(song());
    if std::env::var_os("SYNTHS_WRITE_FIXTURES").is_some() {
        let bytes: Vec<u8> = heard.iter().flat_map(|x| x.to_le_bytes()).collect();
        std::fs::write(path("old-chords.f32"), bytes).unwrap();
        std::fs::write(path("old-chords.sysg"), binary::save(&song())).unwrap();
    }
    let bytes = std::fs::read(path("old-chords.f32")).unwrap();
    let then: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    assert_eq!(then.len(), heard.len());
    assert!(then.iter().any(|x| x.abs() > 0.05), "the fixture is not silence");
    let first = then.iter().zip(&heard).position(|(a, b)| a.to_bits() != b.to_bits());
    // The same machine gives the same bits. Another machine's own
    // arithmetic may differ in the last place, and no further.
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert_eq!(first, None, "the first frame that differs");
    }
    let worst = then.iter().zip(&heard).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 1.0e-4, "{worst}");
}

#[test]
fn an_old_packed_song_is_the_same_song_and_sounds_the_same() {
    let packed = std::fs::read(path("old-chords.sysg")).unwrap();
    let unpacked = binary::load(&packed).unwrap();
    assert_eq!(unpacked, song());
    let (a, b) = (play(unpacked), play(song()));
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}
