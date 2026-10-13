//! FastTracker 2's rules, tested on small modules built here: each is
//! written out as an XM file and loaded back, so the loader is in the loop.

mod replayer_support;
use replayer_support::*;
use synths_core::replayer::{note_of_rate, Player, Source, Start, Voices};
use synths_core::song::*;

/// A module with a looped square wave as instrument 1 and a ramp that plays
/// once as instrument 2. `setup` can change the song before it is written.
fn module_with(channels: usize, patterns: &[&[&str]], setup: impl FnOnce(&mut Song)) -> Song {
    let patterns = patterns.iter().map(|rows| pattern(rows.len().max(8) as u16, rows)).collect();
    let mut song = song(Rules::FastTracker2, channels, vec![square(64, 1), ramp(6000)], patterns);
    setup(&mut song);
    let loaded = load_module(&write_xm(&song)).unwrap();
    assert_eq!(loaded, song);
    loaded
}

fn module(rows: &[&str]) -> Song {
    module_with(2, &[rows], |_| {})
}

fn periods(song: &Song, ticks: usize) -> Vec<i32> {
    track(&run_ticks(song, ticks), 0).iter().map(|t| linear_period(t.0)).collect()
}

fn volumes(song: &Song, ticks: usize) -> Vec<i32> {
    track(&run_ticks(song, ticks), 0).iter().map(|t| volume64(t.1)).collect()
}

/// Panning as the trackers count it, 0 to 255.
fn pans(song: &Song, ticks: usize) -> Vec<i32> {
    track(&run_ticks(song, ticks), 0).iter().map(|t| ((t.2 + 1.0) * 127.5).round() as i32).collect()
}

fn play_ticks(song: &Song, ticks: usize, channel: usize) -> Vec<usize> {
    plays(&run_ticks(song, ticks), channel).iter().map(|p| p.0).collect()
}

#[test]
fn c4_plays_a_sample_at_8363_frames_a_second() {
    let song = module_with(2, &[&["C-4 01 .. 000 | C-5 01 .. 000", "A-4 02 .. 000 | C-4 02 .. 000"]], |s| {
        s.samples[1].relative_note = 12;
        s.samples[1].finetune = 64;
    });
    let ticks = run_ticks(&song, 12);
    assert!((track(&ticks, 0)[0].0 - 8363.0).abs() < 0.01);
    assert!((track(&ticks, 1)[0].0 - 16726.0).abs() < 0.01);
    // The second sample is tuned an octave and half a semitone up.
    let up = |semitones: f64| 8363.0 * (semitones / 12.0).exp2();
    assert!((track(&ticks, 1)[6].0 - up(12.5)).abs() < 0.01);
    assert!((track(&ticks, 0)[6].0 - up(21.5)).abs() < 0.05);
    assert!((note_of_rate(8363.0) - 60.0).abs() < 1e-9);
    assert!((note_of_rate(16726.0) - 72.0).abs() < 1e-9);
}

#[test]
fn the_amiga_table_gives_the_amiga_periods() {
    let song = module_with(2, &[&["C-4 01 .. 000 | A-4 01 .. 000", "... .. .. 110"]], |s| s.frequency_table = FrequencyTable::Amiga);
    let ticks = run_ticks(&song, 12);
    // C-4 is period 1712 (four times ProTracker's 428) and A-4 is 1016.
    assert!((track(&ticks, 0)[0].0 - 8363.0).abs() < 0.01);
    assert!((track(&ticks, 1)[0].0 - 8363.0 * 1712.0 / 1016.0).abs() < 0.01);
    // A slide of 0x10 takes 64 off the period each tick, which is not a
    // fixed musical distance as it is with linear slides.
    assert!((track(&ticks, 0)[8].0 - 8363.0 * 1712.0 / (1712.0 - 128.0)).abs() < 0.01);
}

#[test]
fn slides_coarse_fine_and_extra_fine() {
    // Linear periods: 64 to the semitone. 1xx moves 4 times its number on
    // each tick after the first; E1x the same amount once; X1x a quarter of it.
    let song = module(&["C-4 01 .. 000", "... .. .. 110", "... .. .. 208", "... .. .. E13", "... .. .. E25", "... .. .. X11", "... .. .. X23"]);
    let p = periods(&song, 42);
    assert_eq!(&p[..6], [4608; 6]);
    assert_eq!(&p[6..12], [4608, 4544, 4480, 4416, 4352, 4288]);
    assert_eq!(&p[12..18], [4288, 4320, 4352, 4384, 4416, 4448]);
    assert_eq!(&p[18..24], [4436; 6]);
    assert_eq!(&p[24..30], [4456; 6]);
    assert_eq!(&p[30..36], [4455; 6]);
    assert_eq!(&p[36..42], [4458; 6]);
}

#[test]
fn slides_remember_their_own_last_number_each() {
    let song = module(&["C-4 01 .. 102", "... .. .. 201", "... .. .. 100", "... .. .. 200"]);
    let p = periods(&song, 24);
    assert_eq!(p[5], 4608 - 5 * 8);
    assert_eq!(p[11], p[5] + 5 * 4);
    assert_eq!(p[17], p[11] - 5 * 8);
    assert_eq!(p[23], p[17] + 5 * 4);
}

#[test]
fn tone_portamento_glides_to_the_note_without_starting_it() {
    let song = module(&["C-4 01 .. 000", "E-4 01 .. 310", "... .. .. 300", "C-4 .. F1 000"]);
    let ticks = run_ticks(&song, 24);
    assert_eq!(plays(&ticks, 0).len(), 1);
    let p: Vec<i32> = track(&ticks, 0).iter().map(|t| linear_period(t.0)).collect();
    // A semitone a tick, four semitones to go.
    assert_eq!(&p[6..12], [4608, 4544, 4480, 4416, 4352, 4352]);
    assert_eq!(&p[12..18], [4352; 6]);
    // The volume column's portamento: speed 1 there is 16 here, so 64 a tick.
    assert_eq!(&p[18..24], [4352, 4416, 4480, 4544, 4608, 4608]);
}

#[test]
fn arpeggio_counts_ticks_backwards() {
    // FT2 counts a row's ticks down, so at speed 6 the steps run 0, 2, 1, 0, 2, 1.
    let song = module(&["C-4 01 .. 047", "... .. .. 000"]);
    let p = periods(&song, 12);
    assert_eq!(&p[..6], [4608, 4608 - 7 * 64, 4608 - 4 * 64, 4608, 4608 - 7 * 64, 4608 - 4 * 64]);
    // The row after puts the pitch back.
    assert_eq!(&p[6..], [4608; 6]);
}

#[test]
fn vibrato_swings_the_period_and_the_next_row_puts_it_back() {
    let song = module(&["C-4 01 .. 484", "... .. .. 000"]);
    let p = periods(&song, 12);
    // Speed 8 is 32 positions a tick; depth 4. The sine is 0, 180, 255, 180, 0.
    let wave = |height: i32| (height * 4) >> 5;
    assert_eq!(&p[..6], [4608, 4608, 4608 + wave(180), 4608 + wave(255), 4608 + wave(180), 4608]);
    assert_eq!(&p[6..], [4608; 6]);
}

#[test]
fn the_volume_column() {
    let song = module(&[
        "C-4 01 20 000",
        "... .. 62 000",
        "... .. 73 000",
        "... .. 85 000",
        "... .. 94 000",
        "C-4 01 .. 000",
    ]);
    let v = volumes(&song, 36);
    assert_eq!(&v[..6], [16; 6]);
    assert_eq!(&v[6..12], [16, 14, 12, 10, 8, 6]);
    assert_eq!(&v[12..18], [6, 9, 12, 15, 18, 21]);
    assert_eq!(&v[18..24], [16; 6]);
    assert_eq!(&v[24..30], [20; 6]);
    // An instrument number brings the sample's own volume back.
    assert_eq!(&v[30..36], [64; 6]);
}

#[test]
fn panning_from_the_sample_the_volume_column_and_effects() {
    let song = module_with(2, &[&["C-4 01 .. 000", "... .. C4 000", "... .. .. 8C0", "... .. .. P02", "... .. .. P30", "... .. D3 000", "... .. E0 000", "... .. D0 000"]], |s| {
        s.samples[0].pan = Some(32);
    });
    let p = pans(&song, 48);
    assert_eq!(&p[..6], [32; 6]);
    // The volume column's panning has 16 steps: 4 is 64.
    assert_eq!(&p[6..12], [64; 6]);
    assert_eq!(&p[12..18], [192; 6]);
    assert_eq!(&p[18..24], [192, 190, 188, 186, 184, 182]);
    assert_eq!(&p[24..30], [182, 185, 188, 191, 194, 197]);
    assert_eq!(&p[30..36], [197, 194, 191, 188, 185, 182]);
    // Sliding right by nothing does nothing; sliding left by nothing throws
    // the panning hard left on every tick but the first.
    assert_eq!(&p[36..42], [182; 6]);
    assert_eq!(&p[42..48], [182, 0, 0, 0, 0, 0]);
}

#[test]
fn an_envelope_shapes_the_volume_and_waits_at_its_sustain_point() {
    let song = module_with(2, &[&["C-4 01 .. 000", "", "=== .. .. 000", ""]], |s| {
        s.instruments[0].volume_envelope =
            Envelope { enabled: true, points: vec![(0, 64), (4, 32), (8, 0)], sustain: Some(1), loop_points: None };
    });
    let v = volumes(&song, 24);
    assert_eq!(&v[..12], [64, 56, 48, 40, 32, 32, 32, 32, 32, 32, 32, 32]);
    // Key off lets it run on down to nothing.
    assert_eq!(&v[12..18], [32, 24, 16, 8, 0, 0]);
}

#[test]
fn an_envelope_loops_between_its_loop_points() {
    let song = module_with(2, &[&["C-4 01 .. 000"]], |s| {
        s.instruments[0].volume_envelope =
            Envelope { enabled: true, points: vec![(0, 64), (2, 32), (4, 64)], sustain: None, loop_points: Some((1, 2)) };
    });
    assert_eq!(&volumes(&song, 10), &[64, 48, 32, 48, 32, 48, 32, 48, 32, 48]);
}

#[test]
fn key_off_fades_an_instrument_out_at_its_fadeout_speed() {
    let song = module_with(2, &[&["C-4 01 .. 000", "=== .. .. 000", ""]], |s| {
        s.instruments[0].volume_envelope = Envelope { enabled: true, points: vec![(0, 64), (100, 64)], sustain: None, loop_points: None };
        s.instruments[0].fadeout = 8192;
    });
    let v = volumes(&song, 12);
    assert_eq!(&v[..6], [64; 6]);
    assert_eq!(&v[6..12], [48, 32, 16, 0, 0, 0]);
}

#[test]
fn key_off_with_no_envelope_is_silence_at_once() {
    let song = module(&["C-4 01 .. 000", "=== .. .. 000", "C-4 01 .. K03"]);
    let ticks = run_ticks(&song, 18);
    let v: Vec<i32> = track(&ticks, 0).iter().map(|t| volume64(t.1)).collect();
    assert_eq!(&v[..12], [64, 64, 64, 64, 64, 64, 0, 0, 0, 0, 0, 0]);
    // The sample is not stopped, only silenced; and Kxx lets go on its tick.
    assert!(ticks.iter().flatten().all(|(_, c)| *c != Command::Stop));
    assert_eq!(&v[12..18], [64, 64, 64, 0, 0, 0]);
}

#[test]
fn an_instruments_own_vibrato_bends_the_pitch() {
    let song = module_with(2, &[&["C-4 01 .. 000"]], |s| {
        s.instruments[0].vibrato = AutoVibrato { waveform: 0, sweep: 0, depth: 16, rate: 64 };
    });
    // A quarter turn a tick, 16 periods deep, starting downwards in period.
    assert_eq!(&periods(&song, 8), &[4592, 4608, 4624, 4608, 4592, 4608, 4624, 4608]);

    let song = module_with(2, &[&["C-4 01 .. 000"]], |s| {
        s.instruments[0].vibrato = AutoVibrato { waveform: 1, sweep: 4, depth: 16, rate: 128 };
    });
    // A square wave that grows over four ticks: a quarter more depth each.
    assert_eq!(&periods(&song, 6), &[4608 + 4, 4608 - 8, 4608 + 12, 4608 - 16, 4608 + 16, 4608 - 16]);
}

#[test]
fn multi_retrigger_restarts_the_note_and_changes_its_volume() {
    let song = module(&["C-4 01 .. R12", "... .. .. R00"]);
    let ticks = run_ticks(&song, 12);
    // Every second tick, a step quieter each time; R00 carries on the same.
    assert_eq!(plays(&ticks, 0).iter().map(|p| p.0).collect::<Vec<_>>(), [0, 1, 3, 5, 7, 9, 11]);
    let v: Vec<i32> = track(&ticks, 0).iter().map(|t| volume64(t.1)).collect();
    assert_eq!(v, [64, 63, 63, 62, 62, 61, 61, 60, 60, 59, 59, 58]);

    // A volume beside it puts the first retrigger off by a tick, and is the
    // volume every retrigger goes back to.
    let song = module(&["C-4 01 30 R72"]);
    let ticks = run_ticks(&song, 6);
    assert_eq!(plays(&ticks, 0).iter().map(|p| p.0).collect::<Vec<_>>(), [0, 2, 4]);
    assert_eq!(track(&ticks, 0).iter().map(|t| volume64(t.1)).collect::<Vec<_>>(), [32; 6]);
}

#[test]
fn tremor_switches_the_note_on_and_off() {
    // T12: on for 2 ticks, off for 3, never counting a row's first tick.
    let song = module(&["C-4 01 .. T12", "... .. .. T00"]);
    assert_eq!(volumes(&song, 12), [64, 64, 64, 0, 0, 0, 0, 64, 64, 0, 0, 0]);
}

#[test]
fn global_volume_scales_every_channel() {
    let song = module_with(3, &[&["C-4 01 .. 000 | ... .. .. G20 | C-4 01 .. 000", "... .. .. 000 | ... .. .. H02", "... .. .. 000 | ... .. .. H10"]], |_| {});
    let ticks = run_ticks(&song, 18);
    let v = |channel: usize| track(&ticks, channel).iter().map(|t| volume64(t.1)).collect::<Vec<_>>();
    // The channel to the right of the command has it from the start. The one
    // to its left had already worked out its volume, and has it a tick later.
    assert_eq!(&v(2)[..6], [32; 6]);
    assert_eq!(&v(0)[..6], [64, 32, 32, 32, 32, 32]);
    // Slides: down 2 a tick from 32, then up 1 a tick.
    assert_eq!(&v(2)[6..12], [32, 30, 28, 26, 24, 22]);
    assert_eq!(&v(2)[12..18], [22, 23, 24, 25, 26, 27]);
}

#[test]
fn set_envelope_position_jumps_along_the_envelope() {
    let song = module_with(2, &[&["C-4 01 .. L05"]], |s| {
        s.instruments[0].volume_envelope = Envelope { enabled: true, points: vec![(0, 64), (10, 0)], sustain: None, loop_points: None };
    });
    assert_eq!(&volumes(&song, 6), &[32, 26, 19, 13, 6, 0]);
}

#[test]
fn note_delay_and_its_oddities() {
    let song = module(&["C-4 02 .. ED2", "... .. .. ED3", "... .. .. ED0", "D-4 02 .. ED7", "... .. .. E93", "E-4 .. .. E90"]);
    let ticks = run_ticks(&song, 36);
    // Delayed to tick 2. A delay with no note starts the last note again; ED0
    // does nothing; a delay as long as the row never happens. E93 restarts on
    // every third tick, and E90 once, at the start.
    assert_eq!(plays(&ticks, 0).iter().map(|p| p.0).collect::<Vec<_>>(), [2, 9, 27, 30]);
    assert_eq!(linear_period(track(&ticks, 0)[27].0), 4608);
    assert_eq!(linear_period(track(&ticks, 0)[30].0), 4608 - 4 * 64);
}

#[test]
fn sample_offset_and_its_memory() {
    let song = module(&["C-4 02 .. 902", "... .. .. 905", "C-4 02 .. 900", "C-4 02 .. 000", "C-4 02 .. 9FF", "C-4 02 .. 900"]);
    let ticks = run_ticks(&song, 36);
    // The offset is only remembered beside a note; no offset is the start; and
    // an offset past the end plays nothing at all.
    assert_eq!(plays(&ticks, 0), [(0, 1, 0x200), (12, 1, 0x200), (18, 1, 0)]);
    assert_eq!(ticks[24].iter().filter(|(c, cmd)| *c == 0 && *cmd == Command::Stop).count(), 1);
    assert_eq!(ticks[30].iter().filter(|(c, cmd)| *c == 0 && *cmd == Command::Stop).count(), 1);
}

/// The (order, row) at the start of each row played.
fn rows(song: &Song, ticks: usize) -> Vec<(u16, u16)> {
    let mut player = Player::new(song, RATE);
    let mut mixer = Mixer::new(song, RATE);
    let mut out = Vec::new();
    for _ in 0..ticks {
        player.tick(song, 0, &mut mixer);
        if player.position().tick == 0 {
            out.push((player.position().order, player.position().row));
        }
    }
    out
}

#[test]
fn pattern_loop_and_the_row_it_leaves_the_next_pattern_starting_on() {
    let song = module_with(2, &[&["", "... .. .. E60", "... .. .. E62", ""], &["C-4 01 .. 000", "", "", "... .. .. B00"]], |s| {
        for p in &mut s.patterns {
            p.rows = 4;
        }
    });
    // Rows 1 and 2 three times; then FT2's slip: the next pattern starts at
    // the loop's first row, not at its top.
    assert_eq!(rows(&song, 6 * 12), [(0, 0), (0, 1), (0, 2), (0, 1), (0, 2), (0, 1), (0, 2), (0, 3), (1, 1), (1, 2), (1, 3), (0, 0)]);
}

#[test]
fn jumps_breaks_and_delays() {
    let song = module_with(2, &[&["... .. .. D02", ""], &["", "", "... .. .. EE2", "... .. .. B00 | ... .. .. D01"]], |_| {});
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    let mut seen = Vec::new();
    for _ in 0..36 {
        player.tick(&song, 0, &mut mixer);
        seen.push((player.position().order, player.position().row));
    }
    // Break to row 2 of the next pattern, which a pattern delay holds for
    // three rows' time; then jump to order 0, row 1.
    assert_eq!(&seen[..6], [(0, 0); 6]);
    assert_eq!(&seen[6..24], [(1, 2); 18]);
    assert_eq!(&seen[24..30], [(1, 3); 6]);
    assert_eq!(&seen[30..36], [(0, 1); 6]);
}

#[test]
fn speed_and_tempo_take_hold_at_once() {
    let song = module(&["C-4 01 .. F03", "... .. .. FFA", "D-4 01 .. 000"]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    let lengths: Vec<f64> = (0..7).map(|_| player.tick(&song, 0, &mut mixer)).collect();
    let (slow, fast) = (RATE * 2.5 / 125.0, RATE * 2.5 / 250.0);
    assert_eq!(lengths, [slow, slow, slow, fast, fast, fast, fast]);
    assert_eq!(play_ticks(&song, 9, 0), [0, 6]);
}

#[test]
fn a_note_out_of_range_is_dropped_but_its_instrument_is_not() {
    let song = module_with(2, &[&["C-4 01 .. 000", "B-7 02 .. 000", "C-4 .. .. 000"]], |s| {
        s.samples[1].relative_note = 40;
    });
    let ticks = run_ticks(&song, 18);
    // B-7 and 40 semitones is beyond the tenth octave: no note. The next
    // note, with no instrument number, uses instrument 2 all the same.
    assert_eq!(plays(&ticks, 0), [(0, 0, 0), (12, 1, 0)]);
}

#[test]
fn a_missing_instrument_or_sample_plays_nothing() {
    let song = module_with(2, &[&["C-4 01 .. 000", "C-4 09 .. 000", "C-4 01 .. 000"]], |_| {});
    let ticks = run_ticks(&song, 18);
    assert_eq!(play_ticks(&song, 18, 0), [0, 12]);
    assert!(ticks[6].contains(&(0, Command::Stop)));
}

#[test]
fn a_synth_patch_is_started_by_name_and_told_when_the_key_is_let_go() {
    // XM cannot hold a patch, so this song goes through our own format.
    let mut song = song(Rules::FastTracker2, 2, vec![square(64, 1)], vec![pattern(8, &["C-4 02 .. 000", "E-4 02 20 000", "=== .. .. 000"])]);
    song.instruments.push(Instrument { name: "lead".to_string(), patch: Some("wobble".to_string()), ..Default::default() });
    let saved = native::save(&song);
    let song = native::load(&saved.json, &|_| None).unwrap();

    let ticks = run_ticks(&song, 18);
    let starts: Vec<Start> = ticks.iter().flatten().filter_map(|(_, c)| if let Command::Play(s) = c { Some(*s) } else { None }).collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0].source, Source::Patch(1));
    assert_eq!(song.instruments[1].patch.as_deref(), Some("wobble"));
    assert!((note_of_rate(starts[0].rate) - 60.0).abs() < 1e-6);
    assert!((note_of_rate(starts[1].rate) - 64.0).abs() < 1e-6);
    assert_eq!((volume64(starts[0].volume), volume64(starts[1].volume)), (64, 16));
    assert!(ticks[12].contains(&(0, Command::Release)));
    assert!(ticks[..12].iter().flatten().all(|(_, c)| *c != Command::Release));
}

#[test]
fn volume_changes_say_how_long_to_take() {
    let song = module_with(2, &[&["C-4 01 .. 000", "... .. .. A01", "... .. .. C10"]], |_| {});
    let ticks = run_ticks(&song, 18);
    let ramps: Vec<u32> = ticks.iter().flatten().filter_map(|(_, c)| if let Command::Volume(_, ramp) = c { Some(*ramp) } else { None }).collect();
    // The channels' first volumes and five steps of a slide glide over a tick
    // (882 frames); a set volume takes 5 milliseconds.
    assert_eq!(ramps, [882, 882, 882, 882, 882, 882, 220]);
}

#[test]
fn the_mixer_plays_loops_both_ways() {
    let mut song = song(Rules::FastTracker2, 1, vec![ramp(4), ramp(4), ramp(4)], vec![pattern(8, &[""])]);
    song.samples[1].loop_len = 3;
    song.samples[1].loop_start = 1;
    song.samples[1].loop_kind = LoopKind::Forward;
    song.samples[2].loop_len = 4;
    song.samples[2].loop_kind = LoopKind::PingPong;
    let heard = |sample: usize, offset: u32| {
        let mut mixer = Mixer::new(&song, RATE);
        mixer.nearest = true;
        mixer.gain = 128.0;
        mixer.square_root_pan = false;
        mixer.play(0, 0, &Start { source: Source::Sample(sample), offset, rate: RATE, volume: 1.0, pan: -1.0 });
        mixer.render(12);
        mixer.left.iter().map(|x| x.round() as i32).collect::<Vec<_>>()
    };
    assert_eq!(heard(0, 0), [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(heard(1, 0), [0, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2]);
    assert_eq!(heard(2, 0), [0, 1, 2, 3, 3, 2, 1, 0, 0, 1, 2, 3]);
    // A start past the end goes into the loop, or is silent without one.
    assert_eq!(heard(1, u32::MAX)[..3], [1, 2, 3]);
    assert_eq!(heard(0, 4), [0; 12]);
}

/// One channel's chord instructions: the tick, and the offsets (none for off).
fn chords(ticks: &[Vec<(usize, Command)>]) -> Vec<(usize, Vec<i8>, f32)> {
    let mut out = Vec::new();
    for (t, tick) in ticks.iter().enumerate() {
        for (_, command) in tick {
            if let Command::Arpeggio(a) = command {
                out.push((t, a.map_or(Vec::new(), |a| a.offsets().to_vec()), a.map_or(0.0, |a| a.rate)));
            }
        }
    }
    out
}

#[test]
fn a_chord_is_one_instruction_and_lasts_until_the_next_note_or_an_off() {
    for rules in [Rules::FastTracker2, Rules::ProTracker] {
        let mut p = pattern(16, &["C-4 01 .. 000", "", "E-4 01 .. 000", "G-4 01 .. 000", "", "... .. .. 000", "C-4 01 .. E93", "", "C-4 01 .. FFA"]);
        if rules == Rules::ProTracker {
            p = pattern(16, &["C-4 01 000", "", "E-4 01 000", "G-4 01 000", "", "... .. 000", "C-4 01 E93", "", "C-4 01 FFA"]);
        }
        p.set_chord(0, 0, Some(Chord::arp(&[0, 3, 7], Arp::FAST)));
        p.set_chord(1, 0, Some(Chord::arp(&[0, 4, 7, 11], Arp { rate: ArpRate::PerSecond(60.0), ..Arp::FAST })));
        p.set_chord(3, 0, Some(Chord::arp(&[0, 12], Arp::FAST)));
        p.set_chord(5, 0, Some(Chord::OFF));
        p.set_chord(6, 0, Some(Chord::arp(&[0, 7], Arp { rate: ArpRate::PerTick(2.0), ..Arp::FAST })));
        p.set_chord(8, 0, Some(Chord::arp(&[0, 5], Arp::FAST)));
        let song = song(rules, 1, vec![square(64, 1)], vec![p]);
        let ticks = run_ticks(&song, 54);
        assert_eq!(
            chords(&ticks),
            [
                // One step a tick is 50 a second at 125 beats a minute.
                (0, vec![0, 3, 7], 50.0),
                // A chord on a row with no note changes the one sounding.
                (6, vec![0, 4, 7, 11], 60.0),
                // A note with no chord ends it.
                (12, vec![], 0.0),
                (18, vec![0, 12], 50.0),
                (30, vec![], 0.0),
                // Every time the note is started again, it is given the chord again.
                (36, vec![0, 7], 100.0),
                (39, vec![0, 7], 100.0),
                // The tempo the row finds is the tempo it is timed by.
                (48, vec![0, 5], 50.0),
            ],
            "{rules:?}"
        );
        // The chord comes straight after the note it belongs to.
        let at = ticks[0].iter().position(|(_, c)| matches!(c, Command::Play(_))).unwrap();
        assert!(ticks[0][at + 1..].iter().any(|(_, c)| matches!(c, Command::Arpeggio(Some(_)))));
    }
}

#[test]
fn the_mixer_steps_a_voice_through_its_chord() {
    // An octave arpeggio at 100 steps a second on a 64-frame square at
    // 8363: 131 Hz and 261 Hz by turns, 441 frames each.
    let mut p = pattern(8, &["C-4 01 .. 000"]);
    p.set_chord(0, 0, Some(Chord::arp(&[0, 12], Arp { rate: ArpRate::PerSecond(100.0), ..Arp::FAST })));
    let song = song(Rules::FastTracker2, 1, vec![square(64, 1)], vec![p]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    play(&song, &mut player, &mut mixer, 441 * 8);
    // The time from one upward crossing to the next is the wave's period:
    // 337.5 frames on the even steps, half that on the odd ones.
    let rises: Vec<usize> = (1..mixer.left.len()).filter(|&i| mixer.left[i - 1] <= 0.0 && mixer.left[i] > 0.0).collect();
    let mut seen = [0; 2];
    for pair in rises.windows(2) {
        let (step, next) = (pair[0] / 441, pair[1] / 441);
        if step == next {
            let want = if step % 2 == 0 { 337.5 } else { 168.75 };
            assert!((pair[1] as f64 - pair[0] as f64 - want).abs() < 2.0, "step {step}: a period of {} frames", pair[1] - pair[0]);
            seen[step % 2] += 1;
        }
    }
    assert!(seen[0] >= 2 && seen[1] >= 4, "{seen:?}");
}
