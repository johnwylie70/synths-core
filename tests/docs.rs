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
    assert_eq!(count, 25);
}

#[test]
fn the_document_gives_the_engines_limits() {
    assert_eq!((MAX_VOICES, NUM_PARTS, MAX_SAMPLES), (64, 64, 1024));
    assert_eq!((QUEUE_CAPACITY, EVENT_BUFFER, OUTPUT_FRAMES, EVENT_SIZE), (4096, 256, 4096, 32));
    assert_eq!(synths_core::voice::CONTROL, 16);
    assert_eq!(synths_core::ABI_VERSION, 1);
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
    ] {
        assert!(DOC.contains(text), "the document no longer says: {text}");
    }
}
