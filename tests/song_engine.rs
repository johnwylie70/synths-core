//! Songs played through the real engine: loaded with the C interface,
//! steered with `song` events, and compared with the plain mixer the
//! replayer's own tests use.

mod replayer_support;
use replayer_support::*;
use synths_core::event::*;
use synths_core::replayer::Player;
use synths_core::song::*;
use synths_core::song_abi::*;
use synths_core::song_player::*;
use synths_core::{Engine, Event};

const SR: f32 = 44100.0;

/// The song functions keep the last error and the staged sample files in
/// one place for the whole process, as suits a host with one audio thread.
/// Tests run side by side, so they take turns at those functions.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    static LAST_ERROR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

fn take_turn() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner())
}

/// Load a song, for a test that already has its turn.
fn load_in_turn(engine: &mut Engine, file: &[u8], first_slot: u32) -> i32 {
    let code = unsafe { synths_song_load(engine, file.as_ptr(), file.len() as u32, first_slot) };
    let why = unsafe { std::ffi::CStr::from_ptr(synths_song_error().cast()) }.to_str().unwrap().to_string();
    LAST_ERROR.set(why);
    code
}

fn load(engine: &mut Engine, file: &[u8], first_slot: u32) -> i32 {
    let _turn = take_turn();
    load_in_turn(engine, file, first_slot)
}

fn position(engine: &Engine) -> [u32; 10] {
    let mut out = [99u32; 10];
    assert_eq!(unsafe { synths_song_position(engine, out.as_mut_ptr()) }, 10);
    out
}

/// Why this thread's last load failed.
fn error() -> String {
    LAST_ERROR.with_borrow(|e| e.clone())
}

fn render(engine: &mut Engine, frames: usize, block: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        engine.render(a, b);
    }
    (l, r)
}

fn playing(file: &[u8]) -> Box<Engine> {
    let mut engine = Engine::new(SR);
    assert_eq!(load(&mut engine, file, 0), SONG_OK, "{}", error());
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    engine
}

/// How loud a signal is in each stretch of `size` frames.
fn loudness(x: &[f32], size: usize) -> Vec<f32> {
    x.chunks(size).map(|w| (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt()).collect()
}

/// How alike two loudness curves are in shape: 1 is the same.
fn likeness(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let (na, nb): (f32, f32) = (a.iter().map(|x| x * x).sum(), b.iter().map(|x| x * x).sum());
    dot / (na * nb).sqrt().max(1e-12)
}

/// The same song through the plain mixer.
fn reference(song: &Song, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut player = Player::new(song, SR as f64);
    let mut mixer = Mixer::new(song, SR as f64);
    play(song, &mut player, &mut mixer, frames as u64);
    mixer.left.truncate(frames);
    mixer.right.truncate(frames);
    (mixer.left, mixer.right)
}

/// A tune in four channels for both kinds of module.
fn tune(rules: Rules) -> Song {
    let xm = rules == Rules::FastTracker2;
    let line = |cells: [&str; 4]| -> String {
        // The same cells for both: an XM cell has a volume column as well.
        cells.map(|c| if xm { format!("{} .. {}", &c[..6], &c[7..]) } else { c.to_string() }).join(" | ")
    };
    let rows: Vec<String> = [
        ["C-4 01 000", "C-3 02 000", "... .. 000", "C-5 03 C20"],
        ["... .. 000", "... .. 000", "... .. 000", "... .. 000"],
        ["E-4 01 000", "... .. 000", "C-5 03 C10", "... .. A04"],
        ["... .. 000", "... .. 000", "... .. 000", "... .. 000"],
        ["G-4 01 000", "G-3 02 000", "... .. 000", "C-5 03 C20"],
        ["... .. 000", "... .. 000", "... .. 000", "... .. 000"],
        ["E-4 01 484", "... .. 000", "C-5 03 C10", "... .. 000"],
        ["... .. 484", "... .. 000", "... .. 000", "... .. 000"],
        ["A-4 01 000", "F-3 02 000", "... .. 000", "C-5 03 C20"],
        ["... .. A08", "... .. 000", "... .. 000", "... .. 000"],
        ["G-4 01 000", "... .. 103", "C-5 03 C10", "... .. 000"],
        ["... .. 000", "... .. 000", "... .. 000", "... .. 000"],
        ["C-5 01 047", "C-3 02 000", "... .. 000", "C-5 03 E93"],
        ["... .. 047", "... .. 000", "... .. 000", "... .. 000"],
        ["... .. A08", "... .. A08", "... .. 000", "... .. 000"],
        ["... .. A08", "... .. A08", "... .. 000", "... .. D00"],
    ]
    .into_iter()
    .map(line)
    .collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mut bass = square(128, 1);
    bass.volume = 48;
    let mut hat = ramp(400);
    hat.data = SampleData::Bits8((0..400).map(|i| (((i * 7919) % 251) as i32 - 125) as i8).collect());
    let mut s = song(rules, 4, vec![square(32, 1), bass, hat], vec![pattern(64, &rows)]);
    s.title = "engine tune".to_string();
    if xm {
        s.channel_pan = vec![128; 4];
        s.samples[0].pan = Some(64);
        s.samples[1].pan = Some(192);
        s.instruments[0].volume_envelope = Envelope { enabled: true, points: vec![(0, 64), (6, 32), (30, 16)], sustain: Some(1), loop_points: None };
        s.instruments[0].fadeout = 1024;
    }
    s
}

#[test]
fn a_mod_and_an_xm_sound_the_same_through_the_engine_as_through_the_plain_mixer() {
    for rules in [Rules::ProTracker, Rules::FastTracker2] {
        let song = tune(rules);
        let file = if rules == Rules::ProTracker { write_mod(&song) } else { write_xm(&song) };
        let song = load_module(&file).unwrap();
        let frames = 16 * 6 * 882;
        let mut engine = playing(&file);
        let (l, r) = render(&mut engine, frames, 512);
        let (ml, mr) = reference(&song, frames);
        assert!(l.iter().chain(&r).all(|x| x.is_finite() && x.abs() <= 1.0));
        // The two differ in how they interpolate, pan and smooth, so it is
        // the shape of the loudness that is compared: each side's, a tick at
        // a time, and the level overall.
        for (name, ours, theirs) in [("left", &l, &ml), ("right", &r, &mr)] {
            let (a, b) = (loudness(ours, 882), loudness(theirs, 882));
            let alike = likeness(&a, &b);
            assert!(alike > 0.97, "{rules:?} {name}: likeness {alike}");
            let (ea, eb): (f32, f32) = (a.iter().sum(), b.iter().sum());
            // The engine's master volume is 0.35 and a channel's part is at
            // 0.5, where the plain mixer has a channel at 0.25.
            let scale = 0.35 * CHANNEL_GAIN / 0.25;
            assert!((ea / (eb * scale) - 1.0).abs() < 0.2, "{rules:?} {name}: level {ea} against {}", eb * scale);
        }
        // And silent before it is told to play.
        let mut idle = Engine::new(SR);
        assert_eq!(load(&mut idle, &file, 0), SONG_OK);
        let (l, _) = render(&mut idle, 4000, 512);
        assert!(l.iter().all(|x| *x == 0.0));
        assert_eq!(position(&idle)[0], STATE_STOPPED);
    }
}

#[test]
fn the_sound_does_not_depend_on_the_block_size() {
    let file = write_xm(&tune(Rules::FastTracker2));
    let whole = render(&mut playing(&file), 30000, 30000);
    for block in [1, 64, 333, 4096] {
        assert!(render(&mut playing(&file), 30000, block) == whole, "block {block}");
    }
}

#[test]
fn the_position_can_be_read_as_the_song_plays() {
    let file = write_mod(&tune(Rules::ProTracker));
    let mut engine = Engine::new(SR);
    assert_eq!(position(&engine), [0; 10]);
    assert_eq!(load(&mut engine, &file, 100), SONG_OK);
    let mut info = [0u32; 8];
    assert_eq!(unsafe { synths_song_info(&*engine, info.as_mut_ptr()) }, 8);
    assert_eq!(info, [4, 1, 1, 31, 31, 32, 100, 1]);
    assert_eq!(position(&engine), [STATE_STOPPED, 0, 0, 0, 0, 6, 125, 0, 64, 1]);

    // Told to play at frame 1000: the first tick is on that frame.
    engine.push(Event::song(1000.0, SONG_PLAY, 0.0, 0.0));
    let (l, _) = render(&mut engine, 1000 + 882 * 6 * 2 + 10, 256);
    assert!(l[..1000].iter().all(|x| *x == 0.0));
    assert!(l[1000..1100].iter().any(|x| x.abs() > 0.01));
    let p = position(&engine);
    assert_eq!((p[0], p[1], p[3], p[4], p[5], p[6]), (STATE_PLAYING, 0, 2, 0, 6, 125));

    // The song ends at row 15 (a pattern break) and comes round again.
    render(&mut engine, 882 * 6 * 15, 256);
    let p = position(&engine);
    assert_eq!((p[0], p[3], p[7]), (STATE_PLAYING, 1, 1));
}

#[test]
fn pause_holds_the_song_and_play_lets_it_go_on() {
    let file = write_mod(&tune(Rules::ProTracker));
    let mut engine = playing(&file);
    render(&mut engine, 882 * 7, 256);
    engine.push(Event::song(0.0, SONG_PAUSE, 0.0, 0.0));
    render(&mut engine, 2000, 256);
    let held = position(&engine);
    assert_eq!((held[0], held[3], held[4]), (STATE_PAUSED, 1, 1));
    let (l, r) = render(&mut engine, 20000, 256);
    assert!(l.iter().chain(&r).all(|x| x.abs() < 1e-4), "a paused song is silent");
    assert_eq!(position(&engine), held);

    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (l, _) = render(&mut engine, 882 * 5, 256);
    // The notes that were sounding carry on; no new row was needed for sound.
    assert!(l[..800].iter().any(|x| x.abs() > 0.01));
    let p = position(&engine);
    assert_eq!((p[0], p[3]), (STATE_PLAYING, 2));
}

#[test]
fn stop_seek_repeat_and_volume() {
    let file = write_mod(&tune(Rules::ProTracker));
    let mut engine = playing(&file);
    render(&mut engine, 882 * 20, 256);
    engine.push(Event::song(0.0, SONG_STOP, 0.0, 0.0));
    let (l, _) = render(&mut engine, 3000, 256);
    assert!(l[2000..].iter().all(|x| x.abs() < 1e-4));
    let p = position(&engine);
    assert_eq!(p[0], STATE_STOPPED);

    // Seek while stopped, then play: it starts from there.
    engine.push(Event::song(0.0, SONG_SEEK, 0.0, 8.0));
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    render(&mut engine, 100, 100);
    let p = position(&engine);
    assert_eq!((p[0], p[1], p[3], p[4]), (STATE_PLAYING, 0, 8, 0));

    // With repeat off the song stops at its end, on the last row.
    engine.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
    render(&mut engine, 882 * 6 * 10, 256);
    let p = position(&engine);
    assert_eq!((p[0], p[3], p[7]), (STATE_STOPPED, 15, 1));
    // Play starts it from the top again.
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    render(&mut engine, 882, 256);
    let p = position(&engine);
    assert_eq!((p[0], p[3]), (STATE_PLAYING, 0));

    // Volume scales the whole song.
    let level = |volume: f32| {
        let mut engine = playing(&file);
        engine.push(Event::song(0.0, SONG_VOLUME, volume, 0.0));
        let (l, _) = render(&mut engine, 882 * 12, 256);
        loudness(&l[4000..], 6000)[0]
    };
    let (full, half) = (level(1.0), level(0.5));
    assert!((half / full - 0.5).abs() < 0.02, "{half} against {full}");
}

#[test]
fn a_chord_becomes_the_engines_arpeggio_and_a_patch_becomes_a_note_on_its_part() {
    let text = r#"{"format": "synths-song", "version": 1, "channels": 2, "order": [0],
      "patterns": [{"rows": 8, "events": [
        {"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": [0, 12], "chordHz": 100},
        {"row": 4, "ch": 1, "note": "A-4", "ins": 2},
        {"row": 6, "ch": 1, "note": "off"}
      ]}],
      "instruments": [{"sample": 0}, {"name": "lead", "patch": "my-lead"}],
      "samples": [{"wave": [100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
                            -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100,
                            100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100,
                            -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100, -100],
                   "loop": {}}]}"#;
    let mut engine = Engine::new(SR);
    assert_eq!(load(&mut engine, text.as_bytes(), 0), SONG_OK, "{}", error());
    let mut name = [0u8; 16];
    assert_eq!(unsafe { synths_song_patch_name(&*engine, 2, name.as_mut_ptr(), 16) }, 7);
    assert_eq!(&name[..7], b"my-lead");
    assert_eq!(unsafe { synths_song_patch_name(&*engine, 1, name.as_mut_ptr(), 16) }, 0);
    // The host says which part plays the patch, and sets that part up.
    engine.push(Event::song(0.0, SONG_PATCH_PART, 2.0, 10.0));
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));

    // The chord: 261 Hz and 523 Hz by turns, 441 frames each. Count the
    // periods between upward crossings in the first rows, before the patch.
    let (l, _) = render(&mut engine, 882 * 6 * 4, 256);
    let rises: Vec<usize> = (1..l.len()).filter(|&i| l[i - 1] <= 0.0 && l[i] > 0.0).collect();
    let periods: Vec<usize> = rises.windows(2).map(|w| w[1] - w[0]).collect();
    let near = |want: f32| periods.iter().filter(|&&p| (p as f32 - want).abs() < 4.0).count();
    assert!(near(168.75) > 20 && near(84.4) > 40, "{periods:?}");
    assert_eq!(engine.active_voices(), 1);

    // The patch: a note on part 10 at A-4 (note 69), let go at row 6.
    render(&mut engine, 882 * 3, 256);
    assert_eq!(engine.active_voices(), 2);
    let mut notes = [0u32; 8];
    assert_eq!(unsafe { synths_core::abi::synths_notes_read(&*engine, 10, notes.as_mut_ptr()) }, 8);
    assert_ne!(notes[4 + 69 / 32] & (1 << (69 % 32)), 0, "note 69 is sounding on part 10: {notes:?}");
    render(&mut engine, 882 * 6 * 3, 256);
    assert_eq!(unsafe { synths_core::abi::synths_notes_read(&*engine, 10, notes.as_mut_ptr()) }, 8);
    assert_eq!(notes[69 / 32] & (1 << (69 % 32)), 0, "note 69 is no longer held");
}

#[test]
fn songs_that_cannot_be_loaded_say_why() {
    let _turn = take_turn();
    let mut engine = Engine::new(SR);
    assert_eq!(load_in_turn(&mut engine, b"not a song at all", 0), SONG_BAD);
    assert!(error().contains("too short"), "{}", error());
    let bad = br#"{"format": "synths-song", "version": 1, "tempo": 5}"#;
    assert_eq!(load_in_turn(&mut engine, bad, 0), SONG_BAD);
    assert!(error().contains("\"tempo\" must be a whole number from 32 to 255"), "{}", error());
    let needs = br#"{"format": "synths-song", "version": 1, "samples": [{"file": "samples/a.wav"}]}"#;
    assert_eq!(load_in_turn(&mut engine, needs, 0), SONG_MISSING_FILE);
    assert!(error().contains("samples/a.wav"));
    assert_eq!(position(&engine)[0], STATE_NONE);

    // Given the file, it loads; the file is then forgotten.
    let wav = wav::write(&SampleData::Bits8(vec![1, 2, 3, 4]), 8363);
    let name = "samples/a.wav";
    assert_eq!(unsafe { synths_song_file(name.as_ptr(), name.len() as u32, wav.as_ptr(), wav.len() as u32) }, SONG_OK);
    assert_eq!(load_in_turn(&mut engine, needs, 0), SONG_OK, "{}", error());
    assert_eq!(error(), "");
    assert_eq!(load_in_turn(&mut engine, needs, 0), SONG_MISSING_FILE);

    let file = write_mod(&tune(Rules::ProTracker));
    // A song has parts 32 to 63: 32 channels fit, 33 do not.
    let wide = |channels: usize| write_xm(&song(Rules::FastTracker2, channels, vec![square(32, 1)], vec![pattern(4, &["C-4 01 .. 000"])]));
    assert_eq!(load_in_turn(&mut engine, &wide(33), 0), SONG_NO_PARTS);
    assert!(error().contains("33 channels"), "{}", error());
    assert_eq!(load_in_turn(&mut engine, &wide(32), 0), SONG_OK);
    assert_eq!(load_in_turn(&mut engine, &file, 1000), SONG_NO_SLOTS);
    assert_eq!(unsafe { synths_song_load(core::ptr::null_mut(), file.as_ptr(), 4, 0) }, SONG_NULL);
    assert_eq!(load_in_turn(&mut engine, &file, 993), SONG_OK);
}

#[test]
fn loading_another_song_or_unloading_clears_the_first_away() {
    let mut engine = playing(&write_mod(&tune(Rules::ProTracker)));
    render(&mut engine, 5000, 256);
    assert!(engine.active_voices() > 0);
    assert_eq!(load(&mut engine, &write_xm(&tune(Rules::FastTracker2)), 0), SONG_OK);
    assert_eq!(engine.active_voices(), 0);
    assert_eq!(position(&engine)[0], STATE_STOPPED);
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (l, _) = render(&mut engine, 5000, 256);
    assert!(l.iter().any(|x| x.abs() > 0.01));
    unsafe { synths_song_unload(&mut *engine) };
    assert_eq!(position(&engine), [0; 10]);
    let (l, _) = render(&mut engine, 5000, 256);
    assert!(l[2000..].iter().all(|x| x.abs() < 1e-4));
    // A reset stops the song and takes it back to the start.
    assert_eq!(load(&mut engine, &write_mod(&tune(Rules::ProTracker)), 0), SONG_OK);
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    render(&mut engine, 882 * 14, 256);
    engine.reset();
    assert_eq!(position(&engine)[..5], [STATE_STOPPED, 0, 0, 0, 0]);
}

#[test]
fn the_document_describes_every_song_function_and_the_song_event() {
    let doc = include_str!("../docs/engine-api.md");
    let source = include_str!("../src/song_abi.rs");
    let mut count = 0;
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("pub extern \"C\" fn ").or_else(|| line.strip_prefix("pub unsafe extern \"C\" fn ")) {
            let name = rest.split('(').next().unwrap();
            count += 1;
            assert!(doc.matches(&format!("{name}(")).count() >= 2, "{name} should be in the header and described");
        }
    }
    assert_eq!(count, 9);
    for text in [
        "SONG_OK 0", "SONG_NULL -1", "SONG_BAD -2", "SONG_MISSING_FILE -3", "SONG_NO_PARTS -4", "SONG_NO_SLOTS -5",
        "| 0 | stop |", "| 1 | play |", "| 2 | pause |", "| 3 | seek |", "| 4 | repeat |", "| 5 | volume |", "| 6 | patch part |",
        "fn event(&mut self, event: &Event, voices: &mut Voices<'_>) -> bool",
    ] {
        assert!(doc.contains(text), "the document no longer says: {text}");
    }
    assert_eq!((SONG_STOP, SONG_PLAY, SONG_PAUSE, SONG_SEEK, SONG_REPEAT, SONG_VOLUME, SONG_PATCH_PART), (0, 1, 2, 3, 4, 5, 6));
    assert_eq!((STATE_NONE, STATE_STOPPED, STATE_PLAYING, STATE_PAUSED, POSITION_SIZE, INFO_SIZE), (0, 1, 2, 3, 10, 8));
    assert_eq!(CHANNEL_GAIN, 0.5);
}

/// A sequencer for trying one thing: it runs a closure on each call.
struct Script<F: FnMut(u32, &mut synths_core::sequencer::Voices<'_>) -> u32 + Send>(u32, F);

impl<F: FnMut(u32, &mut synths_core::sequencer::Voices<'_>) -> u32 + Send> synths_core::sequencer::Sequencer for Script<F> {
    fn advance(&mut self, voices: &mut synths_core::sequencer::Voices<'_>) -> u32 {
        self.0 += 1;
        (self.1)(self.0 - 1, voices)
    }
}

/// An engine whose part 0 plays a steady level of 1 from slot 0, with
/// nothing shaping it, and a master volume of 1.
fn steady_engine() -> Box<Engine> {
    use synths_core::params::*;
    let mut engine = Engine::new(SR);
    engine.load_sample(0, &[1.0; 64], 1, SR).unwrap();
    engine.configure_sample(0, 60.0, 1, 0, 64).unwrap();
    for (id, value) in [(AMP_ATTACK, 0.0), (AMP_DECAY, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.0), (SAMPLE_ANTIALIAS, 0.0), (FILTER_TYPE, 0.0)] {
        engine.set_param(0, id, value);
    }
    engine.set_param(0, MASTER_VOLUME, 1.0);
    engine.set_param(0, LIMITER, 0.0);
    engine
}

fn hit(volume: f32, pan: f32) -> synths_core::sequencer::Trigger {
    synths_core::sequencer::Trigger { slot: 0, position: 0.0, rate: SR, volume, pan }
}

#[test]
fn a_voices_volume_and_pan_can_be_slid_over_a_set_time() {
    // Start hard left at full volume; at frame 1000 slide the volume to
    // nothing over 4000 frames; at 6000 jump it back; at 7000 slide the pan
    // to hard right over 2000.
    let mut engine = steady_engine();
    engine.set_sequencer(Some(Box::new(Script(0, |call, v: &mut synths_core::sequencer::Voices<'_>| {
        match call {
            0 => {
                v.trigger(0, 1, &hit(1.0, -1.0));
                1000
            }
            1 => {
                v.ramp_volume(0, 1, 0.0, 4000);
                5000
            }
            2 => {
                v.ramp_volume(0, 1, 1.0, 0);
                1000
            }
            3 => {
                v.ramp_pan(0, 1, 1.0, 2000);
                100_000
            }
            _ => 100_000,
        }
    }))));
    let (l, r) = render(&mut engine, 10000, 256);
    let near = |x: f32, want: f32| (x - want).abs() < 0.02;
    assert!(near(l[500], 1.0) && near(r[500], 0.0));
    // A straight line down: a quarter, a half and three quarters of the way.
    assert!(near(l[2000], 0.75) && near(l[3000], 0.5) && near(l[4000], 0.25), "{} {} {}", l[2000], l[3000], l[4000]);
    assert!(near(l[5100], 0.0));
    assert!(near(l[6020], 1.0), "0 frames is at once: {}", l[6020]);
    // The pan crosses the middle half way, where each side is 0.7071.
    assert!(near(l[8000], 0.7071) && near(r[8000], 0.7071), "{} {}", l[8000], r[8000]);
    assert!(near(l[9100], 0.0) && near(r[9100], 1.0));
    assert!(l.windows(2).all(|w| (w[1] - w[0]).abs() < 0.08), "no steps but the jump back");
}

#[test]
fn a_ramp_on_the_frame_a_voice_starts_is_its_starting_value() {
    let mut engine = steady_engine();
    engine.set_sequencer(Some(Box::new(Script(0, |call, v: &mut synths_core::sequencer::Voices<'_>| {
        if call == 0 {
            v.trigger(0, 1, &hit(1.0, -1.0));
            v.ramp_volume(0, 1, 0.25, 4000);
            v.ramp_pan(0, 1, 1.0, 4000);
        }
        100_000
    }))));
    let (l, r) = render(&mut engine, 200, 64);
    assert!(l[..200].iter().all(|x| x.abs() < 1e-6));
    assert!(r[..200].iter().all(|x| (x - 0.25).abs() < 1e-3), "{}", r[0]);
}

#[test]
fn a_part_can_pan_as_fasttracker_did() {
    use synths_core::params::PAN_LAW;
    let sides = |law: f32, pan: f32| {
        let mut engine = steady_engine();
        engine.set_param(0, PAN_LAW, law);
        engine.set_sequencer(Some(Box::new(Script(0, move |call, v: &mut synths_core::sequencer::Voices<'_>| {
            if call == 0 {
                v.trigger(0, 1, &hit(1.0, pan));
            }
            100_000
        }))));
        let (l, r) = render(&mut engine, 100, 100);
        (l[50], r[50])
    };
    let near = |got: (f32, f32), want: (f32, f32)| (got.0 - want.0).abs() < 0.002 && (got.1 - want.1).abs() < 0.002;
    // Square root: three quarters of the way right is root 0.25 and root 0.75.
    assert!(near(sides(1.0, 0.5), (0.5, 0.8660)), "{:?}", sides(1.0, 0.5));
    assert!(near(sides(1.0, 0.0), (0.7071, 0.7071)));
    assert!(near(sides(1.0, -1.0), (1.0, 0.0)));
    // Equal power, as it always was: the cosine and sine of 67.5 degrees.
    assert!(near(sides(0.0, 0.5), (0.3827, 0.9239)), "{:?}", sides(0.0, 0.5));
    assert!(near(sides(0.0, 0.0), (0.7071, 0.7071)));
}

#[test]
fn a_song_leaves_the_hosts_own_parts_alone() {
    use synths_core::params::{AMP_SUSTAIN, VOLUME};
    let mut engine = Engine::new(SR);
    // The host has a sound of its own on part 3, with settings of its own.
    engine.set_param(3, VOLUME, 0.8);
    engine.set_param(3, AMP_SUSTAIN, 0.6);
    let before: Vec<f32> = (0..32u16).flat_map(|part| (0..128).map(move |id| (part, id))).map(|(part, id)| engine.param(part as usize, id)).collect();
    assert_eq!(load(&mut engine, &write_mod(&tune(Rules::ProTracker)), 0), SONG_OK);
    engine.push(Event::note_on(0.0, 3, 77, 60.0, 1.0));
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    render(&mut engine, 882 * 8, 256);
    engine.push(Event::song(0.0, SONG_VOLUME, 0.3, 0.0));
    engine.push(Event::song(0.0, SONG_SEEK, 0.0, 4.0));
    render(&mut engine, 882 * 8, 256);
    engine.push(Event::song(0.0, SONG_STOP, 0.0, 0.0));
    render(&mut engine, 4000, 256);
    // The host's note is still held, alone, and nothing of parts 0 to 31 changed.
    assert_eq!(engine.active_voices(), 1);
    let after: Vec<f32> = (0..32u16).flat_map(|part| (0..128).map(move |id| (part, id))).map(|(part, id)| engine.param(part as usize, id)).collect();
    assert!(before == after);
    assert!((engine.param(32, VOLUME) - 0.3 * CHANNEL_GAIN).abs() < 1e-6);
    unsafe { synths_song_unload(&mut *engine) };
    assert_eq!(engine.active_voices(), 1);
}

#[test]
fn any_song_can_be_packed_and_the_packed_song_loaded() {
    let mut engine = Engine::new(SR);
    for file in [write_mod(&tune(Rules::ProTracker)), write_xm(&tune(Rules::FastTracker2)), include_bytes!("../examples/songs/first-steps.json").to_vec()] {
        let packed = {
            let _turn = take_turn();
            let length = unsafe { synths_song_pack(file.as_ptr(), file.len() as u32) };
            assert!(length > 0);
            unsafe { core::slice::from_raw_parts(synths_song_packed(), length as usize) }.to_vec()
        };
        assert!(binary::is_binary(&packed));
        // Loaded packed, it is the same song and sounds the same.
        let whole = render(&mut playing(&file), 20000, 512);
        assert_eq!(load(&mut engine, &packed, 0), SONG_OK, "{}", error());
        engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        let mut fresh = Engine::new(SR);
        assert_eq!(load(&mut fresh, &packed, 0), SONG_OK);
        fresh.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        // (A tracker module is played raw; packed, it is played as one of our own.)
        let again = render(&mut fresh, 20000, 512);
        let alike = likeness(&loudness(&whole.0, 441), &loudness(&again.0, 441));
        assert!(alike > 0.995, "{alike}");
    }
    assert_eq!(unsafe { synths_song_pack(b"rubbish".as_ptr(), 7) }, SONG_BAD);
    assert_eq!(unsafe { synths_song_pack(core::ptr::null(), 7) }, SONG_NULL);
}
