//! Recording inside the engine: the live input, a part or the whole output
//! written into a sample slot, between two events.

mod common;
use common::*;
use synths_core::engine::SampleError;
use synths_core::event::{ALL_PARTS, RECORD_INPUT, RECORD_MASTER, RECORD_PART};
use synths_core::math::fast_sin;
use synths_core::params::*;
use synths_core::{Engine, Event};

const PART: u16 = 31;
const SLOT: usize = 5;

/// Everything in a slot: `channels` numbers a frame.
fn slot(e: &Engine, slot: usize, channels: usize) -> Vec<f32> {
    let mut out = vec![0.0; e.sample_frames(slot) * channels];
    assert_eq!(e.read_sample(slot, 0, &mut out), e.sample_frames(slot));
    out
}

/// A second or so of speech-like sound, with nothing in it so quiet that
/// the input would take it for silence.
fn voice(frames: usize) -> Vec<f32> {
    let mut x = speech::speech(SR, frames as f32 / SR + 0.1);
    x.truncate(frames);
    for v in x.iter_mut() {
        if v.abs() < 1.0e-9 {
            *v = 0.0;
        }
    }
    x
}

#[test]
fn a_recording_holds_exactly_the_frames_between_start_and_stop() {
    let x = voice(24000);
    for block in [1, 64, 128, 1000, 4096] {
        let mut e = plain_engine();
        e.record_buffer(SLOT, 48000, 1).unwrap();
        assert_eq!((e.sample_frames(SLOT), e.recording()), (0, [0, 0, 0, 0]));
        e.push(Event::record_start(1003.0, SLOT as u32, RECORD_INPUT, 0));
        e.push(Event::record_stop(15324.0));
        let mut at = 0;
        while at < x.len() {
            let n = block.min(x.len() - at);
            let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
            e.render_io(&x[at..at + n], None, &mut l, &mut r);
            at += n;
            // The count a display reads keeps step with the render. (The
            // event that falls on a block's first frame is carried out
            // when that block is rendered.)
            let wanted = at.clamp(1003, 15324) - 1003;
            assert_eq!(e.recording(), [(at > 1003 && at <= 15324) as u32, if at > 1003 { SLOT as u32 } else { 0 }, wanted as u32, if at > 1003 { 48000 } else { 0 }]);
        }
        assert_eq!(e.sample_frames(SLOT), 15324 - 1003);
        assert!(slot(&e, SLOT, 1) == x[1003..15324], "blocks of {block}: the recording is not the input");
    }
}

#[test]
fn a_recording_is_the_same_whatever_the_block_size() {
    // The input after its gain and gate; a part with every effect on; the
    // whole output with its echo and reverb.
    let x = voice(48000);
    let run = |source: u32, block: usize| {
        let mut e = Engine::new(SR);
        for (id, v) in [(FX_PITCH, -5.0), (FX_RING, 0.4), (FX_COMB, 0.4), (DRIVE, 0.3), (CRUSH_RATE, 4.0), (SEND_DELAY, 0.4), (SEND_REVERB, 0.4), (PAN, 0.3)] {
            e.set_param(PART, id, v);
        }
        e.set_param(0, INPUT_GATE, -30.0);
        e.set_param(0, INPUT_GAIN, 1.7);
        e.record_buffer(SLOT, 40000, 2).unwrap();
        e.push(Event::record_start(777.0, SLOT as u32, source, PART));
        e.push(Event::record_stop(36001.0));
        render_io(&mut e, &x, None, block);
        slot(&e, SLOT, 2)
    };
    for source in [RECORD_INPUT, RECORD_PART, RECORD_MASTER] {
        let whole = run(source, 4096);
        assert_eq!(whole.len(), (36001 - 777) * 2);
        assert!(peak(&whole) > 0.05, "source {source} recorded nothing");
        for block in [1, 16, 128, 333] {
            assert!(run(source, block) == whole, "source {source}: blocks of {block} record something different");
        }
    }
}

#[test]
fn each_source_records_what_it_says() {
    let x = voice(24000);
    let run = |source: u32, monitor: f32| {
        let mut e = plain_engine();
        e.set_param(PART, PAN, 0.0);
        e.set_param(PART, DRIVE, 0.5);
        e.set_param(PART, VOLUME, 0.5);
        e.set_param(PART, SEND_DELAY, 0.5);
        e.set_param(0, DELAY_TIME, 0.05);
        e.set_param(0, INPUT_GAIN, 2.0);
        e.set_param(0, INPUT_MONITOR, monitor);
        e.record_buffer(SLOT, 24000, 2).unwrap();
        e.push(Event::record_start(0.0, SLOT as u32, source, PART));
        let (l, r) = render_io(&mut e, &x, None, 128);
        (slot(&e, SLOT, 2), l, r)
    };
    // The input: after its gain, before the part does anything to it, and
    // whether or not it is being heard.
    for monitor in [1.0, 0.0] {
        let (rec, l, _) = run(RECORD_INPUT, monitor);
        assert!((0..24000).all(|i| rec[i * 2] == x[i] * 2.0 && rec[i * 2 + 1] == x[i] * 2.0));
        assert_eq!(peak(&l) > 0.0, monitor > 0.0);
    }
    // A part: after its distortion and volume, before the echo.
    let (rec, _, _) = run(RECORD_PART, 1.0);
    let trim = 1.0 / (1.0f32 + 30.0 * 0.25).sqrt().sqrt();
    for i in 0..24000 {
        let wanted = synths_core::math::fast_tanh(x[i] * 2.0 * fast_sin(0.375) * (1.0 + 30.0 * 0.25)) * trim * 0.5;
        assert!((rec[i * 2] - wanted).abs() < 1.0e-6, "frame {i}: {} for {wanted}", rec[i * 2]);
    }
    // The whole output: exactly what the render gave.
    let (rec, l, r) = run(RECORD_MASTER, 1.0);
    assert!((0..24000).all(|i| rec[i * 2] == l[i] && rec[i * 2 + 1] == r[i]));
    // Into a slot with one channel, the two sides are averaged.
    let mut e = plain_engine();
    e.set_param(PART, PAN, 1.0);
    e.record_buffer(SLOT, 24000, 1).unwrap();
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_MASTER, 0));
    let (l, r) = render_io(&mut e, &x, None, 128);
    let rec = slot(&e, SLOT, 1);
    assert!((0..24000).all(|i| rec[i] == 0.5 * (l[i] + r[i])) && peak(&rec) > 0.1);
}

#[test]
fn a_recording_of_a_synth_part_and_of_silence() {
    // Any part can be recorded, not only the input's. Before the note and
    // after it has gone, and while the engine sleeps, the recording goes on
    // in silence: its length is the time between the two events.
    let mut e = plain_engine();
    e.set_param(2, OSC1_WAVE, 0.0);
    e.set_param(2, VOLUME, 0.5);
    e.record_buffer(SLOT, 480000, 1).unwrap();
    e.push(Event::record_start(100.0, SLOT as u32, RECORD_PART, 2));
    e.push(Event::note_on(4900.0, 2, 0, 69.0, 1.0));
    e.push(Event::all_notes_off(9700.0, 2, true));
    e.push(Event::record_stop(300100.0));
    let (l, _) = render(&mut e, 320000, 128);
    let rec = slot(&e, SLOT, 1);
    assert_eq!(rec.len(), 300000);
    assert!(rec[..4800].iter().all(|&v| v == 0.0) && rec[20000..].iter().all(|&v| v == 0.0));
    // Hard left, so a mono slot holds half of what the left output had.
    assert!((0..300000).all(|i| rec[i] == 0.5 * l[i + 100]));
    assert!((frequency(&rec[4800..9600], SR) - 440.0).abs() < 0.5);
}

#[test]
fn a_recording_stops_by_itself_when_the_slot_is_full() {
    let x = voice(12000);
    let mut e = plain_engine();
    e.record_buffer(SLOT, 5000, 1).unwrap();
    e.push(Event::record_start(1000.0, SLOT as u32, RECORD_INPUT, 0));
    render_io(&mut e, &x, None, 128);
    assert_eq!(e.recording(), [0, SLOT as u32, 5000, 5000]);
    assert!(slot(&e, SLOT, 1) == x[1000..6000]);
    // Starting again records over it from the beginning.
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
    e.push(Event::record_stop(e.time() as f64 + 100.0));
    render_io(&mut e, &x, None, 128);
    assert!(slot(&e, SLOT, 1) == x[..100]);
}

#[test]
fn there_is_one_recorder_and_a_new_start_moves_it() {
    let x = voice(9600);
    let mut e = plain_engine();
    e.record_buffer(SLOT, 9600, 1).unwrap();
    e.record_buffer(SLOT + 1, 9600, 1).unwrap();
    e.push(Event::record_start(1000.0, SLOT as u32, RECORD_INPUT, 0));
    e.push(Event::record_start(3000.0, SLOT as u32 + 1, RECORD_INPUT, 0));
    // A start into the slot being recorded begins it again.
    e.push(Event::record_start(5000.0, SLOT as u32 + 1, RECORD_INPUT, 0));
    e.push(Event::record_stop(6500.0));
    // A stop with nothing recording does nothing.
    e.push(Event::record_stop(7000.0));
    render_io(&mut e, &x, None, 128);
    assert!(slot(&e, SLOT, 1) == x[1000..3000] && slot(&e, SLOT + 1, 1) == x[5000..6500]);
    assert_eq!(e.recording(), [0, SLOT as u32 + 1, 1500, 9600]);
}

#[test]
fn a_recording_plays_back_as_a_sample() {
    // Record a second of a 440 Hz tone, with a click at its very start.
    let mut x = sine(440.0, 0.5, 48000);
    x[0] = 0.25;
    let mut e = plain_engine();
    e.set_param(0, INPUT_MONITOR, 0.0);
    e.record_buffer(SLOT, 48000, 1).unwrap();
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
    render_io(&mut e, &x, None, 128);
    assert_eq!(e.recording(), [0, SLOT as u32, 48000, 48000]);
    e.set_param(ALL_PARTS, OSC1_WAVE, 4.0);
    e.set_param(ALL_PARTS, OSC1_SAMPLE, SLOT as f32);
    e.set_param(ALL_PARTS, SAMPLE_ANTIALIAS, 0.0);
    // At note 60 it comes back exactly as it went in.
    let t = e.time();
    note_on(&mut e, t, 0, 60.0);
    let (l, _) = render(&mut e, 48000, 128);
    let g = fast_sin(0.25);
    assert!((0..47990).all(|i| (l[i] - x[i] * g).abs() < 1.0e-6));
    e.push(Event::all_notes_off(0.0, ALL_PARTS, true));
    render(&mut e, 4800, 128);
    // An octave up it is twice as high and half as long.
    let t = e.time();
    note_on(&mut e, t, 0, 72.0);
    let (l, _) = render(&mut e, 48000, 128);
    assert!((frequency(&l[2400..21600], SR) - 880.0).abs() < 0.5);
    assert!(peak(&l[26000..]) == 0.0 && e.active_voices() == 0);
    // Backwards, the click comes last.
    e.set_param(0, OSC1_REVERSE, 1.0);
    let t = e.time();
    note_on(&mut e, t, 0, 60.0);
    let (l, _) = render(&mut e, 48000, 128);
    assert!((l[47999] - 0.25 * g).abs() < 1.0e-6 && (l[47998] - x[1] * g).abs() < 1.0e-6);
    e.set_param(0, OSC1_REVERSE, 0.0);
    render(&mut e, 4800, 128);
    // With a loop it goes on for as long as the note is held.
    e.configure_sample(SLOT, 60.0, 1, 4800, 4800 + 4800).unwrap();
    let t = e.time();
    note_on(&mut e, t, 0, 60.0);
    let (l, _) = render(&mut e, 96000, 128);
    assert!((frequency(&l[60000..], SR) - 440.0).abs() < 0.5 && e.active_voices() == 1);
}

#[test]
fn a_slot_can_be_played_while_it_is_recorded_into() {
    // A voice plays the slot from its start while the input is written a
    // little way ahead of it: it follows, and it stops if it catches up.
    let x = sine(330.0, 0.5, 48000);
    let mut e = plain_engine();
    e.set_param(0, INPUT_MONITOR, 0.0);
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, OSC1_SAMPLE, SLOT as f32);
    e.record_buffer(SLOT, 48000, 1).unwrap();
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
    e.push(Event::note_on(2400.0, 0, 0, 60.0, 1.0));
    let (l, _) = render_io(&mut e, &x, None, 128);
    let g = fast_sin(0.25);
    assert!((2400..47000).all(|i| (l[i] - x[i - 2400] * g).abs() < 1.0e-6));
}

#[test]
fn recording_into_nothing_is_refused_and_a_lost_slot_ends_it() {
    let x = voice(9600);
    let mut e = plain_engine();
    // No slot was made: nothing happens.
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
    render_io(&mut e, &x, None, 128);
    assert_eq!((e.recording()[0], e.sample_frames(SLOT)), (0, 0));
    // Slots that do not exist, sources that do not exist, numbers that are not numbers.
    for (slot, source, part) in [(5000.0, 0.0, 0), (-1.0, 0.0, 0), (f32::NAN, 0.0, 0), (SLOT as f32, 7.0, 0), (SLOT as f32, f32::NAN, 0), (SLOT as f32, 1.0, 64), (SLOT as f32, -3.0, 0)] {
        e.record_buffer(SLOT, 1000, 1).unwrap();
        let mut ev = Event::record_start(0.0, 0, 0, part);
        ev.x = slot;
        ev.y = source;
        e.push(ev);
        render_io(&mut e, &x[..256], None, 128);
        assert_eq!(e.recording()[0], 0, "slot {slot}, source {source}, part {part}");
    }
    assert_eq!(e.record_buffer(1024, 100, 1), Err(SampleError::BadSlot));
    assert_eq!(e.record_buffer(SLOT, 0, 1), Err(SampleError::BadFormat));
    assert_eq!(e.record_buffer(SLOT, 100, 3), Err(SampleError::BadFormat));
    // The slot is emptied, or loaded with something else, while recording.
    for load in [false, true] {
        e.record_buffer(SLOT, 48000, 2).unwrap();
        e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
        render_io(&mut e, &x[..1280], None, 128);
        assert_eq!(e.recording(), [1, SLOT as u32, 1280, 48000]);
        if load {
            e.load_sample(SLOT, &[0.5; 64], 1, SR).unwrap();
        } else {
            e.free_sample(SLOT).unwrap();
        }
        render_io(&mut e, &x[..1280], None, 128);
        assert_eq!(e.recording()[0], 0);
        assert_eq!(e.sample_frames(SLOT), if load { 64 } else { 0 });
    }
    // The panic button stops a recording, and keeps what there is of it.
    e.record_buffer(SLOT, 48000, 1).unwrap();
    e.push(Event::record_start(0.0, SLOT as u32, RECORD_INPUT, 0));
    render_io(&mut e, &x[..640], None, 128);
    e.reset();
    render_io(&mut e, &x[..640], None, 128);
    assert_eq!((e.recording()[0], e.sample_frames(SLOT)), (0, 640));
}
