# synths-core

The sound engine behind [synths.js](https://github.com/johnwylie70/synths.js) (JavaScript) and [synths](https://github.com/johnwylie70/synths-py) (Python).

It is one engine written once, in Rust, and built two ways:

- as a WebAssembly module, which synths.js loads into the browser's audio worklet, so sound is worked out on the audio thread and a busy page cannot make it stutter;
- as a static library, for a native app.

Nothing here is ready to use yet. If you want to make sounds, you want synths.js or synths, not this.

## Build

```
cargo build
cargo test
```

## Licence

MIT. See [LICENSE](LICENSE).
