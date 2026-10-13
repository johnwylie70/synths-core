# Changes

## Unreleased

- The repository, with an empty engine that builds.
- The engine: 64 voices shared between 64 parts, each part with its own patch, volume, pan, pitch bend and effect sends.
- Each voice has two oscillators (sine, triangle, saw, pulse or a sample), a sub oscillator and noise (white, pink and two kinds of chip noise), with hard sync, ring modulation and unison. The saw, pulse and triangle are band-limited.
- Samples: mono or stereo, any pitch, a base note, forward and ping-pong loops, a start offset, cubic interpolation.
- A filter per voice (low-pass, high-pass, band-pass, notch; 12 or 24 dB an octave) with resonance, key tracking, an envelope and velocity.
- Three envelopes per voice (amplitude, filter, modulation) with a curve, and two LFOs (sine, triangle, saw, square, random) with delay and fade-in, that move pitch, pulse width, cutoff, loudness and pan.
- Poly, mono and legato modes, and glide.
- Distortion and a bit crusher per part; an echo, a reverb and a chorus that every part can send to; a limiter on the output.
- Events with a time, carried out on the exact sample: note on and off, all notes off, set a parameter (with a glide), pitch bend, and pitch, volume, pan, sample position and cut for a live voice by handle.
- A parameter table, defined once and handed out as JSON.
- A C interface, the same for WebAssembly and for a native library, described in docs/engine-api.md.
- A script that builds the WebAssembly module and checks it in node.
- Tests, a demo that renders phrases to WAV files, and a benchmark.
