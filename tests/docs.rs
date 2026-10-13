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
fn the_document_gives_the_numbers_of_the_reverb() {
    use synths_core::reverb::*;
    assert_eq!((SCALE_SMALLEST, SCALE_BIGGEST, SIZE_DECAY, DECAY_MIN, DECAY_MAX, PREDELAY_MAX, QUIET, TYPE_FADE), (0.6, 1.4, 3.0, 0.1, 30.0, 0.2, 1.0e-6, 0.02));
    assert_eq!((LINES, SPRING_STAGES), (8, 30));
    let ms = |seconds: f32| if seconds == 0.0 { "none".to_string() } else { format!("{} ms", seconds * 1000.0) };
    let short = |seconds: f32| format!("{}", (seconds * 100.0).round() / 100.0);
    for (value, k) in KINDS.iter().enumerate() {
        // Each type's row: its number, its name, its own tail and what size makes of it, its pre-delay.
        let row = DOC.lines().find(|l| l.starts_with(&format!("| {value} | `{}` |", k.name))).unwrap_or_else(|| panic!("no row for {}", k.name));
        let ends = format!("| {} s | {} to {} s | {} |", k.decay, short(k.decay / 3.0), short(k.decay * 3.0), ms(k.predelay));
        assert!(row.ends_with(&ends), "{row}\nshould end: {ends}");
    }
    assert_eq!((treble_ratio(0.0), treble_ratio(1.0)), (1.0, 1.0 / 8.5));
    assert!(treble_ratio(0.4) < 0.5 && treble_ratio(0.4) > 0.4);
    let (treble, bass): (Vec<f32>, Vec<f32>) = KINDS.iter().map(|k| (k.treble_hz, k.bass)).unzip();
    assert_eq!(treble, [5000.0, 4000.0, 9000.0, 3000.0, 3000.0]);
    assert_eq!(bass, [1.0, 1.15, 0.9, 1.25, 1.0]);
    assert_eq!(Reverb::new(48000.0).bytes() / 1024, 440);
    let flat = DOC.split_whitespace().collect::<Vec<_>>().join(" ");
    for text in [
        "0.6 times its own at 0 and 1.4 times at 1",
        "in seconds, from 0.1 to 30",
        "three times shorter at size 0 and three times longer at size 1",
        "within a tenth, from 1 to 30 seconds",
        "`1 / (1 + 7.5 d²)`",
        "an eighth at 1",
        "above 5 kHz in the room, 4 kHz in the hall, 9 kHz in the plate and 3 kHz in the cathedral and the spring",
        "below 180 Hz, lasts 1.15 times as long as the middle in the hall, 1.25 times in the cathedral, 0.9 times in the plate",
        "(0 to 0.2 seconds)",
        "(20 to 1000 Hz, 120 to begin with)",
        "fades the old type out over 20 milliseconds",
        "under a millionth (0.000 001, -120 dB)",
        "a chain of 30 all-pass filters",
        "Eight delay lines",
        "within 3 cents of its pitch",
        "by 15% at the very most",
        "440 KB at 48 kHz",
    ] {
        assert!(flat.contains(text), "the document no longer says: {text}");
    }
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

#[test]
fn the_document_lists_every_function_of_the_sample_maps() {
    let mut count = 0;
    for line in include_str!("../src/zones_abi.rs").lines() {
        if let Some(rest) = line.strip_prefix("pub unsafe extern \"C\" fn ") {
            let name = rest.split('(').next().unwrap();
            count += 1;
            assert!(DOC.matches(&format!("{name}(")).count() >= 2, "{name} should be in the header and described");
        }
    }
    assert_eq!(count, 5);
}

#[test]
fn the_document_gives_the_zone_record_and_the_numbers_of_the_sample_maps() {
    use core::mem::offset_of;
    use synths_core::zones::*;
    assert_eq!((ZONE_SIZE, ZONES_PER_PART, ZONES_TOTAL, MAX_LAYERS, MAX_MATCHES, ROUND_ROBIN_WRAP), (48, 512, 2048, 4, 32, 5040));
    assert_eq!((ZONE_FIXED_PITCH, ZONE_ONE_SHOT, ZONE_LINEAR_FADE, MAP_SILENT_OUTSIDE), (1, 2, 4, 1));
    assert_eq!((ZONE_LOOP_OWN, ZONE_LOOP_NONE, ZONE_LOOP_FORWARD, ZONE_LOOP_PINGPONG), (0, 1, 2, 3));
    // The record's table: where each field is, how big, and its name.
    let fields: &[(usize, usize, &str, &str)] = &[
        (offset_of!(Zone, slot), 2, "u16", "slot"),
        (offset_of!(Zone, key_lo), 1, "u8", "key_lo"),
        (offset_of!(Zone, key_hi), 1, "u8", "key_hi"),
        (offset_of!(Zone, vel_lo), 1, "u8", "vel_lo"),
        (offset_of!(Zone, vel_hi), 1, "u8", "vel_hi"),
        (offset_of!(Zone, group), 1, "u8", "group"),
        (offset_of!(Zone, flags), 1, "u8", "flags"),
        (offset_of!(Zone, root), 4, "f32", "root"),
        (offset_of!(Zone, tune), 4, "f32", "tune"),
        (offset_of!(Zone, volume), 4, "f32", "volume"),
        (offset_of!(Zone, pan), 4, "f32", "pan"),
        (offset_of!(Zone, release), 4, "f32", "release"),
        (offset_of!(Zone, key_fade_lo), 1, "u8", "key_fade_lo"),
        (offset_of!(Zone, key_fade_hi), 1, "u8", "key_fade_hi"),
        (offset_of!(Zone, vel_fade_lo), 1, "u8", "vel_fade_lo"),
        (offset_of!(Zone, vel_fade_hi), 1, "u8", "vel_fade_hi"),
        (offset_of!(Zone, loop_start), 4, "u32", "loop_start"),
        (offset_of!(Zone, loop_end), 4, "u32", "loop_end"),
        (offset_of!(Zone, loop_mode), 1, "u8", "loop_mode"),
        (offset_of!(Zone, reserved), 3, "u8[3]", "reserved"),
        (offset_of!(Zone, start), 4, "u32", "start"),
    ];
    let mut next = 0;
    for &(at, size, kind, name) in fields {
        assert_eq!(at, next, "{name}");
        next = at + size;
        let bytes = if size == 1 { format!("{at}") } else { format!("{at}..{}", at + size - 1) };
        let row = format!("| {bytes} | `{kind}` | `{name}` |");
        assert!(DOC.contains(&row), "docs/engine-api.md has no row: {row}");
        // The C struct in the document has the fields in the same order.
        assert!(DOC.contains(&format!(" {};", if name == "reserved" { "reserved[3]" } else { name })), "{name} is not in the C struct");
    }
    assert_eq!(next, ZONE_SIZE);
    // The memory the maps take: the zones, and each key's round-robin turn.
    assert_eq!((ZONES_TOTAL * ZONE_SIZE + NUM_PARTS * 128 * 2) / 1024, 112);
    let flat = DOC.split_whitespace().collect::<Vec<_>>().join(" ");
    for text in [
        "A zone is 48 bytes",
        "at most 512 zones",
        "2048 between them",
        "at most 4 of them",
        "the first 32 zones that hold the note",
        "every 5040 hits",
        "112 KB",
    ] {
        assert!(flat.contains(text), "the document no longer says: {text}");
    }
}
