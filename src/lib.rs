//! synths-core: the sound engine behind synths.js and synths.
//!
//! One engine, built two ways: as a WebAssembly module that runs in the
//! browser's audio worklet, and as a static library for a native app. The
//! JavaScript and Python libraries send it commands (start a note, change a
//! value, play a song); it works out the samples.
//!
//! Code that runs while sound is being made must not allocate, lock or wait.
//!
//! `docs/engine-api.md` describes the C interface in `abi`, which is what a
//! host uses. Rust code can use `Engine` directly.

pub mod abi;
pub mod arp;
pub mod engine;
pub mod env;
pub mod event;
pub mod filter;
pub mod fx;
pub mod lfo;
pub mod macros;
pub mod math;
pub mod osc;
pub mod params;
pub mod replayer;
pub mod sample;
pub mod sequencer;
pub mod song;
pub mod taps;
pub mod voice;

pub use engine::Engine;
pub use event::Event;

/// The engine's version, the same as the crate's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The version of the C interface. It goes up when a host written for the
/// old one would stop working.
pub const ABI_VERSION: u32 = 1;
