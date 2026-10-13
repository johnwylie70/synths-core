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
- A sequencer can also hold a sample still (a rate of 0), queue the sample a voice carries on with at the end of its loop (ProTracker's sample swap), and give a voice an arpeggio in one call.
- A live input: `synths_render_io` and `synths_render_io_into` render with the frames a microphone has just delivered, mono or stereo. The input plays through a part (`input.part`, 31 to begin with) as if it were that part's one voice, so the part's filter, pan, effects, volume and sends shape it and its scope and meter show it. `input.gain`, `input.monitor` (off: recorded but not heard) and a noise gate (`input.gate`, `input.gate_release`). Silence coming in costs almost nothing, and the input starts on its exact frame whatever the block size.
- Voice effects on every part, before its distortion, in a fixed order: a pitch shifter (`fx.pitch`, -12 to 12 semitones, which lines each of its splices up with the wave so that it does not warble; at most 18.4 ms late), a ring modulator (`fx.ring`, `fx.ring_freq`, and `fx.ring_wave` noise for a whisper) and a comb that can be swept into a flanger (`fx.comb`, `fx.comb_freq`, `fx.comb_feedback`, `fx.comb_rate`, `fx.comb_depth`). Eight parts can use the shifter or the comb at once.
- Recording inside the engine: `synths_record_buffer` makes a sample slot to record into, and a `record` event (type 16) starts and stops on the exact frame, recording the live input, any part, or the whole output. The recording is an ordinary sample. `synths_record_read` says how far it has got, and `synths_sample_frames` and `synths_sample_read` get any sample's frames back out.
- Seven voice changers as data (robot, chipmunk, monster, cave, telephone, alien, whisper), in `synths_core::presets` and in the parameter table's JSON as `voice_presets`. `cargo run --release --example voice` renders a made-up voice through each to `target/demo`.
- A test that counts calls to the allocator while everything renders (there are none), and the benchmark times the live input and each voice effect.
- A change of a part's volume, or the master's, slides while the live input is sounding, as it does while a voice is.
