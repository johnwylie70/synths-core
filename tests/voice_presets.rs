//! The voice changers that come with the engine.

mod common;
use common::*;
use synths_core::params::{self, by_name};
use synths_core::presets::{apply_voice_preset, voice_preset, VOICE_PRESETS};
use synths_core::Engine;

const PART: u16 = 31;

/// Three seconds of speech through a voice changer (`None`: as it is),
/// and a second for it to die away.
fn speak(preset: Option<&str>) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(SR);
    apply_voice_preset(&mut e, PART, preset.map(|name| voice_preset(name).unwrap()));
    let mut x = speech::speech(SR, 3.0);
    x.resize(4 * SR as usize, 0.0);
    render_io(&mut e, &x, None, 128)
}

#[test]
fn every_voice_changer_names_real_parameters_with_values_in_range() {
    let names: Vec<&str> = VOICE_PRESETS.iter().map(|p| p.name).collect();
    assert_eq!(names, ["robot", "chipmunk", "monster", "cave", "telephone", "alien", "whisper"]);
    for preset in VOICE_PRESETS {
        assert!(!preset.help.is_empty() && !preset.values.is_empty());
        for (name, value) in preset.values {
            let info = by_name(name).unwrap_or_else(|| panic!("{}: there is no parameter {name}", preset.name));
            assert!(info.clean(*value) == *value, "{}: {name} cannot be {value}", preset.name);
            assert!(*value != info.default, "{}: {name} is {value} anyway", preset.name);
        }
    }
}

#[test]
fn every_voice_changer_changes_the_voice_and_none_misbehaves() {
    let (dry, _) = speak(None);
    assert!(rms(&dry) > 0.01);
    for preset in VOICE_PRESETS {
        let (l, r) = speak(Some(preset.name));
        assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0));
        // Not much quieter or louder than the plain voice, and not the same.
        let level = db(rms(&l) / rms(&dry));
        assert!(level > -12.0 && level < 9.0, "{} is {level} dB against the plain voice", preset.name);
        let apart = rms(&l.iter().zip(dry.iter()).map(|(a, b)| a - b).collect::<Vec<f32>>());
        assert!(apart > 0.3 * rms(&dry), "{} is hardly different from the plain voice", preset.name);
    }
}

#[test]
fn the_voice_changers_do_what_their_names_say() {
    // A steady vowel at 120 Hz through each.
    let through = |name: &str| {
        let mut e = Engine::new(SR);
        apply_voice_preset(&mut e, PART, voice_preset(name));
        render_io(&mut e, &speech::vowel(SR, 120.0, 2.0), None, 128).0
    };
    let band = |x: &[f32], lo: f32, hi: f32| (0..60).map(|i| amplitude_at(&x[48000..], lo + (hi - lo) * i as f32 / 60.0, SR).powi(2)).sum::<f32>();
    // The pitch of the 120 Hz voice: the strongest of its low harmonics' spacings.
    let harmonic = |x: &[f32], hz: f32| amplitude_at(&x[48000..], hz, SR);
    let (plain, chipmunk, monster) = (through("none"), through("chipmunk"), through("monster"));
    let up = 120.0 * 2.0f32.powf(7.0 / 12.0);
    let down = 120.0 * 2.0f32.powf(-7.0 / 12.0);
    assert!(harmonic(&plain, 120.0) > 10.0 * harmonic(&plain, up));
    assert!(harmonic(&chipmunk, up) > 10.0 * harmonic(&chipmunk, 120.0));
    assert!(harmonic(&monster, down) > 10.0 * harmonic(&monster, 120.0));
    // The telephone has lost the bottom and the top of the voice.
    let phone = through("telephone");
    assert!(band(&phone, 100.0, 250.0) < 0.05 * band(&plain, 100.0, 250.0));
    assert!(band(&phone, 600.0, 1800.0) > 0.2 * band(&plain, 600.0, 1800.0));
    // The robot's voice is no longer at 120 Hz: it is either side of it, 50 Hz away.
    let robot = through("robot");
    assert!(harmonic(&robot, 170.0) + harmonic(&robot, 70.0) > 4.0 * harmonic(&robot, 120.0));
    // The whisper has no steady pitch left: between two harmonics of the
    // voice there is as much as on them.
    let whisper = through("whisper");
    let (on, between) = (band(&whisper, 1070.0, 1090.0), band(&whisper, 1130.0, 1150.0));
    assert!(on < 4.0 * between && between < 4.0 * on, "the whisper still has harmonics: {on} {between}");
    let (on, between) = (band(&plain, 1070.0, 1090.0), band(&plain, 1130.0, 1150.0));
    assert!(on > 30.0 * between);
    // The cave goes on long after the voice has stopped.
    let (cave, _) = speak(Some("cave"));
    let (dry, _) = speak(None);
    assert!(rms(&cave[3 * 48000 + 9600..]) > 0.005 && peak(&dry[3 * 48000 + 9600..]) == 0.0);
}

#[test]
fn the_voice_changers_are_handed_out_with_the_parameter_table() {
    let json = params::table_json();
    for preset in VOICE_PRESETS {
        let mut wanted = format!("{{\"name\":\"{}\",\"help\":\"{}\",\"values\":{{", preset.name, preset.help);
        for (i, (name, value)) in preset.values.iter().enumerate() {
            wanted += &format!("{}\"{name}\":{value}", if i > 0 { "," } else { "" });
        }
        wanted += "}}";
        assert!(json.contains(&wanted), "the table has no {wanted}");
    }
    assert!(json.contains("\"voice_presets\":[{\"name\":\"robot\"") && json.ends_with("}}]}"));
}

#[test]
fn going_back_to_the_plain_voice_undoes_a_voice_changer() {
    let mut e = Engine::new(SR);
    let before: Vec<f32> = params::PARAMS.iter().map(|p| e.param(PART as usize, p.id as usize)).collect();
    for preset in VOICE_PRESETS {
        apply_voice_preset(&mut e, PART, Some(preset));
        for (name, value) in preset.values {
            assert_eq!(e.param(PART as usize, by_name(name).unwrap().id as usize), *value);
        }
    }
    apply_voice_preset(&mut e, PART, None);
    let after: Vec<f32> = params::PARAMS.iter().map(|p| e.param(PART as usize, p.id as usize)).collect();
    assert_eq!(before, after);
}
