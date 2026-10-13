//! Sample maps: one instrument made of several recordings, each covering a
//! range of keys and of loudness.

mod common;
use common::*;
use synths_core::engine::SampleError;
use synths_core::event::ALL_PARTS;
use synths_core::math::Rng;
use synths_core::params::*;
use synths_core::zones::*;
use synths_core::zones_abi::*;
use synths_core::{Engine, Event};

/// A second of a sine at the pitch of `note`, peaking at `level`, in a slot.
fn tone(e: &mut Engine, slot: usize, note: f32, level: f32) {
    e.load_sample(slot, &sine(hz(note), level, SR as usize), 1, SR).unwrap();
}

/// A sample that is one number all the way through, so the output shows
/// exactly which sample is playing, and from which frame.
fn steady(e: &mut Engine, slot: usize, value: f32, frames: usize) {
    e.load_sample(slot, &vec![value; frames], 1, SR).unwrap();
}

/// An engine whose part 0 plays samples, with nothing in the way.
fn engine() -> Box<Engine> {
    let mut e = plain_engine();
    e.set_param(ALL_PARTS, OSC1_WAVE, 4.0);
    e.set_param(ALL_PARTS, VELOCITY_SENS, 0.0);
    e
}

fn on(e: &mut Engine, time: u64, note: f32, velocity: f32) {
    assert!(e.push(Event::note_on(time as f64, 0, 0, note, velocity)));
}

/// Levels in thousandths, to the nearest.
fn heard(x: &[f32]) -> Vec<i32> {
    x.iter().map(|v| (v * 1000.0).round() as i32).collect()
}

fn cents(got: f32, want: f32) -> f32 {
    1200.0 * (got / want).log2()
}

/// A piano of five recordings, at C2, C3, C4, C5 and C6, each covering the
/// keys nearest it: slots 0 to 4, each a little louder than the last so
/// that the output says which one played.
fn five_zones(e: &mut Engine) -> Vec<Zone> {
    let roots = [36u8, 48, 60, 72, 84];
    let zones: Vec<Zone> = roots
        .iter()
        .enumerate()
        .map(|(i, &root)| {
            tone(e, i, root as f32, 0.1 * (i + 1) as f32);
            Zone::new(i as u16, if i == 0 { 0 } else { root - 6 }, if i == 4 { 127 } else { root + 5 }, root as f32)
        })
        .collect();
    e.set_zones(0, &zones, 0).unwrap();
    zones
}

/// What the rule in the document says a note plays, worked out the slow way.
fn by_the_rule(zones: &[Zone], silent_outside: bool, key: i32, vel: i32) -> Vec<usize> {
    let holds = |z: &Zone, k: i32, v: i32| (z.key_lo as i32..=z.key_hi as i32).contains(&k) && (z.vel_lo as i32..=z.vel_hi as i32).contains(&v);
    let (mut k, mut v) = (key, vel);
    if !zones.iter().any(|z| holds(z, key, vel)) {
        if silent_outside {
            return Vec::new();
        }
        // The nearest key some zone has, then the nearest velocity a zone
        // has at that key; the zone given first, of two as near.
        let key_gap = |z: &Zone| (z.key_lo as i32 - key).max(key - z.key_hi as i32).max(0);
        let vel_gap = |z: &Zone| (z.vel_lo as i32 - vel).max(vel - z.vel_hi as i32).max(0);
        let nearest = zones.iter().map(|z| (key_gap(z), vel_gap(z))).min().unwrap();
        let z = zones.iter().find(|z| (key_gap(z), vel_gap(z)) == nearest).unwrap();
        k = key.clamp(z.key_lo as i32, z.key_hi as i32);
        v = vel.clamp(z.vel_lo as i32, z.vel_hi as i32);
    }
    (0..zones.len()).filter(|&i| holds(&zones[i], k, v)).take(MAX_LAYERS).collect()
}

#[test]
fn the_right_zones_are_chosen_for_every_key_and_velocity() {
    // Gaps (keys 0..9, 31..39, 100 up; velocities 0..9 of the first zone),
    // an overlap (keys 50..59), two layers by velocity, and a stack of
    // three zones that are all the same range.
    let mut zones = vec![
        Zone { vel_lo: 10, ..Zone::new(0, 10, 30, 20.0) },
        Zone::new(1, 40, 59, 50.0),
        Zone::new(2, 50, 70, 60.0),
        Zone { vel_hi: 63, ..Zone::new(3, 71, 99, 80.0) },
        Zone { vel_lo: 64, ..Zone::new(4, 71, 99, 80.0) },
    ];
    for slot in 5..8 {
        zones.push(Zone { vel_lo: 100, ..Zone::new(slot, 20, 25, 22.0) });
    }
    for silent in [false, true] {
        let mut maps = ZoneMaps::new();
        maps.set(3, &zones, if silent { MAP_SILENT_OUTSIDE } else { 0 }).unwrap();
        let (mut played, mut layered, mut none) = (0, 0, 0);
        for key in 0..128 {
            for vel in 0..128 {
                let picks = maps.pick(3, key as f32, vel as f32 / 127.0);
                let got: Vec<usize> = picks.index[..picks.count].iter().map(|&i| i as usize).collect();
                assert_eq!(got, by_the_rule(&zones, silent, key, vel), "key {key}, velocity {vel}, silent outside: {silent}");
                assert!(picks.gain[..picks.count].iter().all(|&g| g == 1.0));
                played += (picks.count > 0) as usize;
                layered += (picks.count > 1) as usize;
                none += (picks.count == 0) as usize;
            }
        }
        // Every kind of case did come up.
        assert!(played > 0 && layered > 0 && (none > 0) == silent, "{played} {layered} {none}");
    }
    // A few by hand. In the overlap both zones play, in the order given;
    // a stack of four would play four and a fifth zone would be left out.
    let mut maps = ZoneMaps::new();
    maps.set(0, &zones, 0).unwrap();
    let slots = |maps: &mut ZoneMaps, key: f32, vel: f32| {
        let p = maps.pick(0, key, vel);
        p.index[..p.count].iter().map(|&i| maps.zone(i).slot).collect::<Vec<_>>()
    };
    assert_eq!(slots(&mut maps, 55.0, 0.5), [1, 2]);
    assert_eq!(slots(&mut maps, 22.0, 1.0), [0, 5, 6, 7]);
    assert_eq!(slots(&mut maps, 85.0, 63.4 / 127.0), [3]);
    assert_eq!(slots(&mut maps, 85.0, 63.6 / 127.0), [4]);
    // A note between two keys belongs to the nearer (and a half goes up).
    assert_eq!(slots(&mut maps, 70.4, 0.5), [2]);
    assert_eq!(slots(&mut maps, 70.5, 0.1), [3]);
    // Outside every zone: the nearest by key, then by velocity.
    assert_eq!(slots(&mut maps, 3.0, 0.5), [0]);
    assert_eq!(slots(&mut maps, 35.0, 0.5), [0]);
    assert_eq!(slots(&mut maps, 36.0, 0.5), [1]);
    assert_eq!(slots(&mut maps, 127.0, 0.2), [3]);
    assert_eq!(slots(&mut maps, 15.0, 0.0), [0]);
    let stack: Vec<Zone> = (0..6).map(|slot| Zone::new(slot, 60, 60, 60.0)).collect();
    maps.set(1, &stack, 0).unwrap();
    assert_eq!(maps.pick(1, 60.0, 1.0).count, MAX_LAYERS);
    assert_eq!(maps.pick(1, 90.0, 1.0).count, MAX_LAYERS);
    // A part with no zones plays none.
    assert_eq!(maps.pick(2, 60.0, 1.0).count, 0);
    assert_eq!(maps.pick(1000, 60.0, 1.0).count, 0);
}

#[test]
fn a_note_outside_the_zones_plays_the_nearest_unless_the_map_says_silence() {
    let mut e = engine();
    tone(&mut e, 0, 48.0, 0.2);
    tone(&mut e, 1, 72.0, 0.4);
    let zones = [Zone::new(0, 45, 50, 48.0), Zone::new(1, 70, 75, 72.0)];
    e.set_zones(0, &zones, 0).unwrap();
    for (key, level) in [(30.0, 0.2), (59.0, 0.2), (61.0, 0.4), (90.0, 0.4)] {
        e.reset();
        on(&mut e, 0, key, 1.0);
        let x = render(&mut e, 12000, 128).0;
        assert_eq!(e.active_voices(), 1);
        assert!((peak(&x[2000..]) - level).abs() < 0.01, "key {key}: {}", peak(&x));
        assert!(cents(frequency(&x[2000..], SR), hz(key)).abs() < 1.0, "key {key}");
    }
    e.set_zones(0, &zones, MAP_SILENT_OUTSIDE).unwrap();
    assert_eq!(e.zone_flags(0), MAP_SILENT_OUTSIDE);
    for (key, sounds) in [(30.0, false), (47.0, true), (60.0, false), (75.0, true), (76.0, false)] {
        e.reset();
        on(&mut e, 0, key, 1.0);
        let x = render(&mut e, 4000, 128).0;
        assert_eq!((e.active_voices() == 1, peak(&x) > 0.1), (sounds, sounds), "key {key}");
    }
}

#[test]
fn pitch_is_exact_at_each_root_and_right_between_them() {
    let mut e = engine();
    five_zones(&mut e);
    for key in (24..=96).chain([12, 108]) {
        e.reset();
        on(&mut e, 0, key as f32, 1.0);
        let x = render(&mut e, 10000, 128).0;
        let zone = match key {
            0..=41 => 0,
            42..=53 => 1,
            54..=65 => 2,
            66..=77 => 3,
            _ => 4,
        };
        // Which recording played, and at what pitch.
        if key <= 90 {
            assert!((peak(&x[4000..]) - 0.1 * (zone + 1) as f32).abs() < 0.012, "key {key}: {}", peak(&x));
        }
        let off = cents(frequency(&x[2000..], SR), hz(key as f32));
        assert!(off.abs() < 0.2, "key {key} is {off} cents out");
        if (key - 36) % 12 == 0 && (36..=84).contains(&key) {
            // At its root a recording comes out as it went in.
            let want = sine(hz(key as f32), 0.1 * (zone + 1) as f32, 10000);
            let worst = x.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1.0e-5, "key {key}: {worst}");
        }
    }
    // A note between two keys, a zone's tuning in cents, and a root that is
    // not a number, which is the base note of the sample's slot.
    e.reset();
    on(&mut e, 0, 61.5, 1.0);
    let x = render(&mut e, 16000, 128).0;
    assert!(cents(frequency(&x[2000..], SR), hz(61.5)).abs() < 0.2);
    tone(&mut e, 9, 57.0, 0.3);
    e.configure_sample(9, 57.0, 0, 0, 0).unwrap();
    e.set_zones(0, &[Zone { tune: 30.0, ..Zone::new(9, 0, 127, f32::NAN) }], 0).unwrap();
    e.reset();
    on(&mut e, 0, 64.0, 1.0);
    let x = render(&mut e, 16000, 128).0;
    assert!((cents(frequency(&x[2000..], SR), hz(64.0)) - 30.0).abs() < 0.2);
}

#[test]
fn bend_glide_and_the_arpeggio_keep_the_voices_sample() {
    let mut e = engine();
    five_zones(&mut e);
    // A bend of an octave, out of the zone and into the next: still the C4 recording.
    e.set_param(0, BEND_RANGE, 12.0);
    on(&mut e, 0, 60.0, 1.0);
    e.push(Event::pitch_bend(8000.0, 0, 1.0));
    let x = render(&mut e, 24000, 128).0;
    assert!((peak(&x[10000..]) - 0.3).abs() < 0.02 && cents(frequency(&x[10000..], SR), hz(72.0)).abs() < 0.5);
    assert_eq!(e.active_voices(), 1);
    // A glide from C3 to C5 carries the C3 recording all the way.
    let mut e = engine();
    five_zones(&mut e);
    e.set_param(0, GLIDE, 0.05);
    on(&mut e, 0, 48.0, 1.0);
    e.push(Event::note_off(4000.0, 0, 0, 48.0));
    e.push(Event::note_on(4000.0, 0, 7, 72.0, 1.0));
    e.push(Event::voice_pitch(20000.0, 0, 7, 84.0, 0.0));
    let x = render(&mut e, 30000, 128).0;
    // The note that started at C5 has the C5 recording; it glided up from C3.
    assert!((peak(&x[12000..20000]) - 0.4).abs() < 0.02, "{}", peak(&x[12000..20000]));
    assert!(cents(frequency(&x[12000..20000], SR), hz(72.0)).abs() < 0.5);
    let early = frequency(&x[4100..4500], SR);
    assert!(early < hz(60.0), "the glide starts low: {early}");
    // And moved by `voice_pitch` into C6's zone, it is still the C5 recording.
    assert!((peak(&x[22000..]) - 0.4).abs() < 0.02 && cents(frequency(&x[22000..], SR), hz(84.0)).abs() < 0.5);
    // An arpeggio of two octaves: one voice, one recording.
    let mut e = engine();
    five_zones(&mut e);
    e.apply(&Event { z: 3.0, b: 0x00180c00, ..Event::note_on(0.0, 0, 0, 0.0, 0.0) }.with_kind(13));
    on(&mut e, 0, 48.0, 1.0);
    let x = render(&mut e, 2880, 64).0;
    assert_eq!(e.active_voices(), 1);
    for (step, note) in [48.0, 60.0, 72.0].into_iter().enumerate() {
        let piece = &x[step * 960 + 100..step * 960 + 900];
        assert!((peak(piece) - 0.2).abs() < 0.02 && cents(frequency(piece, SR), hz(note)).abs() < 3.0, "step {step}");
    }
}

trait WithKind {
    fn with_kind(self, kind: u16) -> Self;
}

impl WithKind for Event {
    fn with_kind(mut self, kind: u16) -> Event {
        self.kind = kind;
        self
    }
}

#[test]
fn velocity_switches_layers_at_the_boundary() {
    let mut e = engine();
    tone(&mut e, 0, 60.0, 0.2);
    tone(&mut e, 1, 60.0, 0.5);
    let zones = [Zone { vel_hi: 63, ..Zone::new(0, 0, 127, 60.0) }, Zone { vel_lo: 64, ..Zone::new(1, 0, 127, 60.0) }];
    e.set_zones(0, &zones, 0).unwrap();
    for (velocity, level) in [(0.0, 0.2), (0.3, 0.2), (63.0 / 127.0, 0.2), (63.4 / 127.0, 0.2), (63.6 / 127.0, 0.5), (64.0 / 127.0, 0.5), (1.0, 0.5)] {
        e.reset();
        on(&mut e, 0, 60.0, velocity);
        let x = render(&mut e, 6000, 128).0;
        assert_eq!(e.active_voices(), 1, "a hard switch plays one voice");
        assert!((peak(&x[1000..]) - level).abs() < 0.01, "velocity {velocity}: {}", peak(&x));
    }
    // The part's `velocity` still shapes the loudness within a layer.
    e.set_param(0, VELOCITY_SENS, 1.0);
    e.reset();
    on(&mut e, 0, 60.0, 0.75);
    let x = render(&mut e, 6000, 128).0;
    assert!((peak(&x[1000..]) - 0.5 * 0.75).abs() < 0.01);
}

#[test]
fn round_robin_takes_turns_and_the_same_way_every_time() {
    let run = |reset_after: Option<usize>| {
        let mut e = engine();
        // Three takes of one drum, and a second key with two of its own.
        for (slot, level) in [0.1, 0.2, 0.3, 0.5, 0.6].into_iter().enumerate() {
            steady(&mut e, slot, level, 400);
        }
        let mut zones: Vec<Zone> = (0..3).map(|slot| Zone { group: 1, ..Zone::new(slot, 36, 36, 36.0) }).collect();
        zones.extend((3..5).map(|slot| Zone { group: 1, ..Zone::new(slot, 38, 38, 38.0) }));
        e.set_zones(0, &zones, MAP_SILENT_OUTSIDE).unwrap();
        let mut levels = Vec::new();
        for (hit, key) in [36.0, 36.0, 38.0, 36.0, 36.0, 38.0, 38.0, 36.0, 36.0, 38.0].into_iter().enumerate() {
            if reset_after == Some(hit) {
                e.reset();
            }
            on(&mut e, 0, key, 1.0);
            let x = render(&mut e, 1024, 128).0;
            assert_eq!(e.active_voices(), 0);
            levels.push((x[100] * 10.0).round() as i32);
        }
        levels
    };
    // Each key goes round its own takes, whatever the other key is doing.
    assert_eq!(run(None), [1, 2, 5, 3, 1, 6, 5, 2, 3, 6]);
    assert_eq!(run(None), run(None));
    // A reset starts every turn again.
    assert_eq!(run(Some(4)), [1, 2, 5, 3, 1, 5, 6, 2, 3, 5]);
    // Zones outside a group play on every hit, alongside the group's one.
    let mut maps = ZoneMaps::new();
    let zones = [
        Zone::new(9, 60, 60, 60.0),
        Zone { group: 2, ..Zone::new(0, 60, 60, 60.0) },
        Zone { group: 5, ..Zone::new(1, 60, 60, 60.0) },
        Zone { group: 2, ..Zone::new(2, 60, 60, 60.0) },
    ];
    maps.set(0, &zones, 0).unwrap();
    let mut seen = Vec::new();
    for _ in 0..4 {
        let p = maps.pick(0, 60.0, 1.0);
        seen.push(p.index[..p.count].iter().map(|&i| maps.zone(i).slot).collect::<Vec<_>>());
    }
    assert_eq!(seen, [vec![9, 0, 1], vec![9, 1, 2], vec![9, 0, 1], vec![9, 1, 2]]);
    // The count of hits wraps where every number of takes up to ten divides it.
    assert!((1..=10).all(|takes| ROUND_ROBIN_WRAP % takes == 0));
}

#[test]
fn a_zone_played_as_recorded_ignores_the_key_for_its_pitch() {
    let mut e = engine();
    tone(&mut e, 0, 50.0, 0.2);
    tone(&mut e, 1, 67.0, 0.4);
    // A kit: each key is its own sound at its own pitch, and nothing else plays.
    let zones = [
        Zone { flags: ZONE_FIXED_PITCH, ..Zone::new(0, 36, 36, 0.0) },
        Zone { flags: ZONE_FIXED_PITCH, tune: -100.0, ..Zone::new(1, 38, 40, 99.0) },
    ];
    e.set_zones(0, &zones, MAP_SILENT_OUTSIDE).unwrap();
    for (key, level, note) in [(36.0, 0.2, 50.0), (38.0, 0.4, 66.0), (40.0, 0.4, 66.0)] {
        e.reset();
        on(&mut e, 0, key, 1.0);
        let x = render(&mut e, 16000, 128).0;
        assert!((peak(&x[2000..]) - level).abs() < 0.01);
        assert!(cents(frequency(&x[2000..], SR), hz(note)).abs() < 0.2, "key {key}");
    }
    // Transpose and bend still move it: they are the patch's, not the key's.
    e.reset();
    e.set_param(0, TRANSPOSE, 12.0);
    on(&mut e, 0, 36.0, 1.0);
    let x = render(&mut e, 16000, 128).0;
    assert!(cents(frequency(&x[2000..], SR), hz(62.0)).abs() < 0.2);
}

#[test]
fn crossfades_share_a_note_between_two_zones_by_the_law_in_the_document() {
    // Two zones that overlap by four keys (60..63), each fading over four.
    for linear in [false, true] {
        let flags = if linear { ZONE_LINEAR_FADE } else { 0 };
        let zones = [
            Zone { key_fade_hi: 4, flags, ..Zone::new(0, 40, 63, 50.0) },
            Zone { key_fade_lo: 4, flags, ..Zone::new(1, 60, 90, 70.0) },
        ];
        let mut maps = ZoneMaps::new();
        maps.set(0, &zones, 0).unwrap();
        for tenth in 400..=900 {
            let note = tenth as f32 / 10.0;
            let p = maps.pick(0, note, 1.0);
            let key = (note + 0.5).floor();
            // Two voices inside the overlap, one outside it.
            assert_eq!(p.count, if (60.0..=63.0).contains(&key) { 2 } else { 1 }, "note {note}");
            if p.count == 2 {
                // The lower zone has all of it one key below the overlap and none one key above.
                let t = (key - 59.0) / 5.0;
                let (down, up) = if linear { (1.0 - t, t) } else { ((1.0 - t).sqrt(), t.sqrt()) };
                assert!((p.gain[0] - down).abs() < 1.0e-5 && (p.gain[1] - up).abs() < 1.0e-5, "note {note}: {:?}", p.gain);
                let sum = if linear { p.gain[0] + p.gain[1] } else { p.gain[0] * p.gain[0] + p.gain[1] * p.gain[1] };
                assert!((sum - 1.0).abs() < 1.0e-5, "note {note}: {sum}");
            } else {
                assert_eq!(p.gain[0], 1.0, "note {note}");
            }
        }
    }
    // The same between two velocity layers that overlap from 50 to 69.
    let zones = [
        Zone { vel_hi: 69, vel_fade_hi: 20, ..Zone::new(0, 0, 127, 60.0) },
        Zone { vel_lo: 50, vel_fade_lo: 20, ..Zone::new(1, 0, 127, 60.0) },
    ];
    let mut maps = ZoneMaps::new();
    maps.set(0, &zones, 0).unwrap();
    for v in 0..=1270 {
        let vel = v as f32 / 10.0;
        let p = maps.pick(0, 60.0, vel / 127.0);
        let whole = (vel + 0.5).floor();
        assert_eq!(p.count, if (50.0..=69.0).contains(&whole) { 2 } else { 1 }, "velocity {vel}");
        let power: f32 = p.gain[..p.count].iter().map(|g| g * g).sum();
        assert!((power - 1.0).abs() < 1.0e-4, "velocity {vel}: {power}");
    }

    // Heard: two voices in the overlap, and the lower layer at its share (the other is silence).
    let mut e = engine();
    tone(&mut e, 0, 60.0, 0.4);
    steady(&mut e, 1, 0.0, 48000);
    let zones = [
        Zone { vel_hi: 69, vel_fade_hi: 20, ..Zone::new(0, 0, 127, 60.0) },
        Zone { vel_lo: 50, vel_fade_lo: 20, ..Zone::new(1, 0, 127, 60.0) },
    ];
    e.set_zones(0, &zones, 0).unwrap();
    on(&mut e, 0, 60.0, 60.0 / 127.0);
    let x = render(&mut e, 6000, 128).0;
    assert_eq!(e.active_voices(), 2);
    assert!((peak(&x[1000..]) - 0.4 * (10.0f32 / 21.0).sqrt()).abs() < 0.003, "{}", peak(&x));
    // A fade asked for wider than the zone is brought back to its width.
    e.set_zones(0, &[Zone { key_fade_lo: 200, vel_fade_hi: 200, ..Zone::new(0, 60, 62, 60.0) }], 0).unwrap();
    assert_eq!((e.zones(0)[0].key_fade_lo, e.zones(0)[0].vel_fade_hi), (3, 128));
}

#[test]
fn a_note_takes_its_zone_on_its_exact_frame() {
    for block in [1, 64, 333] {
        let mut e = engine();
        steady(&mut e, 0, 0.125, 4000);
        steady(&mut e, 1, 0.25, 4000);
        let zones = [Zone::new(0, 0, 59, 60.0), Zone::new(1, 60, 127, 60.0)];
        e.set_zones(0, &zones, 0).unwrap();
        on(&mut e, 1000, 59.0, 1.0);
        on(&mut e, 1003, 60.0, 1.0);
        let x = render(&mut e, 1500, block).0;
        assert_eq!(heard(&[x[999], x[1000], x[1002], x[1003], x[1400]]), [0, 125, 125, 375, 375], "blocks of {block}");
        // A new map is for the notes that start after it: the two that are
        // sounding keep the samples they have.
        e.set_zones(0, &[Zone::new(1, 0, 127, 60.0)], 0).unwrap();
        on(&mut e, 1600, 30.0, 1.0);
        let x = render(&mut e, 500, block).0;
        assert_eq!(heard(&[x[99], x[100], x[400]]), [375, 625, 625], "blocks of {block}");
        e.set_zones(0, &[], 0).unwrap();
        let x = render(&mut e, 100, block).0;
        assert_eq!((heard(&x[50..51]), e.active_voices()), (vec![625], 3));
    }
}

#[test]
fn the_map_is_only_used_while_oscillator_1_plays_a_sample() {
    let mut e = engine();
    five_zones(&mut e);
    tone(&mut e, 20, 60.0, 0.7);
    // With no map, `osc1.sample` says which slot plays; with one, the map does.
    e.set_param(0, OSC1_SAMPLE, 20.0);
    on(&mut e, 0, 60.0, 1.0);
    assert!((peak(&render(&mut e, 4000, 128).0[1000..]) - 0.3).abs() < 0.01);
    e.set_zones(0, &[], 0).unwrap();
    e.reset();
    on(&mut e, 0, 60.0, 1.0);
    assert!((peak(&render(&mut e, 4000, 128).0[1000..]) - 0.7).abs() < 0.01);
    // A part whose oscillator 1 is a wave takes no notice of its map.
    five_zones(&mut e);
    e.set_param(0, OSC1_WAVE, 0.0);
    e.reset();
    on(&mut e, 0, 69.0, 1.0);
    let x = render(&mut e, 8000, 128).0;
    assert!((peak(&x[1000..]) - 1.0).abs() < 0.01 && cents(frequency(&x, SR), 440.0).abs() < 0.2);
    // Oscillator 2 keeps its one slot, and plays beside the map's zone.
    e.set_param(0, OSC1_WAVE, 4.0);
    e.set_param(0, OSC2_WAVE, 4.0);
    e.set_param(0, OSC2_SAMPLE, 21.0);
    e.set_param(0, OSC2_LEVEL, 1.0);
    steady(&mut e, 21, 0.125, 48000);
    e.set_param(0, OSC1_LEVEL, 0.0);
    e.reset();
    on(&mut e, 0, 60.0, 1.0);
    assert_eq!(heard(&render(&mut e, 4000, 128).0[2000..2001]), [125]);
}

#[test]
fn the_rest_of_the_patch_applies_to_a_zone() {
    // The filter: a low-pass well below the tone takes most of it away.
    let mut e = engine();
    five_zones(&mut e);
    on(&mut e, 0, 84.0, 1.0);
    let open = rms(&render(&mut e, 8000, 128).0[2000..]);
    e.reset();
    e.set_param(0, FILTER_TYPE, 1.0);
    e.set_param(0, CUTOFF, 200.0);
    on(&mut e, 0, 84.0, 1.0);
    let shut = rms(&render(&mut e, 8000, 128).0[2000..]);
    assert!(db(shut / open) < -20.0, "{}", db(shut / open));
    // The amp envelope: an attack, and a release the zone can replace.
    let mut e = engine();
    let mut zones = five_zones(&mut e);
    e.set_param(0, AMP_ATTACK, 0.1);
    e.set_param(0, AMP_RELEASE, 0.02);
    e.set_param(0, AMP_CURVE, 0.0);
    zones[0].release = 0.2;
    e.set_zones(0, &zones, 0).unwrap();
    let last_sound = |e: &mut Engine, key: f32| {
        e.reset();
        on(e, 0, key, 1.0);
        e.push(Event::note_off(e.time() as f64 + 9600.0, 0, 0, key));
        let x = render(e, 24000, 128).0;
        assert!(peak(&x[..480]) < 0.2 * peak(&x[4800..9600]), "the attack is the part's: {} {}", peak(&x[..480]), peak(&x[4800..9600]));
        x.iter().rposition(|&v| v.abs() > 1.0e-4).unwrap()
    };
    // The low zone rings for its own 0.2 seconds, the others for the part's 0.02.
    let (low, middle) = (last_sound(&mut e, 36.0) as i32, last_sound(&mut e, 60.0) as i32);
    assert!((low - (9600 + 9600)).abs() < 200 && (middle - (9600 + 960)).abs() < 100, "{low} {middle}");
    // A zone's volume and pan trims, on top of the part's.
    let mut e = engine();
    e.set_param(0, PAN, 0.0);
    steady(&mut e, 0, 0.5, 48000);
    e.set_zones(0, &[Zone { volume: 0.5, pan: 1.0, ..Zone::new(0, 0, 127, 60.0) }], 0).unwrap();
    on(&mut e, 0, 60.0, 1.0);
    let (l, r) = render(&mut e, 2000, 128);
    assert!(l[1000].abs() < 1.0e-6 && (r[1000] - 0.25).abs() < 1.0e-6, "{} {}", l[1000], r[1000]);
    // Unison: every zone a note plays gets every unison voice.
    let mut e = engine();
    tone(&mut e, 0, 60.0, 0.2);
    tone(&mut e, 1, 60.0, 0.2);
    e.set_zones(0, &[Zone::new(0, 0, 127, 60.0), Zone::new(1, 0, 127, 60.0)], 0).unwrap();
    e.set_param(0, UNISON, 3.0);
    on(&mut e, 0, 60.0, 1.0);
    render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), 6);
}

/// A sample whose every frame holds its own index (divided by 4000, to
/// keep it under full scale).
fn ramp(e: &mut Engine, slot: usize, frames: usize) {
    let data: Vec<f32> = (0..frames).map(|i| i as f32 / 4000.0).collect();
    e.load_sample(slot, &data, 1, SR).unwrap();
}

fn frames_played(x: &[f32]) -> Vec<i32> {
    x.iter().map(|v| (v * 4000.0).round() as i32).collect()
}

#[test]
fn a_zone_can_have_its_own_loop_its_own_start_and_play_to_its_end() {
    let mut e = engine();
    ramp(&mut e, 0, 50);
    e.configure_sample(0, 60.0, 1, 30, 40).unwrap();
    // The slot's own loop, a loop of the zone's, and no loop at all.
    let own = Zone::new(0, 0, 59, 59.0);
    let other = Zone { loop_mode: ZONE_LOOP_FORWARD, loop_start: 10, loop_end: 20, ..Zone::new(0, 60, 60, 60.0) };
    let none = Zone { loop_mode: ZONE_LOOP_NONE, ..Zone::new(0, 61, 61, 61.0) };
    let ping = Zone { loop_mode: ZONE_LOOP_PINGPONG, loop_start: 10, loop_end: 14, start: 8, ..Zone::new(0, 62, 62, 62.0) };
    let past = Zone { loop_mode: ZONE_LOOP_FORWARD, loop_start: 45, loop_end: 9999, ..Zone::new(0, 63, 63, 63.0) };
    let empty = Zone { loop_mode: ZONE_LOOP_FORWARD, loop_start: 20, loop_end: 20, ..Zone::new(0, 64, 64, 64.0) };
    e.set_zones(0, &[own, other, none, ping, past, empty], 0).unwrap();
    let play = |e: &mut Engine, key: f32| {
        e.reset();
        on(e, 0, key, 1.0);
        frames_played(&render(e, 1500, 7).0)
    };
    let x = play(&mut e, 60.0);
    assert!((0..1500).all(|i| x[i] == if i < 20 { i as i32 } else { 10 + (i as i32 - 10) % 10 }), "{x:?}");
    let x = play(&mut e, 59.0);
    assert_eq!(x[40..50], [30, 31, 32, 33, 34, 35, 36, 37, 38, 39]);
    let x = play(&mut e, 61.0);
    assert_eq!((x[49], x[1499], e.active_voices()), (49, 0, 0));
    // A ping-pong loop, started 8 frames in.
    assert_eq!(play(&mut e, 62.0)[..14], [8, 9, 10, 11, 12, 13, 13, 12, 11, 10, 10, 11, 12, 13]);
    // An end past the sample's is brought back to it; a loop with no frames is no loop.
    assert_eq!(play(&mut e, 63.0)[44..52], [44, 45, 46, 47, 48, 49, 45, 46]);
    assert_eq!((play(&mut e, 64.0)[49], e.active_voices()), (49, 0));

    // A one-shot plays to its end whatever the key does: a note-off, and
    // the part's envelope with it, wait for the sample; and its loop is off.
    let mut e = engine();
    ramp(&mut e, 0, 3000);
    e.configure_sample(0, 60.0, 1, 100, 200).unwrap();
    e.set_zones(0, &[Zone { flags: ZONE_ONE_SHOT, ..Zone::new(0, 36, 36, 36.0) }, Zone::new(0, 37, 37, 37.0)], 0).unwrap();
    on(&mut e, 0, 36.0, 1.0);
    e.push(Event::note_off(500.0, 0, 0, 36.0));
    e.push(Event::all_notes_off(700.0, 0, false));
    let x = frames_played(&render(&mut e, 4096, 128).0);
    assert_eq!((x[499], x[1000], x[2999], x[4090]), (499, 1000, 2999, 0));
    assert_eq!(e.active_voices(), 0);
    // The same sample on the next key is an ordinary note: it stops when let go.
    on(&mut e, 4096, 37.0, 1.0);
    e.push(Event::note_off(4596.0, 0, 0, 37.0));
    let x = frames_played(&render(&mut e, 2000, 128).0);
    assert_eq!((x[499], x[1500], e.active_voices()), (199, 0, 0));
    // A one-shot can still be cut, and ends with its sample even if the
    // patch has something else that would sound for ever.
    e.set_param(0, SUB_LEVEL, 0.5);
    on(&mut e, 6096, 36.0, 1.0);
    render(&mut e, 2000, 128);
    assert_eq!(e.active_voices(), 1);
    render(&mut e, 2000, 128);
    assert_eq!(e.active_voices(), 0);
    e.push(Event::note_on(0.0, 0, 5, 36.0, 1.0));
    e.push(Event::all_notes_off(100.0, 0, true));
    render(&mut e, 256, 128);
    assert_eq!(e.active_voices(), 0);
    // Backwards (`osc1.reverse`) works on a zone as on any sample.
    let mut e = engine();
    ramp(&mut e, 0, 50);
    e.set_zones(0, &[Zone::new(0, 0, 127, 60.0)], 0).unwrap();
    e.set_param(0, OSC1_REVERSE, 1.0);
    on(&mut e, 0, 60.0, 1.0);
    assert_eq!(frames_played(&render(&mut e, 64, 64).0)[..5], [49, 48, 47, 46, 45]);
}

#[test]
fn mono_picks_zones_again_and_legato_keeps_the_voice() {
    // Mono: each new note is the right recording, one voice at a time,
    // and letting go of it goes back to the note still held, in its own zone.
    let mut e = engine();
    five_zones(&mut e);
    e.set_param(0, MODE, 1.0);
    on(&mut e, 0, 48.0, 1.0);
    on(&mut e, 8000, 72.0, 1.0);
    e.push(Event::note_off(16000.0, 0, 0, 72.0));
    e.push(Event::note_off(24000.0, 0, 0, 48.0));
    let x = render(&mut e, 23000, 128).0;
    assert_eq!(e.active_voices(), 1);
    for (piece, level, note) in [(&x[1000..8000], 0.2, 48.0), (&x[9000..16000], 0.4, 72.0), (&x[17000..23000], 0.2, 48.0)] {
        assert!((peak(piece) - level).abs() < 0.02, "{} for {level}", peak(piece));
        assert!(cents(frequency(piece, SR), hz(note)).abs() < 0.5);
    }
    render(&mut e, 2000, 128);
    assert_eq!(e.active_voices(), 0);
    // Legato: the sounding voice moves to the new note and keeps its sample.
    let mut e = engine();
    five_zones(&mut e);
    e.set_param(0, MODE, 2.0);
    on(&mut e, 0, 48.0, 1.0);
    on(&mut e, 8000, 72.0, 1.0);
    let x = render(&mut e, 16000, 128).0;
    assert_eq!(e.active_voices(), 1);
    assert!((peak(&x[9000..]) - 0.2).abs() < 0.02 && cents(frequency(&x[9000..], SR), hz(72.0)).abs() < 0.5);
    // With nothing sounding, a legato note starts like any other: in its own zone.
    e.push(Event::all_notes_off(0.0, 0, true));
    on(&mut e, 17000, 72.0, 1.0);
    let x = render(&mut e, 8000, 128).0;
    assert!((peak(&x[2000..]) - 0.4).abs() < 0.02);
}

/// A scene with a bit of everything, for the tests that render it more than one way.
fn busy(e: &mut Engine) {
    let mut rng = Rng::new(7);
    for slot in 0..8usize {
        let frames = 2000 + slot * 700;
        let data: Vec<f32> = (0..frames * (1 + slot % 2)).map(|i| 0.4 * (i as f32 * 0.01 * (slot + 1) as f32).sin() + 0.1 * rng.bipolar()).collect();
        e.load_sample(slot, &data, 1 + (slot % 2) as u32, 30000.0 + 3000.0 * slot as f32).unwrap();
        if slot % 3 == 0 {
            e.configure_sample(slot, 60.0, 1 + (slot % 2) as u32, 500, 1900).unwrap();
        }
    }
    // A piano with crossfades between its zones and two velocity layers,
    // a kit with round robins and one-shots, and a mono part.
    let mut piano = Vec::new();
    for (i, root) in [36u8, 48, 60, 72].into_iter().enumerate() {
        for layer in 0..2u8 {
            piano.push(Zone {
                vel_lo: if layer == 0 { 0 } else { 60 },
                vel_hi: if layer == 0 { 79 } else { 127 },
                vel_fade_hi: if layer == 0 { 20 } else { 0 },
                vel_fade_lo: if layer == 0 { 0 } else { 20 },
                key_fade_lo: 3,
                key_fade_hi: 3,
                tune: 5.0 * i as f32,
                release: 0.05 * (4 - i) as f32,
                pan: 0.2 * i as f32 - 0.3,
                ..Zone::new((i * 2) as u16 + layer as u16, root.saturating_sub(8), root + 6, root as f32)
            });
        }
    }
    e.set_zones(0, &piano, 0).unwrap();
    let kit: Vec<Zone> = (0..6u16)
        .map(|i| Zone {
            group: 1 + (i / 3) as u8,
            flags: ZONE_FIXED_PITCH | if i % 2 == 0 { ZONE_ONE_SHOT } else { 0 },
            loop_mode: (i % 4) as u8,
            loop_start: 100,
            loop_end: 900,
            start: 10 * i as u32,
            ..Zone::new(i, 36 + (i / 3) as u8, 36 + (i / 3) as u8, 60.0)
        })
        .collect();
    e.set_zones(1, &kit, MAP_SILENT_OUTSIDE).unwrap();
    e.set_zones(2, &piano, 0).unwrap();
    for part in 0..3u16 {
        e.set_param(part, OSC1_WAVE, 4.0);
        e.set_param(part, AMP_RELEASE, 0.1);
    }
    e.set_param(0, FILTER_TYPE, 1.0);
    e.set_param(0, FILTER_ENV, 24.0);
    e.set_param(0, LFO1_PITCH, 0.3);
    e.set_param(0, SEND_REVERB, 0.3);
    e.set_param(1, AMP_SUSTAIN, 0.0);
    e.set_param(1, AMP_DECAY, 0.2);
    e.set_param(2, MODE, 1.0);
    e.set_param(2, GLIDE, 0.03);
    e.set_param(2, UNISON, 2.0);
    for i in 0..60u32 {
        let at = (i * 733) as f64;
        let key = 30.0 + (rng.next_u32() % 54) as f32;
        let part = (i % 3) as u16;
        let note = if part == 1 { 36.0 + (i % 3) as f32 } else { key };
        e.push(Event::note_on(at + 0.25 * i as f64, part, 1 + i, note, 0.1 + 0.9 * rng.unipolar()));
        if i % 7 == 0 {
            e.push(Event { z: 3.0, b: 0x000c0700, ..Event::note_on(at + 1.0, part, 1 + i, 0.0, 0.0) }.with_kind(13));
        }
        if i % 5 == 0 {
            e.push(Event::voice_pitch(at + 900.0, part, 1 + i, note + 14.0, 0.05));
        }
        e.push(Event::note_off(at + 3000.0 + (rng.next_u32() % 3000) as f64, part, 1 + i, 0.0));
    }
    e.push(Event::pitch_bend(11111.0, 0, 0.7));
    e.push(Event::param(20000.0, 0, OSC1_REVERSE, 1.0, 0.0));
    e.push(Event::param(30000.0, 0, OSC1_REVERSE, 0.0, 0.0));
}

#[test]
fn the_sound_is_the_same_whatever_the_block_size() {
    let frames = 60000;
    let mut reference: Option<(Vec<f32>, Vec<f32>)> = None;
    for block in [128, 1, 64, 333, 4096] {
        let mut e = Engine::new(SR);
        busy(&mut e);
        let out = render(&mut e, frames, block);
        assert!(peak(&out.0) > 0.01 && out.0.iter().chain(&out.1).all(|v| v.is_finite()));
        match &reference {
            None => reference = Some(out),
            Some(want) => {
                assert!(out.0.iter().zip(&want.0).all(|(a, b)| a.to_bits() == b.to_bits()), "left differs in blocks of {block}");
                assert!(out.1.iter().zip(&want.1).all(|(a, b)| a.to_bits() == b.to_bits()), "right differs in blocks of {block}");
            }
        }
    }
    // And the same again from a new engine: nothing in it is left to chance.
    let mut e = Engine::new(SR);
    busy(&mut e);
    assert!(render(&mut e, frames, 128) == reference.unwrap());
}

#[test]
fn zones_that_make_no_sense_are_cleaned_or_refused_and_never_panic() {
    let mut e = engine();
    tone(&mut e, 0, 60.0, 0.3);
    // The rule for each number, one at a time.
    let wild = Zone {
        slot: 0,
        key_lo: 90,
        key_hi: 30,
        vel_lo: 200,
        vel_hi: 100,
        group: 255,
        flags: 0xFF,
        root: f32::INFINITY,
        tune: f32::NAN,
        volume: -3.0,
        pan: 9.0,
        release: f32::NAN,
        key_fade_lo: 255,
        key_fade_hi: 2,
        vel_fade_lo: 255,
        vel_fade_hi: 0,
        loop_start: u32::MAX,
        loop_end: 3,
        loop_mode: 77,
        reserved: [1, 2, 3],
        start: u32::MAX,
    };
    e.set_zones(0, &[wild], 0xFFFF_FFFF).unwrap();
    let z = e.zones(0)[0];
    assert_eq!((z.key_lo, z.key_hi, z.vel_lo, z.vel_hi), (30, 90, 100, 127), "ranges are put the right way round and kept to 0..127");
    assert_eq!((z.group, z.flags, z.reserved), (255, ZONE_FIXED_PITCH | ZONE_ONE_SHOT | ZONE_LINEAR_FADE, [0; 3]));
    assert_eq!((z.root, z.tune, z.volume, z.pan, z.release), (200.0, 0.0, 0.0, 1.0, -1.0));
    assert_eq!((z.key_fade_lo, z.key_fade_hi, z.vel_fade_lo, z.vel_fade_hi), (61, 2, 28, 0));
    assert_eq!((z.loop_mode, z.loop_start, z.loop_end, z.start), (ZONE_LOOP_OWN, u32::MAX, 3, u32::MAX));
    assert_eq!(e.zone_flags(0), MAP_SILENT_OUTSIDE);
    let nan = Zone { tune: 9.0e9, volume: f32::NAN, pan: f32::NAN, release: 99.0, ..Zone::new(0, 0, 127, f32::NAN) };
    e.set_zones(0, &[nan], 0).unwrap();
    let z = e.zones(0)[0];
    assert!(z.root.is_nan());
    assert_eq!((z.tune, z.volume, z.pan, z.release), (4800.0, 1.0, 0.0, 20.0));

    // Refused whole, leaving what was there: a slot that does not exist,
    // a part that does not, too many for a part, too many in all.
    let good = [Zone::new(0, 0, 127, 60.0)];
    e.set_zones(0, &good, 0).unwrap();
    assert_eq!(e.set_zones(0, &[good[0], Zone::new(1024, 0, 1, 0.0)], 0), Err(SampleError::BadFormat));
    assert_eq!(e.set_zones(64, &good, 0), Err(SampleError::BadSlot));
    assert_eq!(e.set_zones(0, &vec![good[0]; ZONES_PER_PART + 1], 0), Err(SampleError::BadFormat));
    assert_eq!(e.zones(0), &good);
    for part in 1..=4 {
        e.set_zones(part, &vec![Zone::new(part as u16, 0, 127, 60.0); if part < 4 { ZONES_PER_PART } else { ZONES_TOTAL - 3 * ZONES_PER_PART - 1 }], 0).unwrap();
    }
    assert_eq!(e.set_zones(5, &good, 0), Err(SampleError::BadFormat), "the pool is full");
    // A part can still replace its own with as many, and taking some away makes room.
    e.set_zones(2, &vec![Zone::new(77, 0, 127, 60.0); ZONES_PER_PART], 0).unwrap();
    e.set_zones(1, &[], 0).unwrap();
    e.set_zones(5, &good, 0).unwrap();
    assert_eq!((e.zones(0), e.zones(5), e.zones(1).len()), (&good[..], &good[..], 0));
    assert!(e.zones(2).iter().all(|z| z.slot == 77) && e.zones(3).iter().all(|z| z.slot == 3) && e.zones(4).iter().all(|z| z.slot == 4));
    assert_eq!((e.zones(2).len(), e.zones(3).len(), e.zones(4).len()), (512, 512, 511));

    // Tables of rubbish, bytes and all, played hard.
    let mut rng = Rng::new(99);
    let mut e = Engine::new(SR);
    for slot in 0..6 {
        let frames = 1 + (rng.next_u32() % 2000) as usize;
        let channels = 1 + (slot % 2) as u32;
        let data: Vec<f32> = (0..frames * channels as usize).map(|_| rng.bipolar()).collect();
        e.load_sample(slot, &data, channels, 20000.0 + 30000.0 * rng.unipolar()).unwrap();
        let _ = e.configure_sample(slot, 60.0, rng.next_u32() % 3, rng.next_u32() % 2000, rng.next_u32() % 2000);
    }
    let (mut l, mut r) = (vec![0.0f32; 512], vec![0.0f32; 512]);
    let (mut taken, mut refused, mut loud) = (0, 0, 0);
    for round in 0..400 {
        let part = (rng.next_u32() % 4) as usize;
        let count = (rng.next_u32() % 40) as usize;
        let zones: Vec<Zone> = (0..count)
            .map(|_| {
                let mut bytes = [0u8; ZONE_SIZE];
                bytes.iter_mut().for_each(|b| *b = rng.next_u32() as u8);
                // Most of the time a slot that exists, sometimes one with nothing in it.
                let slot = if rng.next_u32() % 60 == 0 { 2000 } else { (rng.next_u32() % 8) as u16 };
                bytes[..2].copy_from_slice(&slot.to_le_bytes());
                let mut z: Zone = unsafe { core::mem::transmute(bytes) };
                match rng.next_u32() % 4 {
                    0 => z.root = f32::NAN,
                    1 => z.volume = 1.0,
                    2 => (z.volume, z.root, z.key_lo, z.key_hi) = (1.0, 60.0, 0, 127),
                    _ => {}
                }
                z
            })
            .collect();
        match e.set_zones(part, &zones, rng.next_u32()) {
            Ok(()) => taken += 1,
            Err(_) => refused += 1,
        }
        e.set_param(part as u16, OSC1_WAVE, 4.0);
        e.set_param(part as u16, MODE, (round % 3) as f32);
        e.set_param(part as u16, OSC1_REVERSE, (round % 2) as f32);
        for _ in 0..6 {
            let at = e.time() as f64 + (rng.next_u32() % 400) as f64;
            let note = if rng.next_u32() % 30 == 0 { f32::NAN } else { -30.0 + 190.0 * rng.unipolar() };
            e.push(Event::note_on(at, part as u16, rng.next_u32() % 4, note, 1.5 * rng.unipolar()));
            e.push(Event::note_off(at + 200.0, part as u16, rng.next_u32() % 4, note));
        }
        e.render(&mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() <= 1.0));
        loud += (peak(&l) > 0.001) as usize;
        if round % 50 == 49 {
            e.reset();
        }
    }
    assert!(taken > 200 && refused > 20 && loud > 100, "{taken} {refused} {loud}");
}

#[test]
fn a_host_sets_and_reads_zones_through_the_c_interface() {
    unsafe {
        let engine = synths_core::abi::synths_create(SR);
        // The record, byte by byte, as the document lays it out.
        let mut bytes = [0u8; 2 * ZONE_SIZE + 1];
        let record = |b: &mut [u8], slot: u16, lo: u8, hi: u8, root: f32| {
            b[0..2].copy_from_slice(&slot.to_le_bytes());
            b[2] = lo;
            b[3] = hi;
            b[4] = 10;
            b[5] = 120;
            b[6] = 3;
            b[7] = ZONE_ONE_SHOT;
            b[8..12].copy_from_slice(&root.to_le_bytes());
            b[12..16].copy_from_slice(&(-25.0f32).to_le_bytes());
            b[16..20].copy_from_slice(&0.5f32.to_le_bytes());
            b[20..24].copy_from_slice(&(-0.25f32).to_le_bytes());
            b[24..28].copy_from_slice(&1.5f32.to_le_bytes());
            b[28..32].copy_from_slice(&[1, 2, 3, 4]);
            b[32..36].copy_from_slice(&1000u32.to_le_bytes());
            b[36..40].copy_from_slice(&2000u32.to_le_bytes());
            b[40] = ZONE_LOOP_PINGPONG;
            b[44..48].copy_from_slice(&77u32.to_le_bytes());
        };
        // One byte in, so that the records are not lined up in memory.
        record(&mut bytes[1..], 5, 20, 40, 30.5);
        record(&mut bytes[1 + ZONE_SIZE..], 1023, 41, 60, 50.0);
        let records = bytes.as_ptr().add(1) as *const Zone;
        assert_eq!(synths_zones_set(engine, 7, records, 2, MAP_SILENT_OUTSIDE), 0);
        assert_eq!((synths_zones_count(engine, 7), synths_zones_flags(engine, 7)), (2, 1));
        assert_eq!((synths_zones_count(engine, 6), synths_zones_count(engine, 64), synths_zones_flags(engine, 9999)), (0, 0, 0));
        let want = Zone {
            slot: 5,
            key_lo: 20,
            key_hi: 40,
            vel_lo: 10,
            vel_hi: 120,
            group: 3,
            flags: ZONE_ONE_SHOT,
            root: 30.5,
            tune: -25.0,
            volume: 0.5,
            pan: -0.25,
            release: 1.5,
            key_fade_lo: 1,
            key_fade_hi: 2,
            vel_fade_lo: 3,
            vel_fade_hi: 4,
            loop_start: 1000,
            loop_end: 2000,
            loop_mode: ZONE_LOOP_PINGPONG,
            reserved: [0; 3],
            start: 77,
        };
        assert_eq!((*engine).zones(7), &[want, Zone { slot: 1023, key_lo: 41, key_hi: 60, root: 50.0, ..want }]);
        // Read back, into memory that is not lined up either: the same bytes.
        let mut out = [0xAAu8; 3 * ZONE_SIZE + 1];
        assert_eq!(synths_zones_read(engine, 7, out.as_mut_ptr().add(1) as *mut Zone, 3), 2);
        assert_eq!(out[1..1 + 2 * ZONE_SIZE], bytes[1..]);
        assert!(out[1 + 2 * ZONE_SIZE..].iter().all(|&b| b == 0xAA));
        // No more than there is room for.
        let mut out = [0xAAu8; ZONE_SIZE + 4];
        assert_eq!(synths_zones_read(engine, 7, out.as_mut_ptr() as *mut Zone, 1), 1);
        assert_eq!((&out[..ZONE_SIZE], &out[ZONE_SIZE..]), (&bytes[1..1 + ZONE_SIZE], &[0xAAu8; 4][..]));
        assert_eq!(synths_zones_read(engine, 7, core::ptr::null_mut(), 1), 0);
        assert_eq!(synths_zones_read(engine, 99, out.as_mut_ptr() as *mut Zone, 1), 0);

        // What is refused, and with which number.
        assert_eq!(synths_zones_set(core::ptr::null_mut(), 0, records, 2, 0), -4);
        assert_eq!(synths_zones_set(engine, 0, core::ptr::null(), 2, 0), -4);
        assert_eq!(synths_zones_set(engine, 64, records, 2, 0), -1);
        assert_eq!(synths_zones_set(engine, 65535, records, 2, 0), -1);
        assert_eq!(synths_zones_set(engine, 0, records, ZONES_PER_PART as u32 + 1, 0), -2);
        bytes[1..3].copy_from_slice(&1024u16.to_le_bytes());
        assert_eq!(synths_zones_set(engine, 7, records, 2, 0), -2);
        assert_eq!((synths_zones_count(engine, 7), (*engine).zones(7)[0].slot), (2, 5), "a refused map changes nothing");
        // Taking a map away: with a count of 0, or by clearing one part or all.
        assert_eq!(synths_zones_set(engine, 3, records.add(1), 1, 0), 0);
        assert_eq!(synths_zones_set(engine, 7, core::ptr::null(), 0, 1), 0);
        assert_eq!((synths_zones_count(engine, 7), synths_zones_flags(engine, 7), synths_zones_count(engine, 3)), (0, 0, 1));
        assert_eq!(synths_zones_set(engine, 8, records.add(1), 1, 0), 0);
        assert_eq!((synths_zones_clear(engine, 3), synths_zones_count(engine, 3), synths_zones_count(engine, 8)), (0, 0, 1));
        assert_eq!((synths_zones_clear(engine, 65535), synths_zones_count(engine, 8)), (0, 0));
        assert_eq!((synths_zones_clear(engine, 64), synths_zones_clear(core::ptr::null_mut(), 0)), (-1, -4));
        assert_eq!((synths_zones_count(core::ptr::null(), 0), synths_zones_flags(core::ptr::null(), 0)), (0, 0));
        synths_core::abi::synths_destroy(engine);
    }
    // The limits a host reads from the parameter table.
    let table = synths_core::params::table_json();
    assert!(table.contains("\"zone_size\":48,\"zones_part\":512,\"zones_total\":2048,\"zone_layers\":4"));
    assert_eq!((ZONE_SIZE, ZONES_PER_PART, ZONES_TOTAL, MAX_LAYERS, MAX_MATCHES), (48, 512, 2048, 4, 32));
}
