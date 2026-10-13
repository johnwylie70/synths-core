mod replayer_support;
use replayer_support::*;
use synths_core::song::native::{self, Saved};
use synths_core::song::*;

fn load_saved(saved: &Saved) -> Result<Song, LoadError> {
    native::load(&saved.json, &|name| saved.blobs.iter().find(|b| b.name == name).map(|b| b.data.clone()))
}

fn no_files(_: &str) -> Option<Vec<u8>> {
    None
}

#[test]
fn a_saved_song_loads_back_exactly() {
    for rules in [Rules::ProTracker, Rules::FastTracker2] {
        let mut song = busy_song(rules);
        song.restart = 1;
        song.instruments.push(Instrument { name: "lead".to_string(), patch: Some("my-lead".to_string()), ..Default::default() });
        song.patterns[0].set(9, 2, Cell { note: 61, instrument: 4, volume: 0x05, effect: 35, param: 0xFE });
        let saved = native::save(&song);
        assert!(native::is_native(&saved.json));
        assert_eq!(load_saved(&saved).unwrap(), song, "{}", saved.json);
        // Saving what was loaded gives the same text: nothing drifts.
        assert_eq!(native::save(&load_saved(&saved).unwrap()), saved);
    }
}

#[test]
fn modules_survive_being_saved_as_songs() {
    for file in [write_mod(&busy_song(Rules::ProTracker)), write_xm(&busy_song(Rules::FastTracker2))] {
        let song = load_module(&file).unwrap();
        assert_eq!(load_saved(&native::save(&song)).unwrap(), song);
    }
}

#[test]
fn small_samples_are_written_in_the_json_and_big_ones_as_files() {
    let mut song = busy_song(Rules::FastTracker2);
    song.samples.push(song.samples[1].clone());
    let saved = native::save(&song);
    // The square waves are 256 frames; the 600-frame 16-bit hit is a file,
    // and its copy shares that file.
    assert_eq!(saved.blobs.len(), 1);
    let blob = &saved.blobs[0];
    assert!(blob.name.starts_with("samples/") && blob.name.ends_with(".wav") && blob.name.len() == 8 + 16 + 4);
    assert_eq!(&blob.name[8..24], &sha256::hex(&sha256::sha256(&blob.data))[..16]);
    assert_eq!(saved.json.matches(&blob.name).count(), 2);
    assert_eq!(saved.json.matches("\"wave\": [").count(), 2);
    assert_eq!(wav::read(&blob.data).unwrap(), (song.samples[1].data.clone(), 8363));

    assert_eq!(native::load(&saved.json, &no_files), Err(LoadError::MissingSample(blob.name.clone())));
}

#[test]
fn a_song_written_by_hand_loads() {
    let text = r#"{
      "format": "synths-song",
      "version": 1,
      "title": "Three Notes",
      "channels": 2,
      "order": [0, 0],
      "patterns": [
        {"rows": 16, "events": [
          {"row": 0, "ch": 0, "note": "C-4", "ins": 1},
          {"row": 4, "ch": 0, "note": "E-4", "ins": 1, "vol": 32},
          {"row": 8, "ch": 0, "note": 56, "ins": 1, "vol": "+2", "fx": "A04"},
          {"row": 8, "ch": 1, "note": "C-3", "ins": 2, "fx": [12, 48]},
          {"row": 12, "ch": 0, "note": "off"}
        ]}
      ],
      "instruments": [
        {"name": "beep", "sample": 0},
        {"name": "my synth", "patch": "wobble-bass"}
      ],
      "samples": [
        {"name": "square", "wave": [100, 100, 100, 100, -100, -100, -100, -100], "loop": {"start": 0, "length": 8}}
      ],
      "something from the future": {"ignored": true}
    }"#;
    let song = native::load(text, &no_files).unwrap();
    assert_eq!(song.title, "Three Notes");
    assert_eq!((song.rules, song.frequency_table), (Rules::FastTracker2, FrequencyTable::Linear));
    assert_eq!((song.speed, song.tempo), (6, 125));
    assert_eq!(song.channel_pan, [128, 128]);
    assert_eq!(song.orders, [0, 0]);
    let p = &song.patterns[0];
    assert_eq!(p.rows, 16);
    assert_eq!(p.cell(0, 0), cell("C-4 01 .. 000"));
    assert_eq!(p.cell(4, 0), cell("E-4 01 30 000"));
    assert_eq!(p.cell(8, 0), cell("G-4 01 72 A04"));
    assert_eq!(p.cell(8, 1), cell("C-3 02 .. C30"));
    assert_eq!(p.cell(12, 0).note, NOTE_OFF);
    assert_eq!(song.instruments[1].patch.as_deref(), Some("wobble-bass"));
    assert!(song.instruments[1].keymap.is_empty());
    assert_eq!(song.samples[0].loop_range(), Some((0, 8)));
    assert_eq!(song.samples[0].volume, 64);
    assert_eq!(song.samples[0].pan, None);
}

#[test]
fn zones_split_the_keyboard_between_samples() {
    let text = r#"{"format": "synths-song", "version": 1,
      "instruments": [{"name": "kit", "zones": [
        {"from": "C-0", "to": "B-3", "sample": 0}, {"from": "C-5", "to": "C-5", "sample": 1}]}],
      "samples": [{"wave": [1, 2]}, {"wave": [3, 4]}]}"#;
    let song = native::load(text, &no_files).unwrap();
    let kit = &song.instruments[0];
    assert_eq!(kit.sample_for(parse_note("C-0").unwrap()), Some(0));
    assert_eq!(kit.sample_for(parse_note("B-3").unwrap()), Some(0));
    assert_eq!(kit.sample_for(parse_note("C-4").unwrap()), None);
    assert_eq!(kit.sample_for(parse_note("C-5").unwrap()), Some(1));
    assert_eq!(load_saved(&native::save(&song)).unwrap(), song);
}

#[test]
fn a_wav_sample_plays_at_its_own_speed_on_c4_unless_told_otherwise() {
    let sound = SampleData::Bits16((0..1000).map(|i| (i * 30) as i16).collect());
    let file = wav::write(&sound, 16726);
    let text = |extra: &str| {
        format!(r#"{{"format": "synths-song", "version": 1, "samples": [{{"name": "voice", "file": "samples/voice.wav"{extra}}}]}}"#)
    };
    let files = |name: &str| (name == "samples/voice.wav").then(|| file.clone());
    // 16726 is twice 8363: an octave up.
    let song = native::load(&text(""), &files).unwrap();
    assert_eq!((song.samples[0].relative_note, song.samples[0].finetune), (12, 0));
    assert_eq!(song.samples[0].data, sound);
    let song = native::load(&text(r#", "transpose": -3, "finetune": 16"#), &files).unwrap();
    assert_eq!((song.samples[0].relative_note, song.samples[0].finetune), (-3, 16));
}

#[test]
fn wav_files_of_other_shapes_are_read() {
    // 8-bit stereo: the left channel is taken.
    let mut file = wav::write(&SampleData::Bits8(vec![10, -10, 20, -20, 30, -30]), 22050);
    file[22] = 2;
    assert_eq!(wav::read(&file).unwrap(), (SampleData::Bits8(vec![10, 20, 30]), 22050));
    // 32-bit float.
    let mut float = wav::write(&SampleData::Bits16(vec![0; 4]), 44100);
    float[20] = 3;
    float[34] = 32;
    float[44..52].copy_from_slice(&[0.5f32.to_le_bytes(), (-1.0f32).to_le_bytes()].concat());
    assert_eq!(wav::read(&float).unwrap(), (SampleData::Bits16(vec![16383, -32767]), 44100));
    // An extra chunk before the format is skipped.
    let plain = wav::write(&SampleData::Bits8(vec![1, 2, 3]), 8000);
    let mut extra = plain[..12].to_vec();
    extra.extend_from_slice(b"LIST\x03\0\0\0abc\0");
    extra.extend_from_slice(&plain[12..]);
    assert_eq!(wav::read(&extra).unwrap(), (SampleData::Bits8(vec![1, 2, 3]), 8000));
    for bad in [&b""[..], b"RIFF", &plain[..20], &plain[..36], b"RIFF\0\0\0\0WAVEdata\x04\0\0\0abcd"] {
        assert!(wav::read(bad).is_err());
    }
}

#[test]
fn mistakes_in_a_song_are_reported_in_words() {
    let wrap = |body: &str| format!(r#"{{"format": "synths-song", "version": 1, {body}}}"#);
    let message = |body: &str| match native::load(&wrap(body), &no_files) {
        Err(LoadError::Invalid(m)) => m,
        other => panic!("{body} gave {other:?}"),
    };
    assert!(message(r#""tempo": 10"#).contains("\"tempo\" must be a whole number from 32 to 255"));
    assert!(message(r#""channels": 0"#).contains("\"channels\""));
    assert!(message(r#""channels": 2, "pan": [0, 1, 2]"#).contains("more entries"));
    assert!(message(r#""patterns": [{"events": [{"row": 64}]}]"#).contains("\"row\""));
    assert!(message(r#""patterns": [{"events": [{"note": "H-4"}]}]"#).contains("note"));
    assert!(message(r#""patterns": [{"events": [{"vol": 65}]}]"#).contains("\"vol\""));
    assert!(message(r#""patterns": [{"events": [{"vol": "Q1"}]}]"#).contains("\"vol\""));
    assert!(message(r#""patterns": [{"events": [{"fx": "A4"}]}]"#).contains("\"fx\""));
    assert!(message(r#""patterns": [{"events": [{"fx": "é4"}]}]"#).contains("\"fx\""));
    assert!(message(r#""rules": "impulse""#).contains("\"rules\""));
    assert!(message(r#""order": [0, "one"]"#).contains("\"order\""));
    assert!(message(r#""instruments": [{"sample": 0}]"#).contains("\"sample\""));
    assert!(message(r#""samples": [{"wave": [300]}]"#).contains("\"wave\""));
    assert!(message(r#""samples": [{"wave": [1, 2], "loop": {"start": 1, "length": 5}}]"#).contains("\"length\""));
    assert!(message(r#""instruments": [{"volumeEnvelope": {"points": [[0, 99]]}}]"#).contains("envelope point"));
    assert!(message(r#""title": 5"#).contains("\"title\" must be text"));

    assert_eq!(native::load(r#"{"format": "something-else"}"#, &no_files), Err(LoadError::NotRecognised));
    assert_eq!(native::load(r#"[1, 2]"#, &no_files), Err(LoadError::NotRecognised));
    assert!(matches!(native::load("{", &no_files), Err(LoadError::Invalid(_))));
    let newer = r#"{"format": "synths-song", "version": 2}"#;
    assert!(matches!(native::load(newer, &no_files), Err(LoadError::Invalid(m)) if m.contains("newer")));
}

#[test]
fn damaged_song_text_never_panics() {
    let saved = native::save(&busy_song(Rules::FastTracker2));
    let text = saved.json.as_bytes();
    let mut random = Random(7);
    let mut loaded = 0;
    for round in 0..2000 {
        let mut bad = text.to_vec();
        for _ in 0..1 + random.below(4) {
            let at = random.below(bad.len());
            match random.below(4) {
                0 => bad[at] = b"0123456789-[]{}\",:. e"[random.below(21)],
                1 => {
                    bad.remove(at);
                }
                2 => bad.insert(at, b"9-[{\"}],"[random.below(8)]),
                _ => bad.truncate(at.max(1)),
            }
        }
        if let Ok(text) = String::from_utf8(bad) {
            if let Ok(song) = load_saved(&Saved { json: text, blobs: saved.blobs.clone() }) {
                loaded += 1;
                let mut player = synths_core::replayer::Player::new(&song, RATE);
                let mut mixer = Mixer::new(&song, RATE);
                play(&song, &mut player, &mut mixer, 3000);
            }
        }
        let _ = round;
    }
    assert!(loaded > 50, "some damage leaves a song that still loads ({loaded})");
}

#[test]
fn the_example_song_loads_and_plays_to_its_end() {
    let text = include_str!("../examples/songs/first-steps.json");
    let song = native::load(text, &no_files).unwrap();
    assert_eq!(song.title, "First Steps");
    assert_eq!((song.channels(), song.orders.len(), song.patterns.len()), (3, 4, 3));
    let mut player = synths_core::replayer::Player::new(&song, RATE);
    player.set_repeat(false);
    let mut mixer = Mixer::new(&song, RATE);
    while !player.ended() {
        play(&song, &mut player, &mut mixer, 1024);
    }
    // Four patterns of 16 rows of 6 ticks of 882 frames, to the nearest block.
    assert!(mixer.left.len().abs_diff(4 * 16 * 6 * 882) <= 1024);
    let peak = mixer.left.iter().chain(&mixer.right).fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05 && peak < 1.0, "peak {peak}");
    // And it survives being saved.
    assert_eq!(native::load(&native::save(&song).json, &no_files).unwrap(), song);
}

#[test]
fn chords_are_written_on_events_by_name_by_offsets_or_by_notes() {
    let text = r#"{"format": "synths-song", "version": 1, "channels": 1,
      "patterns": [{"rows": 16, "events": [
        {"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "min"},
        {"row": 1, "ch": 0, "note": "A-4", "ins": 1, "chord": "Am7", "chordOrder": "down"},
        {"row": 2, "ch": 0, "ins": 1, "chord": "F#maj7", "chordSpeed": 2},
        {"row": 3, "ch": 0, "note": "C-4", "chord": [0, 4, 7, 12], "chordHz": 50, "chordOrder": "updown"},
        {"row": 4, "ch": 0, "note": "C-4", "chord": ["C-4", "E-4", "G-4", "C-3"], "chordOrder": "written"},
        {"row": 5, "ch": 0, "chord": "sus4", "chordOrder": "random"},
        {"row": 6, "ch": 0, "chord": "off"}
      ]}],
      "instruments": [{"sample": 0}], "samples": [{"wave": [50, -50], "loop": {}}]}"#;
    let song = native::load(text, &no_files).unwrap();
    let p = &song.patterns[0];
    let chord = |row: u16| p.chord(row, 0).unwrap();
    assert_eq!(chord(0), Chord::new(&[0, 3, 7]));
    assert_eq!(chord(1), Chord { order: ChordOrder::Down, ..Chord::new(&[0, 3, 7, 10]) });
    // A name with a root gives the event its note when it has none.
    assert_eq!(p.cell(2, 0).note, parse_note("F#4").unwrap());
    assert_eq!(chord(2), Chord { rate: ChordRate::PerTick(2.0), ..Chord::new(&[0, 4, 7, 11]) });
    assert_eq!(chord(3), Chord { rate: ChordRate::PerSecond(50.0), order: ChordOrder::UpDown, ..Chord::new(&[0, 4, 7, 12]) });
    assert_eq!(chord(4), Chord { order: ChordOrder::Written, ..Chord::new(&[0, 4, 7, -12]) });
    assert!(p.cell(5, 0).is_empty());
    assert_eq!(chord(5).offsets(), [0, 5, 7]);
    assert!(chord(6).is_off());

    // Saved, they are offsets, and load back as the same song.
    let saved = native::save(&song);
    assert!(saved.json.contains(r#""chord": [0, 4, 7, 12], "chordHz": 50, "chordOrder": "updown""#), "{}", saved.json);
    assert!(saved.json.contains(r#"{"row": 6, "ch": 0, "chord": "off"}"#));
    assert_eq!(load_saved(&saved).unwrap(), song);

    let wrap = |event: &str| format!(r#"{{"format": "synths-song", "version": 1, "patterns": [{{"events": [{event}]}}]}}"#);
    for bad in [
        r#"{"chord": "Xm"}"#,
        r#"{"chord": []}"#,
        r#"{"chord": [0, 1, 2, 3, 4, 5, 6, 7, 8]}"#,
        r#"{"chord": ["C-4", "E-4"]}"#,
        r#"{"note": "D-4", "chord": "Cm"}"#,
        r#"{"chord": "min", "chordSpeed": 2, "chordHz": 50}"#,
        r#"{"chord": "min", "chordOrder": "sideways"}"#,
        r#"{"chord": "min", "chordHz": 0}"#,
    ] {
        assert!(matches!(native::load(&wrap(bad), &no_files), Err(LoadError::Invalid(_))), "{bad}");
    }
}

#[test]
fn chord_names() {
    assert_eq!(Chord::named("maj"), Some((None, &[0i8, 4, 7][..])));
    assert_eq!(Chord::named("C"), Some((Some(0), &[0i8, 4, 7][..])));
    assert_eq!(Chord::named("Cm7"), Some((Some(0), &[0i8, 3, 7, 10][..])));
    assert_eq!(Chord::named("Bb7"), Some((Some(10), &[0i8, 4, 7, 10][..])));
    assert_eq!(Chord::named("Adim"), Some((Some(9), &[0i8, 3, 6][..])));
    assert_eq!(Chord::named("dim"), Some((None, &[0i8, 3, 6][..])));
    assert_eq!(Chord::named("add9"), Some((None, &[0i8, 4, 7, 14][..])));
    assert_eq!(Chord::named("m"), Some((None, &[0i8, 3, 7][..])));
    assert_eq!(Chord::named("M7"), Some((None, &[0i8, 4, 7, 11][..])));
    assert_eq!(Chord::named(""), None);
    assert_eq!(Chord::named("Hm"), None);
    assert_eq!(Chord::from_effect(0x47), Chord { order: ChordOrder::Written, ..Chord::new(&[0, 4, 7]) });
}
