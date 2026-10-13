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
    assert_eq!(count, 17);
    for text in ["fn advance(&mut self, voices: &mut Voices<'_>) -> u32", "`Sequencer::reset`", "`Engine::set_sequencer("] {
        assert!(DOC.contains(text), "the document no longer says: {text}");
    }
    for field in ["`slot`", "`position`", "`rate`", "`volume`", "`pan`"] {
        assert!(DOC.contains(field), "Trigger's {field} is not in the document");
    }
}
