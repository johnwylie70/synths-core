//! The voice effects of a part: the pitch shifter, the ring modulator and
//! the comb.

mod common;
use common::*;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::voicefx::{Shifter, FX_UNITS};
use synths_core::{Engine, Event};

/// The part the input plays through unless a host says otherwise.
const PART: u16 = 31;

/// The input through part 31 with a patch, hard left, in blocks of 128.
fn through(patch: &[(usize, f32)], x: &[f32]) -> Vec<f32> {
    let mut e = plain_engine();
    for &(id, v) in patch {
        e.set_param(PART, id, v);
    }
    render_io(&mut e, x, None, 128).0
}

/// How much of `x` is not a steady tone at `hz`, in dB below the tone:
/// what is left after the best-fitting sine wave is taken away.
fn dirt(x: &[f32], hz: f32) -> f32 {
    let w = std::f64::consts::TAU * hz as f64 / SR as f64;
    let (mut ss, mut cc, mut sc, mut xs, mut xc) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        ss += s * s;
        cc += c * c;
        sc += s * c;
        xs += v as f64 * s;
        xc += v as f64 * c;
    }
    let det = ss * cc - sc * sc;
    let (a, b) = ((xs * cc - xc * sc) / det, (xc * ss - xs * sc) / det);
    let (mut tone, mut rest) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let (s, c) = (w * i as f64).sin_cos();
        let fit = a * s + b * c;
        tone += fit * fit;
        rest += (v as f64 - fit).powi(2);
    }
    (10.0 * (rest / tone).max(1.0e-12).log10()) as f32
}

/// The pitch of a voice-like sound, from the lag at which it best matches
/// itself, looked for within 40% of `expect` Hz; and how well it matches, 0..1.
fn pitch_of(x: &[f32], expect: f32) -> (f32, f32) {
    let period = SR / expect;
    let n = x.len() - (period * 1.5) as usize;
    let energy = |from: usize| x[from..from + n].iter().map(|&v| (v as f64).powi(2)).sum::<f64>();
    let e0 = energy(0);
    let score = |lag: usize| (0..n).map(|i| x[i] as f64 * x[i + lag] as f64).sum::<f64>() / (e0 * energy(lag)).sqrt();
    let best = ((period * 0.7) as usize..(period * 1.4) as usize).max_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap();
    // Between samples: a curve through the three scores around the best.
    let (l, c, r) = (score(best - 1), score(best), score(best + 1));
    let lag = best as f64 + 0.5 * (l - r) / (l - 2.0 * c + r);
    ((SR as f64 / lag) as f32, c as f32)
}

fn cents(got: f32, wanted: f32) -> f32 {
    1200.0 * (got / wanted).log2()
}

#[test]
fn the_shifter_moves_a_sine_by_the_right_amount_across_its_range() {
    // Every semitone from an octave down to an octave up, on a low tone and
    // a high one: the pitch is right to within a tenth of a cent, the level
    // to within a tenth of a dB, and what is not the tone is at least 45 dB
    // below it.
    let (mut worst_cents, mut worst_level, mut worst_dirt) = (0.0f32, 0.0f32, -200.0f32);
    for semis in -12..=12 {
        if semis == 0 {
            continue;
        }
        let ratio = 2.0f32.powf(semis as f32 / 12.0);
        for hz in [220.0, 1000.0] {
            let out = through(&[(FX_PITCH, semis as f32)], &sine(hz, 0.5, 72000));
            let y = &out[24000..];
            let off = cents(frequency(y, SR), hz * ratio);
            let level = db(amplitude_at(y, hz * ratio, SR) / 0.5);
            let rest = dirt(y, hz * ratio);
            assert!(off.abs() < 0.1, "{semis:+} semitones at {hz} Hz is {off} cents out");
            assert!(level.abs() < 0.1, "{semis:+} semitones at {hz} Hz is {level} dB out");
            assert!(rest < -45.0, "{semis:+} semitones at {hz} Hz: {rest} dB of dirt");
            worst_cents = worst_cents.max(off.abs());
            worst_level = worst_level.max(level.abs());
            worst_dirt = worst_dirt.max(rest);
        }
    }
    println!("sine, -12..+12 semitones: at worst {worst_cents:.3} cents out, {worst_level:.3} dB out, dirt {worst_dirt:.1} dB");
}

#[test]
fn the_shifter_moves_a_voice_by_the_right_amount_and_keeps_it_a_voice() {
    // A steady vowel at 120 Hz. Shifted, its pitch is right to within five
    // cents, each cycle is still like the last (a voice that warbles or
    // has clicks in it is not), and it is as loud as it was.
    let x = speech::vowel(SR, 120.0, 1.5);
    let level_in = rms(&x[24000..]);
    let (mut worst_cents, mut worst_match, mut worst_level) = (0.0f32, 1.0f32, 0.0f32);
    for semis in [-12.0, -7.0, -4.0, -2.0, 2.0, 4.0, 7.0, 12.0] {
        let ratio = 2.0f32.powf(semis / 12.0);
        let out = through(&[(FX_PITCH, semis)], &x);
        let (pitch, alike) = pitch_of(&out[24000..48000], 120.0 * ratio);
        let off = cents(pitch, 120.0 * ratio);
        let level = db(rms(&out[24000..]) / level_in);
        assert!(off.abs() < 5.0, "{semis:+} semitones: {pitch} Hz, {off} cents out");
        assert!(alike > 0.98, "{semis:+} semitones: cycles only {alike} alike");
        assert!(level.abs() < 0.5, "{semis:+} semitones: {level} dB out");
        worst_cents = worst_cents.max(off.abs());
        worst_match = worst_match.min(alike);
        worst_level = worst_level.max(level.abs());
    }
    println!("vowel at 120 Hz: at worst {worst_cents:.2} cents out, cycles {worst_match:.4} alike, {worst_level:.3} dB out");
}

#[test]
fn the_shifter_and_the_comb_work_at_every_sample_rate() {
    for sr in [8000.0f32, 11025.0, 22050.0, 44100.0, 96000.0, 192000.0] {
        let tone: Vec<f32> = (0..(sr * 1.5) as usize).map(|i| 0.5 * (std::f32::consts::TAU * 200.0 * i as f32 / sr).sin()).collect();
        for semis in [-12.0f32, -3.0, 5.0, 12.0] {
            let mut e = Engine::new(sr);
            e.set_param(0, MASTER_VOLUME, 1.0);
            e.set_param(PART, PAN, -1.0);
            e.set_param(PART, FX_PITCH, semis);
            let (l, _) = render_io(&mut e, &tone, None, 128);
            let y = &l[(sr * 0.5) as usize..];
            let wanted = 200.0 * 2.0f32.powf(semis / 12.0);
            let off = cents(frequency(y, sr), wanted);
            // (At 8 kHz there are few samples to line a splice up by: it is within about a cent.)
            assert!(off.abs() < if sr < 40000.0 { 2.0 } else { 0.2 }, "{semis:+} semitones at {sr} Hz: {off} cents out");
            assert!((peak(y) - 0.5).abs() < 0.03, "{semis:+} semitones at {sr} Hz: peak {}", peak(y));
            assert!(Shifter::max_delay(sr, semis) / sr < 0.021);
        }
        // The comb at 100 Hz makes 200 Hz louder and leaves little between.
        let mut e = Engine::new(sr);
        e.set_param(0, MASTER_VOLUME, 1.0);
        e.set_param(PART, FX_COMB, 1.0);
        e.set_param(PART, FX_COMB_FREQ, 100.0);
        e.set_param(PART, FX_COMB_FEEDBACK, 0.9);
        e.set_param(PART, FX_COMB_DEPTH, 1.0);
        let (l, r) = render_io(&mut e, &tone, None, 128);
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0) && peak(&l) > 0.1, "the comb at {sr} Hz");
    }
}

#[test]
fn the_shifter_holds_the_sound_up_no_longer_than_it_says() {
    // Quiet hiss that suddenly turns loud. The loud part comes out at
    // least a frame later and within the longest delay: under 16.5 ms
    // going down and under 18.5 ms going up, at 48 kHz.
    let (mut shortest, mut longest) = (usize::MAX, 0);
    for semis in [-12.0f32, -5.0, -1.0, 1.0, 5.0, 12.0] {
        let limit = Shifter::max_delay(SR, semis);
        assert!(limit / SR < if semis < 0.0 { 0.0165 } else { 0.0185 }, "{limit}");
        // Wherever in its cycle of catching up and jumping back the shifter is.
        for lead in 0..12 {
            let start = 24000 + lead * 97;
            let mut x = hiss(5, 0.002, start);
            x.extend(hiss(6, 0.5, 4800));
            let out = through(&[(FX_PITCH, semis)], &x);
            let first = out.iter().position(|v| v.abs() > 0.05).unwrap();
            assert!(first > start && (first - start) as f32 <= limit, "{semis:+} semitones: {first} against {start}, limit {limit}");
            shortest = shortest.min(first - start);
            longest = longest.max(first - start);
        }
    }
    let ms = |frames: usize| frames as f32 * 1000.0 / SR;
    println!("a sudden sound waited between {:.1} and {:.1} ms", ms(shortest), ms(longest));
    assert!(ms(longest) > 5.0, "the test never caught the shifter far behind");
    // After silence the shifter starts as near the present as it can be.
    let mut x = vec![0.0f32; 4800];
    x.extend(hiss(5, 0.5, 4800));
    let out = through(&[(FX_PITCH, 7.0)], &x);
    let first = out.iter().position(|v| v.abs() > 1.0e-3).unwrap();
    assert!(out[..4800].iter().all(|&v| v == 0.0) && first < 4800 + 48, "{first}");
    // With no shift there is no shifter: not one frame of delay.
    let out = through(&[(FX_PITCH, 0.0)], &x);
    assert!((0..x.len()).all(|i| out[i] == x[i] * synths_core::math::fast_sin(0.25)));
}

#[test]
fn the_shifter_stays_sane_whatever_it_is_given() {
    // Wild input, a shift that jumps about, parts taking and leaving the
    // pool: every number that comes out is a number, and in range.
    for seed in 1..=4 {
        let mut rng = Rng::new(seed);
        let mut e = Engine::new(SR);
        e.set_param(0, MASTER_VOLUME, 1.0);
        let mut done = 0;
        let (mut l, mut r) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
        let (mut xl, mut xr) = (vec![0.0f32; 2048], vec![0.0f32; 2048]);
        let mut loud = 0;
        while done < 48000 * 4 {
            for _ in 0..rng.next_u32() % 4 {
                let part = if rng.next_u32() % 2 == 0 { PART } else { (rng.next_u32() % 12) as u16 };
                let (id, lo, hi) = [
                    (FX_PITCH, -14.0, 14.0), (FX_RING, -0.5, 1.5), (FX_RING_FREQ, -10.0, 6000.0), (FX_RING_WAVE, -1.0, 2.0),
                    (FX_COMB, -0.5, 1.5), (FX_COMB_FREQ, 0.0, 6000.0), (FX_COMB_FEEDBACK, -2.0, 2.0),
                    (FX_COMB_RATE, -1.0, 12.0), (FX_COMB_DEPTH, -1.0, 2.0),
                ][rng.next_u32() as usize % 9];
                let value = if rng.next_u32() % 12 == 0 { f32::NAN } else { lo + (hi - lo) * rng.unipolar() };
                e.push(Event::param(e.time() as f64 + (rng.next_u32() % 2000) as f64, part, id, value, rng.unipolar() - 0.5));
            }
            if rng.next_u32() % 6 == 0 {
                e.push(Event::note_on(0.0, (rng.next_u32() % 12) as u16, 0, 30.0 + 60.0 * rng.unipolar(), 1.0));
            }
            if rng.next_u32() % 9 == 0 {
                e.push(Event::all_notes_off(0.0, 0xFFFF, rng.next_u32() % 2 == 0));
            }
            let n = 1 + (rng.next_u32() % 2000) as usize;
            let kind = rng.next_u32() % 5;
            for i in 0..n {
                xl[i] = match (kind, rng.next_u32() % 50) {
                    (0, _) => 0.0,
                    (_, 0) => f32::NAN,
                    (_, 1) => f32::INFINITY,
                    (_, 2) => -1.0e30,
                    (_, 3) => 1.0e-39,
                    (1, _) => 0.8 * (i as f32 * 0.03).sin(),
                    _ => rng.bipolar() * 4.0,
                };
                xr[i] = if kind == 3 { -xl[i] } else { rng.bipolar() };
            }
            e.render_io(&xl[..n], if kind == 4 { None } else { Some(&xr[..n]) }, &mut l[..n], &mut r[..n]);
            for i in 0..n {
                assert!(l[i].is_finite() && r[i].is_finite() && l[i].abs() <= 1.0 && r[i].abs() <= 1.0, "seed {seed}: {} {} at frame {}", l[i], r[i], done + i);
            }
            loud += l[..n].iter().filter(|v| v.abs() > 0.01).count();
            done += n;
        }
        assert!(loud > 48000, "seed {seed}: only {loud} loud frames");
    }
}

#[test]
fn a_synth_part_can_use_the_shifter_and_only_so_many_parts_at_once() {
    // A sine note an octave up through the shifter is a sine an octave higher.
    let play = |parts: usize| {
        let mut e = plain_engine();
        for part in 0..parts as u16 {
            e.set_param(part, OSC1_WAVE, 0.0);
            e.set_param(part, FX_PITCH, 12.0);
            e.set_param(part, VOLUME, 0.05);
            note_on(&mut e, 0, part, 57.0 + part as f32);
        }
        render(&mut e, 48000, 128).0
    };
    let one = play(1);
    assert!(cents(frequency(&one[24000..], SR), 440.0).abs() < 1.0);
    // There are FX_UNITS shifters to go round. With one part too many, the
    // last part to ask goes without, and sounds at the pitch it was played.
    assert_eq!(FX_UNITS, 8);
    let nine = play(9);
    let tone = |x: &[f32], note: f32| amplitude_at(&x[24000..], hz(note), SR);
    for part in 0..8 {
        assert!(tone(&nine, 69.0 + part as f32) > 0.04, "part {part} was not shifted");
    }
    assert!(tone(&nine, 65.0) > 0.04 && tone(&nine, 77.0) < 0.002, "the ninth part should go unshifted");
    // A part that turns its shifter off hands it back for another to use.
    let mut e = plain_engine();
    for part in 0..9u16 {
        e.set_param(part, OSC1_WAVE, 0.0);
        e.set_param(part, FX_PITCH, 12.0);
        e.set_param(part, VOLUME, 0.05);
        note_on(&mut e, 0, part, 57.0 + part as f32);
    }
    e.push(Event::param(2000.0, 0, FX_PITCH, 0.0, 0.0));
    let (l, _) = render(&mut e, 48000, 128);
    assert!(tone(&l, 77.0) > 0.04 && tone(&l, 57.0) > 0.04 && tone(&l, 69.0) < 0.002);
}

#[test]
fn what_is_still_in_the_shifter_comes_out_after_the_note_and_then_nothing() {
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, FX_PITCH, -12.0);
    note_on(&mut e, 0, 0, 69.0);
    e.push(Event::all_notes_off(9600.0, 0, true));
    let (l, r) = render(&mut e, 48000, 128);
    // The voice is cut at frame 9600; the shifter was up to 14 ms behind.
    let last = l.iter().rposition(|&v| v.abs() > 1.0e-3).unwrap();
    assert!(last > 9600 + 48 && last < 9600 + 800, "the last sound was at frame {last}");
    assert!(l[9600 + 4800..].iter().chain(r[9600 + 4800..].iter()).all(|&v| v == 0.0));
    assert_eq!(l.iter().filter(|v| v.is_subnormal()).count(), 0);
}

#[test]
fn the_ring_modulator_makes_the_sum_and_the_difference() {
    // 1000 Hz times 200 Hz is 800 Hz and 1200 Hz, each at half the level,
    // and no 1000 Hz at all.
    let x = sine(1000.0, 0.5, 48000);
    let out = through(&[(FX_RING, 1.0), (FX_RING_FREQ, 200.0)], &x);
    let at = |x: &[f32], hz: f32| amplitude_at(&x[4800..], hz, SR);
    assert!((at(&out, 800.0) - 0.25).abs() < 0.002 && (at(&out, 1200.0) - 0.25).abs() < 0.002);
    assert!(at(&out, 1000.0) < 0.001 && at(&out, 200.0) < 0.001);
    // Half way, half of the plain tone is left and the two new ones are half as loud.
    let out = through(&[(FX_RING, 0.5), (FX_RING_FREQ, 200.0)], &x);
    assert!((at(&out, 1000.0) - 0.25).abs() < 0.002 && (at(&out, 800.0) - 0.125).abs() < 0.002);
    // It is the same wave whatever the block size, and on a synth part too.
    let mut e = plain_engine();
    e.set_param(0, OSC1_WAVE, 0.0);
    e.set_param(0, FX_RING, 1.0);
    e.set_param(0, FX_RING_FREQ, 60.0);
    note_on(&mut e, 0, 0, 69.0);
    let (l, _) = render(&mut e, 48000, 128);
    assert!((at(&l, 380.0) - 0.5).abs() < 0.01 && (at(&l, 500.0) - 0.5).abs() < 0.01 && at(&l, 440.0) < 0.005);
}

#[test]
fn the_ring_modulators_noise_blurs_a_pitch_into_a_band_of_noise() {
    // A steady tone times a rumble 300 Hz wide: 3 dB quieter, as it is
    // after the sine (the two new tones each have half the level), no
    // longer a tone, and still centred where the tone was.
    let x = sine(2000.0, 0.5, 96000);
    let out = through(&[(FX_RING, 1.0), (FX_RING_WAVE, 1.0), (FX_RING_FREQ, 300.0)], &x);
    let y = &out[4800..];
    assert!((db(rms(y) / rms(&x)) + 3.0).abs() < 1.5, "{}", db(rms(y) / rms(&x)));
    assert!(amplitude_at(y, 2000.0, SR) < 0.05, "the tone is still there: {}", amplitude_at(y, 2000.0, SR));
    let band = |lo: f32, hi: f32| (0..40).map(|i| amplitude_at(&y[..9600], lo + (hi - lo) * i as f32 / 40.0, SR).powi(2)).sum::<f32>();
    assert!(band(1700.0, 2300.0) > 20.0 * band(4000.0, 4600.0));
}

#[test]
fn the_comb_rings_at_its_note_and_dies_away_to_nothing() {
    // Hiss through a comb at 200 Hz: far more at 200, 400, 600 Hz than in between.
    let x = hiss(9, 0.2, 96000);
    let out = through(&[(FX_COMB, 1.0), (FX_COMB_FREQ, 200.0), (FX_COMB_FEEDBACK, 0.9)], &x);
    let at = |hz: f32| (0..9).map(|i| amplitude_at(&out[9600..], hz - 4.0 + i as f32, SR).powi(2)).sum::<f32>().sqrt();
    for k in 1..4 {
        let (on, off) = (at(200.0 * k as f32), at(200.0 * k as f32 + 100.0));
        assert!(db(on / off) > 15.0, "at {} Hz the comb stands only {} dB proud", 200 * k, db(on / off));
    }
    assert!(db(rms(&out[9600..]) / rms(&x)).abs() < 2.0, "the comb changed the loudness by {} dB", db(rms(&out[9600..]) / rms(&x)));
    // Turned half up, the plain sound is still half of it.
    let half = through(&[(FX_COMB, 0.0)], &x);
    assert!(half.iter().zip(x.iter()).all(|(a, b)| *a == *b * synths_core::math::fast_sin(0.25)));
    // After a short burst it rings on (a 100 Hz buzz), then reaches exactly nothing.
    let mut burst = hiss(2, 0.5, 480);
    burst.resize(48000 * 8, 0.0);
    let out = through(&[(FX_COMB, 1.0), (FX_COMB_FREQ, 100.0), (FX_COMB_FEEDBACK, 0.95)], &burst);
    assert!(peak(&out[9600..14400]) > 0.01, "it should still be ringing");
    assert_eq!(out.iter().filter(|v| v.is_subnormal()).count(), 0);
    assert!(out[48000 * 7..].iter().all(|&v| v == 0.0), "the comb never reached nothing");
    // Swept, it is a flanger: the notes it rings at move.
    let swept = through(&[(FX_COMB, 0.5), (FX_COMB_FREQ, 500.0), (FX_COMB_DEPTH, 0.8), (FX_COMB_RATE, 2.0)], &x);
    let (early, late) = (amplitude_at(&swept[9600..14400], 500.0, SR), amplitude_at(&swept[21600..26400], 500.0, SR));
    assert!(swept.iter().all(|v| v.is_finite()) && (db(early / late)).abs() > 1.0);
}

#[test]
fn the_effects_come_in_a_fixed_order() {
    // Shifter, then ring modulator: 500 Hz up an octave is 1000 Hz, and
    // times 200 Hz that is 800 and 1200 Hz. (The other way round it would
    // be 600 and 1400 Hz.)
    let x = sine(500.0, 0.5, 72000);
    let out = through(&[(FX_PITCH, 12.0), (FX_RING, 1.0), (FX_RING_FREQ, 200.0)], &x);
    let at = |hz: f32| amplitude_at(&out[24000..], hz, SR);
    assert!(at(800.0) > 0.2 && at(1200.0) > 0.2 && at(600.0) < 0.01 && at(1400.0) < 0.01);
    // Ring modulator, then distortion: flat-topped, so the distortion came last.
    let out = through(&[(FX_RING, 1.0), (FX_RING_FREQ, 50.0), (DRIVE, 1.0)], &sine(1000.0, 0.5, 24000));
    let top = peak(&out);
    let flat = out.iter().filter(|v| v.abs() > 0.97 * top).count();
    assert!(flat > 24000 / 4, "only {flat} frames near the top");
}
