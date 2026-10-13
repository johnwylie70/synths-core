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
- Step macros: each part can have a table of up to 64 steps that set the wave (including noise), a pitch offset, the pulse width, a filter offset and the volume, with a loop, a release point and an option to slide between steps. They run at 50 steps a second, or any rate, or on `macro_tick` events from a tracker.
- The filter's outputs can be added together (low + band, band + high, all three), as a sound chip's filter is switched.
- The pulse width goes all the way from 0 to 1, and the pulse wave has no steady offset, so it fades to silence at the ends.
- Taps for a display, readable while sound is being made: the last 4096 frames of the output and 1024 of each part, peak and RMS levels, and the notes held and sounding on each part.
- A per-note arpeggio: the `arp` event gives one note (by handle, or the next note of a part) up to eight semitone offsets to step through, at a rate or on tracker ticks, up, down, up and down, as given or at random. Steps land on the exact frame, and a sounding note's arpeggio can be changed without starting the note again.
- Tests that everything that rings on (echo, reverb, chorus, filter, a long release) falls to exactly nothing and never puts out a denormal number.
- Samples play backwards (`osc1.reverse`, `osc2.reverse`): one-shots from their last frame, forward loops the other way round.
- A sample played faster than it was recorded is smoothed so that it does not alias harshly (`sample.antialias`, on to begin with; off gives the raw sound of an old tracker).
- The 24 dB filter resonates in both of its stages: its peak is as high as the 12 dB filter's at the same setting (it used to be about 3 dB lower, with only the first stage ringing).
- Mono and legato mode remember the notes held down: letting go of the sounding note goes back to the latest one still held.
- A hook for a sequencer inside the engine (`Sequencer`, `Engine::set_sequencer`): it is called from within the render on the exact frames it asks for, and plays voices directly: start a sample at a position and rate on a part and handle, set rate, pitch, volume, pan and position, cut, fade, step macros.
- A voice that has finished is let go at the end of its control period, so a one-shot sample's last moments no longer depend on the size of the blocks rendered.
