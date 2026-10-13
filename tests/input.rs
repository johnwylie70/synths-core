//! The live input: sound handed to the engine as it renders, playing
//! through a part.

mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::input::{GATE_ATTACK, GATE_CLOSE, GATE_FALL};
use synths_core::math::fast_sin;
use synths_core::params::*;
use synths_core::{Engine, Event};

/// The part the input plays through unless a host says otherwise.
const PART: u16 = 31;

/// The same but for numbers under 0.000_000_000_1, which the input takes as silence.
fn same(out: f32, wanted: f32) -> bool {
    out == wanted || (out == 0.0 && wanted.abs() < 1.0e-10)
}

/// A voice-like mixture that never repeats: three tones and some hiss.
fn babble(frames: usize) -> Vec<f32> {
    let h = hiss(7, 0.1, frames);
    (0..frames)
        .map(|i| {
            let t = i as f32 / SR;
            0.3 * (t * 1380.0).sin() + 0.2 * (t * 4410.0).sin() + 0.1 * (t * 15000.0).sin() + h[i]
        })
        .collect()
}

#[test]
fn the_input_comes_out_unchanged_but_for_the_pan() {
    let x = babble(20000);
    // A mono input is placed like a mono voice: hard left it is all on the
    // left, and in the middle 0.7071 of it is on each side.
    for (pan, gl, gr) in [(-1.0, fast_sin(0.25), 0.0), (0.0, fast_sin(0.375), fast_sin(0.125)), (1.0, 0.0, fast_sin(0.25))] {
        for block in [1, 16, 128, 333, 4096] {
            let mut e = plain_engine();
            e.set_param(PART, PAN, pan);
            let (l, r) = render_io(&mut e, &x, None, block);
            for i in 0..x.len() {
                assert_eq!((l[i], r[i]), (x[i] * gl, x[i] * gr), "pan {pan}, blocks of {block}, frame {i}");
            }
        }
    }
    assert!((fast_sin(0.375) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1.0e-5);
}

#[test]
fn a_stereo_input_keeps_its_two_sides() {
    let (xl, xr) = (babble(9000), sine(300.0, 0.4, 9000));
    let mut e = plain_engine();
    e.set_param(PART, PAN, 0.0);
    let (l, r) = render_io(&mut e, &xl, Some(&xr), 128);
    assert_eq!((l, r), (xl.clone(), xr.clone()));
    // Panned, the far side is turned down.
    let mut e = plain_engine();
    e.set_param(PART, PAN, 0.5);
    let (l, r) = render_io(&mut e, &xl, Some(&xr), 128);
    assert!(l.iter().zip(xl.iter()).all(|(a, b)| *a == b * 0.5) && r == xr);
}

#[test]
fn the_input_plays_through_whichever_part_it_is_given() {
    let x = sine(500.0, 0.5, 9600);
    let mut e = plain_engine();
    e.set_param(0, INPUT_PART, 3.0);
    e.set_param(3, VOLUME, 0.5);
    e.set_param(0, INPUT_GAIN, 3.0);
    let (l, _) = render_io(&mut e, &x, None, 128);
    // Three times as loud coming in, and the part at half volume.
    assert!((peak(&l[4800..]) - 0.75).abs() < 0.001, "{}", peak(&l[4800..]));
    // The part's meter and scope show it, and no other part's do.
    let levels = e.taps().read_levels(3).unwrap();
    assert!(levels[0] > 0.7 && e.taps().read_levels(PART as u32).unwrap() == [0.0; 4]);
    let mut scope = [0.0f32; 256];
    assert_eq!(e.taps().read_scope(3, &mut scope), 256);
    assert!(peak(&scope) > 0.3);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn the_parts_filter_distortion_and_sends_shape_the_input() {
    // The part's filter is the input's tone control: a band-pass at 1200 Hz
    // lets 1200 Hz through and takes most of 100 Hz away.
    let level = |hz: f32, patch: &[(usize, f32)]| {
        let mut e = plain_engine();
        for &(id, v) in patch {
            e.set_param(PART, id, v);
        }
        let (l, _) = render_io(&mut e, &sine(hz, 0.25, 24000), None, 128);
        rms(&l[12000..])
    };
    let band = [(FILTER_TYPE, 3.0), (FILTER_SLOPE, 1.0), (CUTOFF, 1200.0), (RESONANCE, 0.2)];
    let dry = level(1200.0, &[]);
    assert!(db(level(1200.0, &band) / dry).abs() < 3.0);
    assert!(db(level(100.0, &band) / dry) < -30.0);
    // Distortion makes it louder and squarer; the bit crusher makes it coarse.
    assert!(level(300.0, &[(DRIVE, 0.8)]) > 1.5 * dry);
    let mut e = plain_engine();
    e.set_param(PART, CRUSH_BITS, 3.0);
    let (l, _) = render_io(&mut e, &sine(300.0, 0.25, 4800), None, 128);
    assert!(l.iter().all(|v| (v * 4.0).fract() == 0.0), "three bits leave steps of a quarter");
    // A send to the echo: the input comes back 0.1 seconds later.
    let mut e = plain_engine();
    e.set_param(PART, SEND_DELAY, 1.0);
    e.set_param(0, DELAY_TIME, 0.1);
    e.set_param(0, DELAY_TONE, 20000.0);
    let mut x = sine(1000.0, 0.5, 480);
    x.resize(9600, 0.0);
    let (l, _) = render_io(&mut e, &x, None, 128);
    assert!(peak(&l[1000..4700]) < 0.001 && peak(&l[4800..5280]) > 0.3);
}

#[test]
fn the_gate_opens_at_its_level_and_closes_under_it() {
    let attack = (GATE_ATTACK * SR) as usize;
    let release = 0.05;
    let mut e = plain_engine();
    e.set_param(0, INPUT_GATE, -20.0);
    e.set_param(0, INPUT_GATE_RELEASE, release);
    // Hiss a little under the level (0.1), then a tone well over it, then
    // a quiet tone that is under the level at which the gate closes (0.05).
    let mut x = hiss(3, 0.09, 4800);
    x.extend(sine(1000.0, 0.4, 9600));
    x.extend(sine(1000.0, 0.02, 14400));
    let (l, r) = render_io(&mut e, &x, None, 128);
    let g = fast_sin(0.25);
    // Shut: exactly nothing.
    assert!(l[..4800].iter().chain(r.iter()).all(|&v| v == 0.0));
    // It opens on the first frame that reaches the level, takes 2 ms to
    // open all the way, and from then on changes nothing.
    let first = (4800..).find(|&i| x[i].abs() >= 0.1).unwrap();
    assert!(l[4800..first].iter().all(|&v| v == 0.0) && l[first] != 0.0, "opened at {:?}, not {first}", l.iter().position(|&v| v != 0.0));
    assert!(l[first].abs() < x[first].abs() * 0.02);
    let half = first + attack / 2;
    assert!((l[half] / (x[half] * g) - 0.5).abs() < 0.02);
    for i in first + attack..14400 {
        assert!(same(l[i], x[i] * g), "frame {i}");
    }
    // Once the tone drops, the gate's measure of loudness falls from the
    // tone's last peak (0.4) to the closing level in GATE_FALL * ln(0.4 / 0.05)
    // seconds; then the gate closes in a straight line over the release time.
    let last_peak = (0..14400).rev().find(|&i| x[i].abs() > 0.3999).unwrap();
    let closing = last_peak + (GATE_FALL * SR * (0.4f32 / (0.1 * GATE_CLOSE)).ln()) as usize;
    let closed = closing + (release * SR) as usize;
    let gain = |i: usize| l[i] / (x[i] * g);
    let near_peak = |from: usize| (from..from + 48).max_by(|&a, &b| x[a].abs().total_cmp(&x[b].abs())).unwrap();
    assert!((gain(near_peak(closing - 100)) - 1.0).abs() < 1.0e-6);
    let mid = near_peak(closing + (release * SR * 0.5) as usize);
    assert!((gain(mid) - 0.5).abs() < 0.03, "half closed: {}", gain(mid));
    assert!(peak(&l[closed - 100..closed - 10]) > 0.0);
    assert!(l[closed + 10..].iter().all(|&v| v == 0.0), "it never closed");
}

#[test]
fn with_the_gate_off_the_input_is_never_touched() {
    // Very quiet sound still comes through exactly.
    let x = hiss(11, 1.0e-6, 4800);
    let mut e = plain_engine();
    let (l, _) = render_io(&mut e, &x, None, 128);
    let g = fast_sin(0.25);
    assert!(l.iter().zip(x.iter()).all(|(a, b)| *a == b * g));
}

#[test]
fn the_monitor_can_be_turned_off_and_on() {
    let x = sine(440.0, 0.5, 14400);
    let mut e = plain_engine();
    e.set_param(0, INPUT_MONITOR, 0.0);
    e.push(Event::param(4800.0, 0, INPUT_MONITOR, 1.0, 0.0));
    e.push(Event::param(9600.0, 0, INPUT_MONITOR, 0.0, 0.0));
    let (l, r) = render_io(&mut e, &x, None, 128);
    let g = fast_sin(0.25);
    assert!(l[..4800].iter().chain(l[9600..].iter()).chain(r.iter()).all(|&v| v == 0.0));
    assert!((4800..9600).all(|i| same(l[i], x[i] * g)));
}

#[test]
fn a_volume_moved_while_the_input_sounds_slides_and_does_not_jump() {
    // A steady level coming in; the part's volume, then the master's, is
    // halved. The output gets there over a few milliseconds, never in one step.
    for (part, id) in [(PART, VOLUME), (0, MASTER_VOLUME)] {
        let mut e = plain_engine();
        e.push(Event::param(4800.0, part, id, 0.5, 0.0));
        let (l, _) = render_io(&mut e, &vec![0.5; 9600], None, 128);
        let g = fast_sin(0.25);
        assert!((l[4799] - 0.5 * g).abs() < 1.0e-6 && (l[9000] - 0.25 * g).abs() < 1.0e-4);
        let step = (4800..9600).map(|i| (l[i] - l[i - 1]).abs()).fold(0.0f32, f32::max);
        assert!(step < 0.002, "the output jumped by {step}");
    }
}

#[test]
fn silence_and_garbage_coming_in_do_no_harm() {
    // Silence, and numbers too small to matter, are nothing at all: the
    // engine stays asleep and its output is exact zeros.
    let mut e = Engine::new(SR);
    let mut x = vec![0.0f32; 48000];
    for (i, v) in x.iter_mut().enumerate() {
        if i % 3 == 0 {
            *v = if i % 2 == 0 { 1.0e-40 } else { -3.0e-12 };
        }
    }
    let (l, r) = render_io(&mut e, &x, Some(&x), 128);
    assert!(l.iter().chain(r.iter()).all(|&v| v == 0.0));
    assert_eq!(e.taps().read_levels(PART as u32).unwrap(), [0.0; 4]);
    // Numbers that are not numbers count as silence; huge ones are held back.
    let mut e = Engine::new(SR);
    e.set_param(PART, SEND_REVERB, 1.0);
    e.set_param(PART, SEND_DELAY, 1.0);
    e.set_param(PART, FX_PITCH, 5.0);
    e.set_param(PART, FX_COMB, 1.0);
    e.set_param(0, INPUT_GAIN, 8.0);
    let wild = [f32::NAN, f32::INFINITY, -f32::INFINITY, 1.0e30, -1.0e30, 1.0e-40, 0.5, -3.0];
    let x: Vec<f32> = (0..48000).map(|i| wild[(i * 7 + i / 5) % wild.len()]).collect();
    let (l, r) = render_io(&mut e, &x, Some(&x), 61);
    assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0));
    assert!(peak(&l) > 0.5);
    // And when it stops, everything dies away to exactly nothing.
    let (l, r) = render_io(&mut e, &vec![0.0; 48000 * 12], None, 128);
    assert_eq!(l.iter().chain(r.iter()).filter(|v| v.is_subnormal()).count(), 0);
    assert!(l[l.len() - 4800..].iter().all(|&v| v == 0.0));
}

#[test]
fn the_input_sounds_the_same_whatever_the_block_size() {
    // Everything at once: a gate that opens and closes, the part's filter,
    // every voice effect, the distortion and the bit crusher, the sends,
    // a synth note on the same part, and parameters moving.
    let run = |block: usize| {
        let mut e = Engine::new(SR);
        for (id, v) in [
            (FILTER_TYPE, 1.0), (CUTOFF, 3000.0), (FX_PITCH, 5.0), (FX_RING, 0.5), (FX_RING_FREQ, 70.0),
            (FX_COMB, 0.5), (FX_COMB_DEPTH, 0.6), (DRIVE, 0.3), (CRUSH_BITS, 9.0), (CRUSH_RATE, 3.0),
            (SEND_DELAY, 0.3), (SEND_REVERB, 0.3), (SEND_CHORUS, 0.3),
        ] {
            e.set_param(PART, id, v);
        }
        e.set_param(0, INPUT_GATE, -30.0);
        e.set_param(0, INPUT_GATE_RELEASE, 0.02);
        e.push(Event::param(5000.0, PART, FX_PITCH, -7.0, 0.2));
        e.push(Event::param(7001.0, PART, PAN, 0.8, 0.1));
        e.push(Event::param(9003.0, PART, CUTOFF, 800.0, 0.05));
        e.push(Event::param(11111.0, PART, FX_RING_WAVE, 1.0, 0.0));
        e.push(Event::note_on(13001.0, PART, 0, 52.0, 0.8));
        e.push(Event::note_off(15007.0, PART, 0, 52.0));
        e.push(Event::param(20011.0, PART, FX_PITCH, 0.0, 0.0));
        e.push(Event::param(30001.0, 0, INPUT_PART, 2.0, 0.0));
        let mut x = speech::speech(SR, 1.0);
        x[20000..23000].fill(0.0);
        x.resize(96000, 0.0);
        render_io(&mut e, &x, None, block)
    };
    let (l, r) = run(128);
    assert!(peak(&l) > 0.05 && peak(&r) > 0.05);
    for block in [1, 7, 16, 100, 4096] {
        let (bl, br) = run(block);
        assert!(l == bl && r == br, "blocks of {block} sound different from blocks of 128");
    }
}

#[test]
fn a_synth_can_play_while_the_input_does() {
    // The input on its part and a note on another: the output is their sum.
    let x = sine(700.0, 0.2, 9600);
    let alone = |with_input: bool, with_note: bool| {
        let mut e = plain_engine();
        e.set_param(ALL_PARTS, OSC1_WAVE, 0.0);
        e.set_param(0, VOLUME, 0.5);
        if with_note {
            note_on(&mut e, 0, 0, 60.0);
        }
        if with_input {
            render_io(&mut e, &x, None, 128).0
        } else {
            render(&mut e, 9600, 128).0
        }
    };
    let (both, input, note) = (alone(true, true), alone(true, false), alone(false, true));
    assert!((0..9600).all(|i| (both[i] - (input[i] + note[i])).abs() < 1.0e-6));
}
