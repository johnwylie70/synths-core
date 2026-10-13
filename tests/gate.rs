//! The trance gate: a part's sound chopped in a pattern of steps in time
//! with the tempo, and the beat the engine keeps for it.

mod common;
use common::*;
use synths_core::event::ALL_PARTS;
use synths_core::gate::{self, GATE_HELP, NAMED_GATES};
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::pattern_abi::*;
use synths_core::{Engine, Event};

/// A steady tone on part 0 that fills the left output exactly: every
/// sample of a pulse wave's top is the same, so the output is the gate.
fn held_note(pattern: &str, tempo: f32) -> Box<Engine> {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, TEMPO, tempo);
    let (levels, n) = gate::parse(pattern).unwrap();
    e.set_gate(0, &levels[..n]).unwrap();
    e.push(Event::note_on(0.0, 0, 1, 81.0, 1.0));
    e
}

/// How open the gate is over a stretch: the sound's level against the same sound ungated.
fn open(x: &[f32], plain: &[f32], from: usize, to: usize) -> f32 {
    rms(&x[from..to]) / rms(&plain[from..to])
}

#[test]
fn steps_open_and_close_on_the_beat_at_any_tempo_and_rate() {
    for (tempo, rate) in [(120.0f32, 4.0f32), (140.0, 4.0), (97.0, 2.0), (174.0, 8.0), (120.0, 3.0)] {
        let step = SR as f64 * 60.0 / (tempo as f64 * rate as f64);
        let frames = (step * 16.0) as usize;
        let mut plain = held_note("x", tempo);
        let reference = render(&mut plain, frames, 128).0;
        let mut e = held_note("x.x.xx.x", tempo);
        e.set_param(0, GATE_DEPTH, 1.0);
        e.set_param(0, GATE_RATE, rate);
        e.set_param(0, GATE_SMOOTH, 0.001);
        let x = render(&mut e, frames, 128).0;
        for (k, on) in "x.x.xx.xx.x.xx.x".bytes().map(|c| c == b'x').enumerate() {
            // The step begins on its exact frame: up to there the gate is
            // where the last step left it, and from there it moves.
            let at = (k as f64 * step).ceil() as usize;
            let was_on = k == 0 || "x.x.xx.xx.x.xx.x".as_bytes()[k - 1] == b'x';
            if k > 0 && on != was_on {
                let before = x[at - 1] / reference[at - 1];
                let after = x[at + 40] / reference[at + 40];
                if reference[at - 1].abs() > 0.05 && reference[at + 40].abs() > 0.05 {
                    assert!((before - was_on as u8 as f32).abs() < 0.02, "tempo {tempo} rate {rate} step {k}: {before} just before");
                    assert!((after - before).abs() > 0.3, "tempo {tempo} rate {rate} step {k}: it has begun to move");
                }
            }
            // And by a fifth of the way through, it is there.
            let (from, to) = (at + (step * 0.3) as usize, at + (step * 0.95) as usize);
            let level = open(&x, &reference, from, to);
            assert!((level - on as u8 as f32).abs() < 0.01, "tempo {tempo} rate {rate} step {k}: {level}");
        }
    }
}

#[test]
fn levels_depth_and_the_default_pattern() {
    let frames = 6000 * 8;
    let reference = render(&mut held_note("x", 120.0), frames, 128).0;
    let steps = |e: &mut Engine| {
        let x = render(e, frames, 128).0;
        (0..8).map(|k| (open(&x, &reference, k * 6000 + 2000, k * 6000 + 5800) * 100.0).round() as i32).collect::<Vec<_>>()
    };
    // Levels between open and closed: 0 to 9 are ninths.
    let mut e = held_note("x0369.x.", 120.0);
    e.set_param(0, GATE_DEPTH, 1.0);
    assert_eq!(steps(&mut e), [100, 0, 33, 67, 100, 0, 100, 0]);
    // Depth: how far a closed step closes.
    let mut e = held_note("x.x3", 120.0);
    e.set_param(0, GATE_DEPTH, 0.75);
    assert_eq!(steps(&mut e), [100, 25, 100, 50, 100, 25, 100, 50]);
    // No depth, no gate; and with depth but no pattern, open and closed by turns.
    let mut e = held_note("x.x3", 120.0);
    assert_eq!(steps(&mut e), [100; 8]);
    let mut e = held_note("x", 120.0);
    e.set_gate(0, &[]).unwrap();
    e.set_param(0, GATE_DEPTH, 1.0);
    assert_eq!(steps(&mut e), [100, 0, 100, 0, 100, 0, 100, 0]);
    // Swing: every second step begins late, so a quarter of the way into
    // it the gate is still where the first step left it.
    let mut e = held_note("x.", 120.0);
    e.set_param(0, GATE_DEPTH, 1.0);
    e.set_param(0, GATE_SWING, 0.5);
    let x = render(&mut e, frames, 128).0;
    assert!(open(&x, &reference, 6000 + 500, 6000 + 2500) > 0.99 && open(&x, &reference, 6000 + 3500, 11900) < 0.01);
    // Left and right by turns.
    let mut e = held_note("xx", 120.0);
    e.set_param(0, PAN, 0.0);
    e.set_param(0, GATE_DEPTH, 1.0);
    e.set_param(0, GATE_STEREO, 1.0);
    let (l, r) = render(&mut e, frames, 128);
    assert!(rms(&l[2000..5800]) > 0.1 && rms(&r[2000..5800]) < 1.0e-3);
    assert!(rms(&r[8000..11800]) > 0.1 && rms(&l[8000..11800]) < 1.0e-3);
}

#[test]
fn the_gate_never_clicks_however_it_is_changed() {
    // A constant sound in: the output is the gate itself, and its steepest
    // slope is what its smoothing allows.
    for smooth in [0.001f32, 0.008, 0.05] {
        let mut e = plain_engine();
        e.set_param(0, INPUT_PART, 0.0);
        e.set_param(0, TEMPO, 150.0);
        e.set_param(0, GATE_DEPTH, 1.0);
        e.set_param(0, GATE_SMOOTH, smooth);
        let ones = vec![0.5f32; 48000];
        let mut rng = Rng::new(3);
        // The pattern, the rate, the depth and the beat itself all change as it runs.
        for i in 0..40 {
            let at = i as f64 * 1200.0;
            e.push(Event::param(at, 0, GATE_RATE, [4.0, 8.0, 3.0, 16.0][rng.next_u32() as usize % 4], 0.0));
            e.push(Event::param(at + 400.0, 0, GATE_DEPTH, [1.0, 0.5, 0.0, 1.0][rng.next_u32() as usize % 4], 0.0));
            e.push(Event::clock(at + 800.0, (rng.next_u32() % 16) as f32 * 0.25));
        }
        let mut out = Vec::new();
        for block in 0..375 {
            if block % 20 == 0 {
                let name = NAMED_GATES[block / 20 % NAMED_GATES.len()].0;
                assert!(unsafe { synths_gate_text(&mut *e, 0, name.as_ptr(), name.len() as u32) } > 0);
            }
            let from = block * 128;
            let (l, _) = render_io(&mut e, &ones[from..from + 128], None, 128);
            out.extend(l);
        }
        let steepest = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // Each frame it goes a fixed part of the way: at most that part of the whole way.
        let ease = 1.0 - (-3.0 / (smooth * SR)).exp();
        assert!(steepest <= 0.5 * ease * 1.001 && steepest > 0.5 * ease * 0.5, "smooth {smooth}: {steepest} against {}", 0.5 * ease);
        assert!(out.iter().all(|v| (0.0..=0.5001).contains(v)));
    }
}

#[test]
fn the_beat_follows_the_tempo_and_the_clock_puts_it_right() {
    let mut e = plain_engine();
    e.set_param(0, TEMPO, 120.0);
    render(&mut e, 24000, 128);
    assert!((e.beat() - 1.0).abs() < 1.0e-9);
    // A change of tempo is taken up from its own frame: the beat carries on from where it was.
    e.push(Event::param(24000.0 + 12000.0, 0, TEMPO, 240.0, 0.0));
    render(&mut e, 24000, 128);
    assert!((e.beat() - 2.5).abs() < 1.0e-9, "{}", e.beat());
    // The clock says which beat it is, on a frame.
    e.push(Event::clock(48000.0 + 100.0, 8.0));
    render(&mut e, 6100, 128);
    assert!((e.beat() - 8.5).abs() < 1.0e-9);
    e.reset();
    assert_eq!(e.beat(), 0.0);

    // The gate keeps to the beat through both: a bar of "x..." in
    // sixteenths, with the tempo doubled half way through its second step.
    let mut e = held_note("x...", 120.0);
    e.set_param(0, GATE_DEPTH, 1.0);
    e.set_param(0, GATE_SMOOTH, 0.001);
    e.push(Event::param(9000.0, 0, TEMPO, 240.0, 0.0));
    let reference = render(&mut held_note("x", 120.0), 30000, 128).0;
    let x = render(&mut e, 30000, 128).0;
    // Steps begin at 0, 6000, then (3000 frames left of step 1 at twice the speed) 10500, 13500, 16500...
    for (from, to, on) in [(600, 5900, true), (6600, 10400, false), (11100, 16400, false), (17100, 19400, true), (20100, 22400, false)] {
        assert!((open(&x, &reference, from, to) - on as u8 as f32).abs() < 0.01, "{from}");
    }
    // It is on an open step now (the ninth). Told by the clock that it is
    // the second sixteenth of the bar, it closes at once.
    e.push(Event::clock(30000.0 + 1000.0, 0.25));
    let x = render(&mut e, 2400, 128).0;
    assert!(rms(&x[200..900]) > 0.3 && rms(&x[1300..2300]) < 0.01);
}

#[test]
fn the_sends_are_gated_or_not_as_asked_and_blocks_of_any_size_sound_the_same() {
    let run = |sends: f32, block: usize| {
        let mut e = held_note("x...", 120.0);
        e.set_param(0, GATE_DEPTH, 1.0);
        e.set_param(0, GATE_SENDS, sends);
        e.set_param(0, SEND_DELAY, 1.0);
        e.set_param(0, DELAY_TIME, 0.0625);
        e.set_param(0, DELAY_FEEDBACK, 0.0);
        render(&mut e, 48000, block).0
    };
    // The echo is half a step late. Sent the gated sound, it sounds only
    // where the open step's echo falls; sent the sound from before the
    // gate, it sounds all the time.
    let (after, before) = (run(1.0, 128), run(0.0, 128));
    assert!(rms(&after[24000 + 12000..24000 + 17000]) < 1.0e-3 && rms(&before[24000 + 12000..24000 + 17000]) > 0.05);
    assert!(rms(&after[24000 + 3500..24000 + 8500]) > 0.05);
    for block in [1, 64, 333, 4096] {
        assert!(run(1.0, block).iter().zip(&after).all(|(a, b)| a.to_bits() == b.to_bits()), "block {block}");
    }
}

#[test]
fn gate_text_reads_back_and_bad_text_is_refused_in_words() {
    let (levels, n) = gate::parse("x.x.xx.x x.x.x.xx").unwrap();
    assert_eq!(n, 16);
    assert_eq!(&levels[..8], [1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
    assert_eq!(gate::to_text(&levels[..n]).as_deref(), Some("x.x.xx.xx.x.x.xx"));
    let (levels, n) = gate::parse("0 3 6 9 X - |5|").unwrap();
    assert_eq!(&levels[..n], [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 0.0, 5.0 / 9.0]);
    assert_eq!(gate::to_text(&levels[..n]).as_deref(), Some(".36xx.5"));
    assert_eq!(gate::to_text(&[0.5]), None, "a level no character has");
    for (name, text) in NAMED_GATES {
        assert_eq!(gate::parse(name).unwrap().0, gate::parse(text).unwrap().0, "{name}");
    }
    for bad in ["", "  ", "x.y", "banana", &"x".repeat(33)] {
        assert_eq!(gate::parse(bad).err(), Some(GATE_HELP), "{bad:?}");
    }
    assert_eq!(gate::parse(&"x. ".repeat(16)).unwrap().1, 32);
    let mut e = Engine::new(SR);
    unsafe {
        assert_eq!(synths_gate_text(&mut *e, 3, "x.x3".as_ptr(), 4), 4);
        assert_eq!(e.gate(3), [1.0, 0.0, 1.0, 1.0 / 3.0]);
        assert_eq!(synths_gate_text(&mut *e, 3, "nope".as_ptr(), 4), -2);
        assert_eq!(synths_gate_text(&mut *e, 64, "x".as_ptr(), 1), -1);
        assert_eq!(e.gate(3).len(), 4);
        let levels = [1.0f32, 0.5, f32::NAN, 7.0];
        assert_eq!(synths_gate_set(&mut *e, 65535, levels.as_ptr(), 4), 0);
        assert_eq!(e.gate(63), [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(synths_gate_set(&mut *e, 0, levels.as_ptr(), 33), -2);
        assert_eq!(synths_gate_set(&mut *e, 0, core::ptr::null(), 0), 0);
        assert!(e.gate(0).is_empty());
    }
    let mut rng = Rng::new(8);
    for _ in 0..20000 {
        let text: String = (0..rng.next_u32() % 40).map(|_| b"x.X-0123456789 |q"[rng.next_u32() as usize % 17] as char).collect();
        if let Ok((levels, n)) = gate::parse(&text) {
            assert!(n >= 1 && levels[..n].iter().all(|l| (0.0..=1.0).contains(l)));
        }
    }
    let _ = ALL_PARTS;
}
