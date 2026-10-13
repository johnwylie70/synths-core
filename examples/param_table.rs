//! Prints the parameter table as the Markdown table in docs/engine-api.md,
//! or, with `presets`, the table of voice changers there.
//!
//!     cargo run --example param_table
//!     cargo run --example param_table presets

use synths_core::params::{doc_row, PARAMS};
use synths_core::presets::{self, VOICE_PRESETS};

fn main() {
    if std::env::args().any(|a| a == "presets") {
        println!("| name | values | what it is |");
        println!("|---|---|---|");
        for p in VOICE_PRESETS {
            println!("{}", presets::doc_row(p));
        }
        return;
    }
    println!("| id | name | kind | min | max | default | unit | what it does |");
    println!("|---|---|---|---|---|---|---|---|");
    for p in PARAMS {
        println!("{}", doc_row(p));
    }
}
