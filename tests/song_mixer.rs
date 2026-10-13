//! A song's mix: each channel's volume, pan and sends, the song's reverb
//! and delay, its master volume and its loop, as keys of the song and as
//! commands while it plays; and the channels' level meters.

use synths_core::event::*;
use synths_core::params::*;
use synths_core::song::{binary, native, ChannelMix, Song, Time};
use synths_core::song_player::{self, FIRST_PART, STATE_PLAYING, STATE_STOPPED};
use synths_core::{Engine, Event};

const SR: f32 = 44100.0;
const LEAD: u16 = 10;
/// A row at 125 beats a minute and six ticks to the row, in frames.
const ROW: usize = 5292;

/// A song of one 16-row pattern on three channels. Instrument 1 is a
/// sample that is the same number all the way through (so the output shows
/// the mix exactly), 2 a square wave, 3 a synth patch. `top` is any other
/// top-level keys, each with its comma.
fn text(top: &str, events: &str) -> String {
    format!(
        r#"{{"format": "synths-song", "version": 2, {top} "channels": 3, "order": [0],
        "patterns": [{{"rows": 16, "events": [{events}]}}],
        "instruments": [{{"name": "steady", "sample": 0}}, {{"name": "square", "sample": 1}}, {{"name": "lead", "patch": "lead"}}],
        "samples": [{{"wave": [100, 100, 100, 100, 100, 100, 100, 100], "loop": {{}}}},
                    {{"wave": [100, 100, 100, 100, 100, 100, 100, 100, -100, -100, -100, -100, -100, -100, -100, -100], "loop": {{}}}}]}}"#
    )
}

fn song(top: &str, events: &str) -> Song {
    native::load(&text(top, events), &|_| None).unwrap()
}

fn playing(song: Song) -> Box<Engine> {
    let mut e = Engine::new(SR);
    for (id, value) in [(OSC1_WAVE, 0.0), (AMP_ATTACK, 0.0), (AMP_DECAY, 0.0), (AMP_SUSTAIN, 1.0), (AMP_RELEASE, 0.0), (VELOCITY_SENS, 0.0)] {
        e.set_param(LEAD, id, value);
    }
    e.set_param(0, LIMITER, 0.0);
    e.set_param(0, MASTER_VOLUME, 1.0);
    song_player::install(&mut e, song, FIRST_PART, 0, false).unwrap();
    e.push(Event::song(0.0, SONG_PATCH_PART, 3.0, LEAD as f32));
    e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    e
}

fn render(e: &mut Engine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(256).zip(r.chunks_mut(256)) {
        e.render(a, b);
    }
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn same(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

const NOTES: &str = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 0, "ch": 1, "note": "C-5", "ins": 2}, {"row": 0, "ch": 2, "note": "A-4", "ins": 3}"#;

#[test]
fn a_song_that_sets_no_mix_is_the_song_it_was() {
    let plain = song("", NOTES);
    // A mixer with everything where it is by default says nothing, and is the same song.
    let unity = song(r#""mixer": {"volume": 1, "channels": [{}, {"volume": 1}, {"reverb": 0, "echo": 0}]}, "effects": {}, "loop": {},"#, NOTES);
    assert_eq!(plain, unity);
    assert!(!plain.has_mix());
    let json = native::save(&plain).json;
    assert!(!json.contains("mixer") && !json.contains("effects") && !json.contains("\n  \"loop\"") && json.contains(r#""version": 1,"#), "{json}");
    assert!(!binary::save(&plain).windows(4).any(|w| w == b"MIXR"));
    // And the engine is left as it was: no parameter of its own or of the host's parts moved.
    let mut e = playing(plain);
    let (l, r) = render(&mut e, ROW * 8);
    assert!(rms(&l) > 0.05 && rms(&r) > 0.05);
    for id in [REVERB_TYPE, REVERB_DECAY, REVERB_SIZE, REVERB_LEVEL, DELAY_TIME, DELAY_FEEDBACK, DELAY_LEVEL] {
        assert_eq!(e.param(0, id), info(id).unwrap().default);
    }
    for c in 0..3 {
        assert_eq!((e.param(32 + c, VOLUME), e.param(32 + c, PAN), e.param(32 + c, SEND_REVERB), e.param(32 + c, SEND_DELAY)), (0.5, 0.0, 0.0, 0.0));
    }
}

#[test]
fn the_mix_reads_back_from_json_and_from_the_packed_form() {
    let top = r#""pan": [96, 128, 255], "mixer": {"volume": 0.8, "channels": [{"volume": 0.5, "reverb": 0.3}, {}, {"echo": 0.75, "volume": 1.5}]},
        "effects": {"reverbType": "hall", "reverbDecay": 2.5, "reverbSize": 0.75, "reverbDamping": 0.25, "reverbPredelay": 0.02, "reverbLowCut": 200, "reverbLevel": 0.5,
                    "echoTime": "1/8.", "echoFeedback": 0.25, "echoTone": 3000, "echoBounce": true, "echoLevel": 1.5},
        "loop": {"start": [0, 4], "end": [0, 12]},"#;
    let s = song(top, NOTES);
    assert_eq!(s.volume, 0.8);
    assert_eq!(s.mix, [ChannelMix { volume: 0.5, reverb: 0.3, echo: 0.0 }, ChannelMix::default(), ChannelMix { volume: 1.5, reverb: 0.0, echo: 0.75 }]);
    assert_eq!(s.channel_pan, [96, 128, 255], "a channel's pan is the song's pan, as it always was");
    let fx = s.effects;
    assert_eq!((fx.get("reverbType"), fx.get("reverbDecay"), fx.get("reverbSize"), fx.get("reverbLevel")), (Some(1.0), Some(2.5), Some(0.75), Some(0.5)));
    assert_eq!((fx.get("reverbDamping"), fx.get("reverbPredelay"), fx.get("reverbLowCut")), (Some(0.25), Some(0.02), Some(200.0)));
    assert_eq!((fx.get("echoFeedback"), fx.get("echoTone"), fx.get("echoBounce"), fx.get("echoLevel")), (Some(0.25), Some(3000.0), Some(1.0), Some(1.5)));
    assert!(matches!(fx.echo_time, Some(Time::Note(_))));
    assert_eq!((s.loop_start, s.loop_end), (Some((0, 4)), Some((0, 12))));
    let json = native::save(&s).json;
    for piece in [
        r#""mixer": {"volume": 0.8, "channels": [{"volume": 0.5, "reverb": 0.3}, {}, {"volume": 1.5, "echo": 0.75}]}"#,
        r#""effects": {"reverbType": "hall", "reverbDecay": 2.5, "reverbSize": 0.75, "reverbDamping": 0.25, "reverbPredelay": 0.02, "reverbLowCut": 200, "reverbLevel": 0.5, "echoTime": "1/8.", "echoFeedback": 0.25, "echoTone": 3000, "echoBounce": true, "echoLevel": 1.5}"#,
        r#""loop": {"start": [0, 4], "end": [0, 12]}"#,
        r#""version": 2,"#,
    ] {
        assert!(json.contains(piece), "{piece}\n{json}");
    }
    assert_eq!(native::load(&json, &|_| None).unwrap(), s);
    let packed = binary::save(&s);
    assert_eq!(binary::load(&packed).unwrap(), s);
    assert_eq!(binary::save(&binary::load(&packed).unwrap()), packed);
    // A reader that does not know the mix reads the song without it: cut off before it, it is the plain song.
    let at = packed.windows(4).position(|w| w == b"MIXR").unwrap();
    assert_eq!(binary::load(&packed[..at]).unwrap(), song(r#""pan": [96, 128, 255],"#, NOTES));
    // An echo in seconds, a reverb with nothing but its type, a loop with only an end:
    // what the song does not mention is not set.
    let s = song(r#""effects": {"echoTime": 0.25, "reverbType": "spring"}, "loop": {"end": [0, 8]},"#, NOTES);
    assert_eq!((s.effects.echo_time, s.effects.get("reverbType"), s.effects.get("reverbSize"), s.loop_start, s.loop_end), (Some(Time::Seconds(0.25)), Some(4.0), None, None, Some((0, 8))));
    let json = native::save(&s).json;
    assert!(json.contains(r#""effects": {"reverbType": "spring", "echoTime": 0.25}"#) && json.contains(r#""loop": {"end": [0, 8]}"#), "{json}");
    assert_eq!(native::load(&json, &|_| None).unwrap(), s);
    assert_eq!(binary::load(&binary::save(&s)).unwrap(), s);
    for bad in [
        r#""mixer": 3,"#,
        r#""mixer": {"volume": 5},"#,
        r#""mixer": {"channels": [{}, {}, {}, {}]},"#,
        r#""mixer": {"channels": [{"echo": 2}]},"#,
        r#""mixer": {"channels": [0.5]},"#,
        r#""effects": {"reverbType": "cave"},"#,
        r#""effects": {"reverbDecay": 99},"#,
        r#""effects": {"echoTime": 9},"#,
        r#""effects": {"echoTime": "soon"},"#,
        r#""effects": {"echoFeedback": 1},"#,
        r#""effects": {"echoBounce": 1},"#,
        r#""loop": {"start": [3, 0]},"#,
        r#""loop": {"end": 4},"#,
    ] {
        let why = native::load(&text(bad, NOTES), &|_| None).unwrap_err().to_string();
        assert!(["mixer", "channels", "volume", "echo", "reverbType", "reverbDecay", "echoTime", "echoFeedback", "echoBounce", "loop"].iter().any(|k| why.contains(k)), "{bad}: {why}");
    }
}

/// The level of the steady sample's channel alone, left and right, over a stretch.
fn levels(x: &(Vec<f32>, Vec<f32>), from: usize, to: usize) -> (f32, f32) {
    (rms(&x.0[from..to]), rms(&x.1[from..to]))
}

#[test]
fn a_channels_volume_and_pan_are_in_the_song_and_move_live_without_a_click() {
    let one = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}"#;
    let plain = render(&mut playing(song("", one)), ROW * 4);
    let (full, _) = levels(&plain, 2000, ROW * 4);
    // Half volume in the song is half the level; the song's own volume multiplies it.
    let half = render(&mut playing(song(r#""mixer": {"channels": [{"volume": 0.5}]},"#, one)), ROW * 4);
    assert!((levels(&half, 2000, ROW * 4).0 / full - 0.5).abs() < 1.0e-3);
    let quarter = render(&mut playing(song(r#""mixer": {"volume": 0.5, "channels": [{"volume": 0.5}]},"#, one)), ROW * 4);
    assert!((levels(&quarter, 2000, ROW * 4).0 / full - 0.25).abs() < 1.0e-3);
    // Panned hard right, by the song's own pan, there is nothing on the left.
    let right = render(&mut playing(song(r#""pan": [255],"#, one)), ROW * 4);
    let (l, r) = levels(&right, 2000, ROW * 4);
    assert!(l < 1.0e-4 && r > full * 0.9, "{l} {r}");

    // A fader moved while it plays: the sound gets to the new level, and on
    // the way no frame is more than a little different from the one before.
    let mut e = playing(song("", one));
    e.push(Event::song(ROW as f64 * 2.0, SONG_CHANNEL_VOLUME, 0.0, 0.25));
    e.push(Event::song(ROW as f64 * 4.0, SONG_CHANNEL_PAN, 0.0, -1.0));
    e.push(Event::song(ROW as f64 * 6.0, SONG_CHANNEL_VOLUME, 0.0, 2.0));
    let x = render(&mut e, ROW * 8);
    assert!((levels(&x, ROW * 3, ROW * 4).0 / full - 0.25).abs() < 1.0e-3);
    let (l, r) = levels(&x, ROW * 5, ROW * 6);
    assert!(r < l * 0.1, "panned left: {l} {r}");
    let steepest = |x: &[f32]| x[2000..].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    let peak = x.0.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(steepest(&x.0) < peak * 0.004 && steepest(&x.1) < peak * 0.004, "{} of {peak}", steepest(&x.0));
    assert!(steepest(&x.0) > 0.0);

    // The same for a synth patch's note, which is on a part of the host's:
    // its volume and pan are the channel's too.
    let lead = r#"{"row": 0, "ch": 2, "note": "A-4", "ins": 3}"#;
    let plain = render(&mut playing(song("", lead)), ROW * 4);
    let mixed = render(&mut playing(song(r#""pan": [128, 128, 255], "mixer": {"channels": [{}, {}, {"volume": 0.5}]},"#, lead)), ROW * 4);
    let ((pl, _), (ml, mr)) = (levels(&plain, 2000, ROW * 4), levels(&mixed, 2000, ROW * 4));
    assert!(ml < 1.0e-4 && mr > 0.0 && pl > 0.0);
    let mut e = playing(song("", lead));
    e.push(Event::song(ROW as f64 * 2.0, SONG_CHANNEL_VOLUME, 2.0, 0.5));
    let x = render(&mut e, ROW * 4);
    assert!((levels(&x, ROW * 3, ROW * 4).0 / levels(&x, 2000, ROW * 2).0 - 0.5).abs() < 0.01);
    assert_eq!(e.param(LEAD as usize, VOLUME), 1.0, "the host's part is left alone");
    // A command for a channel the song has not, or with no number, does nothing.
    e.push(Event::song(0.0, SONG_CHANNEL_VOLUME, 9.0, 0.0));
    e.push(Event::song(0.0, SONG_CHANNEL_PAN, 0.0, f32::NAN));
    render(&mut e, 512);
}

#[test]
fn a_send_is_heard_only_in_what_the_effect_gives_back() {
    // A note for two rows, then key off. Its channel sends to the delay,
    // a quarter of a second long, with no feedback: one echo.
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 2, "ch": 0, "note": "off"}"#;
    let dry = render(&mut playing(song("", events)), ROW * 8);
    let echo = |level: f32, send: f32| {
        let top = format!(r#""mixer": {{"channels": [{{"echo": {send}}}]}}, "effects": {{"echoTime": 0.25, "echoFeedback": 0, "echoLevel": {level}}},"#);
        render(&mut playing(song(&top, events)), ROW * 8)
    };
    // With the delay giving nothing back, the send changes nothing at all.
    let silent = echo(0.0, 1.0);
    assert!(same(&silent.0, &dry.0) && same(&silent.1, &dry.1));
    // Given back, the echo is there a quarter of a second after the note,
    // where the dry sound has already stopped; the dry sound is as it was.
    let wet = echo(1.0, 1.0);
    let (after, during) = (ROW * 2 + 2000..ROW * 2 + 9000, 3000..9000);
    assert!(rms(&dry.0[after.clone()]) < 1.0e-4 && rms(&wet.0[after.clone()]) > 0.05);
    assert!(same(&wet.0[..11000], &dry.0[..11000]), "nothing of the send is in the dry sound");
    // Half the send, or half the return, is half the echo.
    let (full, half_send, half_back) = (rms(&wet.0[after.clone()]), rms(&echo(1.0, 0.5).0[after.clone()]), rms(&echo(0.5, 1.0).0[after.clone()]));
    assert!((half_send / full - 0.5).abs() < 0.01 && (half_back / full - 0.5).abs() < 0.01, "{half_send} {half_back} {full}");
    let _ = during;

    // The reverb the same: with a send, a tail after the note that the dry song has not.
    let hall = render(&mut playing(song(r#""mixer": {"channels": [{"reverb": 0.5}]}, "effects": {"reverbType": "hall", "reverbDecay": 3, "reverbLevel": 1},"#, events)), ROW * 8);
    assert!(rms(&hall.0[ROW * 5..ROW * 8]) > 1.0e-3 && rms(&dry.0[ROW * 5..ROW * 8]) < 1.0e-5);
    // A send moved while the song plays is heard, and a reverb the song
    // set is the engine's reverb as the engine has it.
    let mut e = playing(song(r#""effects": {"reverbType": "plate", "reverbDecay": 2, "reverbSize": 0.25, "reverbLevel": 0.5},"#, r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 6, "ch": 0, "note": "off"}"#));
    assert_eq!((e.param(0, REVERB_TYPE), e.param(0, REVERB_DECAY), e.param(0, REVERB_SIZE), e.param(0, REVERB_LEVEL)), (2.0, 2.0, 0.25, 0.5));
    e.push(Event::song(ROW as f64 * 3.0, SONG_CHANNEL_REVERB, 0.0, 1.0));
    let x = render(&mut e, ROW * 12);
    assert!(rms(&x.0[ROW * 8..ROW * 11]) > 1.0e-3);
    assert!((e.param(32, SEND_REVERB) - 1.0).abs() < 1.0e-6 && e.param(33, SEND_REVERB) == 0.0);
}

#[test]
fn a_delay_timed_in_notes_keeps_to_the_songs_tempo() {
    // A dotted eighth is three rows: 0.36 s at 125 beats a minute, 0.3 s at 150.
    let mut e = playing(song(r#""effects": {"echoTime": "1/8."},"#, r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 4, "ch": 1, "fx": "F96"}"#));
    assert!((e.param(0, DELAY_TIME) - 0.36).abs() < 1.0e-6);
    render(&mut e, ROW * 3);
    assert!((e.param(0, DELAY_TIME) - 0.36).abs() < 1.0e-6);
    render(&mut e, ROW * 3);
    assert!((e.param(0, DELAY_TIME) - 0.3).abs() < 1.0e-6, "{}", e.param(0, DELAY_TIME));
    assert_eq!((e.param(0, DELAY_FEEDBACK), e.param(0, DELAY_LEVEL)), (0.4, 1.0), "what the song does not mention is left alone");
}

/// The four numbers a display reads for a part: peak left and right, RMS left and right.
fn meter(e: &Engine, part: u32) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    assert_eq!(unsafe { synths_core::abi::synths_levels_read(e, part, 1, out.as_mut_ptr()) }, 1);
    out
}

#[test]
fn a_channels_meter_shows_a_synth_patchs_note_as_well_as_a_samples() {
    // Channel 1 plays a sample on its own part; channel 2 a synth patch, on
    // part 10; channel 0 nothing.
    let events = r#"{"row": 0, "ch": 1, "note": "C-5", "ins": 2}, {"row": 0, "ch": 2, "note": "A-4", "ins": 3, "arp": {"chord": "min", "rate": "1/16", "gate": 0.8}},
        {"row": 8, "ch": 1, "note": "off"}, {"row": 8, "ch": 2, "note": "off"}"#;
    let mut e = playing(song(r#""mixer": {"channels": [{}, {}, {"volume": 0.5}]},"#, events));
    render(&mut e, ROW * 4);
    let (none, sample, patch, host) = (meter(&e, 32), meter(&e, 33), meter(&e, 34), meter(&e, LEAD as u32));
    assert_eq!(none, [0.0; 4]);
    assert!(sample[0] > 0.05 && sample[2] > 0.02);
    // The patch's channel reads what the host's part reads: the note is that part's only sound.
    assert!(patch[0] > 0.05 && patch[2] > 0.02, "{patch:?}");
    assert!((patch[0] / host[0] - 1.0).abs() < 0.05 && (patch[2] / host[2] - 1.0).abs() < 0.1, "{patch:?} {host:?}");
    // The master's meter is the engine's.
    assert!(meter(&e, 65535)[0] > 0.05);
    // The same song with the fader up reads twice as much.
    let mut louder = playing(song("", events));
    render(&mut louder, ROW * 4);
    assert!((meter(&louder, 34)[2] / patch[2] - 2.0).abs() < 0.1);
    // After key off the meters fall back to nothing.
    e.push(Event::song(e.time() as f64, SONG_REPEAT, 0.0, 0.0));
    render(&mut e, ROW * 4 + 44100 * 3);
    assert!(meter(&e, 34)[0] < 0.01 && meter(&e, 33)[0] < 0.01 && meter(&e, 34)[2] == 0.0, "{:?}", meter(&e, 34));
}

fn position(e: &Engine) -> [u32; 10] {
    song_player::shared(e).unwrap().position()
}

#[test]
fn the_song_goes_round_its_loop() {
    // Rows 4 to 11, round and round: row 12 is the end, and is never played.
    let events = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 12, "ch": 1, "note": "C-5", "ins": 2}"#;
    let mut e = playing(song(r#""loop": {"start": [0, 4], "end": [0, 12]},"#, events));
    let mut rows = Vec::new();
    for _ in 0..30 {
        render(&mut e, ROW);
        rows.push(position(&e)[3]);
    }
    assert_eq!(rows[..22], [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 4, 5, 6, 7, 8, 9, 10, 11, 4, 5]);
    assert_eq!((position(&e)[0], position(&e)[7]), (STATE_PLAYING, 3), "three times round by now");
    assert_eq!(e.active_voices(), 1, "the note on row 12 never started");
    // The loop's end is on its exact frame: the row after 11 begins when 12 would have.
    let mut e = playing(song(r#""loop": {"start": [0, 4], "end": [0, 12]},"#, events));
    render(&mut e, ROW * 12 - 1);
    assert_eq!(position(&e)[3], 11);
    render(&mut e, 2);
    assert_eq!(position(&e)[3], 4);
    // With repeat off, the loop's end is the song's end.
    let mut e = playing(song(r#""loop": {"start": [0, 4], "end": [0, 12]},"#, events));
    e.push(Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
    render(&mut e, ROW * 14);
    assert_eq!(position(&e)[0], STATE_STOPPED);
    // A start with no end: the song plays to its last row and goes back to the start, not to the top.
    let mut e = playing(song(r#""loop": {"start": [0, 10]},"#, events));
    render(&mut e, ROW * 16 + 100);
    assert_eq!(position(&e)[3], 10);
    // Set and taken away while it plays.
    let mut e = playing(song("", events));
    e.push(Event::song(0.0, SONG_LOOP_START, 0.0, 2.0));
    e.push(Event::song(0.0, SONG_LOOP_END, 0.0, 6.0));
    render(&mut e, ROW * 7 + 100);
    assert_eq!(position(&e)[3], 3);
    e.push(Event::song(e.time() as f64, SONG_LOOP_END, -1.0, 0.0));
    render(&mut e, ROW * 4);
    assert_eq!(position(&e)[3], 7);
}

#[test]
fn an_edited_songs_mix_takes_over_what_the_edit_moved_and_no_more() {
    let one = r#"{"row": 0, "ch": 0, "note": "C-4", "ins": 1}, {"row": 0, "ch": 1, "note": "C-5", "ins": 2}"#;
    let before = r#""mixer": {"channels": [{"volume": 0.5}, {"volume": 0.5}]},"#;
    let after = r#""pan": [128, 191], "mixer": {"volume": 0.5, "channels": [{"volume": 0.5}, {"volume": 1, "reverb": 0.25}]}, "effects": {"reverbType": "hall"}, "loop": {"end": [0, 8]},"#;
    let mut e = playing(song(before, one));
    // The host moves channel 0's fader by hand; then the song is edited, and channel 1's is what the edit changed.
    e.push(Event::song(0.0, SONG_CHANNEL_VOLUME, 0.0, 0.75));
    render(&mut e, ROW * 2);
    assert_eq!(song_player::replace(&mut e, song(after, one), 0, false), Ok(true));
    render(&mut e, ROW * 2);
    // Channel 0 is where the host put it; channel 1 and the song's volume are the new song's.
    let gain = 0.5 * 0.5;
    assert!((e.param(32, VOLUME) - gain * 0.75).abs() < 1.0e-6 && (e.param(33, VOLUME) - gain * 1.0).abs() < 1.0e-6, "{} {}", e.param(32, VOLUME), e.param(33, VOLUME));
    assert!((e.param(33, PAN) - (191.0 / 127.5 - 128.0 / 127.5)).abs() < 1.0e-5 && (e.param(33, SEND_REVERB) - 0.25).abs() < 1.0e-6, "{}", e.param(33, PAN));
    assert_eq!(e.param(0, REVERB_TYPE), 1.0);
    // And its loop: back to the top from row 8.
    render(&mut e, ROW * 5);
    assert_eq!(position(&e)[3], 0);
}

#[test]
fn a_trackers_glide_saved_as_a_plain_key_is_the_same_song_and_modules_keep_their_pan() {
    use synths_core::song::{load_module, Rules};
    // The effects of a MOD, under its own rules, written out as JSON: 3xx
    // becomes "glide", and the song read back is the same song, so it plays the same.
    let json = text(r#""rules": "protracker", "slides": "amiga","#, r#"{"row": 0, "ch": 0, "note": "C-2", "ins": 2}, {"row": 2, "ch": 0, "note": "G-2", "ins": 2, "fx": "308"}, {"row": 4, "ch": 0, "fx": "300"}, {"row": 6, "ch": 0, "note": "C-3", "ins": 2, "fx": "504"}"#).replace(r#""channels": 3"#, r#""channels": 4"#);
    let s = native::load(&json, &|_| None).unwrap();
    assert_eq!(s.rules, Rules::ProTracker);
    let saved = native::save(&s).json;
    assert!(saved.contains(r#""glide": 8"#) && saved.contains(r#""glide": true"#) && saved.contains(r#""fx": "504""#), "{saved}");
    let back = native::load(&saved, &|_| None).unwrap();
    assert_eq!(back, s);
    let play = |s: Song| {
        let mut e = Engine::new(SR);
        song_player::install(&mut e, s, FIRST_PART, 0, true).unwrap();
        e.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
        render(&mut e, ROW * 10)
    };
    let (a, b) = (play(s), play(back));
    assert!(same(&a.0, &b.0) && same(&a.1, &b.1) && rms(&a.0) > 0.01);
    // Every module that has been fetched for the player tests: through JSON and back, the same song.
    let mut modules = 0;
    for kind in ["mod", "xm"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/openmpt-tests").join(kind);
        for file in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(song) = load_module(&std::fs::read(file.path()).unwrap()) else { continue };
            let json = native::save_in_one(&song);
            assert_eq!(native::load(&json, &|_| None).unwrap(), song, "{:?}", file.path());
            assert_eq!(binary::load(&binary::save(&song)).unwrap(), song, "{:?}", file.path());
            // A MOD's channels are panned left, right, right, left, and that is in the song.
            if kind == "mod" && song.channels() == 4 {
                assert_eq!(song.channel_pan, [0, 255, 255, 0], "{:?}", file.path());
                assert!(json.contains(r#""pan": [0, 255, 255, 0]"#));
            }
            modules += 1;
        }
    }
    println!("{modules} modules went through JSON and back");
}

#[test]
fn the_example_and_the_documents_mix_are_songs_this_reads() {
    let file = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/songs/mix.json")).unwrap();
    let s = native::load(&file, &|_| None).unwrap();
    assert!(s.has_mix() && s.mix.len() == 3 && !s.effects.is_empty() && s.loop_end.is_some());
    assert_eq!(native::load(&native::save(&s).json, &|_| None).unwrap(), s);
    assert_eq!(binary::load(&binary::save(&s)).unwrap(), s);
    // The three keys as the song format's document gives them.
    let doc = include_str!("../docs/song-format.md");
    let at = doc.find("## The mix").unwrap();
    let block = doc[at..].split("```json\n").nth(1).unwrap().split("```").next().unwrap();
    let s = native::load(&text(&format!("{},", block.trim_end()).replace(r#""end": [5, 0]"#, r#""end": [0, 8]"#).replace(r#""start": [1, 0]"#, r#""start": [0, 0]"#), NOTES), &|_| None).unwrap();
    assert_eq!((s.volume, s.mix.len(), s.effects.get("reverbType"), s.effects.get("echoFeedback")), (0.9, 3, Some(1.0), Some(0.45)));
    for key in ["volume", "reverb", "echo", "echoTime"].into_iter().chain(synths_core::song::EFFECT_KEYS.iter().map(|(key, _)| *key)) {
        assert!(doc[at..].contains(&format!("| `{key}` |")), "{key}");
    }
    // The engine's document gives the commands their numbers.
    let api = include_str!("../docs/engine-api.md");
    for (n, what) in [(SONG_CHANNEL_VOLUME, "a channel's volume"), (SONG_CHANNEL_PAN, "a channel's pan"), (SONG_CHANNEL_REVERB, "a channel's reverb send"), (SONG_CHANNEL_DELAY, "a channel's echo send"), (SONG_LOOP_START, "the loop's start"), (SONG_LOOP_END, "the loop's end")] {
        assert!(api.contains(&format!("| {n} | {what} |")), "{what}");
    }
    assert_eq!((REVERB_LEVEL, DELAY_LEVEL), (225, 226));
}
