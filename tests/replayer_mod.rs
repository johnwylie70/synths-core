//! ProTracker's rules, tested on small modules built here: each is written
//! out as a MOD file and loaded back, so the loader is in the loop too.

mod replayer_support;
use replayer_support::*;
use synths_core::replayer::Player;
use synths_core::song::*;

/// A 4-channel module with a looped square wave as sample 1 and a ramp that
/// plays once as sample 2, and the given rows as its first pattern.
fn module(rows: &[&str]) -> Song {
    modules(&[rows])
}

fn modules(patterns: &[&[&str]]) -> Song {
    let patterns = patterns.iter().map(|rows| pattern(64, rows)).collect();
    let mut quiet = square(64, 1);
    quiet.volume = 32;
    quiet.finetune = -16;
    let song = song(Rules::ProTracker, 4, vec![square(64, 1), ramp(6000), quiet], patterns);
    let loaded = load_module(&write_mod(&song)).unwrap();
    assert_eq!(loaded.rules, Rules::ProTracker);
    loaded
}

/// The period channel 0 is at on each tick.
fn periods(song: &Song, ticks: usize) -> Vec<u32> {
    track(&run_ticks(song, ticks), 0).iter().map(|t| if t.0 == 0.0 { 0 } else { amiga_period(t.0) }).collect()
}

fn volumes(song: &Song, ticks: usize) -> Vec<i32> {
    track(&run_ticks(song, ticks), 0).iter().map(|t| volume64(t.1)).collect()
}

#[test]
fn notes_start_on_their_row_at_the_tables_period() {
    let song = module(&["C-4 01 000 | ... .. 000 | ... .. 000 | A-4 03 000", "", "D-4 01 000"]);
    let ticks = run_ticks(&song, 18);
    // C-4 here is ProTracker's C-2: period 428. Rows are 6 ticks.
    assert_eq!(plays(&ticks, 0), [(0, 0, 0), (12, 0, 0)]);
    let ch0 = track(&ticks, 0);
    assert_eq!(amiga_period(ch0[0].0), 428);
    assert_eq!(amiga_period(ch0[12].0), 381);
    assert_eq!(volume64(ch0[0].1), 64);
    // Sample 3 has finetune -1 and volume 32: A-4 is 254 at finetune 0, 256 at -1.
    let ch3 = track(&ticks, 3);
    assert_eq!((amiga_period(ch3[0].0), volume64(ch3[0].1)), (256, 32));
    // Left, right, right, left.
    assert_eq!((ch0[0].2, ch3[0].2), (-1.0, -1.0));
    let song = module(&["... .. 000 | C-4 01 000 | C-4 01 000"]);
    let ticks = run_ticks(&song, 1);
    assert_eq!((track(&ticks, 1)[0].2, track(&ticks, 2)[0].2), (1.0, 1.0));
}

#[test]
fn arpeggio_steps_through_three_notes() {
    let song = module(&["C-4 01 047", "... .. 047"]);
    // C-4 428, E-4 (4 up) 339, G-4 (7 up) 285; the tick number modulo 3 picks.
    assert_eq!(periods(&song, 12), [428, 339, 285, 428, 339, 285, 428, 339, 285, 428, 339, 285]);
}

#[test]
fn arpeggio_past_the_top_of_the_table_reads_what_lies_beyond() {
    // B-5 is the last note; one step up is the 0 that ends the row (which the
    // chip plays as 65536), two is the next finetune's first period.
    let song = module(&["B-5 01 012"]);
    let p = periods(&song, 3);
    assert_eq!(p[0], 113);
    assert_eq!(p[1], 65536);
    assert_eq!(p[2], 850);
}

#[test]
fn slides_move_the_period_each_tick_but_the_first_and_stop_at_the_amiga_limits() {
    let song = module(&["C-4 01 000", "... .. 110", "... .. 220"]);
    let p = periods(&song, 18);
    assert_eq!(&p[..6], [428; 6]);
    assert_eq!(&p[6..12], [428, 412, 396, 380, 364, 348]);
    assert_eq!(&p[12..18], [348, 380, 412, 444, 476, 508]);

    let song = module(&["B-5 01 105", "C-3 01 2FF"]);
    let p = periods(&song, 12);
    assert_eq!(&p[..6], [113, 113, 113, 113, 113, 113]);
    assert_eq!(&p[6..], [856, 856, 856, 856, 856, 856]);
}

#[test]
fn fine_slides_act_once_on_the_first_tick() {
    let song = module(&["C-4 01 E13", "... .. E25", "... .. EA0"]);
    let p = periods(&song, 18);
    assert_eq!(&p[..6], [425; 6]);
    assert_eq!(&p[6..12], [430; 6]);
}

#[test]
fn tone_portamento_slides_to_the_note_and_stops_there() {
    let song = module(&["C-4 01 000", "D-4 .. 310", "... .. 300", "... .. 300", "... .. 108"]);
    let p = periods(&song, 30);
    // 428 down to D-4 at 381, 16 a tick; the note is not started again.
    assert_eq!(&p[6..12], [428, 412, 396, 381, 381, 381]);
    assert_eq!(&p[12..24], [381; 12]);
    assert_eq!(plays(&run_ticks(&song, 30), 0).len(), 1);
    // After arriving the target is forgotten: a slide away is not undone later.
    assert_eq!(p[29], 381 - 5 * 8);
}

#[test]
fn glissando_makes_the_portamento_step_in_semitones() {
    let song = module(&["C-4 01 E31", "E-4 .. 308", "... .. 300"]);
    let p = periods(&song, 18);
    // 428 falling 8 a tick towards 339: heard as the note at or above it.
    assert_eq!(&p[6..12], [428, 404, 404, 404, 381, 381]);
    // A row's first tick sends the true period, between the notes: 388.
    assert_eq!(&p[12..18], [388, 360, 360, 360, 339, 339]);
}

#[test]
fn vibrato_leaves_the_first_tick_of_each_row_alone() {
    let song = module(&["C-4 01 48F", "... .. 400"]);
    let p = periods(&song, 12);
    // Speed 8 moves the wave's position 32 a tick (8 steps of the sine); depth F.
    // Tick 1 is at position 0: no offset yet. Then sine 180, 255, 180, 0.
    let wave = |height: u32| (height * 15) >> 7;
    assert_eq!(&p[..6], [428, 428, 428 + wave(180), 428 + wave(255), 428 + wave(180), 428]);
    // The second half of the wave is below, and the row's first tick is plain.
    assert_eq!(&p[6..10], [428, 428 - wave(180), 428 - wave(255), 428 - wave(180)]);
}

#[test]
fn volume_commands() {
    let song = module(&["C-4 01 C20", "... .. A04", "... .. A30", "... .. EA5", "... .. EB9", "... .. EC2", "... .. CFF"]);
    let v = volumes(&song, 42);
    assert_eq!(&v[..6], [32; 6]);
    assert_eq!(&v[6..12], [32, 28, 24, 20, 16, 12]);
    assert_eq!(&v[12..18], [12, 15, 18, 21, 24, 27]);
    assert_eq!(&v[18..24], [32; 6]);
    assert_eq!(&v[24..30], [23; 6]);
    assert_eq!(&v[30..36], [23, 23, 0, 0, 0, 0]);
    assert_eq!(&v[36..42], [64; 6]);
}

#[test]
fn tremolo_moves_the_volume_and_the_volume_comes_back() {
    let song = module(&["C-4 01 C20", "... .. 78F", "... .. 000"]);
    let v = volumes(&song, 18);
    let wave = |height: i32| (height * 15) >> 6;
    assert_eq!(&v[6..11], [32, 32, (32 + wave(180)).min(64), 64, (32 + wave(180)).min(64)]);
    assert_eq!(&v[13..18], [32; 5]);
}

#[test]
fn sample_offset_starts_part_way_in_and_is_added_twice() {
    let song = module(&["C-4 02 902", "C-4 .. 000", "C-4 02 000", "C-4 02 9FF"]);
    let ticks = run_ticks(&song, 24);
    // 902 starts 0x200 frames in. ProTracker adds the offset again after the
    // note has started, so a later note with no instrument starts at 0x400;
    // an instrument number puts it back. Past the end, nothing of the sample
    // plays.
    assert_eq!(plays(&ticks, 0), [(0, 1, 0x200), (6, 1, 0x400), (12, 1, 0), (18, 1, u32::MAX)]);
}

#[test]
fn an_instrument_number_alone_changes_the_volume_now_and_the_sample_later() {
    let song = module(&["C-4 01 C10", "... 03 000", "... 02 000", "... .. 000 | ... 01 000"]);
    let ticks = run_ticks(&song, 24);
    assert_eq!(plays(&ticks, 0), [(0, 0, 0)]);
    assert_eq!(volume64(track(&ticks, 0)[5].1), 16);
    assert_eq!(volume64(track(&ticks, 0)[6].1), 32);
    assert_eq!(volume64(track(&ticks, 0)[12].1), 64);
    // The sample to carry on with is queued; it is not started.
    let queued: Vec<(usize, Option<usize>)> = ticks
        .iter()
        .enumerate()
        .flat_map(|(t, tick)| tick.iter().filter_map(move |(c, cmd)| matches!((c, cmd), (0, Command::Queue(_))).then_some((t, *cmd))))
        .map(|(t, cmd)| match cmd {
            Command::Queue(q) => (t, q),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(queued, [(6, Some(2)), (12, Some(1))]);
    // The period is not worked out again, so sample 3's finetune does not show.
    assert_eq!(amiga_period(track(&ticks, 0)[11].0), 428);
    // A channel that has never played has nothing to swap, and stays silent.
    assert!(ticks[18].iter().all(|(c, cmd)| *c != 1 || matches!(cmd, Command::Volume(..))));
}

#[test]
fn a_queued_sample_takes_over_at_the_end_of_the_loop() {
    // Sample 1 is a 64-frame loop; sample 2 plays once, so swapping to it
    // silences the channel when the loop next comes round.
    let song = module(&["C-4 01 000", "... 02 000"]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    play(&song, &mut player, &mut mixer, 882 * 12);
    let loud = |from: usize, to: usize| mixer.left[from..to].iter().any(|x| x.abs() > 0.01);
    assert!(loud(0, 882 * 6));
    // 64 frames at 8287 a second last 341 output frames: silent within that.
    assert!(!loud(882 * 6 + 400, 882 * 12));
}

#[test]
fn note_delay_and_retrigger() {
    let song = module(&["C-4 02 ED3", "... .. E92", "D-4 02 E92", "... .. E90"]);
    let ticks = run_ticks(&song, 24);
    // Delayed to tick 3. Retrigger every 2 ticks: with no note in the row that
    // includes the first tick; with a note, the note itself is the first.
    assert_eq!(plays(&ticks, 0).iter().map(|p| p.0).collect::<Vec<_>>(), [3, 6, 8, 10, 12, 14, 16]);
}

#[test]
fn a_note_delayed_past_the_row_changes_the_pitch_of_the_note_still_playing() {
    let song = module(&["C-4 01 000", "D-4 01 ED7", "... .. 000"]);
    let ticks = run_ticks(&song, 18);
    assert_eq!(plays(&ticks, 0).len(), 1);
    let p: Vec<u32> = track(&ticks, 0).iter().map(|t| amiga_period(t.0)).collect();
    assert_eq!(&p[..12], [428; 12]);
    assert_eq!(&p[12..], [381; 6]);
}

/// The (order, row) the player reports on each tick.
fn rows(song: &Song, ticks: usize) -> Vec<(u16, u16)> {
    let mut player = Player::new(song, RATE);
    let mut mixer = Mixer::new(song, RATE);
    let mut out: Vec<(u16, u16)> = Vec::new();
    for _ in 0..ticks {
        player.tick(song, 0, &mut mixer);
        let at = (player.position().order, player.position().row);
        if out.last() != Some(&at) || player.position().tick == 0 {
            out.push(at);
        }
    }
    out
}

#[test]
fn position_jump_and_pattern_break() {
    // Break to row 2 of the next order; jump to order 0 and, with a break to
    // its right, to row 3 of it; a break to the left of a jump is forgotten.
    let song = modules(&[
        &["C-4 01 D02", "", "", "... .. 000", "... .. B01 | ... .. D04"],
        &["", "", "... .. B00 | ... .. D03", "", "... .. D09 | ... .. B01"],
    ]);
    assert_eq!(rows(&song, 6 * 7), [(0, 0), (1, 2), (0, 3), (0, 4), (1, 4), (1, 0), (1, 1)]);
}

#[test]
fn pattern_loop_plays_the_rows_again() {
    let song = module(&["C-4 01 000", "... .. E60", "", "... .. E62", "... .. 000"]);
    let r: Vec<u16> = rows(&song, 6 * 11).iter().map(|r| r.1).collect();
    assert_eq!(r, [0, 1, 2, 3, 1, 2, 3, 1, 2, 3, 4]);
}

#[test]
fn pattern_delay_holds_the_row_and_runs_its_effects_again() {
    let song = module(&["C-4 01 C20", "... .. EE2 | C-4 02 E93 | ... .. EA4", "... .. 000"]);
    let ticks = run_ticks(&song, 36);
    // Row 1 lasts three rows' worth of ticks.
    let r = rows(&song, 36);
    assert_eq!(r.len(), 2 + 3 - 1 + 2);
    // The note is played once, at the start. Its retrigger runs in every
    // repeat, counting ticks from the repeat's start, and (as the row has a
    // note) never on a first tick.
    assert_eq!(plays(&ticks, 1).iter().map(|p| p.0).collect::<Vec<_>>(), [6, 9, 15, 21]);
    // The fine volume slide beside it acts once in each repeat: 4, 8, 12.
    assert_eq!([7, 13, 19].map(|t| volume64(track(&ticks, 2)[t].1)), [4, 8, 12]);
    assert_eq!(track(&ticks, 0).iter().map(|t| volume64(t.1)).collect::<Vec<_>>()[5..], [32; 31]);
}

#[test]
fn speed_and_tempo() {
    // F03: 3 ticks a row. F7D and up are beats a minute, and take hold one
    // tick late, as the Amiga's timer did.
    let song = module(&["C-4 01 F03", "... .. FFA", "D-4 01 000"]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    let lengths: Vec<f64> = (0..9).map(|_| player.tick(&song, 0, &mut mixer)).collect();
    let (slow, fast) = (RATE * 2.5 / 125.0, RATE * 2.5 / 250.0);
    assert_eq!(lengths, [slow, slow, slow, slow, fast, fast, fast, fast, fast]);
    assert_eq!(plays(&run_ticks(&song, 9), 0).iter().map(|p| p.0).collect::<Vec<_>>(), [0, 6]);
    assert_eq!(player.position().speed, 3);
    assert_eq!(player.position().tempo, 250);
}

#[test]
fn ticks_fall_on_the_same_frames_whatever_the_block_size() {
    let song = module(&["C-4 01 000", "D-4 01 FA5", "E-4 02 ED1", "... .. E91"]);
    let frames_of = |block: u32| {
        let mut player = Player::new(&song, RATE);
        let mut mixer = Mixer::new(&song, RATE);
        let mut done = 0;
        while done < 30000 {
            player.process(&song, block, &mut mixer);
            mixer.render(block);
            done += block;
        }
        let log: Vec<Logged> = mixer.log.into_iter().filter(|l| l.frame < 30000).collect();
        (log, mixer.left[..30000].to_vec())
    };
    let (log, sound) = frames_of(64);
    assert_eq!(log[0].frame, 0);
    // 125 beats a minute at 44100 is 882 frames a tick; the second row starts
    // at tick 6. At 165 a tick is 668.18 frames, so ticks land between frames.
    assert!(log.iter().any(|l| l.frame == 882 * 6 && matches!(l.command, Command::Play(_))));
    for block in [1, 7, 128, 1000, 4096] {
        let (other, other_sound) = frames_of(block);
        assert_eq!(other, log, "block size {block}");
        assert_eq!(other_sound, sound, "block size {block}");
    }
}

#[test]
fn the_song_ends_when_it_comes_back_to_a_row_it_has_played() {
    let song = modules(&[&["C-4 01 000", "... .. D00"], &["D-4 01 000", "... .. B00"]]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    for _ in 0..24 {
        player.tick(&song, 0, &mut mixer);
        assert!(!player.ended());
    }
    player.tick(&song, 0, &mut mixer);
    assert!(player.ended());
    assert_eq!(player.position().loops, 1);
    // It carries on round, as a tracker does.
    assert_eq!((player.position().order, player.position().row), (0, 0));
    assert_eq!(plays(&run_ticks(&song, 25), 0).len(), 3);

    // Told not to repeat, it stops before playing the first row again.
    let mut player = Player::new(&song, RATE);
    player.set_repeat(false);
    let mut mixer = Mixer::new(&song, RATE);
    for _ in 0..40 {
        player.tick(&song, 0, &mut mixer);
    }
    assert!(player.ended());
    assert_eq!(mixer.log.iter().filter(|l| matches!(l.command, Command::Play(_))).count(), 2);
    assert_eq!((player.position().order, player.position().row), (1, 1));
}

#[test]
fn seeking_stops_the_voices_and_plays_from_the_row() {
    let song = modules(&[&["C-4 01 000"], &["", "", "E-4 02 000", "F-4 01 000"]]);
    let mut player = Player::new(&song, RATE);
    let mut mixer = Mixer::new(&song, RATE);
    play(&song, &mut player, &mut mixer, 2000);
    player.seek(&song, 1, 2, &mut mixer);
    let before = mixer.log.len();
    assert_eq!(mixer.log[before - 4..].iter().filter(|l| l.command == Command::Stop).count(), 4);
    play(&song, &mut player, &mut mixer, 882 * 7);
    assert_eq!((player.position().order, player.position().row), (1, 3));
    let started: Vec<u64> = mixer.log[before..].iter().filter(|l| matches!(l.command, Command::Play(_))).map(|l| l.frame - 2000).collect();
    assert_eq!(started, [0, 882 * 6]);
    // Out of range asks are brought into range.
    player.seek(&song, 99, 999, &mut mixer);
    player.tick(&song, 0, &mut mixer);
    assert_eq!((player.position().order, player.position().row), (1, 63));
}

#[test]
fn the_filter_command_reaches_the_voices() {
    let song = module(&["C-4 01 E01", "... .. E00"]);
    let ticks = run_ticks(&song, 12);
    assert!(ticks[0].contains(&(0, Command::Filter(false))));
    assert!(ticks[6].contains(&(0, Command::Filter(true))));
}
