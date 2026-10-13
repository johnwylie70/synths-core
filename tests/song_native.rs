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
    // 8-bit stereo: the two channels are mixed to one, each counting half.
    let mut file = wav::write(&SampleData::Bits8(vec![10, -10, 20, 40, -30, -50]), 22050);
    file[22] = 2;
    assert_eq!(wav::read(&file).unwrap(), (SampleData::Bits8(vec![0, 30, -40]), 22050));
    // 16-bit stereo, and 24-bit, whose top 16 bits are kept.
    let mut wide = wav::write(&SampleData::Bits16(vec![1000, 3000, -2000, -4000]), 44100);
    wide[22] = 2;
    assert_eq!(wav::read(&wide).unwrap(), (SampleData::Bits16(vec![2000, -3000]), 44100));
    let mut deep = wav::write(&SampleData::Bits16(vec![0; 3]), 44100);
    deep[34] = 24;
    deep[44..50].copy_from_slice(&[0xFF, 0x34, 0x12, 0x00, 0x00, 0x80]);
    assert_eq!(wav::read(&deep).unwrap(), (SampleData::Bits16(vec![0x1234, -32768]), 44100));
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
    let newer = r#"{"format": "synths-song", "version": 3}"#;
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
fn in_a_version_1_song_a_chord_is_the_fast_arpeggio_by_name_by_offsets_or_by_notes() {
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
    assert_eq!(chord(0), Chord::arp(&[0, 3, 7], Arp::FAST));
    assert_eq!(chord(1), Chord::arp(&[0, 3, 7, 10], Arp { order: ArpOrder::Down, ..Arp::FAST }));
    // A name with a root gives the event its note when it has none.
    assert_eq!(p.cell(2, 0).note, parse_note("F#4").unwrap());
    assert_eq!(chord(2), Chord::arp(&[0, 4, 7, 11], Arp { rate: ArpRate::PerTick(2.0), ..Arp::FAST }));
    assert_eq!(chord(3), Chord::arp(&[0, 4, 7, 12], Arp { rate: ArpRate::PerSecond(50.0), order: ArpOrder::UpDown, ..Arp::FAST }));
    assert_eq!(chord(4), Chord::arp(&[0, 4, 7, -12], Arp { order: ArpOrder::Written, ..Arp::FAST }));
    assert!(p.cell(5, 0).is_empty());
    assert_eq!(chord(5).offsets(), [0, 5, 7]);
    assert!(chord(6).is_off());

    // Saved, they are arpeggios as version 2 writes them, with offsets
    // for notes, and load back as the same song.
    let saved = native::save(&song);
    assert!(saved.json.contains(r#""version": 2,"#), "{}", saved.json);
    assert!(saved.json.contains(r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": [0, 3, 7], "perTick": 1}}"#), "{}", saved.json);
    assert!(saved.json.contains(r#""arp": {"chord": [0, 4, 7, 12], "hz": 50, "order": "updown"}"#), "{}", saved.json);
    assert!(saved.json.contains(r#"{"row": 6, "ch": 0, "arp": "off"}"#));
    assert!(!saved.json.contains("chordHz") && !saved.json.contains("chordOrder") && !saved.json.contains("chordSpeed"));
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

/// A song of one pattern with these events, in version 2 unless it says otherwise.
fn with_events(events: &str) -> String {
    format!(r#"{{"format": "synths-song", "version": 2, "channels": 2, "patterns": [{{"rows": 64, "events": [{events}]}}], "instruments": [{{"sample": 0}}], "samples": [{{"wave": [50, -50], "loop": {{}}}}]}}"#)
}

#[test]
fn a_chord_is_its_notes_together_and_an_arp_is_its_notes_one_at_a_time() {
    let text = with_events(
        r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "m7"},
        {"row": 1, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "order": "up", "octaves": 2, "gate": 0.5}},
        {"row": 2, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "hz": 50}},
        {"row": 3, "ch": 0, "ins": 1, "chord": "Gm"},
        {"row": 4, "ch": 0, "note": "D-4", "arp": {"chord": ["D-4", "F-4", "A-4"]}},
        {"row": 5, "ch": 0, "arp": {"chord": "F#sus4", "rate": "1/8t", "order": "updown2", "style": "slide", "swing": 0.25}},
        {"row": 6, "ch": 0, "arp": {"chord": [0, 7], "perRow": 2, "order": "together", "gate": 0.25, "accent": [15, 0, 8]}},
        {"row": 7, "ch": 0, "arp": {"chord": [0, 7], "perTick": 0.5, "style": "strike", "order": "random"}},
        {"row": 8, "ch": 0, "arp": {"chord": [0, 12], "rate": "1/4.", "order": "down"}},
        {"row": 9, "ch": 0, "arp": "off"},
        {"row": 10, "ch": 0, "chord": "off"},
        {"row": 11, "ch": 0, "note": "C-4", "chord": "maj", "chordHz": 50},
        {"row": 12, "ch": 0, "note": "C-4", "chord": "maj", "chordOrder": "down"}"#,
    );
    let song = native::load(&text, &no_files).unwrap();
    let p = &song.patterns[0];
    let chord = |row: u16| p.chord(row, 0).unwrap();
    let note = |division, feel| ArpRate::Note(NoteLength { division, feel });
    let strike = Arp::SIXTEENTHS;
    assert_eq!(strike, Arp { rate: note(16, Feel::Plain), order: ArpOrder::Up, style: ArpStyle::Strike, gate: 0.5, octaves: 1, swing: 0.0, accents: 0 });
    // The four notes together.
    assert_eq!(chord(0), Chord::new(&[0, 3, 7, 10]));
    assert_eq!(chord(0).arp, None);
    // A trance arpeggio in sixteenths over two octaves.
    assert_eq!(chord(1), Chord::arp(&[0, 3, 7, 10], Arp { octaves: 2, ..strike }));
    // The Commodore 64's chord: timed in seconds, so one voice unless it says otherwise.
    assert_eq!(chord(2), Chord::arp(&[0, 4, 7], Arp { rate: ArpRate::PerSecond(50.0), ..Arp::FAST }));
    assert!(chord(2).arp.unwrap().plain() && !chord(1).arp.unwrap().plain());
    // A chord's root is the event's note when it has none, for both.
    assert_eq!((chord(3), p.cell(3, 0).note), (Chord::new(&[0, 3, 7]), parse_note("G-4").unwrap()));
    // An arp that says nothing but its notes: sixteenths, up, struck, held half a step.
    assert_eq!(chord(4), Chord::arp(&[0, 3, 7], strike));
    assert_eq!((chord(5), p.cell(5, 0).note), (Chord::arp(&[0, 5, 7], Arp { rate: note(8, Feel::Triplet), order: ArpOrder::UpDownEnds, style: ArpStyle::Slide, swing: 0.25, ..strike }), parse_note("F#4").unwrap()));
    assert_eq!(chord(6), Chord::arp(&[0, 7], Arp { rate: ArpRate::PerRow(2.0), order: ArpOrder::Together, gate: 0.25, accents: 0x80F, ..strike }));
    assert_eq!(chord(7), Chord::arp(&[0, 7], Arp { rate: ArpRate::PerTick(0.5), order: ArpOrder::Random, ..strike }));
    assert_eq!(chord(8), Chord::arp(&[0, 12], Arp { rate: note(4, Feel::Dotted), order: ArpOrder::Down, ..strike }));
    assert!(chord(9).is_off() && chord(10).is_off());
    // The keys of before, beside a chord, still mean what they meant.
    assert_eq!(chord(11), Chord::arp(&[0, 4, 7], Arp { rate: ArpRate::PerSecond(50.0), ..Arp::FAST }));
    assert_eq!(chord(12), Chord::arp(&[0, 4, 7], Arp { order: ArpOrder::Down, ..Arp::FAST }));

    // Saved: the new shape, with what is at its default left out, and the same song again.
    let saved = native::save(&song);
    for line in [
        r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": [0, 3, 7, 10]}"#,
        r#"{"row": 1, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": [0, 3, 7, 10], "octaves": 2}}"#,
        r#"{"row": 2, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": [0, 4, 7], "hz": 50}}"#,
        r#"{"row": 4, "ch": 0, "note": "D-4", "arp": {"chord": [0, 3, 7]}}"#,
        r#""arp": {"chord": [0, 5, 7], "rate": "1/8t", "order": "updown2", "style": "slide", "swing": 0.25}}"#,
        r#"{"row": 6, "ch": 0, "arp": {"chord": [0, 7], "perRow": 2, "order": "together", "gate": 0.25, "accent": [15, 0, 8]}}"#,
        r#"{"row": 7, "ch": 0, "arp": {"chord": [0, 7], "perTick": 0.5, "order": "random", "style": "strike"}}"#,
        r#"{"row": 8, "ch": 0, "arp": {"chord": [0, 12], "rate": "1/4.", "order": "down"}}"#,
        r#"{"row": 9, "ch": 0, "arp": "off"}"#,
        r#"{"row": 11, "ch": 0, "note": "C-4", "arp": {"chord": [0, 4, 7], "hz": 50}}"#,
        r#"{"row": 12, "ch": 0, "note": "C-4", "arp": {"chord": [0, 4, 7], "perTick": 1, "order": "down"}}"#,
    ] {
        assert!(saved.json.contains(line), "{line}\n{}", saved.json);
    }
    assert!(saved.json.contains(r#""version": 2,"#));
    assert_eq!(load_saved(&saved).unwrap(), song);
    // And packed: the same song both ways, in version 2 of the packed form.
    let packed = binary::save(&song);
    assert_eq!(&packed[..6], b"SYSG\x02\x00");
    assert_eq!(binary::load(&packed).unwrap(), song);
    assert_eq!(binary::save(&binary::load(&packed).unwrap()), packed);
}

#[test]
fn the_version_says_what_a_bare_chord_means() {
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "maj"}"#;
    let two = native::load(&with_events(events), &no_files).unwrap();
    assert_eq!(two.patterns[0].chord(0, 0), Some(Chord::new(&[0, 4, 7])));
    for version in [r#""version": 1,"#, ""] {
        let one = native::load(&with_events(events).replace(r#""version": 2,"#, version), &no_files).unwrap();
        assert_eq!(one.patterns[0].chord(0, 0), Some(Chord::arp(&[0, 4, 7], Arp::FAST)));
        // A version 1 song may still use the new key.
        let arp = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj"}}"#;
        let one = native::load(&with_events(arp).replace(r#""version": 2,"#, version), &no_files).unwrap();
        assert_eq!(one.patterns[0].chord(0, 0), Some(Chord::arp(&[0, 4, 7], Arp::SIXTEENTHS)));
    }
    // A song with no chord in it is still written as version 1, and one
    // that only has what version 1 could say still packs as version 1.
    let plain = native::load(&with_events(r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}"#), &no_files).unwrap();
    assert!(native::save(&plain).json.contains(r#""version": 1,"#));
    assert_eq!(&binary::save(&plain)[..6], b"SYSG\x01\x00");
    let old = native::load(&with_events(r#"{"row": 0, "ch": 0, "note": "C-4", "chord": "maj", "chordHz": 60}, {"row": 4, "ch": 0, "arp": "off"}"#), &no_files).unwrap();
    assert_eq!(&binary::save(&old)[..6], b"SYSG\x01\x00");
    assert_eq!(binary::load(&binary::save(&old)).unwrap(), old);
    assert_eq!(&binary::save(&two)[..6], b"SYSG\x02\x00");
}

#[test]
fn every_rate_an_arp_can_have() {
    // Ticks to a step at 6 ticks to the row and 125 beats a minute, where a
    // tick is a fiftieth of a second and a row is a sixteenth note.
    for (rate, ticks) in [
        (r#""rate": "1/1""#, 96.0),
        (r#""rate": "1/2""#, 48.0),
        (r#""rate": "1/4""#, 24.0),
        (r#""rate": "1/8""#, 12.0),
        (r#""rate": "1/16""#, 6.0),
        (r#""rate": "1/32""#, 3.0),
        (r#""rate": "1/64""#, 1.5),
        (r#""rate": "1/4t""#, 16.0),
        (r#""rate": "1/8t""#, 8.0),
        (r#""rate": "1/16t""#, 4.0),
        (r#""rate": "1/8.""#, 18.0),
        (r#""rate": "1/16.""#, 9.0),
        (r#""perRow": 1"#, 6.0),
        (r#""perRow": 3"#, 2.0),
        (r#""perRow": 0.25"#, 24.0),
        (r#""perTick": 1"#, 1.0),
        (r#""perTick": 0.5"#, 2.0),
        (r#""hz": 50"#, 1.0),
        (r#""hz": 12.5"#, 4.0),
    ] {
        let text = with_events(&format!(r#"{{"row": 0, "ch": 0, "note": "C-4", "arp": {{"chord": "maj", {rate}}}}}"#));
        let song = native::load(&text, &no_files).unwrap();
        let arp = song.patterns[0].chord(0, 0).unwrap().arp.unwrap();
        assert!((arp.rate.ticks(6, 125) - ticks).abs() < 1.0e-9, "{rate}: {}", arp.rate.ticks(6, 125));
        // Struck when it is timed as music is, one voice when as a chip's.
        assert_eq!(arp.style == ArpStyle::Strike, rate.contains("rate") || rate.contains("perRow"), "{rate}");
        assert_eq!(load_saved(&native::save(&song)).unwrap(), song, "{rate}");
        assert_eq!(binary::load(&binary::save(&song)).unwrap(), song, "{rate}");
    }
    // The song's speed moves what is counted in rows, and its tempo what is in seconds.
    let sixteenth = ArpRate::Note(NoteLength::parse("1/16").unwrap());
    assert_eq!((sixteenth.ticks(3, 125), sixteenth.ticks(6, 250), ArpRate::PerSecond(50.0).ticks(6, 250)), (3.0, 6.0, 2.0));
    assert_eq!(NoteLength::parse("1/8t").unwrap().name(), "1/8t");
    assert_eq!(NoteLength::parse("1/3"), None);
}

#[test]
fn mistakes_in_a_chord_or_an_arp_are_reported_in_words() {
    let message = |event: &str| match native::load(&with_events(event), &no_files) {
        Err(LoadError::Invalid(m)) => m,
        other => panic!("{event}: {other:?}"),
    };
    assert!(message(r#"{"chord": "maj", "arp": {"chord": "maj"}}"#).contains("not both"));
    assert!(message(r#"{"arp": {"rate": "1/16"}}"#).contains("needs a \"chord\""));
    assert!(message(r#"{"arp": "maj"}"#).contains("\"arp\" must be"));
    assert!(message(r#"{"arp": {"chord": "maj", "rate": "1/5"}}"#).contains("note length"));
    assert!(message(r#"{"arp": {"chord": "maj", "rate": 16}}"#).contains("note length"));
    assert!(message(r#"{"arp": {"chord": "maj", "rate": "1/16", "hz": 50}}"#).contains("not more"));
    assert!(message(r#"{"arp": {"chord": "maj", "perRow": 0}}"#).contains("\"perRow\""));
    assert!(message(r#"{"arp": {"chord": "maj", "hz": 5000}}"#).contains("\"hz\""));
    assert!(message(r#"{"arp": {"chord": "maj", "order": "sideways"}}"#).contains("\"up\", \"down\", \"updown\", \"written\", \"random\", \"updown2\" or \"together\""));
    assert!(message(r#"{"arp": {"chord": "maj", "style": "pluck"}}"#).contains("\"strike\""));
    assert!(message(r#"{"arp": {"chord": "maj", "octaves": 5}}"#).contains("\"octaves\""));
    assert!(message(r#"{"arp": {"chord": "maj", "gate": 0}}"#).contains("\"gate\""));
    assert!(message(r#"{"arp": {"chord": "maj", "swing": 0.9}}"#).contains("\"swing\""));
    assert!(message(r#"{"arp": {"chord": "maj", "accent": [15, 0]}}"#).contains("\"accent\""));
    assert!(message(r#"{"arp": {"chord": "maj", "accent": [16]}}"#).contains("\"accent\""));
    assert!(message(r#"{"arp": {"chord": "Xm"}}"#).contains("\"chord\" must be"));
    assert!(message(r#"{"arp": {"chord": "maj"}, "chordHz": 50}"#).contains("go with \"chord\""));
    assert!(message(r#"{"chord": "maj", "chordOrder": "together"}"#).contains("\"chordOrder\" must be \"up\", \"down\", \"updown\", \"written\" or \"random\""));
}

#[test]
fn the_examples_in_the_document_are_songs_this_reads() {
    let doc = include_str!("../docs/song-format.md");
    let mut whole = 0;
    let mut lines = 0;
    for block in doc.split("```json\n").skip(1) {
        let block = block.split("```").next().unwrap();
        if block.contains("\"format\"") {
            let song = native::load(block, &no_files).unwrap();
            assert_eq!(load_saved(&native::save(&song)).unwrap(), song);
            whole += 1;
        } else if block.starts_with("{\"row\"") {
            let song = native::load(&with_events(block.trim_end().trim_end_matches(',')), &no_files).unwrap_or_else(|e| panic!("{e}: {block}"));
            assert!(song.patterns[0].events.iter().any(|e| e.chord.is_some()), "{block}");
            lines += block.lines().count();
        }
    }
    assert!(whole == 1 && lines >= 4, "{whole} {lines}");
    // The first lines of "Chords and arpeggios" are what the text says they are.
    let at = doc.find("### Chords and arpeggios").unwrap();
    let block = doc[at..].split("```json\n").nth(1).unwrap().split("```").next().unwrap();
    let song = native::load(&with_events(block), &no_files).unwrap();
    let p = &song.patterns[0];
    assert_eq!(p.chord(0, 0), Some(Chord::new(&[0, 3, 7, 10])));
    assert_eq!(p.chord(16, 0), Some(Chord::arp(&[0, 3, 7, 10], Arp { octaves: 2, ..Arp::SIXTEENTHS })));
    assert_eq!(p.chord(32, 0), Some(Chord::arp(&[0, 4, 7], Arp { rate: ArpRate::PerSecond(50.0), ..Arp::FAST })));
    assert_eq!(p.chord(48, 0), Some(Chord::OFF));
    // Every key and every order the document's table gives is one the reader takes.
    for key in ["chord", "rate", "perRow", "perTick", "hz", "order", "style", "gate", "octaves", "swing", "accent"] {
        assert!(doc.contains(&format!("| `{key}` |")), "{key}");
    }
    for order in ["up", "down", "updown", "updown2", "written", "random", "together"] {
        assert!(doc.contains(&format!("`\"{order}\"`")), "{order}");
        native::load(&with_events(&format!(r#"{{"row": 0, "ch": 0, "note": "C-4", "arp": {{"chord": "maj", "order": "{order}"}}}}"#)), &no_files).unwrap();
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
    assert_eq!(Chord::from_effect(0x47), Chord::arp(&[0, 4, 7], Arp { order: ArpOrder::Written, ..Arp::FAST }));
}

#[test]
fn a_packed_song_unpacks_as_the_same_song() {
    for rules in [Rules::ProTracker, Rules::FastTracker2] {
        let mut song = busy_song(rules);
        song.restart = 1;
        song.instruments.push(Instrument { name: "lead".to_string(), patch: Some("my-lead".to_string()), ..Default::default() });
        song.patterns[0].set_chord(0, 0, Some(Chord::arp(&[0, -5, 7, 12], Arp { rate: ArpRate::PerSecond(60.0), order: ArpOrder::UpDown, ..Arp::FAST })));
        song.patterns[0].set_chord(9, 3, Some(Chord::OFF));
        song.instruments[0].keymap[40] = 1;
        song.samples[0].pan = None;
        let packed = binary::save(&song);
        assert!(binary::is_binary(&packed));
        assert_eq!(binary::load(&packed).unwrap(), song);
        assert_eq!(load_module(&packed).unwrap(), song);
        assert_eq!(binary::save(&binary::load(&packed).unwrap()), packed);
        // The packed form holds what the JSON and its sample files hold.
        assert_eq!(binary::load(&binary::save(&load_saved(&native::save(&song)).unwrap())).unwrap(), song);
    }
    // The first bytes are fixed: the magic, version 1, and the flags.
    let packed = binary::save(&busy_song(Rules::ProTracker));
    assert_eq!(&packed[..8], b"SYSG\x01\x00\x03\x00");
    assert_eq!(binary::load(b"SYSG\x03\x00\x00\x00\x06\x7d\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00").map(|_| ()), Err(LoadError::Invalid("this packed song is newer than this player reads".to_string())));
    assert_eq!(binary::load(b"SYSG\x01"), Err(LoadError::TooShort));
    assert_eq!(binary::load(b"nope"), Err(LoadError::NotRecognised));
}

#[test]
fn damaged_packed_songs_never_panic() {
    let packed = binary::save(&busy_song(Rules::FastTracker2));
    let mut random = Random(99);
    let mut loaded = 0;
    for len in (0..packed.len()).step_by(11) {
        loaded += binary::load(&packed[..len]).is_ok() as usize;
    }
    for _ in 0..3000 {
        let mut bad = packed.clone();
        for _ in 0..1 + random.below(6) {
            let span = if random.below(3) == 0 { bad.len() } else { 400 };
            let at = random.below(span);
            bad[at] = [0, 0xFF, 0x7F, random.below(256) as u8][random.below(4)];
        }
        if let Ok(song) = binary::load(&bad) {
            loaded += 1;
            let mut player = synths_core::replayer::Player::new(&song, RATE);
            let mut mixer = Mixer::new(&song, RATE);
            play(&song, &mut player, &mut mixer, 3000);
        }
    }
    assert!(loaded > 1000, "{loaded}");
}

#[test]
fn base64_reads_what_it_should_and_nothing_else() {
    use synths_core::song::base64::{decode, encode};
    // The examples of the standard that defines it.
    for (plain, coded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
        assert_eq!(decode(coded).as_deref(), Some(plain.as_bytes()), "{coded}");
        assert_eq!(encode(plain.as_bytes()), coded);
        // The = signs may be left off.
        assert_eq!(decode(coded.trim_end_matches('=')).as_deref(), Some(plain.as_bytes()));
    }
    let every: Vec<u8> = (0..=255).collect();
    assert_eq!(decode(&encode(&every)), Some(every.clone()));
    // Line breaks and spaces are skipped; the web's alphabet is read too;
    // and so is a data address.
    assert_eq!(decode("Zm9v\r\n YmFy\n").as_deref(), Some(&b"foobar"[..]));
    assert_eq!(decode("-_-_"), decode("+/+/"));
    assert_eq!(decode("data:audio/wav;base64,Zm9v").as_deref(), Some(&b"foo"[..]));
    // Anything else is refused, not guessed at.
    for bad in ["Zm9*", "Z", "Zm9vY", "Zg==Zg", "data:audio/wav;base64", "Zm\u{e9}v"] {
        assert_eq!(decode(bad), None, "{bad}");
    }
}

#[test]
fn a_samples_sound_can_be_in_the_json_itself_as_a_wav_file_in_base64() {
    use synths_core::song::base64::encode;
    let sound = SampleData::Bits16((0..1000).map(|i| (i * 30 - 15000) as i16).collect());
    let file = wav::write(&sound, 16726);
    let text = |data: &str, extra: &str| format!(r#"{{"format": "synths-song", "version": 1, "samples": [{{"name": "voice", "data": "{data}"{extra}}}]}}"#);
    // The same sample as when the WAV is a file beside the song.
    let beside = native::load(r#"{"format": "synths-song", "version": 1, "samples": [{"name": "voice", "file": "v.wav"}]}"#, &|_| Some(file.clone())).unwrap();
    let inside = native::load(&text(&encode(&file), ""), &no_files).unwrap();
    assert_eq!(inside, beside);
    assert_eq!((inside.samples[0].data.clone(), inside.samples[0].relative_note), (sound.clone(), 12));
    // As a data address, with its own tuning and a loop.
    let song = native::load(&text(&format!("data:audio/wav;base64,{}", encode(&file)), r#", "transpose": 0, "loop": {"start": 10}"#), &no_files).unwrap();
    assert_eq!((song.samples[0].relative_note, song.samples[0].loop_range()), (0, Some((10, 1000))));
    // 8-bit, and stereo mixed to one channel.
    let small = wav::write(&SampleData::Bits8(vec![5, 15, -20, -40]), 8363);
    assert_eq!(native::load(&text(&encode(&small), ""), &no_files).unwrap().samples[0].data, SampleData::Bits8(vec![5, 15, -20, -40]));
    let mut stereo = small.clone();
    stereo[22] = 2;
    assert_eq!(native::load(&text(&encode(&stereo), ""), &no_files).unwrap().samples[0].data, SampleData::Bits8(vec![10, -30]));
    // Text that is not base64, and base64 that is not a WAV file, say so.
    let message = |data: &str| native::load(&text(data, ""), &no_files).unwrap_err().to_string();
    assert!(message("not base64!").contains("\"data\""), "{}", message("not base64!"));
    assert!(message(&encode(b"RIFFjunk")).contains("WAV"));
    assert!(native::load(&text("", "").replace("\"\"", "7"), &no_files).is_err());
    // A WAV that claims more sound than it has is read as far as it goes.
    let mut short = file.clone();
    short.truncate(44 + 500);
    assert_eq!(native::load(&text(&encode(&short), ""), &no_files).unwrap().samples[0].data.len(), 250);
}

#[test]
fn a_song_can_be_written_as_one_json_text_and_read_back() {
    for rules in [Rules::ProTracker, Rules::FastTracker2] {
        let mut song = busy_song(rules);
        song.instruments.push(Instrument { name: "lead".to_string(), patch: Some("my-lead".to_string()), synth: Some(r#"{"preset":"lead","set":{"filter.cutoff":800}}"#.to_string()), ..Default::default() });
        let text = native::save_in_one(&song);
        // Nothing beside it: the big sample is in the text, as "data".
        assert!(text.contains("\"data\": \"UklGR") && !text.contains("\"file\""));
        assert_eq!(native::load(&text, &no_files).unwrap(), song);
        // The packed form carries the same song, sound and all.
        assert_eq!(binary::load(&binary::save(&native::load(&text, &no_files).unwrap())).unwrap(), song);
    }
}

#[test]
fn what_a_host_writes_about_a_synth_sound_is_kept_and_not_read() {
    let text = |synth: &str| format!(r#"{{"format": "synths-song", "version": 1, "instruments": [{{"name": "a", "sample": 0}}, {{"name": "lead", "patch": "p", "synth": {synth}}}], "samples": [{{"wave": [1, 2]}}]}}"#);
    let written = r#"{ "preset": "soft lead", "set": { "filter.cutoff": 800, "osc1.wave": "saw", "amp.attack": 0.25, "deep": [1, {"x": null}, true] } }"#;
    let song = native::load(&text(written), &no_files).unwrap();
    // Kept as JSON, closed up, in the order it was written.
    let kept = r#"{"preset":"soft lead","set":{"filter.cutoff":800,"osc1.wave":"saw","amp.attack":0.25,"deep":[1,{"x":null},true]}}"#;
    assert_eq!((song.instruments[0].synth.as_deref(), song.instruments[1].synth.as_deref()), (None, Some(kept)));
    // Through a save, as files or all in one, and through the packed form.
    let saved = native::save(&song);
    assert!(saved.json.contains(&format!("\"synth\": {kept}")));
    assert_eq!(load_saved(&saved).unwrap(), song);
    assert_eq!(native::load(&native::save_in_one(&song), &no_files).unwrap(), song);
    let packed = binary::save(&song);
    assert_eq!(binary::load(&packed).unwrap(), song);
    // A packed song cut short in the middle of it still loads.
    for cut in 1..40 {
        assert!(binary::load(&packed[..packed.len() - cut]).is_ok());
    }
    // A song without any packs to the same bytes as it always did.
    let plain = native::load(&text("null"), &no_files).unwrap();
    assert!(plain.instruments[1].synth.is_none() && !binary::save(&plain).windows(4).any(|w| w == b"SYNT"));
    // It need not be an object: whatever is there is the host's business.
    assert_eq!(native::load(&text(r#""just a name""#), &no_files).unwrap().instruments[1].synth.as_deref(), Some(r#""just a name""#));
}

#[test]
fn keys_the_reader_does_not_know_are_passed_over_wherever_they_are() {
    let plain = r#"{"format": "synths-song", "version": 1, "channels": 2, "order": [0],
      "patterns": [{"rows": 8, "events": [{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "maj"}]}],
      "instruments": [{"name": "beep", "sample": 0, "volumeEnvelope": {"points": [[0, 64], [8, 0]]}, "vibrato": {"depth": 2, "rate": 9}}],
      "samples": [{"wave": [100, -100], "loop": {"start": 0}}]}"#;
    let extra = r##"{"format": "synths-song", "version": 1, "channels": 2, "order": [0], "editor": {"zoom": 2, "tabs": [1, 2]}, "colour": "red",
      "patterns": [{"rows": 8, "name": "intro", "colour": [1, 2, 3], "events": [{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "maj", "selected": true, "id": "e1"}]}],
      "instruments": [{"name": "beep", "sample": 0, "colour": "#ff0000", "folder": null, "volumeEnvelope": {"points": [[0, 64], [8, 0]], "zoom": 3}, "vibrato": {"depth": 2, "rate": 9, "shown": false}}],
      "samples": [{"wave": [100, -100], "loop": {"start": 0, "label": "all"}, "source": {"path": "beep.wav", "trim": [0, 2]}, "notes": "hand drawn"}],
      "tracks": [{"name": "lead", "muted": false}]}"##;
    assert_eq!(native::load(extra, &no_files).unwrap(), native::load(plain, &no_files).unwrap());
}
