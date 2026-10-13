mod replayer_support;
use replayer_support::*;
use synths_core::replayer::Player;
use synths_core::song::*;

/// The parts of two songs that a module file can carry.
fn assert_same_music(a: &Song, b: &Song) {
    assert_eq!(a.title, b.title);
    assert_eq!(a.rules, b.rules);
    assert_eq!(a.frequency_table, b.frequency_table);
    assert_eq!(a.channel_pan, b.channel_pan);
    assert_eq!(a.orders, b.orders);
    assert_eq!(a.patterns, b.patterns);
    assert_eq!((a.speed, a.tempo), (b.speed, b.tempo));
}

#[test]
fn a_mod_file_loads_as_the_song_it_was_written_from() {
    let song = busy_song(Rules::ProTracker);
    let loaded = modfile::load(&write_mod(&song)).unwrap();
    assert_same_music(&song, &loaded);
    assert_eq!(loaded.instruments.len(), 31);
    assert_eq!(loaded.samples.len(), 31);
    assert_eq!(&loaded.samples[..3], &song.samples[..]);
    assert!(loaded.samples[3..].iter().all(|s| s.data.is_empty()));
    assert_eq!(loaded.instruments[1].sample_for(49), Some(1));
    assert_eq!(load_module(&write_mod(&song)).unwrap(), loaded);
}

#[test]
fn the_tag_decides_the_channels_and_the_rules() {
    for (channels, tag, rules) in [
        (4, "M.K.", Rules::ProTracker),
        (4, "M!K!", Rules::ProTracker),
        (4, "FLT4", Rules::ProTracker),
        (4, "4CHN", Rules::FastTracker2),
        (6, "6CHN", Rules::FastTracker2),
        (8, "8CHN", Rules::FastTracker2),
        (8, "CD81", Rules::FastTracker2),
        (8, "OCTA", Rules::FastTracker2),
        (16, "16CH", Rules::FastTracker2),
        (32, "32CH", Rules::FastTracker2),
    ] {
        let lines: Vec<String> = (0..3).map(|row| (0..channels).map(|c| if c == row { "C-4 01 000" } else { "... .. 000" }).collect::<Vec<_>>().join("|")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let song = song(Rules::FastTracker2, channels, vec![square(32, 2)], vec![pattern(64, &lines)]);
        let mut file = write_mod(&song);
        file[1080..1084].copy_from_slice(tag.as_bytes());
        let loaded = modfile::load(&file).unwrap_or_else(|e| panic!("{tag}: {e}"));
        assert_eq!(loaded.channels(), channels, "{tag}");
        assert_eq!(loaded.rules, rules, "{tag}");
        assert_eq!(loaded.patterns[0], song.patterns[0], "{tag}");
        assert_eq!(loaded.frequency_table, FrequencyTable::Amiga);
    }
    let mut file = write_mod(&busy_song(Rules::ProTracker));
    for bad in ["XXXX", "0CHN", "33CH", "\0\0\0\0"] {
        file[1080..1084].copy_from_slice(bad.as_bytes());
        assert_eq!(modfile::load(&file), Err(LoadError::NotRecognised), "{bad:?}");
    }
}

#[test]
fn a_note_outside_the_amiga_range_means_fasttracker_rules() {
    let song = song(Rules::FastTracker2, 4, vec![square(32, 2)], vec![pattern(64, &["C-6 01 000", "C-2 01 000"])]);
    let loaded = modfile::load(&write_mod(&song)).unwrap();
    assert_eq!(loaded.rules, Rules::FastTracker2);
    assert_eq!(loaded.patterns[0].cell(0, 0).note, parse_note("C-6").unwrap());
    assert_eq!(loaded.patterns[0].cell(1, 0).note, parse_note("C-2").unwrap());
}

#[test]
fn startrekker_8_channel_patterns_are_two_patterns_side_by_side() {
    // FLT8 stores channels 1 to 4 of a pattern and then channels 5 to 8, and
    // numbers its orders in halves.
    let left = song(Rules::ProTracker, 4, vec![square(32, 2)], vec![pattern(64, &["C-4 01 000 | ... .. 000 | ... .. 000 | D-4 01 000"])]);
    let right = pattern(64, &["... .. 000 | E-4 01 C20 | ... .. 000 | ... .. 000", "F-4 01 000"]);
    let mut both = left.clone();
    both.patterns.push(right);
    both.orders = vec![0];
    let mut file = write_mod(&both);
    file[1080..1084].copy_from_slice(b"FLT8");
    let loaded = modfile::load(&file).unwrap();
    assert_eq!(loaded.channels(), 8);
    assert_eq!(loaded.patterns.len(), 1);
    let p = &loaded.patterns[0];
    assert_eq!(p.cell(0, 0), cell("C-4 01 000"));
    assert_eq!(p.cell(0, 3), cell("D-4 01 000"));
    assert_eq!(p.cell(0, 5), cell("E-4 01 C20"));
    assert_eq!(p.cell(1, 4), cell("F-4 01 000"));
    assert_eq!(p.events.len(), 4);
}

#[test]
fn a_15_sample_soundtracker_file_is_recognised_by_its_shape() {
    let song = busy_song(Rules::ProTracker);
    let full = write_mod(&song);
    // Cut out samples 16 to 31 and the tag.
    let mut file = full[..20 + 15 * 30].to_vec();
    file.extend_from_slice(&full[950..1080]);
    file.extend_from_slice(&full[1084..]);
    // Soundtracker counts the loop start in bytes.
    file[20 + 2 * 30 + 26..20 + 2 * 30 + 28].copy_from_slice(&64u16.to_be_bytes());
    let loaded = modfile::load(&file).unwrap();
    assert_eq!(loaded.samples.len(), 15);
    assert_eq!(loaded.rules, Rules::ProTracker);
    assert_eq!(loaded.patterns, song.patterns);
    assert_eq!(&loaded.samples[..3], &song.samples[..]);

    // Text is not a module, however long.
    let text = "It was the best of times, it was the worst of times. ".repeat(60);
    assert_eq!(modfile::load(text.as_bytes()), Err(LoadError::NotRecognised));
    assert_eq!(modfile::load(&[0u8; 100]), Err(LoadError::TooShort));
    assert_eq!(modfile::load(&[]), Err(LoadError::TooShort));
}

#[test]
fn an_xm_file_loads_as_the_song_it_was_written_from() {
    for table in [FrequencyTable::Linear, FrequencyTable::Amiga] {
        let mut song = busy_song(Rules::FastTracker2);
        song.frequency_table = table;
        song.speed = 3;
        song.tempo = 150;
        song.restart = 1;
        song.patterns[1].rows = 17;
        let file = write_xm(&song);
        assert!(xm::is_xm(&file));
        let loaded = load_module(&file).unwrap();
        assert_eq!(loaded, song);
    }
}

#[test]
fn xm_header_sizes_are_believed_even_when_unusual() {
    let song = busy_song(Rules::FastTracker2);
    let file = write_xm(&song);
    // Make the first pattern's header 4 bytes longer than usual.
    let at = 60 + 276;
    let mut odd = file[..at].to_vec();
    odd.extend_from_slice(&13u32.to_le_bytes());
    odd.extend_from_slice(&file[at + 4..at + 9]);
    odd.extend_from_slice(&[0xAA; 4]);
    odd.extend_from_slice(&file[at + 9..]);
    assert_eq!(xm::load(&odd).unwrap(), song);
}

#[test]
fn xm_files_that_are_not_playable_are_refused_politely() {
    let file = write_xm(&busy_song(Rules::FastTracker2));
    let mut old = file.clone();
    old[58..60].copy_from_slice(&0x0102u16.to_le_bytes());
    assert!(matches!(xm::load(&old), Err(LoadError::Invalid(_))));
    let mut none = file.clone();
    none[68..70].copy_from_slice(&0u16.to_le_bytes());
    assert!(matches!(xm::load(&none), Err(LoadError::Invalid(_))));
    let mut many = file.clone();
    many[68..70].copy_from_slice(&500u16.to_le_bytes());
    assert!(matches!(xm::load(&many), Err(LoadError::Invalid(_))));
    assert_eq!(xm::load(&file[..40]), Err(LoadError::TooShort));
    assert_eq!(xm::load(b"not an xm"), Err(LoadError::NotRecognised));
}

/// Loads whatever it is given and, if it loads, plays some of it: neither
/// may panic, whatever the bytes are.
fn load_and_play(data: &[u8]) -> bool {
    let Ok(song) = load_module(data) else { return false };
    assert!(song.channels() >= 1 && song.channels() <= MAX_CHANNELS);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    play(&song, &mut player, &mut mixer, 6000);
    player.seek(&song, song.orders.len() - 1, 255, &mut mixer);
    play(&song, &mut player, &mut mixer, 2000);
    assert!(mixer.left.iter().all(|x| x.is_finite()));
    true
}

#[test]
fn a_file_cut_short_anywhere_never_panics() {
    for file in [write_mod(&busy_song(Rules::ProTracker)), write_xm(&busy_song(Rules::FastTracker2))] {
        let mut loaded = 0;
        for len in (0..file.len()).step_by(7).chain(file.len() - 40..file.len()) {
            loaded += load_and_play(&file[..len]) as usize;
        }
        assert!(loaded > 10, "some cut files should still load");
    }
}

#[test]
fn damaged_files_never_panic() {
    let mut random = Random(0x5EED_1234_5678_9ABC);
    for file in [write_mod(&busy_song(Rules::ProTracker)), write_xm(&busy_song(Rules::FastTracker2))] {
        let mut loaded = 0;
        for round in 0..1500 {
            let mut bad = file.clone();
            // A few bytes changed, mostly in the headers, where the damage
            // does most; sometimes whole runs, sometimes a cut as well.
            let changes = 1 + random.below(8);
            for _ in 0..changes {
                let span = if random.below(3) == 0 { bad.len() } else { 1400.min(bad.len()) };
                let at = random.below(span);
                let run = if random.below(5) == 0 { 1 + random.below(16) } else { 1 };
                let fill = [0x00, 0xFF, 0x7F, 0x80, random.below(256) as u8][random.below(5)];
                for b in bad.iter_mut().skip(at).take(run) {
                    *b = if random.below(2) == 0 { fill } else { random.below(256) as u8 };
                }
            }
            if round % 4 == 0 {
                bad.truncate(random.below(bad.len()));
            }
            loaded += load_and_play(&bad) as usize;
        }
        assert!(loaded > 100, "most damaged files still load ({loaded})");
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut random = Random(42);
    for round in 0..300 {
        let mut data: Vec<u8> = (0..random.below(3000)).map(|_| random.below(256) as u8).collect();
        // Half the time, give it a header to get past the first check.
        if round % 2 == 0 && data.len() > 1084 {
            if round % 4 == 0 {
                data[..17].copy_from_slice(b"Extended Module: ");
                data[58..60].copy_from_slice(&0x0104u16.to_le_bytes());
                data[68] = 1 + random.below(32) as u8;
                data[69] = 0;
                data[70..74].copy_from_slice(&[random.below(8) as u8, 0, random.below(8) as u8, 0]);
            } else {
                data[1080..1084].copy_from_slice(b"M.K.");
            }
        }
        load_and_play(&data);
    }
}

#[test]
fn note_names_read_and_write() {
    assert_eq!(note_name(1), "C-0");
    assert_eq!(note_name(49), "C-4");
    assert_eq!(note_name(96), "B-7");
    assert_eq!(note_name(NOTE_OFF), "off");
    for n in 1..=97 {
        assert_eq!(parse_note(&note_name(n)), Some(n));
    }
    assert_eq!(parse_note("c4"), Some(49));
    assert_eq!(parse_note("Db4"), Some(50));
    assert_eq!(parse_note("H-4"), None);
    assert_eq!(parse_note("C-9"), None);
    assert_eq!(parse_note(""), None);
}

#[test]
fn a_pattern_keeps_its_events_in_order() {
    let mut p = Pattern::new(8);
    p.set(3, 1, cell("C-4 01 000"));
    p.set(0, 2, cell("D-4 01 000"));
    p.set(3, 0, cell("E-4 01 000"));
    p.set(0, 2, cell("F-4 01 000"));
    assert_eq!(p.events.iter().map(|e| (e.row, e.channel)).collect::<Vec<_>>(), [(0, 2), (3, 0), (3, 1)]);
    assert_eq!(p.cell(0, 2), cell("F-4 01 000"));
    assert_eq!(p.row(3).len(), 2);
    assert_eq!(p.row(1).len(), 0);
    p.set(3, 0, Cell::default());
    assert_eq!(p.row(3).len(), 1);
    assert!(p.cell(7, 7).is_empty());
}
