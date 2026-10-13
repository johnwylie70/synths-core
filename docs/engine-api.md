# The engine's interface

This is the contract between synths-core and anything that hosts it: the audio
worklet of synths.js, the Python binding, a native app. It describes interface
version **1** (`synths_abi_version()`).

The engine is one object. A host creates it for a sample rate, sends it
**events** (start a note, set a parameter, move a live voice) stamped with the
time they should happen, and asks it to **render** blocks of stereo sound. The
host never works out samples itself. For drawing, it can read back the latest
sound, the levels and the notes that are playing.

- [Building](#building)
- [Words and units](#words-and-units)
- [Functions](#functions)
- [Events](#events)
- [Parameters](#parameters)
- [Samples](#samples)
- [Step macros](#step-macros)
- [The arpeggio of a note](#the-arpeggio-of-a-note)
- [Scope, meters and notes](#scope-meters-and-notes)
- [How notes get voices](#how-notes-get-voices)
- [The path of the sound](#the-path-of-the-sound)
- [Threads](#threads)
- [WebAssembly](#webassembly)
- [A host loop](#a-host-loop)
- [Using the engine from Rust](#using-the-engine-from-rust)

## Building

| what | command | result |
|---|---|---|
| WebAssembly module | `scripts/build-wasm.sh` | `target/wasm/synths_core.wasm` |
| iOS static library | `cargo build --release --target aarch64-apple-ios` | `target/aarch64-apple-ios/release/libsynths_core.a` |
| library for this machine | `cargo build --release` | `target/release/libsynths_core.a` and `.dylib` (or `.so`) |

The functions are the same in all three.

## Words and units

- **Frame**: one sample of each channel. The engine always makes two channels.
- **Time**: a count of frames since the engine was created. It starts at 0 and
  goes up by the number of frames rendered. `synths_time` reads it.
- **Note**: a MIDI note number, where 60 is middle C and 69 is the A above it
  (440 Hz). Notes are floating point, so 60.5 is a quarter tone above middle C.
- **Part**: one instrument. There are 64, numbered from 0. Each has its own
  patch (a value for every part parameter), volume, pan, pitch bend and
  effect sends. `65535` (`0xFFFF`) means every part, where an event allows it.
- **Voice**: one sounding note. There are 64 to share between all the parts.
- **Handle**: a number, not 0, that a host gives a note when it starts it, to
  steer that note later. See [handles](#handles).
- **Slot**: one of 1024 places a sample can be loaded into, numbered from 0.
- **Control period**: 16 frames. Slow-moving values (pitch, filter cutoff,
  loudness, pan) are worked out once per control period and slide in a
  straight line in between.
- All numbers are little-endian, as they are in WebAssembly and on every
  machine the engine is built for. `f32` and `f64` are IEEE 754.

## Functions

As a C header:

```c
#include <stdint.h>

typedef struct SynthsEngine SynthsEngine;

uint32_t       synths_abi_version(void);
const uint8_t *synths_version(void);
const uint8_t *synths_param_table(void);
uint32_t       synths_param_table_len(void);

SynthsEngine  *synths_create(float sample_rate);
void           synths_destroy(SynthsEngine *engine);
void           synths_reset(SynthsEngine *engine);
float          synths_sample_rate(const SynthsEngine *engine);
double         synths_time(const SynthsEngine *engine);
uint32_t       synths_active_voices(const SynthsEngine *engine);
float          synths_get_param(const SynthsEngine *engine, uint32_t part, uint32_t id);

uint32_t       synths_render(SynthsEngine *engine, uint32_t frames);
const float   *synths_output_left(const SynthsEngine *engine);
const float   *synths_output_right(const SynthsEngine *engine);
uint32_t       synths_output_capacity(void);
void           synths_render_into(SynthsEngine *engine, float *left, float *right, uint32_t frames);

uint8_t       *synths_event_buffer(SynthsEngine *engine);
uint32_t       synths_event_capacity(void);
uint32_t       synths_push_events(SynthsEngine *engine, uint32_t count);
uint32_t       synths_event_space(const SynthsEngine *engine);

uint8_t       *synths_alloc(uint32_t bytes);
void           synths_free(uint8_t *ptr);
int32_t        synths_sample_load(SynthsEngine *engine, uint32_t slot, const float *data,
                                  uint32_t frames, uint32_t channels, float sample_rate);
int32_t        synths_sample_config(SynthsEngine *engine, uint32_t slot, float base_note,
                                    uint32_t loop_mode, uint32_t loop_start, uint32_t loop_end);
int32_t        synths_sample_free(SynthsEngine *engine, uint32_t slot);

int32_t        synths_macro_set(SynthsEngine *engine, uint32_t part, const float *steps, uint32_t count,
                                uint32_t loop_start, uint32_t loop_end, uint32_t release, uint32_t flags);

uint32_t       synths_scope_read(const SynthsEngine *engine, uint32_t source, float *out, uint32_t frames);
uint32_t       synths_levels_read(const SynthsEngine *engine, uint32_t source, uint32_t count, float *out);
uint32_t       synths_notes_read(const SynthsEngine *engine, uint32_t part, uint32_t *out);
```

In WebAssembly a pointer is a `u32` offset into the module's `memory`, `float`
is `f32` and `double` is `f64`.

Every function that takes an engine accepts a null engine and does nothing
(returning 0 or null).

### Version and the parameter table

**`synths_abi_version() -> u32`** The version of this interface: 1. It goes up
when a host written for the old one would stop working. Check it on load.

**`synths_version() -> *const u8`** The engine's version (the crate's, for
example `0.0.0`) as UTF-8 text ending in a zero byte. The pointer is good for
ever.

**`synths_param_table() -> *const u8`** and **`synths_param_table_len() -> u32`**
The [parameter table](#the-table-as-json), the event types and the engine's
limits as UTF-8 JSON text, and its length in bytes. The text also ends in a
zero byte, which the length does not count. The pointer is good for ever. The
first call builds the text (it allocates), so call it while setting up, not
while sound is playing. It needs no engine.

### Making and ending an engine

**`synths_create(sample_rate: f32) -> *mut Engine`** Make an engine. The
sample rate is brought into 8000..192000. Returns null if the rate is not a
number or not above 0. This allocates everything the engine will ever need,
apart from samples: between 1 and 2 MB at 48 kHz, and more at higher rates
(most of it is the echo's two seconds of memory). A new engine holds every
parameter's default, no samples and no events, and its time is 0.

**`synths_destroy(engine)`** Free an engine and its samples. Do not use the
pointer afterwards.

**`synths_reset(engine)`** The panic button: stop every voice at once, forget
every queued event and every running glide, and empty the echo, reverb and
chorus. Parameters, samples and the time stay as they are.

### Reading the engine

**`synths_sample_rate(engine) -> f32`** The rate the engine was made with,
after it was brought into range.

**`synths_time(engine) -> f64`** The engine's time: how many frames it has
rendered. It is a whole number, exact up to 2^53.

**`synths_active_voices(engine) -> u32`** How many voices are sounding, 0 to 64.

**`synths_get_param(engine, part: u32, id: u32) -> f32`** A parameter's value
now (part way through a glide, the value it has reached). `part` is ignored
for an engine-wide parameter. An id or part that does not exist gives 0.

### Rendering

There are two ways; use whichever suits the host. Both advance the time, and
both first carry out every queued event whose time has come.

**`synths_render(engine, frames: u32) -> u32`** Render into the engine's own
two buffers, which hold `synths_output_capacity()` (4096) frames. If `frames`
is more than that, only 4096 are rendered. Returns how many were rendered.
Read them from:

**`synths_output_left(engine) -> *const f32`** and
**`synths_output_right(engine) -> *const f32`** The buffers `synths_render`
writes: `frames` numbers each, left and right separately (not interleaved).
The pointers do not change for the life of the engine. In WebAssembly a typed
array made over them stops working when the memory grows; see
[WebAssembly](#webassembly).

**`synths_render_into(engine, left: *mut f32, right: *mut f32, frames: u32)`**
Render any number of frames straight into the host's own two buffers, which
must not overlap. Does nothing if either pointer is null.

Any block size works, and the sound does not depend on it: rendering 128
frames at a time, or 1, or 4096, gives exactly the same numbers.

Every number rendered is finite and inside -1..1, whatever events arrive.

### Sending events

**`synths_event_buffer(engine) -> *mut u8`** A buffer inside the engine with
room for `synths_event_capacity()` (256) [event records](#the-event-record) of
32 bytes each. It is aligned to 8 bytes and does not move.

**`synths_push_events(engine, count: u32) -> u32`** Copy the first `count`
records of the event buffer into the engine's queue. Returns how many were
taken. Fewer than `count` means the queue (4096 events) is full: the rest
were not taken, and the host should send them again after rendering has made
room. `count` above 256 is treated as 256.

**`synths_event_space(engine) -> u32`** How many more events the queue has
room for.

The host can reuse the event buffer as soon as `synths_push_events` returns.

### Samples

**`synths_alloc(bytes: u32) -> *mut u8`** Get memory to put sample data in
before loading it. It is aligned to 16 bytes. Null if there is none. A
WebAssembly host needs this, because it can only hand the engine data that is
inside the module's memory; a native host can pass `synths_sample_load` any
pointer of its own.

**`synths_free(ptr)`** Give back memory from `synths_alloc`. Null is allowed.

**`synths_sample_load(engine, slot, data: *const f32, frames, channels, sample_rate: f32) -> i32`**
Copy a sample into a slot (0..1023), replacing any sample there. `data` holds
`frames * channels` numbers; `channels` is 1 or 2, and with 2 the numbers
alternate left, right. `sample_rate` is the rate it was recorded at. The
engine keeps its own copy, so the host frees `data` afterwards. The new
sample has no loop and its base note is 60. See [Samples](#samples-1).

**`synths_sample_config(engine, slot, base_note: f32, loop_mode, loop_start, loop_end) -> i32`**
Set the note at which the sample plays back unchanged, and its loop.
`loop_mode` is 0 (none), 1 (forward) or 2 (ping-pong). The loop runs from
frame `loop_start` up to but not including frame `loop_end`.

**`synths_sample_free(engine, slot) -> i32`** Empty a slot and free its memory.
Voices playing the sample stop.

These three return 0 when they work, or:

| value | meaning |
|---|---|
| -1 | the slot number is 1024 or more |
| -2 | not 1 or 2 channels, no frames, or a sample rate (or base note) that is not a sensible number |
| -3 | there is no sample in the slot |
| -4 | the engine or the data pointer is null |

They allocate and free memory, so they are not for the middle of a render; see
[Threads](#threads).

### Macros

**`synths_macro_set(engine, part, steps: *const f32, count, loop_start, loop_end, release, flags) -> i32`**
Give a part its [step macro](#step-macros), replacing any it had. `steps`
holds `count * 5` numbers, five a step. `count` is 0 to 64; 0 takes the
macro away (and `steps` may then be null). `part` 65535 gives the same macro
to every part. The engine copies the numbers. Voices already sounding on the
part carry on from the same step number in the new macro. Returns 0, or -1
(the part does not exist), -2 (more than 64 steps) or -4 (the engine, or
`steps` with a count above 0, is null). It allocates nothing.

### Reading the sound back

These three are for a display: see [Scope, meters and notes](#scope-meters-and-notes).
They only read, and unlike everything else that takes an engine they may be
called while another thread is rendering.

**`synths_scope_read(engine, source, out: *mut f32, frames) -> u32`** Copy the
most recent `frames` frames of a part's sound into `out`, oldest first, or of
the master output for `source` 65535. Returns how many were written: at most
4096 for the master and 1024 for a part, and 0 for a part that does not
exist.

**`synths_levels_read(engine, source, count, out: *mut f32) -> u32`** Write
the levels of `count` parts, starting at part `source`, into `out`: four
numbers each. Stops at the last part. For `source` 65535 it writes the master
output's four numbers and nothing more. Returns how many sets of four it
wrote.

**`synths_notes_read(engine, part, out: *mut u32) -> u32`** Write which notes
a part is playing into `out`: eight 32-bit words. `part` 65535 gives every
part together. Returns 8, or 0 for a part that does not exist.

In WebAssembly `out` must be inside the module's memory: get it once with
`synths_alloc` and keep it.

## Events

### The event record

An event is 32 bytes:

| bytes | type | name | meaning |
|---|---|---|---|
| 0..7 | `f64` | `time` | when it happens: a time in frames, as `synths_time` counts them |
| 8..9 | `u16` | `type` | which event, from the table below |
| 10..11 | `u16` | `part` | the part, 0..63, or 65535 for every part where the event allows it |
| 12..15 | `u32` | `a` | a whole-number argument: a handle or a parameter id |
| 16..19 | `f32` | `x` | first value |
| 20..23 | `f32` | `y` | second value |
| 24..27 | `f32` | `z` | third value; 0 where an event does not use it |
| 28..31 | `u32` | `b` | a second whole-number argument; 0 where an event does not use it |

In JavaScript, with `view` a `DataView` over the event buffer and `at` the
record's offset (its index times 32):

```js
view.setFloat64(at, time, true);
view.setUint16(at + 8, type, true);
view.setUint16(at + 10, part, true);
view.setUint32(at + 12, a, true);
view.setFloat32(at + 16, x, true);
view.setFloat32(at + 20, y, true);
view.setFloat32(at + 24, 0, true);
view.setUint32(at + 28, 0, true);
```

In Python: `struct.pack('<dHHIfffI', time, type, part, a, x, y, 0.0, 0)`.

### Timing

- An event happens on exactly the frame its time names: a note-on at time
  1000 puts its first sample in frame 1000, wherever that falls in a block.
- A fractional time is rounded down.
- An event whose time has already passed, or is 0, negative or not a number,
  happens at the start of the next render. So "now" is simply time 0.
- Events with the same time happen in the order they were pushed. Events may
  be pushed in any order of time.
- An event may be pushed any length of time ahead. It occupies one of the
  queue's 4096 places until it happens.
- To schedule in seconds: `time = synths_time(engine) + seconds * sample_rate`.

Notes start, stop and are cut on the exact frame. An event that changes a
slow-moving value (a parameter, a voice's pitch, volume or pan) is also
applied on its exact frame, and the value then slides to its new setting over
what is left of the current control period: at most 16 frames, a third of a
millisecond at 48 kHz.

### Event types

| type | name | part | `a` | `x` | `y` |
|---|---|---|---|---|---|
| 1 | `note_on` | one part | handle, or 0 | note | velocity, 0..1 |
| 2 | `note_off` | one part or all | handle, or 0 to match by note | note (when `a` is 0) | |
| 3 | `all_notes_off` | one part or all | 0: let go, 1: cut at once | | |
| 4 | `param` | one part or all | parameter id | value | glide, in seconds |
| 5 | `pitch_bend` | one part or all | | bend, -1..1 | glide, in seconds |
| 6 | `voice_pitch` | one part or all | handle | note | glide, in seconds |
| 7 | `voice_freq` | one part or all | handle | frequency in Hz | glide, in seconds |
| 8 | `voice_volume` | one part or all | handle | volume, 0..4 | |
| 9 | `voice_pan` | one part or all | handle | pan, -1..1 | |
| 10 | `voice_position` | one part or all | handle | position in frames | |
| 11 | `voice_kill` | one part or all | handle | | |
| 12 | `macro_tick` | one part or all | | | |
| 13 | `arp` | one part or all | handle, or 0 for the part's next note | steps a second | order |
| 14 | `arp_more` | one part or all | handle, or 0 for the part's next note | | |

Events 13 and 14 also use `z` and `b`; see [their descriptions](#the-arpeggio-of-a-note).

An event with a type not in this table, or a part that does not exist, is
ignored. So is one whose `x` is not a number, if it uses `x`. A velocity that
is not a number counts as 1, and a glide that is not a number as 0.

**1 `note_on`** Start a note on a part with the part's patch as it is at that
moment. The note is brought into -24..160 and the velocity into 0..1. How
velocity affects loudness is the part's `velocity` parameter. What happens
when the part is in mono or legato mode, or has unison voices, or there is no
free voice, is under [How notes get voices](#how-notes-get-voices).

**2 `note_off`** Let a note go: its envelopes move to their release stage and
the voice ends when the amplitude envelope reaches nothing. With `a` 0, every
held voice of the part that was started with note `x` (within 0.005 of it) is
let go. With `a` a handle, every held voice of the part with that handle is.
A note-off that matches nothing does nothing.

**3 `all_notes_off`** With `a` 0, let go every held voice of the part. With
`a` 1, cut every voice of the part at once, including those already dying
away.

**4 `param`** Set a parameter. The value is brought into the parameter's
range, and rounded to a whole number for the kinds `int`, `bool` and
`choice`. For an engine-wide parameter `part` is ignored. With `y` above 0
and a `float` parameter, the value glides in a straight line from where it is
to `x` over `y` seconds, one step per control period; a second `param` event
for the same parameter of the same part replaces a glide that is still
running. At most 64 glides run at once (each part counts separately, so a
glide sent to every part uses all 64); past that a glide jumps straight to
its value. A change to a patch parameter affects the notes already sounding
on the part as well as new ones, except for what is only read when a note
starts: `osc1.start`, `osc2.start`, `unison.*`, `mode` and `glide`.
  (`osc1.reverse` is read all the time: changing it turns a sounding sample
  round where it is.)

**5 `pitch_bend`** The same as a `param` event for the `bend` parameter (id 4).
The pitch moves by `x` times the part's `bend_range`.

**6 `voice_pitch`**, **7 `voice_freq`** Move live voices to a new note, given
as a note number or as a frequency in Hz (a frequency of 0 or less is
ignored). With `y` above 0 the pitch slides there over `y` seconds, in a
straight line in semitones. This replaces the note the voice was started
with, and everything else that moves pitch (transpose, tune, bend, LFOs, the
modulation envelope, oscillator tuning) still applies on top. A `note_off`
by note still matches the note the voice was started with.

**8 `voice_volume`** Set the volume of live voices: a multiplier, 1 when a
voice starts, brought into 0..4. It is separate from velocity.

**9 `voice_pan`** Set the pan of live voices, -1..1, 0 when a voice starts.
It is added to the part's pan.

**10 `voice_position`** Jump to a position in the sample, in frames of the
sample (fractions allowed). It moves both oscillators' sample playback. A
position at or past the end of a sample with no loop ends that oscillator. An
`f32` holds whole numbers exactly up to 16,777,216.

**11 `voice_kill`** Cut live voices at once.

**12 `macro_tick`** Move the step macro of every sounding voice of the part
on by one step, on this exact frame, if the part's `macro.sync` is on. If it
is off the tick is ignored. A tracker sends one of these, to every part, on
each of its ticks. It also steps any arpeggio that was asked to step on
ticks. See [Step macros](#step-macros).

**13 `arp`**, **14 `arp_more`** Give a note an arpeggio, change it, or take it
away. See [The arpeggio of a note](#the-arpeggio-of-a-note).

### Handles

A handle is any number from 1 to 4294967295 that the host chooses and puts in
a `note_on`. Events 6 to 11 and `note_off` then address the voices of that
part started with that handle, whether held or dying away. With `part` 65535
they address voices with that handle on every part.

- Handle 0 means no handle. Events 6 to 11 with handle 0 do nothing.
- Handles belong to a part: handle 5 on part 0 and handle 5 on part 1 are
  different notes.
- Several voices can share a handle, and an event addresses them all. That is
  what happens with unison, and when a host sends a second `note_on` with a
  handle that is still sounding: the engine does **not** cut the first. A
  host that wants one voice per handle (a tracker channel) sends `voice_kill`
  or `note_off` before the new `note_on`, with the same time.
- When a voice ends, its handle addresses nothing. Nothing tells the host a
  voice has ended, and events to a handle with no voices are ignored.

A voice that is cut (by `voice_kill`, `all_notes_off` with `a` 1, being taken
over by a new note, or `voice_position`) does not click: whatever it was
putting out at that moment fades away over a few milliseconds.

## Parameters

Every parameter has an id that will never change, a name, a kind, a range, a
default and a unit. Ids 0 to 127 are **part** parameters: each part has its
own value, and together they are the part's patch. Ids 200 to 231 are
**engine** parameters: there is one value for the whole engine.

Ids not in the table do not exist yet. Setting one does nothing.

| id | name | kind | min | max | default | unit | what it does |
|---|---|---|---|---|---|---|---|
| 0 | `volume` | float | 0 | 2 | 1 | gain | How loud the part is (1 is unchanged). |
| 1 | `pan` | float | -1 | 1 | 0 | pan | Left (-1) to right (1). |
| 2 | `transpose` | float | -48 | 48 | 0 | semitones | Shifts every note of the part. |
| 3 | `tune` | float | -100 | 100 | 0 | cents | Fine tuning of the part. |
| 4 | `bend` | float | -1 | 1 | 0 |  | The pitch bend wheel: -1 to 1 times bend_range. |
| 5 | `bend_range` | float | 0 | 48 | 2 | semitones | How far a full pitch bend moves the pitch. |
| 6 | `glide` | float | 0 | 10 | 0 | seconds | How long the pitch takes to slide from the last note to a new one. |
| 7 | `mode` | choice: 0 poly, 1 mono, 2 legato | 0 | 2 | 0 |  | poly: every note gets its own voice. mono: one note at a time. legato: one note at a time, and a new note does not restart the envelopes. |
| 8 | `unison.voices` | int | 1 | 8 | 1 | voices | How many detuned voices each note plays. |
| 9 | `unison.detune` | float | 0 | 100 | 20 | cents | How far apart the unison voices are tuned. |
| 10 | `unison.spread` | float | 0 | 1 | 0.5 |  | How far apart the unison voices are panned. |
| 11 | `velocity` | float | 0 | 1 | 1 |  | How much a note's velocity changes its loudness (0: not at all). |
| 16 | `osc1.wave` | choice: 0 sine, 1 triangle, 2 saw, 3 pulse, 4 sample | 0 | 4 | 1 |  | The shape of the wave, or a loaded sample. |
| 17 | `osc1.level` | float | 0 | 1 | 1 | gain | How loud oscillator 1 is. |
| 18 | `osc1.coarse` | float | -48 | 48 | 0 | semitones | Tuning in semitones. |
| 19 | `osc1.fine` | float | -100 | 100 | 0 | cents | Fine tuning. |
| 20 | `osc1.pulse_width` | float | 0 | 1 | 0.5 |  | The width of the pulse wave (0.5 is a square; at 0 and 1 it is silent). |
| 21 | `osc1.sample` | int | 0 | 1023 | 0 | slot | Which sample slot the sample wave plays. |
| 22 | `osc1.start` | float | 0 | 1 | 0 |  | Where in the sample a note starts (0: the beginning, 1: the end). |
| 23 | `osc1.keytrack` | bool | 0 | 1 | 1 |  | On: the pitch follows the note. Off: every note plays at the same pitch (drums). |
| 24 | `osc2.wave` | choice: 0 sine, 1 triangle, 2 saw, 3 pulse, 4 sample | 0 | 4 | 2 |  | The shape of the wave, or a loaded sample. |
| 25 | `osc2.level` | float | 0 | 1 | 0 | gain | How loud oscillator 2 is. |
| 26 | `osc2.coarse` | float | -48 | 48 | 0 | semitones | Tuning in semitones. |
| 27 | `osc2.fine` | float | -100 | 100 | 0 | cents | Fine tuning. |
| 28 | `osc2.pulse_width` | float | 0 | 1 | 0.5 |  | The width of the pulse wave (0.5 is a square; at 0 and 1 it is silent). |
| 29 | `osc2.sample` | int | 0 | 1023 | 0 | slot | Which sample slot the sample wave plays. |
| 30 | `osc2.start` | float | 0 | 1 | 0 |  | Where in the sample a note starts. |
| 31 | `osc2.keytrack` | bool | 0 | 1 | 1 |  | On: the pitch follows the note. |
| 32 | `osc2.sync` | bool | 0 | 1 | 0 |  | Hard sync: oscillator 2 restarts its wave every time oscillator 1 does. |
| 33 | `osc2.ring` | float | 0 | 1 | 0 |  | Ring modulation: how much oscillator 2 is multiplied by oscillator 1. |
| 34 | `osc1.reverse` | bool | 0 | 1 | 0 |  | On: oscillator 1's sample plays backwards. |
| 35 | `osc2.reverse` | bool | 0 | 1 | 0 |  | On: oscillator 2's sample plays backwards. |
| 36 | `sub.level` | float | 0 | 1 | 0 | gain | How loud the sub oscillator is. |
| 37 | `sub.wave` | choice: 0 sine, 1 square, 2 triangle | 0 | 2 | 0 |  | The shape of the sub oscillator. |
| 38 | `sub.octave` | int | -2 | -1 | -1 | octaves | One or two octaves below the note. |
| 39 | `sample.antialias` | bool | 0 | 1 | 1 |  | On: a sample played faster than it was recorded is smoothed, so it does not turn harsh. Off: it is left raw, as an old sampler or a tracker of the 1980s would play it. |
| 40 | `noise.level` | float | 0 | 1 | 0 | gain | How loud the noise is. |
| 41 | `noise.type` | choice: 0 white, 1 pink, 2 lfsr, 3 lfsr_short | 0 | 3 | 0 |  | white and pink are hiss; lfsr is the pitched noise of an 8-bit sound chip; lfsr_short is its buzzy, metallic version. |
| 42 | `noise.pitch` | float | -48 | 48 | 0 | semitones | Tuning of the lfsr noise, which follows the note. |
| 48 | `filter.type` | choice: 0 off, 1 lowpass, 2 highpass, 3 bandpass, 4 notch, 5 low_band, 6 band_high, 7 all | 0 | 7 | 0 |  | What the filter lets through. The last four add outputs together, as a sound chip does: notch is low + high, low_band is low + band, band_high is band + high, all is all three. |
| 49 | `filter.slope` | choice: 0 12db, 1 24db | 0 | 1 | 0 |  | How steeply the filter cuts. |
| 50 | `filter.cutoff` | float | 20 | 20000 | 2000 | hz | The frequency where the filter starts to cut. |
| 51 | `filter.resonance` | float | 0 | 1 | 0.15 |  | How much the filter emphasises the cutoff frequency. |
| 52 | `filter.keytrack` | float | 0 | 1 | 0 |  | How much the cutoff follows the note (1: exactly, around middle C). |
| 53 | `filter.env` | float | -96 | 96 | 0 | semitones | How far the filter envelope moves the cutoff. |
| 54 | `filter.velocity` | float | -96 | 96 | 0 | semitones | How far a full-velocity note moves the cutoff. |
| 56 | `amp.attack` | float | 0 | 20 | 0.005 | seconds | How long a note takes to reach full loudness. |
| 57 | `amp.decay` | float | 0 | 20 | 0.1 | seconds | How long it then takes to fall to the sustain level. |
| 58 | `amp.sustain` | float | 0 | 1 | 0.8 |  | The loudness while the note is held. |
| 59 | `amp.release` | float | 0 | 20 | 0.15 | seconds | How long the note takes to die away after it is let go. |
| 60 | `amp.curve` | float | 0 | 1 | 0.5 |  | The shape of the envelope: 0 is straight lines, 1 is strongly curved. |
| 64 | `filter_env.attack` | float | 0 | 20 | 0.005 | seconds | Attack time of the filter envelope. |
| 65 | `filter_env.decay` | float | 0 | 20 | 0.2 | seconds | Decay time of the filter envelope. |
| 66 | `filter_env.sustain` | float | 0 | 1 | 0.3 |  | Sustain level of the filter envelope. |
| 67 | `filter_env.release` | float | 0 | 20 | 0.2 | seconds | Release time of the filter envelope. |
| 68 | `filter_env.curve` | float | 0 | 1 | 0.5 |  | The shape of the filter envelope. |
| 72 | `mod_env.attack` | float | 0 | 20 | 0.005 | seconds | Attack time of the modulation envelope. |
| 73 | `mod_env.decay` | float | 0 | 20 | 0.2 | seconds | Decay time of the modulation envelope. |
| 74 | `mod_env.sustain` | float | 0 | 1 | 0 |  | Sustain level of the modulation envelope. |
| 75 | `mod_env.release` | float | 0 | 20 | 0.2 | seconds | Release time of the modulation envelope. |
| 76 | `mod_env.curve` | float | 0 | 1 | 0.5 |  | The shape of the modulation envelope. |
| 77 | `mod_env.pitch` | float | -48 | 48 | 0 | semitones | How far the modulation envelope moves the pitch of the whole voice. |
| 78 | `mod_env.osc2_pitch` | float | -48 | 48 | 0 | semitones | How far it moves the pitch of oscillator 2 only (sweeps a synced oscillator). |
| 79 | `mod_env.pulse_width` | float | -1 | 1 | 0 |  | How far it moves the pulse width. |
| 80 | `lfo1.wave` | choice: 0 sine, 1 triangle, 2 saw, 3 square, 4 random | 0 | 4 | 0 |  | The shape of the slow wave. random steps to a new value each cycle. |
| 81 | `lfo1.rate` | float | 0.01 | 50 | 5 | hz | How fast it goes. |
| 82 | `lfo1.delay` | float | 0 | 10 | 0 | seconds | How long after the note starts before it begins. |
| 83 | `lfo1.fade` | float | 0 | 10 | 0 | seconds | How long it then takes to reach full strength. |
| 84 | `lfo1.sync` | bool | 0 | 1 | 1 |  | On: it restarts with every note. Off: all the notes of the part share one. |
| 85 | `lfo1.pitch` | float | -48 | 48 | 0 | semitones | Vibrato: how far it moves the pitch. |
| 86 | `lfo1.pulse_width` | float | -1 | 1 | 0 |  | How far it moves the pulse width. |
| 87 | `lfo1.cutoff` | float | -96 | 96 | 0 | semitones | How far it moves the filter cutoff. |
| 88 | `lfo1.amp` | float | 0 | 1 | 0 |  | Tremolo: how much it changes the loudness. |
| 89 | `lfo1.pan` | float | 0 | 1 | 0 |  | How far it moves the sound left and right. |
| 96 | `lfo2.wave` | choice: 0 sine, 1 triangle, 2 saw, 3 square, 4 random | 0 | 4 | 1 |  | The shape of the slow wave. |
| 97 | `lfo2.rate` | float | 0.01 | 50 | 0.5 | hz | How fast it goes. |
| 98 | `lfo2.delay` | float | 0 | 10 | 0 | seconds | How long after the note starts before it begins. |
| 99 | `lfo2.fade` | float | 0 | 10 | 0 | seconds | How long it then takes to reach full strength. |
| 100 | `lfo2.sync` | bool | 0 | 1 | 0 |  | On: it restarts with every note. Off: all the notes of the part share one. |
| 101 | `lfo2.pitch` | float | -48 | 48 | 0 | semitones | How far it moves the pitch. |
| 102 | `lfo2.pulse_width` | float | -1 | 1 | 0 |  | How far it moves the pulse width. |
| 103 | `lfo2.cutoff` | float | -96 | 96 | 0 | semitones | How far it moves the filter cutoff. |
| 104 | `lfo2.amp` | float | 0 | 1 | 0 |  | How much it changes the loudness. |
| 105 | `lfo2.pan` | float | 0 | 1 | 0 |  | How far it moves the sound left and right. |
| 112 | `fx.drive` | float | 0 | 1 | 0 |  | Distortion (0 is off). |
| 113 | `fx.bits` | float | 1 | 16 | 16 | bits | Bit crusher: how many bits each sample keeps (16 is off). |
| 114 | `fx.downsample` | int | 1 | 64 | 1 |  | Bit crusher: keep only one sample in this many (1 is off). |
| 115 | `send.delay` | float | 0 | 1 | 0 | gain | How much of the part goes to the echo. |
| 116 | `send.reverb` | float | 0 | 1 | 0 | gain | How much of the part goes to the reverb. |
| 117 | `send.chorus` | float | 0 | 1 | 0 | gain | How much of the part goes to the chorus. |
| 120 | `macro.rate` | float | 1 | 1000 | 50 | hz | How many steps of the part's macro play each second. |
| 121 | `macro.sync` | bool | 0 | 1 | 0 |  | On: the macro steps on each macro_tick event (a tracker's tick) instead of by the clock. |
| 200 | `master.volume` | float | 0 | 2 | 0.35 | gain | How loud everything is. |
| 201 | `master.limiter` | bool | 0 | 1 | 1 |  | Turns loud passages down so they do not distort. The output never goes past -1..1 either way. |
| 202 | `delay.time` | float | 0.001 | 2 | 0.3 | seconds | The time between echoes. |
| 203 | `delay.feedback` | float | 0 | 0.95 | 0.4 |  | How much of each echo is fed back to make the next. |
| 204 | `delay.tone` | float | 200 | 20000 | 6000 | hz | Each echo is duller than the last; this is how much treble survives. |
| 205 | `delay.pingpong` | bool | 0 | 1 | 0 |  | On: the echoes bounce between left and right. |
| 208 | `reverb.size` | float | 0 | 1 | 0.6 |  | How big the room is (how long the reverb lasts). |
| 209 | `reverb.damping` | float | 0 | 1 | 0.4 |  | How quickly the treble of the reverb dies away. |
| 212 | `chorus.rate` | float | 0.05 | 10 | 0.8 | hz | How fast the chorus wobbles. |
| 213 | `chorus.depth` | float | 0 | 1 | 0.5 |  | How far the chorus wobbles. |
| 216 | `voices` | int | 1 | 64 | 64 | voices | The most voices that sound at once; a new note past this takes over the oldest. |

Kinds: a `float` takes any value in its range; an `int` is rounded to a whole
number; a `bool` is 0 or 1; a `choice` is a whole number that picks from the
list shown.

Oscillator 2's parameters are oscillator 1's with 8 added to the id (as far
as `keytrack`; `osc2.reverse` is `osc1.reverse` with 1 added), and LFO 2's are
LFO 1's with 16 added.

A part's step macro is not in this table, because it is a list and not one
number: it is set with `synths_macro_set`.

### Notes on some parameters

- **`volume`, `master.volume`** are plain multipliers. A change slides over
  about 15 milliseconds unless nothing is sounding, when it is immediate.
- **`pan`** A voice's pan is the part's `pan`, plus its `voice_pan`, plus its
  unison spread, plus the LFOs, brought into -1..1. A mono voice is placed
  with equal power: in the middle it is 0.7071 of its level on each side, and
  at one edge all of it is on that side. A voice whose oscillator 1 is a
  stereo sample keeps both sides at full level in the middle, and pan turns
  the far side down.
- **`velocity`** With `s` this parameter and `v` the note's velocity, the
  note's level is `1 - s + s * v`: a straight line, not a curve.
- **`mode`, `glide`, `unison.*`** See [How notes get voices](#how-notes-get-voices).
- **`osc1.wave`** 4 plays the sample in slot `osc1.sample`. An empty slot is
  silent.
- **`osc1.pulse_width`** The width used is this (or the macro step's), plus
  what the LFOs and the modulation envelope add, kept within 0..1. The pulse
  wave is made without its steady offset, so it has none at any width, and
  as the width reaches 0 or 1 the wave thins away to silence rather than
  jumping. A narrow pulse therefore stands taller than a square: up to twice
  the height at the extremes. The edges stay band-limited while the width
  moves.
- **`osc1.keytrack`** Off, a wave plays at the pitch of note 60 and a sample
  at its own pitch, whatever note is played. Tuning, bend and modulation
  still apply.
- **`osc1.reverse`, `sample.antialias`** See [Samples](#samples-1).
- **`osc2.sync`** works when both oscillators are waves, not samples.
  Oscillator 2 is then one sample late.
- **`osc2.ring`** At 1, oscillator 2 is replaced by oscillator 1 times
  oscillator 2; in between it is a mix. Oscillator 1 is used at full level
  for this whatever `osc1.level` says.
- **`noise.type`** `lfsr` is a 23-bit shift register read 8 bits at a time
  and stepped 16 times per cycle of the note (moved by `noise.pitch`), at
  most 4 steps a frame. `lfsr_short` is a 15-bit register in its 93-step
  mode, which gives a pitched buzz.
- **`filter.cutoff`** is the cutoff for note 60 with no modulation. The
  cutoff used is that, moved by `filter.keytrack` times the note's distance
  from 60, `filter.env` times the filter envelope (0..1), `filter.velocity`
  times the velocity, and the LFOs, all in semitones, and then kept between
  16 Hz and 0.45 of the sample rate.
- **`filter.type`** The filter has three outputs at once: low-pass,
  band-pass and high-pass. Types 1 to 3 pick one. Types 4 to 7 add outputs
  together, the way the filter of an 8-bit sound chip is switched: 4 `notch`
  is low + high, 5 `low_band` is low + band, 6 `band_high` is band + high,
  and 7 `all` is all three (everything passes, with a lift at the cutoff
  that grows with the resonance).
- **`filter.resonance`** `r` gives a Q of `0.5 * 2^(6.5 * r)`: 0.5 (no peak
  at all) at 0, 1.4 at 0.23, about 45 at 1. It rings but does not
  oscillate by itself. With `filter.slope` 24 dB there are two stages, one
  after the other, each with the square root of that Q, so that together
  they peak exactly as high at the cutoff as the 12 dB filter does (and
  with no resonance are 6 dB down there, as it is).
- **Envelope times** are in seconds and exact to the frame. `curve` 0 gives
  straight lines; towards 1 each stage moves quickly at first and then
  slowly. A sustain of 0 ends the voice when the decay finishes, with no
  note-off needed (drums, plucks).
- **`lfo1.sync`** On, each voice has its own LFO, which starts from the
  beginning of its wave when the note's `lfo1.delay` is over. Off, the
  voices of the part share one LFO that runs all the time. `lfo1.fade` then
  brings its strength up from nothing.
- **LFO amounts** are how far the LFO moves the thing at the top of its
  swing. The pulse width amount is halved (the modulation envelope's too): 1
  swings the width by 0.5. The
  `amp` amount only ever turns the sound down: at 1 the level swings between
  nothing and full.
- **`fx.drive`** `d` above 0 puts the part through `tanh(g * x) / g^0.25`
  with `g = 1 + 30 * d * d`. The part's `volume` comes after the drive and
  the bit crusher, so it does not change their sound.
- **`master.limiter`** On, the output never passes 0.98. Off, the output is
  cut off flat at 1. The limiter adds no delay.
- **`voices`** Lowering it does not stop voices already sounding; it takes
  effect as new notes arrive.

### The table as JSON

`synths_param_table` gives the same table as data, so that a binding or an
editor can be made from it rather than typed in again:

```json
{
  "version": "0.0.0",
  "abi": 1,
  "limits": {"voices": 64, "parts": 64, "samples": 1024, "event_size": 32,
             "event_buffer": 256, "event_queue": 4096, "output_frames": 4096,
             "macro_steps": 64, "scope_master": 4096, "scope_part": 1024},
  "events": [
    {"type": 1, "name": "note_on", "help": "part; a: handle (0 for none); x: note; y: velocity 0..1"}
  ],
  "params": [
    {"id": 0, "name": "volume", "scope": "part", "kind": "float",
     "min": 0, "max": 2, "default": 1, "unit": "gain",
     "help": "How loud the part is (1 is unchanged)."},
    {"id": 16, "name": "osc1.wave", "scope": "part", "kind": "choice",
     "min": 0, "max": 4, "default": 1, "unit": "",
     "choices": ["sine", "triangle", "saw", "pulse", "sample"],
     "help": "The shape of the wave, or a loaded sample."}
  ]
}
```

`scope` is `part` or `engine`. `kind` is `float`, `int`, `bool` or `choice`.
`choices` is there only for a `choice`, and lists the names for the values 0,
1, 2 and so on. A name with a dot belongs to a group (`osc1`, `filter`,
`lfo2`): the part before the dot is the group. The text has no spaces or line
breaks; it is laid out here to be read.

## Samples

A sample is `f32` data in a slot, with the rate it was recorded at, a base
note and an optional loop. A part plays it by setting `osc1.wave` (or
`osc2.wave`) to 4 and `osc1.sample` to the slot.

- **Pitch.** Played at its base note, a sample comes out at its own speed.
  Each semitone above or below changes the speed by a twelfth of an octave.
  In frames of the sample per second, with `note` everything that makes up
  the pitch of the voice:

  `speed = sample_rate_of_the_sample * 2^((note - base_note) / 12)`

  So a host that thinks in playback rates (a tracker working from periods)
  loads the sample with the rate that should sound at the base note, and to
  play at `r` frames per second sends
  `note = base_note + 12 * log2(r / sample_rate_of_the_sample)`.
  The base note may be a fraction, which is how a sample is fine tuned.
- **Loops.** A forward loop plays up to `loop_end - 1` and continues from
  `loop_start`. A ping-pong loop plays up to `loop_end - 1`, then backwards
  down to `loop_start`, then forwards again; the frame at each end is played
  twice, once in each direction. A note begins at the start of the sample
  (or at `osc1.start`), not at the loop. The loop carries on through the
  release. If `loop_end` is past the end of the sample it is brought back to
  it, and a loop with no frames in it is no loop.
- **One-shots.** With no loop the sample plays to its end. If nothing else
  in the voice is making sound (the other oscillator, the sub oscillator and
  the noise are at level 0, or are samples that have ended too) the voice
  ends there.
- **Between frames** the engine draws a smooth curve through the four
  nearest frames (cubic interpolation), and gets this right across the ends
  of a loop. Played at its own speed and starting on a frame, a sample comes
  out exactly as it went in.
- **Played faster than it was recorded**, a sample's highest tones would
  land above what the engine's sample rate can hold and fold back down as
  harsh tones that were never in it. With `sample.antialias` on (it is, to
  begin with), the engine instead takes a weighted average of the frames
  either side of the position, as many as it is stepping over (at most 8
  each side), the nearest counting most. That takes out most of what would
  fold: 24 dB or more of it two octaves up. It is cheap rather than perfect:
  the top octave of what is left is dulled by a few dB, and above three
  octaves up (8 frames a step) the protection thins out. It comes in
  gradually over the first octave above the sample's own speed, and at or
  below that speed it does nothing at all. Turn it off, for a part, to hear
  samples the raw way old trackers played them.
- **Backwards.** With `osc1.reverse` on, a one-shot starts at its last frame
  and plays back to its first, then ends; `osc1.start` is then measured from
  the end. A forward loop starts at `loop_end - 1` and goes round the other
  way, and never plays what comes before the loop. A ping-pong loop already
  goes both ways, so there it only changes where the note starts: at the end
  of the loop, on the way back. Changing `osc1.reverse` while a one-shot or a
  forward loop sounds turns it round where it is.
- **Stereo.** A stereo sample plays in stereo as oscillator 1. As oscillator
  2 its two sides are mixed to one.
- **Bad data.** A number that is infinite or not a number is stored as 0,
  and others are kept within -8..8.
- **Replacing.** Loading into a slot that is in use, or freeing it, is safe
  while voices are playing it: they carry on with the new sample from the
  same position, or stop.
- **Positions** (`osc1.start`, `voice_position`) are in frames of the sample.
  In a ping-pong loop a position past `loop_end` is on the way back.

## Step macros

A step macro is a short table that belongs to a part and plays through, one
step at a time, on every note of that part. It is what gives the music of
8-bit sound chips its character: a fast arpeggio that makes one voice sound
like a chord, a drum made of a burst of noise and a falling tone, a pulse
width that keeps moving. The macro runs inside the engine, on the audio
thread; the host only supplies the table.

### A step

Each step is five `f32` numbers, in this order:

| # | name | values | what it does |
|---|---|---|---|
| 0 | wave | -1, or 0 to 4 | Oscillator 1's wave for this step: 0 sine, 1 triangle, 2 saw, 3 pulse, 4 noise. -1 leaves the patch's `osc1.wave`. |
| 1 | pitch | -96..96 | Semitones added to the pitch of the whole voice. 0 for none. |
| 2 | pulse width | -1, or 0..1 | The pulse width of both oscillators for this step, in place of the patch's. -1 (anything below 0) leaves the patch's. |
| 3 | cutoff | -120..120 | Semitones added to the filter cutoff. 0 for none. |
| 4 | volume | 0..1 | Multiplies the loudness of the voice. 1 for unchanged. |

So a step that changes nothing is `-1, 0, -1, 0, 1`. Numbers out of range
are brought in; a number that is not a number becomes the "changes nothing"
value of its column.

For a step of **noise** (wave 4), oscillator 1 falls silent and the voice's
noise sounds at `osc1.level` (or `noise.level` if that is higher), of the
kind the patch's `noise.type` says. With `noise.type` 2 (`lfsr`) the step's
pitch tunes the noise, as on a sound chip.

Everything else in the patch still applies on top of the macro: envelopes,
LFOs, bend, the filter envelope. The macro's pitch is added to them and its
volume multiplies them.

### How the steps go by

- A voice starts at step 0 on the frame its note starts.
- With `macro.sync` off (the default), steps go by `macro.rate` times a
  second: 50 unless changed, which is the frame rate chip music was written
  to. A step takes effect at the first control period at or after its time,
  so within 16 frames of it.
- With `macro.sync` on, the clock is ignored and each `macro_tick` event
  moves the macro on one step, on the exact frame of the event. This is how
  a tracker keeps macros in time with its own ticks, whatever its tempo.
- **Loop.** If `loop_end` is above `loop_start`, then while the note is
  held, the step after `loop_end - 1` is `loop_start`. With no loop
  (`loop_start` and `loop_end` both 0), the macro plays to its last step.
- **The end.** The last step holds for as long as the voice lasts.
- **Release.** `release` is the step to jump to, at once, when the note is
  let go (a `note_off`, or `all_notes_off` with `a` 0). From there the macro
  plays on to its last step and holds, with no looping. A `release` of 255,
  or any number not below the count of steps, means there is none: the macro
  takes no notice of the note being let go.
- **Flags.** Bit 0 (the value 1): slide. Pulse width, cutoff and volume move
  in a straight line from each step's value to the next's instead of
  jumping, so a slow sweep needs only a few steps. Wave and pitch always
  jump. Sliding does nothing when `macro.sync` is on. The other bits are not
  used yet; write 0.
- A note in mono mode that starts the envelopes again starts the macro
  again too; a legato note does not.
- Changes of pitch, width, cutoff and volume from one step to the next are
  slid over at most 16 frames, like every slow-moving value, so they do not
  click.

### Three classic patches

With `step(wave, pitch, width, cutoff, volume)`:

A **snare drum**: `osc1.wave` 3, `noise.type` 2, `amp.attack` 0, `amp.decay`
0.16, `amp.sustain` 0, and the macro, with no loop:

```
step(4, 24, -1, 0, 1.0)     noise, tuned high
step(3,  0, .5, 0, 1.0)     a square wave at the note
step(3, -5, .5, 0, 0.9)     and a fourth lower
step(4, 18, -1, 0, 0.8)     then noise, falling and fading
step(4, 14, -1, 0, 0.6)
step(4, 12, -1, 0, 0.45)
```

A **major chord from one voice**: any wave, and three steps looping
(`loop_start` 0, `loop_end` 3):

```
step(-1, 0, -1, 0, 1)
step(-1, 4, -1, 0, 1)
step(-1, 7, -1, 0, 1)
```

A **bass with a sweeping pulse width**: `osc1.wave` 3, `macro.rate` 4,
flags 1 (slide), eight steps looping:

```
step(3, 0, 0.06, 0, 1)   step(3, 0, 0.2, 0, 1)   step(3, 0, 0.5, 0, 1)   step(3, 0, 0.8, 0, 1)
step(3, 0, 0.94, 0, 1)   step(3, 0, 0.8, 0, 1)   step(3, 0, 0.5, 0, 1)   step(3, 0, 0.2, 0, 1)
```

`cargo run --release --example demo` renders all three
(`target/demo/8-macro-snare.wav`, `9-macro-arpeggio.wav`,
`10-macro-pulse-bass.wav`).

Hard sync (`osc2.sync`), ring modulation (`osc2.ring`), the pitched noise
(`noise.type` 2) and the combined filter outputs (`filter.type` 4 to 7) are
the other things a chip-style patch reaches for. They are ordinary
parameters.

## The arpeggio of a note

One event turns a note into a fast arpeggio chord, the sound of a C64 tune:
a single voice stepping through a few pitches 50 times a second. It belongs
to the note, not the part, so every note can have its own chord, and a
sounding note's chord can be changed.

**13 `arp`**

| field | meaning |
|---|---|
| `part` | the part; with a handle, 65535 for every part |
| `a` | the handle of the note, or 0 for the next note that starts on the part |
| `x` | steps a second, 0.1..1000. 0 means 50. Below 0: no clock, one step on each `macro_tick` event. |
| `y` | the order: 0 up, 1 down, 2 up then down, 3 as given, 4 random |
| `z` | how many offsets there are, 0..8. 0 takes the arpeggio away. |
| `b` | the first four offsets, one signed byte each, in semitones (-128..127): offset 0 in the lowest byte |

So in JavaScript, for offsets `o` (whole numbers):
`b = ((o[0] & 255) | (o[1] & 255) << 8 | (o[2] & 255) << 16 | (o[3] & 255) << 24) >>> 0`.
A major chord is offsets 0, 4, 7 with `z` 3: `b` is `0x00070400`.

**14 `arp_more`** carries offsets 4 to 7 in its `b`, packed the same way, for
an arpeggio of more than four. Send it straight after the `arp` event (which
gives the count), with the same time, part and `a`. Its `x`, `y` and `z` are
not used. Sent later to a sounding note, it changes those offsets without
starting the pattern again.

How it behaves:

- **With a handle**, the event addresses the sounding voices of the part
  with that handle. To give a note an arpeggio from its first frame, send
  the `note_on` and then the `arp`, with the same time. Sent later, it
  changes the arpeggio of the note as it sounds: the pattern starts again
  from its first step on the frame of the event, and nothing else about the
  note changes (its envelopes, its LFOs and its macro carry on). If no voice
  has the handle, nothing happens.
- **With handle 0**, the arpeggio waits on the part and is taken by the next
  `note_on` of that part, whatever handle that note has. It is used once.
  Send it before the `note_on`, with the same time. This is the form for
  notes that have no handle.
- **Steps.** The first step sounds at once. With a rate, step `k` begins on
  frame `start + floor(k * sample_rate / rate)`, counted from the frame the
  arpeggio was set: exactly, wherever that falls in a block. At 50 steps a
  second and 48 kHz that is every 960 frames. With `x` below 0, a step
  begins on the frame of each `macro_tick` event instead (whatever
  `macro.sync` says). At each step the pitch jumps; it does not slide.
- **Order.** Up, down and up-then-down play the offsets sorted by pitch,
  whatever order they were given in; up-then-down does not repeat the top or
  the bottom note (0, 4, 7, 12 plays 0 4 7 12 7 4 and round again). As given
  plays them in the order given, so an offset can appear twice. Random picks
  one of them at each step; the choice comes from the engine's seeded
  generator, so the same events always make the same tune.
- **What it adds to.** The offset is added to the pitch of the whole voice,
  on top of the part's macro, transpose, bend, LFOs and so on. Each unison
  voice of the note runs it.
- An arpeggio lasts as long as its voice. In mono and legato modes, where a
  new note moves the sounding voice, the voice keeps the arpeggio it has
  unless the new note brings another.

To play a C minor chord from one voice on part 0, now:

```js
event(now, 13 /* arp */,     0, 0, 0, 0, 3, 0x00070300);  // x: 50 a second, y: up, z: 3 offsets, b: 0, 3, 7
event(now, 1  /* note_on */, 0, 0, 60, 0.8);
```

## Scope, meters and notes

Three things a host can read at any time, to draw with. The engine keeps
them up to date as it renders; reading them changes nothing and costs the
audio nothing.

They are made so that a display on one thread can read while audio renders
on another, with no lock: the engine only ever stores single numbers and the
reader only ever loads them. A reading taken in the middle of a render can
be a few frames out of step with itself, which a drawing does not show.

### Scope

The engine keeps the last 4096 frames of the master output (what
`synths_render` wrote, after the limiter) and the last 1024 frames of each
part (after the part's drive, bit crusher and volume, before the sends and
the master volume). Each frame is one number: the mean of left and right.

`synths_scope_read` copies out the most recent frames, oldest first, so the
last number is the newest frame rendered. Silence is real silence: a part
that has stopped, or an engine that has gone to sleep, reads as zeros from
the frame it stopped. To draw a steady waveform, read more than you draw and
start at an upward crossing of zero.

### Levels

Four numbers for each part and four for the master output:

| # | what |
|---|---|
| 0 | peak, left |
| 1 | peak, right |
| 2 | RMS, left |
| 3 | RMS, right |

They are plain amplitudes (1 is full scale), measured at the same two places
as the scope. The peak jumps up to the loudest sample at once and falls back
to 37% in half a second. The RMS is the level averaged over about a tenth of
a second: a steady sine reads 0.707 of its peak, a square wave the same as
its peak. Both reach exactly 0 in the end. For decibels,
`20 * log10(level)`.

### Notes

Eight 32-bit words for each part, which are two sets of 128 bits, one bit a
note:

- words 0 to 3: the notes **held**: started and not yet let go;
- words 4 to 7: the notes **sounding**: held, or let go and still dying away.

Note `n` (0..127) is bit `n % 32` of word `n / 32` of its set. In JavaScript,
with `w` a `Uint32Array` over the eight words:
`held = (w[n >> 5] >>> (n & 31)) & 1`, and
`sounding = (w[4 + (n >> 5)] >>> (n & 31)) & 1`.

A voice's note is the note it is playing now, rounded to the nearest whole
note and kept within 0..127: the note it was started with, or the one a
`voice_pitch` event or a mono-mode note has since moved it to. Transpose,
bend and macros do not move it. The notes are brought up to date every 256
frames (about 5 milliseconds at 48 kHz), and at once by `synths_reset`.

## How notes get voices

A `note_on` normally takes a free voice. A part's `mode` and `unison.voices`
change that.

- **Unison.** With `unison.voices` above 1 each note takes that many voices,
  tuned across `unison.detune` cents either side of the note and panned
  across `unison.spread` either side of the part's pan, starting at random
  points of their waves, each at `1 / sqrt(n)` of full level. They share the
  note's handle and are let go together.
- **`mode` mono.** If the part has a voice sounding, a new note does not
  take another: the sounding voice moves to the new note (sliding over
  `glide` seconds) and its envelopes start again from where they are. It
  takes the new note's handle and velocity.
- **`mode` legato.** The same, but while the old note is still held the
  envelopes do not start again and the velocity does not change.
- In both, the part remembers the notes that are held down (the last 16).
  A `note_off` for the sounding note, while others are still held, goes back
  to the latest of them: the voice slides there over `glide` seconds and
  takes that note's handle and velocity, restarting its envelopes in mono
  mode and not in legato. A `note_off` for a held note that is not the one
  sounding just forgets it. When the last held note is let go, the voice is
  let go. `all_notes_off`, `synths_reset` and a `note_on` in poly mode
  forget the held notes.
- **`glide`** in poly mode slides each new voice from the last note played
  on the part.
- **Taking over.** When all the voices allowed (the `voices` parameter, at
  most 64) are in use, a new note takes over the oldest voice that has
  been let go or, if none has been let go, the oldest voice of all, on any
  part. (Oldest means started longest ago.) The voice taken is cut, without a click, on the
  same frame the new note starts.
- A voice ends, and is free again, when its amplitude envelope has finished
  its release, when its sustain level is 0 and its decay has finished, or
  when it has nothing left to play (see one-shots).

## The path of the sound

```
voice:   oscillator 1 ─┐
         oscillator 2 ─┼─ filter ─ amplifier (envelope, velocity, voice volume, tremolo) ─ pan ─┐
         sub, noise   ─┘                                                                        │
part:    all its voices ─ drive ─ bit crusher ─ volume ─┬───────────────────────────────────────┼─ mix
                                                         ├─ send.delay  ─ echo   ───────────────┤
                                                         ├─ send.reverb ─ reverb ───────────────┤
                                                         └─ send.chorus ─ chorus ───────────────┘
engine:  mix ─ master.volume ─ limiter ─ out
```

- Levels multiply straight through. A single voice at full velocity with
  every default, on a part panned to the middle, peaks at about
  `0.7071 * 0.35 = 0.25` on each side.
- The sends are taken after the part's volume. The effects add their sound
  to the mix; there is no dry/wet control, the send level is it.
- When no voice has sounded and no echo or reverb has rung for 2.5 seconds,
  the engine goes to sleep: it writes exact zeros and does almost no work,
  until the next event wakes it.

## Threads

The engine has no locks, and none of its functions may run at the same time
as another on the same engine. In practice:

- **In a browser** the module lives in the `AudioWorkletProcessor`, so every
  call is on the audio thread and the rule keeps itself. Messages from the
  page (events to push, samples to load) arrive between calls to `process`.
- **In a native app** call `synths_render` or `synths_render_into` and
  `synths_push_events` from the audio callback only. To send events from
  another thread, put the 32-byte records in a lock-free queue of your own
  and, at the top of the audio callback, copy them into the event buffer and
  push them.
- `synths_sample_load`, `synths_sample_config`, `synths_sample_free`,
  `synths_reset`, `synths_create` and `synths_destroy` allocate or free
  memory or take a while. They must not overlap a render either. Call them
  before audio starts, or while the audio callback is known not to be
  running; a native host that must load a sample while sound plays should
  stop calling render for that moment and put out silence.
- `synths_alloc` and `synths_free` touch only the memory allocator and are
  safe from any thread in a native build.
- `synths_scope_read`, `synths_levels_read` and `synths_notes_read` are the
  exception to the rule: they may be called from any one thread at any time,
  even during a render. (Not during `synths_destroy`.)
- `synths_macro_set` allocates nothing and is quick, but it changes the
  engine, so it must not overlap a render: in a native app call it from the
  audio callback, between renders, like `synths_push_events`.
- `synths_abi_version`, `synths_version`, `synths_param_table`,
  `synths_param_table_len`, `synths_output_capacity` and
  `synths_event_capacity` need no engine and are safe from any thread.

Inside `synths_render`, `synths_render_into` and `synths_push_events` the
engine does not allocate, lock, wait, make a system call or loop without
bound.

## WebAssembly

`scripts/build-wasm.sh` builds `target/wasm/synths_core.wasm` for the
`wasm32-wasip1` target, then loads it in node (`scripts/check-wasm.mjs`),
plays a note through it, reads the taps, sets a macro and times it. The
module is about 160 KB (62 KB gzipped).

**Exports**: `memory` and the functions above. There is no `_start` and no
`_initialize`; the module is ready as soon as it is instantiated.

**Imports**: four functions from `wasi_snapshot_preview1`. The engine never
calls them while it works: Rust's standard library refers to them for
printing a message if the program crashes. A host supplies stand-ins:

```js
let memory;
const wasi_snapshot_preview1 = {
  fd_write(fd, iovs, iovsLen, nwritten) {
    const view = new DataView(memory.buffer);
    let total = 0;
    for (let i = 0; i < iovsLen; i++) total += view.getUint32(iovs + i * 8 + 4, true);
    view.setUint32(nwritten, total, true);
    return 0;
  },
  environ_sizes_get(count, size) {
    const view = new DataView(memory.buffer);
    view.setUint32(count, 0, true);
    view.setUint32(size, 0, true);
    return 0;
  },
  environ_get() { return 0; },
  proc_exit(code) { throw new Error('synths-core stopped (' + code + ')'); },
};
const { exports } = new WebAssembly.Instance(module, { wasi_snapshot_preview1 });
memory = exports.memory;
```

In an `AudioWorkletGlobalScope` there is no `fetch`, so the page fetches the
bytes (or compiles a `WebAssembly.Module`) and posts them to the processor,
which instantiates the module synchronously as above.

**Memory growth.** `memory.buffer` is replaced by a new `ArrayBuffer`
whenever the module's memory grows, and typed arrays made over the old one
become empty. Memory can grow in `synths_create`, `synths_alloc`,
`synths_sample_load` and the first `synths_param_table`; it never grows in
`synths_render` or `synths_push_events`. So make the views over the output
and event buffers after creating the engine, and make them again after any
of those calls (or whenever `view.buffer !== memory.buffer`).

One module can hold several engines; each `synths_create` is separate.

## A host loop

An audio worklet, in outline:

```js
class SynthsProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const module = options.processorOptions.module;       // a WebAssembly.Module
    this.x = new WebAssembly.Instance(module, { wasi_snapshot_preview1 }).exports;
    memory = this.x.memory;
    this.engine = this.x.synths_create(sampleRate);
    this.pending = [];                                     // event records waiting to go in
    this.port.onmessage = (m) => this.onMessage(m.data);
  }

  onMessage(msg) {
    const x = this.x;
    if (msg.events) {
      // msg.events: an ArrayBuffer of 32-byte records, built on the page.
      this.pending.push(new Uint8Array(msg.events));
    } else if (msg.sample) {
      const data = msg.sample.data;                        // Float32Array
      const ptr = x.synths_alloc(data.length * 4);
      new Float32Array(x.memory.buffer, ptr, data.length).set(data);
      x.synths_sample_load(this.engine, msg.sample.slot, ptr,
                           data.length / msg.sample.channels, msg.sample.channels, msg.sample.rate);
      x.synths_free(ptr);
      x.synths_sample_config(this.engine, msg.sample.slot, msg.sample.baseNote,
                             msg.sample.loopMode, msg.sample.loopStart, msg.sample.loopEnd);
    }
  }

  process(inputs, outputs) {
    const x = this.x, engine = this.engine;
    // 1. Hand over waiting events, 256 at a time, while the queue has room.
    while (this.pending.length) {
      const bytes = this.pending[0];
      const count = Math.min(bytes.length / 32, x.synths_event_capacity(), x.synths_event_space(engine));
      if (count === 0) break;
      new Uint8Array(x.memory.buffer, x.synths_event_buffer(engine), count * 32).set(bytes.subarray(0, count * 32));
      x.synths_push_events(engine, count);
      if (count * 32 === bytes.length) this.pending.shift();
      else this.pending[0] = bytes.subarray(count * 32);
    }
    // 2. Render one block and copy it out.
    const out = outputs[0];
    const n = x.synths_render(engine, out[0].length);
    out[0].set(new Float32Array(x.memory.buffer, x.synths_output_left(engine), n));
    if (out[1]) out[1].set(new Float32Array(x.memory.buffer, x.synths_output_right(engine), n));
    return true;
  }
}
```

(A real processor keeps its typed arrays instead of making new ones every
block, and remakes them when `memory.buffer` changes. It also tells the page
the engine's time now and then, so the page can stamp events:
`time = engineTime + (audioContext.currentTime - contextTimeThen) * sampleRate`.)

The same in C, for a native audio callback:

```c
static SynthsEngine *engine;   /* synths_create(48000) before audio starts */

void audio_callback(float *left, float *right, uint32_t frames) {
    /* 1. Move events from the app's own lock-free queue into the engine. */
    uint8_t *buffer = synths_event_buffer(engine);
    uint32_t count = 0;
    while (count < synths_event_capacity() && count < synths_event_space(engine)
           && app_queue_pop(buffer + count * 32)) {
        count++;
    }
    synths_push_events(engine, count);
    /* 2. Render straight into the buffers the system gave us. */
    synths_render_into(engine, left, right, frames);
}
```

To play middle C for half a second, starting now, on part 0:

```c
double now = synths_time(engine);
write_event(buffer, 0, now,           1 /* note_on  */, 0, 0, 60.0f, 0.8f);
write_event(buffer, 1, now + 24000.0, 2 /* note_off */, 0, 0, 60.0f, 0.0f);
synths_push_events(engine, 2);
```

## Using the engine from Rust

Code inside this crate, or linking it as a Rust library, can skip the C
functions and use `synths_core::Engine` directly. It is the same engine:

```rust
use synths_core::{Engine, Event};
use synths_core::params::*;

let mut engine = Engine::new(48000.0);
engine.set_param(0, OSC1_WAVE, 2.0);                 // at once
engine.push(Event::note_on(0.0, 0, 0, 60.0, 0.8));   // through the queue, at a time
engine.push(Event::note_off(24000.0, 0, 0, 60.0));
let (mut left, mut right) = ([0.0f32; 128], [0.0f32; 128]);
engine.render(&mut left, &mut right);
```

- `Engine::push(event) -> bool` queues an event; `Engine::apply(&event)`
  carries one out at once, at the engine's current time.
- `Engine::load_sample`, `configure_sample`, `free_sample`, `set_macro`,
  `param`, `set_param`, `time`, `active_voices`, `reset` are the functions
  above without the pointers, and `Engine::taps()` gives `read_scope`,
  `read_levels` and `read_notes`.
- `synths_core::params` has a constant for every parameter id (`CUTOFF`,
  `OSC1_WAVE`, ...) and the table itself (`PARAMS`).
- A sequencer that lives inside the core (a tracker song player) drives the
  engine the same way a host does. Before each block it works out which of
  its ticks fall inside the block and pushes their events with the exact
  frame of each tick; `render` then splits the block at those frames.
