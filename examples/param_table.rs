//! Prints the parameter table as the Markdown table in docs/engine-api.md.
//!
//!     cargo run --example param_table

use synths_core::params::{doc_row, PARAMS};

fn main() {
    println!("| id | name | kind | min | max | default | unit | what it does |");
    println!("|---|---|---|---|---|---|---|---|");
    for p in PARAMS {
        println!("{}", doc_row(p));
    }
}
