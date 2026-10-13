//! The reverb: its types, its tail and what it costs when idle.

mod common;
use common::measure::{self, band, correlation, echo_density, energy};
use common::old_reverb::OldReverb;
use common::*;
use synths_core::reverb::{self, Reverb, Settings, KINDS};
use synths_core::Engine;

/// The reverb's answer to one frame of 1 on both sides, with `p`.
fn impulse(p: &Settings, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (seconds * SR) as usize;
    let mut input = vec![0.0f32; n];
    input[0] = 1.0;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut rev = Reverb::new(SR);
    for at in (0..n).step_by(16) {
        let to = (at + 16).min(n);
        rev.process(&input[at..to], &input[at..to], &mut l[at..to], &mut r[at..to], p);
    }
    (l, r)
}

/// The old reverb's answer to the same.
fn old_impulse(size: f32, damping: f32, seconds: f32) -> (Vec<f32>, Vec<f32>) {
    let n = (seconds * SR) as usize;
    let mut input = vec![0.0f32; n];
    input[0] = 1.0;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    OldReverb::new(SR).process(&input, &input, &mut l, &mut r, size, damping);
    (l, r)
}

fn of(kind: usize) -> Settings {
    Settings { kind, ..Settings::default() }
}

fn describe(name: &str, l: &[f32], r: &[f32]) {
    let t = measure::rt60(&band(l, 500.0, 1000.0, SR), SR);
    let wide = measure::rt60(&band(l, 100.0, 12000.0, SR), SR);
    let hi = measure::rt60(&band(l, 4000.0, 8000.0, SR), SR);
    let lo = measure::rt60(&band(l, 80.0, 160.0, SR), SR);
    let first = l.iter().position(|&v| v.abs() > 1.0e-4).unwrap_or(0);
    let density = echo_density(&l[first..], SR);
    let mixed = density.iter().find(|d| d.1 >= 0.9).map_or(-1.0, |d| d.0);
    let at = |t: f32| density.iter().find(|d| d.0 >= t).map_or(0.0, |d| d.1);
    let from = first + (0.15 * SR) as usize;
    let mut tail = measure::tail(l, from, 32768, t, SR);
    // The flatness of a single stretch of a tail is a little a matter of luck: take four.
    let four = [measure::tail(r, from, 32768, t, SR), measure::tail(l, from + 9600, 32768, t, SR), measure::tail(r, from + 9600, 32768, t, SR)];
    tail.flatness = (tail.flatness + four.iter().map(|t| t.flatness).sum::<f32>()) / 4.0;
    tail.repeats = four.iter().fold(tail.repeats, |m, t| m.max(t.repeats));
    let c = correlation(&l[from..], &r[from..]);
    let sum: Vec<f32> = l.iter().zip(r).map(|(a, b)| a + b).collect();
    let mono = energy(&sum[from..]) / (energy(&l[from..]) + energy(&r[from..]));
    println!(
        "{name:22} rt60 {t:5.2} (all {wide:5.2}, low {lo:5.2}, high {hi:5.2})  density 20ms {:.2} 50ms {:.2} 100ms {:.2} mixed at {:4.0} ms  flatness {:.3} peak {:4.1} dB repeats {:.3} at {:4.1} ms  l/r {c:+.3} mono {mono:.2}  energy {:.4} peak {:.3}",
        at(0.02), at(0.05), at(0.1), mixed * 1000.0, tail.flatness, tail.peak_db, tail.repeats, tail.repeats_at * 1000.0, energy(l) + energy(r), peak(l)
    );
}

/// Run `input` (the same on both sides) through a reverb whose settings are `at(frame)`.
fn through(input: &[f32], at: impl Fn(usize) -> Settings) -> (Vec<f32>, Vec<f32>) {
    let n = input.len();
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut rev = Reverb::new(SR);
    for from in (0..n).step_by(16) {
        let to = (from + 16).min(n);
        rev.process(&input[from..to], &input[from..to], &mut l[from..to], &mut r[from..to], &at(from));
    }
    (l, r)
}

/// How long the middle of an impulse response (500 to 1000 Hz) takes to fall by 60 dB.
fn measured(p: &Settings) -> f32 {
    let want = reverb::rt60(p.kind, p.size, p.decay);
    let (l, _) = impulse(p, want * 0.75 + 1.0);
    measure::rt60(&band(&l, 500.0, 1000.0, SR), SR)
}

/// The numbers of a tail that the tests below hold the reverb to.
struct Smooth {
    /// When the echoes have become too thick to tell apart, in seconds after the first.
    mixed: f32,
    /// How flat the spectrum of the late tail is: 0.56 for noise.
    flatness: f32,
    repeats: f32,
}

fn smoothness(l: &[f32], r: &[f32], t60: f32) -> Smooth {
    let first = l.iter().position(|&v| v.abs() > 1.0e-4).unwrap();
    let mixed = echo_density(&l[first..], SR).iter().find(|d| d.1 >= 0.9).map_or(f32::MAX, |d| d.0);
    let from = first + (0.15 * SR) as usize;
    // The flatness of one stretch of a tail is a little a matter of luck: take four.
    let four = [(l, from), (r, from), (l, from + 9600), (r, from + 9600)].map(|(x, at)| measure::tail(x, at, 32768, t60, SR));
    Smooth { mixed, flatness: four.iter().map(|t| t.flatness).sum::<f32>() / 4.0, repeats: four.iter().fold(0.0, |m, t| m.max(t.repeats)) }
}

#[test]
fn the_types_are_the_choices_of_the_parameter() {
    use synths_core::params::*;
    let names: Vec<&str> = KINDS.iter().map(|k| k.name).collect();
    assert_eq!(names.join("|"), by_name("reverb.type").unwrap().choices);
    assert_eq!(info(REVERB_TYPE).unwrap().max as usize, KINDS.len() - 1);
    // What a reverb is given when nothing has been set is what the table starts with.
    let usual = Settings::default();
    let d = |id: usize| info(id).unwrap().default;
    assert_eq!((usual.kind as f32, usual.size, usual.damping), (d(REVERB_TYPE), d(REVERB_SIZE), d(REVERB_DAMP)));
    assert_eq!((usual.decay, usual.predelay, usual.low_cut), (d(REVERB_DECAY), d(REVERB_PREDELAY), d(REVERB_LOW_CUT)));
    assert_eq!((info(REVERB_DECAY).unwrap().max, info(REVERB_PREDELAY).unwrap().max), (reverb::DECAY_MAX, reverb::PREDELAY_MAX));
    assert_eq!(KINDS[usual.kind].name, "room");
}

#[test]
fn the_tail_lasts_as_long_as_was_asked() {
    // Within a tenth, from one second to thirty, for every type and at any size.
    let mut worst = 0.0f32;
    let mut check = |p: Settings| {
        let want = reverb::rt60(p.kind, p.size, p.decay);
        let got = measured(&p);
        let off = (got / want - 1.0).abs();
        assert!(off < 0.1, "{} with {p:?}: {got} s, not {want}", KINDS[p.kind].name);
        worst = worst.max(off);
    };
    for kind in 0..KINDS.len() {
        for decay in [1.0, 2.0, 4.0, 8.0, 16.0, 30.0] {
            check(Settings { decay, ..of(kind) });
        }
        for size in [0.0, 1.0] {
            check(Settings { decay: 2.0, size, ..of(kind) });
        }
        // Left to the type and the size: the type's own time at 0.5, a
        // third of it at 0 and three times it at 1.
        assert_eq!(reverb::rt60(kind, 0.5, 0.0), KINDS[kind].decay);
        assert!((reverb::rt60(kind, 0.0, 0.0) * 3.0 / KINDS[kind].decay - 1.0).abs() < 1.0e-5);
        assert!((reverb::rt60(kind, 1.0, 0.0) / 3.0 / KINDS[kind].decay - 1.0).abs() < 1.0e-5);
        for size in [0.0, 0.25, 0.5, 0.75, 1.0] {
            check(Settings { size, ..of(kind) });
        }
    }
    // The room and the plate, whose echoes are close together, as short as 0.3 s.
    for kind in [reverb::ROOM, reverb::PLATE] {
        for decay in [0.3, 0.6] {
            check(Settings { decay, ..of(kind) });
        }
    }
    println!("the furthest a tail was from what was asked: {:.1}%", 100.0 * worst);
    // Out of range is brought in.
    assert_eq!(reverb::rt60(0, 0.5, 0.01), reverb::DECAY_MIN);
    assert_eq!(reverb::rt60(0, 0.5, 1000.0), reverb::DECAY_MAX);
}

#[test]
fn every_type_is_steady_at_every_extreme() {
    // A burst of noise, then silence, with every setting at each of its ends.
    let n = (1.5 * SR) as usize;
    let mut input = hiss(7, 0.5, n);
    input[12000..].fill(0.0);
    let mut tried = 0;
    for kind in 0..KINDS.len() {
        for size in [0.0, 1.0] {
            for damping in [0.0, 1.0] {
                for decay in [0.0, 0.001, 30.0, 1.0e9] {
                    for (predelay, low_cut) in [(0.0, 20.0), (0.2, 1000.0), (5.0, 0.0), (-1.0, 1.0e9)] {
                        let p = Settings { kind, size, damping, decay, predelay, low_cut };
                        let (l, r) = through(&input, |_| p);
                        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()), "{p:?}");
                        assert!(peak(&l).max(peak(&r)) < 8.0, "{p:?}: {}", peak(&l));
                        // It only ever dies away once the sound has stopped.
                        let (early, late) = (energy(&l[36000..48000]) + energy(&r[36000..48000]), energy(&l[60000..]) + energy(&r[60000..]));
                        assert!(late <= early, "{p:?}: {early} then {late}");
                        tried += 1;
                    }
                }
            }
        }
    }
    assert_eq!(tried, 320);
    // Settings that are not numbers, and a type there is not, are survived.
    let wild = Settings { kind: 99, size: f32::NAN, damping: f32::INFINITY, decay: f32::NAN, predelay: f32::NEG_INFINITY, low_cut: f32::NAN };
    let (l, r) = through(&input, |_| wild);
    assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()) && peak(&l) > 0.01 && peak(&l) < 8.0);
}

#[test]
fn settings_changed_at_random_while_it_rings_never_break_it() {
    // Twenty seconds of noise and a tone, with a setting or two changed
    // every few hundred frames, a type among them.
    let n = (20.0 * SR) as usize;
    let tone = sine(330.0, 0.3, n);
    let input: Vec<f32> = hiss(3, 0.2, n).iter().zip(&tone).map(|(a, b)| a + b).collect();
    let mut rng = synths_core::math::Rng::new(99);
    let mut rev = Reverb::new(SR);
    let mut p = Settings::default();
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    let mut at = 0;
    let mut changes = 0;
    while at < n {
        let to = (at + 1 + (rng.next_u32() % 700) as usize).min(n);
        match rng.next_u32() % 8 {
            0 => p.kind = (rng.next_u32() % 5) as usize,
            1 | 2 => p.size = rng.unipolar(),
            3 => p.damping = rng.unipolar(),
            4 => p.decay = if rng.next_u32() % 3 == 0 { 0.0 } else { 0.1 * 300.0f32.powf(rng.unipolar()) },
            5 => p.predelay = 0.2 * rng.unipolar(),
            6 => p.low_cut = 20.0 * 50.0f32.powf(rng.unipolar()),
            _ => {}
        }
        changes += 1;
        // In pieces of no more than 16 frames, as the engine gives them.
        for from in (at..to).step_by(16) {
            let end = (from + 16).min(to);
            rev.process(&input[from..end], &input[from..end], &mut l[from..end], &mut r[from..end], &p);
        }
        at = to;
    }
    assert!(changes > 2000);
    assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
    let loudest = peak(&l).max(peak(&r));
    println!("with {changes} changes of setting, the loudest moment was {loudest:.2}");
    assert!(loudest > 0.05 && loudest < 8.0, "{loudest}");
}

/// The biggest sudden jump in `x`: how far any frame lies from the straight
/// line through the frames either side of it. A smooth low tone has next to
/// none; a click is all jump.
fn jump(x: &[f32]) -> f32 {
    x.windows(3).map(|w| (w[0] - 2.0 * w[1] + w[2]).abs()).fold(0.0, f32::max)
}

#[test]
fn a_change_of_type_size_or_anything_else_does_not_click() {
    // A low steady tone, whose reverb is as smooth as it is: any click
    // stands out as a jump from one frame to the next.
    let n = (16.0 * SR) as usize;
    let input = sine(110.0, 0.5, n);
    let second = SR as usize;
    let steady = through(&input, |_| Settings::default());
    let smooth = jump(&steady.0[second..]);
    let level = peak(&steady.0[second..]);
    assert!(smooth < 0.001 * level, "{smooth} against {level}");
    // Every second something changes: the type goes round all five, the
    // size from one end to the other, and the rest from end to end.
    let changing = |frame: usize| {
        let step = frame / second;
        let mut p = Settings::default();
        p.kind = [0, 0, 0, 1, 1, 2, 3, 3, 4, 4, 0, 0, 0, 0, 0, 0][step];
        p.size = [0.5, 1.0, 0.0, 0.0, 1.0, 0.5, 0.5, 0.0, 0.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5][step];
        p.decay = if step >= 11 { [8.0, 0.2, 30.0, 0.0, 0.0][step - 11] } else { 0.0 };
        p.predelay = if step >= 12 { [0.2, 0.0, 0.1, 0.0][step - 12] } else { 0.0 };
        p.damping = if step % 2 == 0 { 0.4 } else { 1.0 };
        p.low_cut = if step % 3 == 0 { 120.0 } else { 20.0 };
        p
    };
    let (l, r) = through(&input, changing);
    let (loud, worst) = (peak(&l).max(peak(&r)), jump(&l).max(jump(&r)));
    println!("a steady tone's reverb jumps by {:.5} of its level at most; with everything changing, {:.5}", smooth / level, worst / loud);
    // A click would be a jump of a good part of the level. What there is
    // is the bend in the tone where a slide of the size or the pre-delay
    // begins, and its pitch with it.
    assert!(worst < 0.01 * loud, "{worst} against {loud}");
}

#[test]
fn the_tail_is_far_smoother_than_the_old_reverbs() {
    // The old reverb, at three sizes: its echoes never thicken into a wash
    // within a sixth of a second, and its tail is a few ringing pitches.
    let mut old_flatness = 0.0f32;
    for size in [0.3, 0.6, 0.9] {
        let (l, r) = old_impulse(size, 0.4, 30.0);
        let t = measure::rt60(&band(&l, 500.0, 1000.0, SR), SR);
        let s = smoothness(&l, &r, t);
        println!("old, size {size}: thick after {:.0} ms, flatness {:.3}, repeats {:.2}", 1000.0 * s.mixed, s.flatness, s.repeats);
        assert!(s.mixed > 0.15 && s.flatness < 0.1);
        old_flatness = old_flatness.max(s.flatness);
    }
    // The new one: (the type, thick within, flatness at least).
    for (kind, within, flat) in [(reverb::ROOM, 0.09, 0.19), (reverb::HALL, 0.18, 0.34), (reverb::PLATE, 0.075, 0.23), (reverb::CATHEDRAL, 0.23, 0.42)] {
        let t = KINDS[kind].decay;
        let (l, r) = impulse(&of(kind), (t * 2.5).max(4.0));
        let s = smoothness(&l, &r, t);
        println!("{}: thick after {:.0} ms, flatness {:.3}, repeats {:.2}", KINDS[kind].name, 1000.0 * s.mixed, s.flatness, s.repeats);
        assert!(s.mixed <= within, "{}: {}", KINDS[kind].name, s.mixed);
        assert!(s.flatness >= flat && s.flatness > 2.4 * old_flatness, "{}: {}", KINDS[kind].name, s.flatness);
        assert!(s.repeats < 0.3, "{}: {}", KINDS[kind].name, s.repeats);
    }
    // The spring is the other way about, and meant to be: it repeats.
    let (l, r) = impulse(&of(reverb::SPRING), 5.0);
    let s = smoothness(&l, &r, 2.0);
    println!("spring: flatness {:.3}, repeats {:.2}", s.flatness, s.repeats);
    assert!(s.repeats > 0.5 && s.flatness < 0.05);
}

#[test]
fn left_and_right_are_unlike_each_other_and_add_up() {
    for kind in 0..KINDS.len() {
        let (l, r) = impulse(&of(kind), 4.0);
        let from = (0.15 * SR) as usize;
        let alike = correlation(&l[from..], &r[from..]);
        let sum: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a + b).collect();
        // Played on one loudspeaker, nothing cancels: the sum has the
        // energy of the two sides together.
        let mono = energy(&sum[from..]) / (energy(&l[from..]) + energy(&r[from..]));
        println!("{}: left and right alike by {alike:+.3}, mono keeps {mono:.2}", KINDS[kind].name);
        assert!(alike.abs() < 0.25, "{}: {alike}", KINDS[kind].name);
        assert!(mono > 0.75 && mono < 1.25, "{}: {mono}", KINDS[kind].name);
        assert!((energy(&l) / energy(&r)).log10().abs() < 0.2, "{}: one side is louder", KINDS[kind].name);
    }
    // A sound on the left alone is heard on both sides.
    let n = SR as usize;
    let mut input = vec![0.0f32; n];
    input[0] = 1.0;
    let (mut l, mut r) = (vec![0.0f32; n], vec![0.0f32; n]);
    Reverb::new(SR).process(&input, &vec![0.0; n], &mut l, &mut r, &Settings::default());
    assert!((energy(&l) / energy(&r)).log10().abs() < 0.5);
}

#[test]
fn it_adds_no_steady_offset() {
    // A steady push on the input, for four seconds: once the thump of its
    // start has died away (in a second, here) nothing steady comes out.
    for kind in 0..KINDS.len() {
        let n = (4.0 * SR) as usize;
        let (l, r) = through(&vec![0.5; n], |_| Settings { decay: 1.0, ..of(kind) });
        let mean = |x: &[f32]| x.iter().map(|&v| v as f64).sum::<f64>() / x.len() as f64;
        let (ml, mr) = (mean(&l[n - 24000..]), mean(&r[n - 24000..]));
        assert!(ml.abs() < 1.0e-4 && mr.abs() < 1.0e-4, "{}: {ml} {mr}", KINDS[kind].name);
    }
}

#[test]
fn silence_in_is_silence_out_and_an_idle_reverb_does_nothing() {
    for kind in 0..KINDS.len() {
        let p = of(kind);
        let mut rev = Reverb::new(SR);
        assert!(rev.idle());
        // Silence into a new reverb: it stays idle and touches nothing.
        let (mut l, mut r) = ([0.25f32; 64], [-0.5f32; 64]);
        rev.process(&[0.0; 64], &[0.0; 64], &mut l, &mut r, &p);
        assert!(rev.idle() && l == [0.25; 64] && r == [-0.5; 64]);
        // A click: it rings, falls below a millionth, and then there is
        // exactly nothing, and it is idle again.
        let t = reverb::rt60(kind, p.size, p.decay);
        let n = ((2.6 * t + 1.5) * SR) as usize;
        let mut input = vec![0.0f32; n];
        input[100] = 1.0;
        let (l, r) = through(&input, |_| p);
        assert!(peak(&l[..n / 4]) > 0.01);
        let last = l.iter().zip(&r).rposition(|(a, b)| *a != 0.0 || *b != 0.0).unwrap();
        let ended = last as f32 / SR;
        println!("{}: exactly silent {ended:.2} s after a click ({:.1} times its decay)", KINDS[kind].name, ended / t);
        assert!(last < n - 24000, "{}: still sounding at the end", KINDS[kind].name);
        // The last thing it put out was under the threshold, and nothing very small ever came out.
        assert!(l[last].abs() + r[last].abs() < reverb::QUIET);
        assert!(l.iter().chain(r.iter()).all(|v| !v.is_subnormal()));
        // And the same reverb, used again, gives the same again.
        let mut rev = Reverb::new(SR);
        let mut twice = [vec![0.0f32; n], vec![0.0f32; n]];
        for out in twice.iter_mut() {
            let mut other = vec![0.0f32; n];
            rev.process(&input, &input, out, &mut other, &p);
            assert!(rev.idle());
        }
        assert!(twice[0] == twice[1] && twice[0] == l, "{}: the second click sounded different", KINDS[kind].name);
    }
}

#[test]
fn the_spring_chirps_and_the_others_do_not() {
    // In a spring the high part of a click comes out later than the low
    // part, every time round: that is the "boing".
    let arrivals = |kind: usize| {
        let (l, _) = impulse(&of(kind), 0.035);
        (measure::arrival(&l, 300.0, 900.0, SR), measure::arrival(&l, 3000.0, 3800.0, SR))
    };
    let (low, high) = arrivals(reverb::SPRING);
    println!("spring: the low part of a click arrives after {:.1} ms, the top after {:.1} ms", low * 1000.0, high * 1000.0);
    assert!(high > low + 0.004, "{low} {high}");
    // By the second time round the top is later still.
    let (l, _) = impulse(&of(reverb::SPRING), 0.2);
    let second = (0.04 * SR) as usize;
    let (low2, high2) = (measure::arrival(&l[second..3 * second], 300.0, 900.0, SR), measure::arrival(&l[second..3 * second], 3000.0, 3800.0, SR));
    assert!(high2 - low2 > 1.5 * (high - low), "{low2} {high2}");
    // In the others the top of a click is never the later part.
    for kind in 0..reverb::SPRING {
        let (low, high) = arrivals(kind);
        assert!(high - low < 0.002, "{}: {low} {high}", KINDS[kind].name);
    }
}

#[test]
fn a_held_note_does_not_wobble() {
    // The lines' lengths drift, to keep any one pitch from ringing on. It
    // must not be heard as a chorus: the reverb of a steady tone stays a
    // steady tone, at its own pitch.
    for kind in 0..KINDS.len() {
        for hz in [220.0, 440.0] {
            // Long enough for the reverb to have filled up, and then three seconds of it.
            let n = ((2.0 * KINDS[kind].decay + 5.0) * SR) as usize;
            let (l, _) = through(&sine(hz, 0.5, n), |_| of(kind));
            let from = n - (3.0 * SR) as usize;
            let quarter = (0.25 * SR) as usize;
            let mut off = 0.0f32;
            for at in (from..n - quarter).step_by(quarter / 2) {
                off = off.max((1200.0 * (frequency(&l[at..at + quarter], SR) / hz).log2()).abs());
            }
            // How much of it is the tone itself, and not something moving about it.
            let share = amplitude_at(&l[from..from + 131072], hz, SR) / (rms(&l[from..from + 131072]) * std::f32::consts::SQRT_2);
            println!("{} at {hz} Hz: within {off:.2} cents, {:.1}% the steady tone", KINDS[kind].name, 100.0 * share);
            assert!(off < 3.0, "{} at {hz}: {off} cents", KINDS[kind].name);
            assert!(share > 0.85, "{} at {hz}: {share}", KINDS[kind].name);
        }
    }
}

#[test]
fn damping_low_cut_and_predelay_do_what_they_say() {
    for kind in 0..KINDS.len() {
        let name = KINDS[kind].name;
        // Damping: the treble dies away sooner than the middle, and sooner the more there is.
        let times = |damping: f32| {
            let (l, _) = impulse(&Settings { damping, decay: 2.0, ..of(kind) }, 3.5);
            let top = if kind == reverb::SPRING { (2500.0, 3800.0) } else { (6000.0, 10000.0) };
            (measure::rt60(&band(&l, 500.0, 1000.0, SR), SR), measure::rt60(&band(&l, top.0, top.1, SR), SR))
        };
        let (mid0, high0) = times(0.0);
        let (mid, high) = times(0.4);
        let (_, high1) = times(1.0);
        println!("{name}: 2 s asked; the middle lasts {mid:.2} s, and the treble {high0:.2}, {high:.2} and {high1:.2} s with damping 0, 0.4 and 1");
        assert!(high1 < 0.7 * high && high < 0.85 * high0 && (mid / mid0 - 1.0).abs() < 0.1, "{name}");
        // The low cut: bass below it is kept out.
        let bass = |low_cut: f32| {
            let n = SR as usize;
            let (l, _) = through(&sine(60.0, 0.5, n), |_| Settings { low_cut, ..of(kind) });
            rms(&l[n / 2..])
        };
        assert!(bass(1000.0) < 0.2 * bass(20.0), "{name}: {} {}", bass(1000.0), bass(20.0));
        // The pre-delay: the whole reverb comes that much later.
        let starts = |predelay: f32| {
            let (l, _) = impulse(&Settings { predelay, ..of(kind) }, 0.5);
            l.iter().position(|&v| v.abs() > 1.0e-4).unwrap() as f32 / SR
        };
        assert!((starts(0.1) - starts(0.0) - 0.1).abs() < 0.0005, "{name}: {} {}", starts(0.0), starts(0.1));
    }
}

#[test]
fn the_engine_gives_the_same_sound_whatever_the_size_of_the_blocks() {
    use synths_core::params::*;
    use synths_core::Event;
    let play = |block: usize| {
        let mut e = Engine::new(SR);
        e.set_param(0, SEND_REVERB, 0.8);
        e.set_param(1, SEND_REVERB, 0.5);
        e.set_param(1, OSC1_WAVE, 2.0);
        let mut ok = true;
        for (i, note) in [60.0, 67.0, 72.0, 55.0, 64.0].into_iter().enumerate() {
            ok &= e.push(Event::note_on(1001.0 + 9001.0 * i as f64, (i % 2) as u16, 0, note, 0.9));
            ok &= e.push(Event::note_off(4003.0 + 9001.0 * i as f64, (i % 2) as u16, 0, note));
        }
        // Everything about the reverb changes while it rings, on odd frames.
        for (at, id, value, glide) in [
            (7001.0, REVERB_SIZE, 1.0, 0.0), (9013.0, REVERB_TYPE, 1.0, 0.0), (15017.0, REVERB_DECAY, 0.4, 0.0),
            (20011.0, REVERB_SIZE, 0.1, 0.05), (23003.0, REVERB_TYPE, 4.0, 0.0), (23501.0, REVERB_TYPE, 2.0, 0.0),
            (27007.0, REVERB_PREDELAY, 0.07, 0.0), (31001.0, REVERB_DAMP, 0.9, 0.1), (33007.0, REVERB_LOW_CUT, 400.0, 0.0),
            (35003.0, REVERB_TYPE, 3.0, 0.0), (36001.0, REVERB_DECAY, 0.3, 0.0), (41077.0, REVERB_TYPE, 0.0, 0.0),
        ] {
            ok &= e.push(Event::param(at, 0, id, value, glide));
        }
        // And after it has fallen silent and emptied itself, one more note wakes it.
        ok &= e.push(Event::note_on(170_003.0, 0, 0, 62.0, 0.9));
        ok &= e.push(Event::note_off(172_001.0, 0, 0, 62.0));
        assert!(ok);
        render(&mut e, 200_000, block)
    };
    let (l, r) = play(128);
    assert!(peak(&l[..9000]) > 0.01 && peak(&l[130_000..170_000]) == 0.0 && peak(&l[175_000..]) > 0.001);
    for block in [1, 64, 333, 4096] {
        let (bl, br) = play(block);
        assert!(l == bl && r == br, "blocks of {block} frames gave a different sound");
    }
}

#[test]
fn report() {
    for (size, damping) in [(0.6, 0.4), (0.3, 0.4), (0.9, 0.4)] {
        let (l, r) = old_impulse(size, damping, 30.0);
        describe(&format!("old size {size}"), &l, &r);
    }
    for kind in 0..KINDS.len() {
        let p = of(kind);
        let want = reverb::rt60(kind, p.size, p.decay);
        let (l, r) = impulse(&p, (want * 2.5).max(4.0));
        describe(&format!("{} ({want:.1} s)", KINDS[kind].name), &l, &r);
    }
    for kind in 0..KINDS.len() {
        let p = Settings { decay: 1.5, ..of(kind) };
        let (l, r) = impulse(&p, 5.0);
        describe(&format!("{} at 1.5 s", KINDS[kind].name), &l, &r);
    }
}

