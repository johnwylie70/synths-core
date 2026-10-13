//! synths-core: the sound engine behind synths.js and synths.
//!
//! One engine, built two ways: as a WebAssembly module that runs in the
//! browser's audio worklet, and as a static library for a native app. The
//! JavaScript and Python libraries send it commands (start a note, change a
//! value, play a song); it works out the samples.
//!
//! Code that runs while sound is being made must not allocate, lock or wait.

/// The engine's version, the same as the crate's.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
