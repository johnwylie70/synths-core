//! docs/engine-api.md is the contract other code is written against. These
//! tests fail if it falls out of step with the engine.

use synths_core::engine::*;
use synths_core::event::{EVENT_SIZE, EVENT_TYPES};
use synths_core::params::{doc_row, PARAMS};

const DOC: &str = include_str!("../docs/engine-api.md");
const ABI: &str = include_str!("../src/abi.rs");

#[test]
fn the_document_lists_every_parameter_as_the_engine_has_it() {
    for p in PARAMS {
        let row = doc_row(p);
        assert!(DOC.contains(&row), "docs/engine-api.md is missing or has out of date:\n{row}\nRun: cargo run --example param_table");
    }
    let rows = DOC.lines().filter(|l| l.starts_with("| ") && l.contains("| `") && l.matches('|').count() == 9).count();
    assert_eq!(rows, PARAMS.len(), "the document has a parameter the engine does not");
}

#[test]
fn the_document_lists_every_event_type() {
    for (id, name, _) in EVENT_TYPES {
        assert!(DOC.contains(&format!("| {id} | `{name}` |")), "event {id} {name}");
        assert!(DOC.contains(&format!("**{id} `{name}`**")), "event {id} {name} has no description");
    }
}

#[test]
fn the_document_lists_every_function() {
    let mut count = 0;
    for line in ABI.lines() {
        if let Some(rest) = line.strip_prefix("pub extern \"C\" fn ").or_else(|| line.strip_prefix("pub unsafe extern \"C\" fn ")) {
            let name = rest.split('(').next().unwrap();
            count += 1;
            assert!(DOC.matches(&format!("{name}(")).count() >= 2, "{name} should be in the header and described");
        }
    }
    assert_eq!(count, 29);
}

#[test]
fn the_document_lists_every_function_of_the_live_input_and_the_recorder() {
    let mut count = 0;
    for line in include_str!("../src/input_abi.rs").lines() {
        if let Some(rest) = line.strip_prefix("pub unsafe extern \"C\" fn ") {
            let name = rest.split('(').next().unwrap();
            count += 1;
            assert!(DOC.matches(&format!("{name}(")).count() >= 2, "{name} should be in the header and described");
        }
    }
    assert_eq!(count, 8);
}

#[test]
fn the_document_lists_every_voice_changer_as_the_engine_has_it() {
    use synths_core::presets::{doc_row, VOICE_PRESETS};
    for preset in VOICE_PRESETS {
        let row = doc_row(preset);
        assert!(DOC.contains(&row), "docs/engine-api.md is missing or has out of date:\n{row}\nRun: cargo run --example param_table presets");
    }
    let rows = DOC.lines().filter(|l| l.starts_with("| `") && l.matches('|').count() == 4).count();
    assert_eq!(rows, VOICE_PRESETS.len(), "the document has a voice changer the engine does not");
    assert!(DOC.contains(&format!("{} voices to begin with", ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight"][VOICE_PRESETS.len()])));
}

#[test]
fn the_document_gives_the_numbers_of_the_input_and_the_voice_effects() {
    use synths_core::input::{GATE_ATTACK, GATE_CLOSE, GATE_FALL, GATE_OFF, RECORD_SIZE};
    use synths_core::voicefx::{Shifter, COMB_LOWEST, FX_UNITS, SPLICE_FADE, SPLICE_MAX, SPLICE_MIN};
    assert_eq!((GATE_ATTACK, GATE_FALL, GATE_CLOSE, RECORD_SIZE), (0.002, 0.03, 0.5, 4));
    assert!(GATE_OFF > -100.0 && GATE_OFF < -99.0);
    assert_eq!((SPLICE_MIN, SPLICE_MAX, SPLICE_FADE, FX_UNITS, COMB_LOWEST), (0.0025, 0.014, 0.004, 8, 50.0));
    // The longest the shifter holds the sound up, at 48 kHz, in milliseconds.
    let ms = |semitones: f32| Shifter::max_delay(48000.0, semitones) / 48.0;
    assert_eq!((format!("{:.1}", ms(12.0)), format!("{:.1}", ms(-12.0))), ("18.4".to_string(), "16.4".to_string()));
    for semitones in [-12.0f32, -5.0, 3.0, 7.0, 12.0] {
        let r = 2.0f32.powf(semitones / 12.0);
        assert!((ms(semitones) - (0.4 + 14.0 + 4.0 * (r - 1.0).abs())).abs() < 0.05);
    }
    // Wherever the lines of the document happen to break.
    let flat = DOC.split_whitespace().collect::<Vec<_>>().join(" ");
    for text in [
        "takes 2 milliseconds to open",
        "falls back to 37% of it in 30 milliseconds",
        "fallen to half the level (6 dB under it",
        "-100, the default, is no gate",
        "between 2.5 and 14 milliseconds away",
        "crossfades to it over at most 4 milliseconds",
        "`0.4 + 14 + 4 * |r - 1|` milliseconds late",
        "at most 18.4 ms an octave up and 16.4 ms an octave down",
        "the engine makes 8 of them",
        "If more than 8 parts ask at once",
        "`fx.comb_freq`, 50 to 5000 Hz",
        "silent for 60 milliseconds",
        "of `synths_output_capacity()` (4096) frames each",
        "four numbers, and return 4",
        "then kept within -4..4",
        "Anything smaller than 0.000 000 000 1",
        "(31 to begin with",
    ] {
        assert!(flat.contains(text), "the document no longer says: {text}");
    }
    assert_eq!(synths_core::params::by_name("input.part").unwrap().default, 31.0);
}

#[test]
fn the_document_gives_the_engines_limits() {
    assert_eq!((MAX_VOICES, NUM_PARTS, MAX_SAMPLES), (64, 64, 1024));
    assert_eq!((QUEUE_CAPACITY, EVENT_BUFFER, OUTPUT_FRAMES, EVENT_SIZE), (4096, 256, 4096, 32));
    assert_eq!(synths_core::voice::CONTROL, 16);
    assert_eq!(synths_core::ABI_VERSION, 1);
    use synths_core::{macros, taps};
    assert_eq!((macros::MAX_STEPS, macros::STEP_SIZE, taps::SCOPE_MASTER, taps::SCOPE_PART), (64, 5, 4096, 1024));
    assert_eq!((taps::LEVEL_SIZE, taps::NOTES_SIZE, taps::MASTER), (4, 8, 65535));
    for text in [
        "There are 64, numbered from 0",
        "There are 64 to share",
        "one of 1024 places",
        "**Control period**: 16 frames",
        "`synths_output_capacity()` (4096) frames",
        "`synths_event_capacity()` (256)",
        "the queue (4096 events) is full",
        "An event is 32 bytes",
        "version **1**",
        "`count` is 0 to 64",
        "the last 4096 frames of the master output",
        "the last 1024 frames of each",
        "Each step is five `f32` numbers",
        "every 256\nframes",
    ] {
        assert!(DOC.contains(text), "the document no longer says: {text}");
    }
}

#[test]
fn the_document_lists_everything_a_sequencer_can_do() {
    let source = include_str!("../src/sequencer.rs");
    let mut count = 0;
    for line in source.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("pub fn ") {
            let name = rest.split('(').next().unwrap();
            count += 1;
            assert!(DOC.contains(&format!("`{name}(")), "Voices::{name} is not in the document");
        }
    }
    assert_eq!(count, 21);
    for text in ["fn advance(&mut self, voices: &mut Voices<'_>) -> u32", "`Sequencer::reset`", "`Engine::set_sequencer("] {
        assert!(DOC.contains(text), "the document no longer says: {text}");
    }
    for field in ["`slot`", "`position`", "`rate`", "`volume`", "`pan`"] {
        assert!(DOC.contains(field), "Trigger's {field} is not in the document");
    }
}
