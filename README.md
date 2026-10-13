# synths-core

The sound engine behind [synths.js](https://github.com/johnwylie70/synths.js) (JavaScript) and [synths](https://github.com/johnwylie70/synths-py) (Python).

It is one engine written once, in Rust, and built two ways:

- as a WebAssembly module, which synths.js loads into the browser's audio worklet, so sound is worked out on the audio thread and a busy page cannot make it stutter;
- as a static library, for a native app.

If you want to make sounds, you want synths.js or synths, not this. This is for the people writing those.

## What it does

The libraries never work out samples. They send the engine commands with a time on each (start this note, set that value, move this voice) and the engine renders the sound.

- 64 voices shared between 64 parts. A part is one instrument, with its own patch, volume, pan, pitch bend and effect sends.
- Each voice has two oscillators (sine, triangle, saw, pulse, or a sample), a sub oscillator and noise (white, pink, and two kinds of 8-bit chip noise), with hard sync, ring modulation and unison. The waves are band-limited, so high notes do not alias.
- Samples play at any pitch, with forward and ping-pong loops, smoothed between frames.
- A filter per voice (low-pass, high-pass, band-pass, notch; 12 or 24 dB), three envelopes and two LFOs.
- Distortion and a bit crusher per part; an echo, a reverb and a chorus to send to; a limiter on the way out, so nothing clips.
- Events are timed to the exact sample, and the same events always give exactly the same sound.

[docs/engine-api.md](docs/engine-api.md) is the full description of the interface: every function, the layout of an event, every parameter.

## Build

You need Rust (1.82 or later).

```
cargo build --release          # the library for this machine
cargo test                     # the tests; they take a few seconds
```

For the web, the WebAssembly module (it needs the `wasm32-wasip1` target, `rustup target add wasm32-wasip1`; the script finds a rustup toolchain that has it, and with node installed it also loads the module, plays a note through it and times it):

```
scripts/build-wasm.sh          # makes target/wasm/synths_core.wasm
```

For an iOS app, the static library (`rustup target add aarch64-apple-ios`):

```
cargo build --release --target aarch64-apple-ios    # makes target/aarch64-apple-ios/release/libsynths_core.a
```

## Listen and measure

```
cargo run --release --example demo     # renders seven short phrases to target/demo/*.wav
cargo run --release --example bench    # how many voices this machine plays in real time
cargo run --example param_table        # the parameter table, as it appears in the document
```

## Where things are

| file | what is in it |
|---|---|
| `src/abi.rs` | the C interface a host calls |
| `src/engine.rs` | parts, voices, the event queue, the mix |
| `src/params.rs` | the parameter table: every value, defined once |
| `src/event.rs` | the event record and the queue |
| `src/voice.rs` | one note: oscillators, filter, envelopes, LFOs |
| `src/osc.rs`, `src/sample.rs`, `src/filter.rs`, `src/env.rs`, `src/lfo.rs` | the parts of a voice |
| `src/fx.rs` | echo, reverb, chorus, distortion, bit crusher, limiter |

To add a parameter, add a line to the table in `src/params.rs` with an id that has never been used, use it in the engine, then run `cargo run --example param_table` and paste the result into the document; a test fails if the two disagree.

## Licence

MIT. See [LICENSE](LICENSE).
