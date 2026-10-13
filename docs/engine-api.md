# The engine's interface

This is the contract between synths-core and anything that hosts it: the audio
worklet of synths.js, the Python binding, a native app. It describes interface
version **1** (`synths_abi_version()`).

The engine is one object. A host creates it for a sample rate, sends it
**events** (start a note, set a parameter, move a live voice) stamped with the
time they should happen, and asks it to **render** blocks of stereo sound. The
host never works out samples itself. It can also hand the engine sound as it
renders (a microphone), to be changed, mixed in and recorded. For drawing, it can read back the latest
sound, the levels and the notes that are playing.

- [Building](#building)
- [Words and units](#words-and-units)
- [Functions](#functions)
- [Events](#events)
- [Parameters](#parameters)
- [Samples](#samples)
- [Step macros](#step-macros)
- [The arpeggio of a note](#the-arpeggio-of-a-note)
- [Songs](#songs)
- [The live input](#the-live-input)
- [Voice effects](#voice-effects)
- [Recording](#recording)
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

The functions are the same in all three, but for two cargo features, both on
unless turned off. `songs` is the tracker song player and every
`synths_song_` function; `scripts/build-wasm.sh --no-songs` builds a module
without it, `target/wasm/synths_core.synth-only.wasm`, for a page that only
wants the synth. `json` (which needs `songs`) is the reader of songs as JSON;
`scripts/build-wasm.sh --no-json` builds a module without it,
`target/wasm/synths_core.no-json.wasm`. See
[Which kinds of song a build reads](#which-kinds-of-song-a-build-reads).

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

float         *synths_input_left(SynthsEngine *engine);
float         *synths_input_right(SynthsEngine *engine);
uint32_t       synths_render_io(SynthsEngine *engine, uint32_t frames, uint32_t channels);
void           synths_render_io_into(SynthsEngine *engine, const float *in_left, const float *in_right,
                                     float *left, float *right, uint32_t frames);
int32_t        synths_record_buffer(SynthsEngine *engine, uint32_t slot, uint32_t frames, uint32_t channels);
uint32_t       synths_record_read(const SynthsEngine *engine, uint32_t *out);
uint32_t       synths_sample_frames(const SynthsEngine *engine, uint32_t slot);
uint32_t       synths_sample_read(const SynthsEngine *engine, uint32_t slot, uint32_t start,
                                  float *out, uint32_t frames);

int32_t        synths_song_file(const uint8_t *name, uint32_t name_len, const uint8_t *data, uint32_t len);
int32_t        synths_song_load(SynthsEngine *engine, const uint8_t *data, uint32_t len,
                                uint32_t first_slot);
const uint8_t *synths_song_error(void);
int32_t        synths_song_pack(const uint8_t *data, uint32_t len);
const uint8_t *synths_song_packed(void);
void           synths_song_unload(SynthsEngine *engine);
uint32_t       synths_song_info(const SynthsEngine *engine, uint32_t *out);
uint32_t       synths_song_position(const SynthsEngine *engine, uint32_t *out);
uint32_t       synths_song_patch_name(const SynthsEngine *engine, uint32_t instrument,
                                      uint8_t *out, uint32_t capacity);
uint32_t       synths_song_synth(const SynthsEngine *engine, uint32_t instrument,
                                 uint8_t *out, uint32_t capacity);
int32_t        synths_song_replace(SynthsEngine *engine, const uint8_t *data, uint32_t len,
                                   uint32_t first_slot);
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
apart from samples: about 2.6 MB at 48 kHz, and more at higher rates (1 MB
of it is the echo's two seconds of memory and 0.4 MB the reverb's). A new
engine holds every
parameter's default, no samples and no events, and its time is 0.

**`synths_destroy(engine)`** Free an engine and its samples. Do not use the
pointer afterwards.

**`synths_reset(engine)`** The panic button: stop every voice at once, forget
every queued event and every running glide, empty the echo, reverb, chorus
and voice effects, and stop a recording (what it has recorded stays). Parameters, samples and the time stay as they are.

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
both first carry out every queued event whose time has come. Each has a twin
that also takes sound in: see
[The live input and recording](#the-live-input-and-recording).

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

### The live input and recording

A host with a microphone renders with one of these two in place of
`synths_render` and `synths_render_into`. What becomes of the sound is under
[The live input](#the-live-input) and [Recording](#recording).

**`synths_input_left(engine) -> *mut f32`** and
**`synths_input_right(engine) -> *mut f32`** Two buffers inside the engine,
of `synths_output_capacity()` (4096) frames each, for the host to put the
input in before `synths_render_io`. The pointers do not change for the life
of the engine.

**`synths_render_io(engine, frames: u32, channels: u32) -> u32`**
`synths_render`, with a live input: the first `frames` numbers of the input
buffers are the sound that came in while the frames about to be rendered
were due. `channels` is 1 for a mono input (the left buffer only), 2 for
stereo, and 0 for none, which makes it exactly `synths_render`. Returns how
many frames were rendered (at most 4096); read them from
`synths_output_left` and `synths_output_right` as usual.

**`synths_render_io_into(engine, in_left: *const f32, in_right: *const f32, left: *mut f32, right: *mut f32, frames: u32)`**
`synths_render_into`, with a live input in the host's own buffers.
`in_right` null is a mono input; `in_left` null is no input, and exactly
`synths_render_into`. The input buffers hold `frames` numbers each and must
not overlap the output buffers.

The input must be at the engine's sample rate: the engine does not convert
it. A host may change from one of the four ways of rendering to another
from block to block; a block rendered with no input counts as silence
coming in.

**`synths_record_buffer(engine, slot, frames, channels) -> i32`** Make a
sample slot to record into: memory for `frames` frames of 1 or 2 channels,
at the engine's sample rate, with a base note of 60 and nothing in it yet.
It replaces any sample in the slot. This is where a recording's memory
comes from, so `frames` is the longest the recording can be: 30 seconds at
48 kHz is 1,440,000 frames and 5.8 MB (twice that in stereo). Returns 0, or
-1 (no such slot), -2 (not 1 or 2 channels, no frames, or no memory to be
had) or -4 (a null engine). It allocates, so it is for before the
recording, not during a render.

**`synths_record_read(engine, out: *mut u32) -> u32`** Write the state of
the recorder into `out`, four numbers, and return 4:

| | |
|---|---|
| 0 | 1 while recording, 0 when not |
| 1 | the slot of the recording, or of the last one |
| 2 | how many frames have been recorded |
| 3 | how many frames the slot has room for |

Like the taps, it only reads, and may be called while another thread
renders.

**`synths_sample_frames(engine, slot) -> u32`** How many frames the sample
in a slot has: for a recording, how many were recorded. 0 for an empty
slot.

**`synths_sample_read(engine, slot, start, out: *mut f32, frames) -> u32`**
Copy `frames` frames of the sample in a slot into `out`, from frame `start`
on: one number a frame for a mono sample, two (left, right) for a stereo
one, so `out` needs room for `frames` or `2 * frames` numbers. Returns how
many frames were copied, which is fewer at the end of the sample. This is
how a host gets a recording out to save it. It works for any sample, and
must not overlap a render.

### Songs

These load a tracker song and read where it is; playing it is done with the
`song` event. See [Songs](#songs) for the whole of it.

**`synths_song_load(engine, data: *const u8, len, first_slot) -> i32`**
Load a song from the bytes of a file: a MOD, an XM, or one of our own songs
(`docs/song-format.md`), packed or, where the build reads it, as JSON. The
engine works out which. The song's
channels take parts 32 to 63, one each, and its samples the sample slots
from `first_slot` on; a song already loaded is unloaded first.
The song is then ready and stopped. Returns one of:

| result | meaning |
|---|---|
| `SONG_OK 0` | Loaded. |
| `SONG_NULL -1` | A null pointer. |
| `SONG_BAD -2` | Not a song the engine knows, or too damaged to load. |
| `SONG_MISSING_FILE -3` | A JSON song names a sample file that was not given with `synths_song_file`. |
| `SONG_NO_PARTS -4` | The song has more than 32 channels. |
| `SONG_NO_SLOTS -5` | The song has more samples than there are slots from `first_slot` on. |

It copies what it needs: `data` can be freed afterwards. It allocates, so it
is for between renders.

**`synths_song_error() -> *const u8`** Why the last `synths_song_load` failed,
in plain words, as UTF-8 text ending in a zero byte: for a JSON song it names
the key that is wrong. Empty after a load that worked. The pointer is good
until the next `synths_song_load`.

**`synths_song_file(name: *const u8, name_len, data: *const u8, len) -> i32`**
Give a sample file for the next song loaded. `name` is the path the song's
JSON gives for it (`name_len` bytes of UTF-8, no zero byte) and `data` the
WAV file. Call it once for each file, then `synths_song_load`, which uses the
files and forgets them. Returns 0, or -1 for a null pointer or a name that is
not UTF-8. MOD and XM files carry their samples and need none.

**`synths_song_pack(data: *const u8, len) -> i32`** Turn a song file of any
kind this build reads into the packed form of our own format, which every
build reads: a MOD or an XM for an editor to open, say. Returns the packed
song's length in bytes, or a negative number as `synths_song_load` does
(with `synths_song_error` saying why). It takes no engine and changes none.
Sample files given with `synths_song_file` are used and forgotten.

**`synths_song_packed() -> *const u8`** The bytes of the song
`synths_song_pack` last packed. The pointer is good until the next
`synths_song_pack`.

**`synths_song_unload(engine)`** Take the song out: its voices are cut (those
of its synth patches too) and
its sample slots emptied. The parts keep the settings the song gave them.

**`synths_song_info(engine, out: *mut u32) -> u32`** Write eight facts about
the loaded song into `out`: channels, orders, patterns, instruments, samples,
first part, first slot, and its rules (0 FastTracker 2, 1 ProTracker).
Returns 8, or 0 (writing nothing) if no song is loaded.

**`synths_song_position(engine, out: *mut u32) -> u32`** Write where the song
is into `out`, ten numbers, and return 10. Like the three functions above it
only reads, and may be called while another thread renders. See
[Where the song is](#where-the-song-is).

**`synths_song_patch_name(engine, instrument, out: *mut u8, capacity) -> u32`**
Copy the name of the synth patch an instrument plays into `out`, at most
`capacity` bytes of UTF-8 with no zero byte. Instruments count from 1.
Returns the name's whole length: 0 for an instrument that plays samples, or
when no song is loaded. See [Instruments that are synth patches](#instruments-that-are-synth-patches).

**`synths_song_synth(engine, instrument, out: *mut u8, capacity) -> u32`**
Copy what the song's JSON held under an instrument's `synth` key into `out`,
as JSON text with no spaces: at most `capacity` bytes of UTF-8 with no zero
byte. Returns the text's whole length, so a host can ask with a capacity of
0 and come back with a buffer that fits; 0 for an instrument with none. The
engine keeps this text for the host and does nothing with it. It comes
through the packed form of a song as well.

**`synths_song_replace(engine, data: *const u8, len, first_slot) -> i32`**
Put a song in the place of the one that is loaded, for an editor whose song
changes while it plays. It takes what `synths_song_load` takes and returns
what it returns; if it fails, the loaded song is left as it was. See
[Changing the song while it plays](#changing-the-song-while-it-plays). It
allocates, so it is for between renders.

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
| 15 | `song` | | what to do, 0 to 9 | its first number | its second number |
| 16 | `record` | the part to record, when `y` is 1 | 0: stop, 1: start | sample slot | what to record: 0, 1 or 2 |

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

**15 `song`** Steer the loaded song: play, pause, stop, seek and so on,
chosen by `a`. `part` is ignored. With no song loaded it does nothing. See
[Steering the song](#steering-the-song).

**16 `record`** Start or stop recording into a sample slot, on this exact
frame. `x` is a slot made with `synths_record_buffer`, and `y` says what is
recorded: 0 the live input, 1 the part `part`, 2 the whole output. See
[Recording](#recording).

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
| 12 | `pan.law` | choice: 0 equal_power, 1 square_root | 0 | 1 | 0 |  | How pan shares a sound between left and right. square_root is FastTracker 2's, for songs written with it. |
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
| 106 | `fx.pitch` | float | -12 | 12 | 0 | semitones | Pitch shifter: moves the part's whole sound up or down without changing its speed (0 is off). Up is a chipmunk, down is a monster. |
| 107 | `fx.ring` | float | 0 | 1 | 0 |  | Ring modulator: how much of the sound is multiplied by a steady wave (0 is off). The robot voice. |
| 108 | `fx.ring_freq` | float | 1 | 5000 | 30 | hz | Ring modulator: how fast the wave goes. Slow is a wobble, 30 to 80 is a robot, hundreds is a bell-like alien. |
| 109 | `fx.ring_wave` | choice: 0 sine, 1 noise | 0 | 1 | 0 |  | Ring modulator: sine is the robot. noise multiplies by a rumble as wide as fx.ring_freq, which blurs the pitch of a voice into a hoarse whisper. |
| 110 | `fx.comb` | float | 0 | 1 | 0 |  | Comb: how much of the sound is the ringing, metallic copy (0 is off). |
| 111 | `fx.comb_freq` | float | 50 | 5000 | 200 | hz | Comb: the note it rings at. Low is a robot in a pipe; high, and swept, is a flanger. |
| 112 | `fx.drive` | float | 0 | 1 | 0 |  | Distortion (0 is off). |
| 113 | `fx.bits` | float | 1 | 16 | 16 | bits | Bit crusher: how many bits each sample keeps (16 is off). |
| 114 | `fx.downsample` | int | 1 | 64 | 1 |  | Bit crusher: keep only one sample in this many (1 is off). |
| 115 | `send.delay` | float | 0 | 1 | 0 | gain | How much of the part goes to the echo. |
| 116 | `send.reverb` | float | 0 | 1 | 0 | gain | How much of the part goes to the reverb. |
| 117 | `send.chorus` | float | 0 | 1 | 0 | gain | How much of the part goes to the chorus. |
| 120 | `macro.rate` | float | 1 | 1000 | 50 | hz | How many steps of the part's macro play each second. |
| 121 | `macro.sync` | bool | 0 | 1 | 0 |  | On: the macro steps on each macro_tick event (a tracker's tick) instead of by the clock. |
| 122 | `fx.comb_feedback` | float | -0.97 | 0.97 | 0.7 |  | Comb: how long it rings. Below 0 it rings an octave lower and hollow. |
| 123 | `fx.comb_rate` | float | 0.01 | 10 | 0.3 | hz | Comb: how fast its note sweeps up and down. |
| 124 | `fx.comb_depth` | float | 0 | 1 | 0 |  | Comb: how far its note sweeps (0: it stays put). |
| 200 | `master.volume` | float | 0 | 2 | 0.35 | gain | How loud everything is. |
| 201 | `master.limiter` | bool | 0 | 1 | 1 |  | Turns loud passages down so they do not distort. The output never goes past -1..1 either way. |
| 202 | `delay.time` | float | 0.001 | 2 | 0.3 | seconds | The time between echoes. |
| 203 | `delay.feedback` | float | 0 | 0.95 | 0.4 |  | How much of each echo is fed back to make the next. |
| 204 | `delay.tone` | float | 200 | 20000 | 6000 | hz | Each echo is duller than the last; this is how much treble survives. |
| 205 | `delay.pingpong` | bool | 0 | 1 | 0 |  | On: the echoes bounce between left and right. |
| 208 | `reverb.size` | float | 0 | 1 | 0.5 |  | How big the room is: a bigger one has its echoes further apart and, unless reverb.decay says otherwise, a longer tail (three times shorter at 0, three times longer at 1). |
| 209 | `reverb.damping` | float | 0 | 1 | 0.4 |  | How quickly the treble of the reverb dies away (0: as slowly as the rest, a bright tail; 1: at once, a dull one). |
| 210 | `reverb.type` | choice: 0 room, 1 hall, 2 plate, 3 cathedral, 4 spring | 0 | 4 | 0 |  | What kind of space it is. room: a small room, a short soft tail. hall: a concert hall, big and smooth. plate: a sheet of metal, bright and thick at once. cathedral: enormous, and it goes on and on. spring: the twangy spring of a guitar amp, which boings. |
| 211 | `reverb.decay` | float | 0 | 30 | 0 | seconds | How long the tail takes to die away (to fall by 60 dB). 0 leaves it to the type and reverb.size. |
| 212 | `chorus.rate` | float | 0.05 | 10 | 0.8 | hz | How fast the chorus wobbles. |
| 213 | `chorus.depth` | float | 0 | 1 | 0.5 |  | How far the chorus wobbles. |
| 216 | `voices` | int | 1 | 64 | 64 | voices | The most voices that sound at once; a new note past this takes over the oldest. |
| 217 | `input.part` | int | 0 | 63 | 31 | part | The part the live input plays through: it gets that part's filter, pan, effects, volume and sends. |
| 218 | `input.gain` | float | 0 | 8 | 1 | gain | How much the live input is turned up before anything else (1 is unchanged). |
| 219 | `input.monitor` | bool | 0 | 1 | 1 |  | On: the live input is heard. Off: it can still be recorded, in silence. |
| 220 | `input.gate` | float | -100 | 0 | -100 | db | Noise gate: the input is let through only while it is louder than this, which keeps a microphone's hiss out (-100 is off; try -45). |
| 221 | `input.gate_release` | float | 0.005 | 2 | 0.15 | seconds | Noise gate: how long it takes to close once the input has gone quiet. |
| 222 | `reverb.predelay` | float | 0 | 0.2 | 0 | seconds | A gap before the reverb starts, on top of the type's own: it keeps a sound clear of its own reverb. |
| 223 | `reverb.low_cut` | float | 20 | 1000 | 120 | hz | Bass below this is kept out of the reverb, so that it does not turn to mud. |

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
  the far side down. With `pan.law` 1 a mono voice is placed as FastTracker 2
  placed it: with `r = (pan + 1) / 2`, the left is the square root of `1 - r`
  and the right the square root of `r`. The two laws agree in the middle and
  at the edges and differ a little between.
- **`velocity`** With `s` this parameter and `v` the note's velocity, the
  note's level is `1 - s + s * v`: a straight line, not a curve.
- **`mode`, `glide`, `unison.*`** See [How notes get voices](#how-notes-get-voices).
- **`reverb.*`** See [The reverb](#the-reverb).
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
- **`fx.pitch`, `fx.ring`, `fx.comb`** and the parameters that go with them
  are under [Voice effects](#voice-effects).
- **`fx.drive`** `d` above 0 puts the part through `tanh(g * x) / g^0.25`
  with `g = 1 + 30 * d * d`. The part's `volume` comes after the drive and
  the bit crusher, so it does not change their sound.
- **`input.*`** are under [The live input](#the-live-input).
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
  ],
  "voice_presets": [
    {"name": "chipmunk", "help": "Small and squeaky: everything seven semitones higher.",
     "values": {"fx.pitch": 7}}
  ]
}
```

`scope` is `part` or `engine`. `kind` is `float`, `int`, `bool` or `choice`.
`choices` is there only for a `choice`, and lists the names for the values 0,
1, 2 and so on. A name with a dot belongs to a group (`osc1`, `filter`,
`lfo2`): the part before the dot is the group. `voice_presets` are the
[voice changers](#voice-changers): for each, parameter names and the values
to give them. The text has no spaces or line breaks; it is laid out here to
be read.

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

## Songs

The engine can play a whole tracker song by itself. A song is loaded with
`synths_song_load` from the bytes of a MOD file, an XM file, or one of our own
JSON songs. From then on the song's player lives inside the engine: it is
called from within the render on the exact frame of each of the song's ticks,
so the song keeps perfect time whatever the block size, and the host has
nothing to schedule.

MOD files are played by ProTracker's rules and XM files by FastTracker 2's,
quirks included. Our own songs say which they want.

### Which kinds of song a build reads

| kind | read by |
|---|---|
| MOD, XM | every build with the `songs` feature |
| our own format, packed | every build with the `songs` feature |
| our own format, as JSON | builds with the `json` feature, which is on unless a build turns it off |

The JSON reader is about 35 KB of a WebAssembly module. `scripts/build-wasm.sh`
leaves it in, so that one reader decides what a song's JSON means for a page,
for Python and for a native app alike; a module built with
`scripts/build-wasm.sh --no-json` is that much smaller and reads MOD, XM and
packed songs only. `synths_song_pack` turns a MOD or an XM into the packed form for
a page to unpack into its own JSON (`docs/song-format.md` gives both, field
for field), which is how an editor opens a tracker module.

A MOD or an XM loaded as it is has `sample.antialias` turned off on its
parts, for the sound those trackers had. One of our own songs, packed or
JSON, has it on, whatever it was made from; a host that wants the raw sound
for a song it packed from a module sets parameter 39 on parts 32 up to 0
after loading.

### What a song takes over

- **Parts.** Each channel of the song has a part to itself: channel 0 is
  part 32, channel 1 is part 33, and so on up to part 63. Parts 0 to 31 are
  the host's own: no song ever changes them or their voices, whether it is
  loaded, played, sought, stopped or unloaded, so a game can play its sound
  effects over a song. Loading sets those parts up to
  play samples as they are: `volume` 0.5, `pan` 0, the amp envelope full on
  at once and gone at once (`amp.attack`, `amp.decay` and `amp.release` 0,
  `amp.sustain` 1), `velocity` 0, `filter.type` off, `pan.law` to suit the
  song's rules, and `sample.antialias` as said above. Everything else about the parts is
  left alone, so their effect sends, drive and so on are the host's to set,
  before or after loading. The channel's one voice has the handle
  `channel + 1`.
- **Sample slots.** The song's samples go into the slots from `first_slot`
  on, in order, replacing what was there. A sample of the song that is empty
  leaves its slot empty.
- **The sequencer.** There is one song at a time: loading another unloads the
  first.

The host can go on playing notes of its own on parts 0 to 31 while a song
plays. It should leave the song's parts and slots alone, and not use handles
1 to the number of channels on the parts it has given to synth patches (see
below).

A MOD has 31 samples and 4 to 32 channels; an XM can have more channels
than 32 (few do), and several samples for each of up to 128 instruments.
`SONG_NO_PARTS` says a song has too many channels to play, and
`SONG_NO_SLOTS` that its samples do not fit from `first_slot` on.

### Steering the song

A `song` event (type 15) tells the song what to do, on the event's exact
frame. `a` says what; `x` and `y` are its numbers.

| `a` | what | `x` | `y` | |
|---|---|---|---|---|
| 0 | stop | | | Cut the song's voices, on its own parts and on the parts of synth patches, and go back to the start. Like every cut, it does not click. |
| 1 | play | | | Start, or carry on after a pause. After a stop, or at the end of a song that does not repeat, it starts from the top. The first tick is played on this frame. |
| 2 | pause | | | Hold every sample where it is, in silence, until `play`. |
| 3 | seek | order | row | Go to a row of a place in the order. The song's voices are cut. If the song is playing it carries on from there on this frame; if not, that is where `play` will start. Numbers past the end are brought back to the last order and row. |
| 4 | repeat | 0 off, 1 on | | On (as it is to begin with), the song goes round and round as trackers play it. Off, it stops at its end and lets the last notes die away: a sample with an end plays to it, a sample that loops is faded out over a quarter of a second, and a synth patch's note is let go, to finish as its patch's release says. |
| 5 | volume | 0 to 2 | | How loud the song is; 1 to begin with. It sets the `volume` of the song's own parts (32 and up), and no others, to half of this, sliding there over 20 milliseconds. A synth patch plays on a part of the host's, which the song does not touch: there the song's notes carry its volume themselves (the voice volume of each is multiplied by it), so the whole song is scaled, samples and patches alike. |
| 6 | patch part | instrument, from 1 | part | Say which part plays an instrument that is a synth patch. |
| 7 | mute | channel, from 0; below 0 for every channel | 0 off, 1 on | Silence a channel of the song, or let it be heard again. The song goes on playing it, so it comes back in the right place. |
| 8 | solo | channel, from 0; below 0 for every channel | 0 off, 1 on | While any channel is soloed, only the soloed channels are heard (and of those, only the ones not muted). |
| 9 | loop this pattern | 0 off, 1 on | | On, the song stays in the place of the order it is in and plays that pattern round and round; a jump or the pattern's end brings it back to the pattern's top (a pattern break to its row). That is not the song ending, whatever `repeat` says. Off, it carries on through the song from where it is. A `seek` while it is on moves the loop to the place sought. |

In JavaScript, with the `event` function of [A host loop](#a-host-loop):

```js
event(0, 15 /* song */, 0, 1 /* play */, 0, 0);
event(0, 15 /* song */, 0, 3 /* seek */, 4, 16);   // order 4, row 16
```

A song's end is where its order list runs out, or where it comes back to a
row it has already played (which is how a song that loops by jumping back is
caught). A song can also be written never to end.

`synths_reset` stops the song and takes it back to the start; it stays
loaded.

### Where the song is

`synths_song_position` writes ten numbers:

| | |
|---|---|
| 0 | State: 0 no song is loaded, 1 stopped, 2 playing, 3 paused. |
| 1 | The place in the order list, from 0. |
| 2 | The pattern playing there. |
| 3 | The row, from 0. |
| 4 | The tick within the row, from 0. |
| 5 | Ticks in a row (the song's speed). |
| 6 | Beats a minute (its tempo): a tick lasts 2.5 / tempo seconds. |
| 7 | How many times the song has come round to its start. |
| 8 | How many rows this pattern has. |
| 9 | How many places the order list has. |

The audio side stores these one number at a time, once a tick, and this
function only loads them, so a display can call it on every frame it draws,
from any thread. A reading taken in the middle of a tick can have the new
row with the old tick; it is for drawing a cursor, not for arithmetic.

A muted channel changes over 20 milliseconds, without a click. A channel
of samples is muted through its part's `volume`, as the song's volume is
set; a synth patch's notes, on a part of the host's, are muted through
their own volume, and nothing of the host's part is changed. Mutes, solos
and the pattern loop last until they are changed or another song is
loaded; `synths_reset` and `stop` leave them.

### Changing the song while it plays

An editor that plays a song through the engine hands over the whole song
again each time it is edited, with `synths_song_replace`. What happens
depends on how alike the two are.

- **Nothing stops** if the new song has the same number of channels and the
  same rules as the loaded one, no fewer instruments and no fewer samples,
  and is given the same `first_slot`. The notes that are sounding carry on,
  the player keeps its place, its tick, its speed and its tempo, and from
  the next row it plays it reads the new song. Only the samples that have
  changed are loaded again (a note playing one carries on in the new
  sound). Played this way, the result is exactly what the new song would
  have sounded like from the top, from the first row that differs. If the
  place it had reached is not in the new song (a shorter order list or
  pattern), it carries on from the top of the nearest place that is.
- **Otherwise** the new song is loaded as `synths_song_load` loads it, the
  sounding notes are cut, and what the host had set is put back: the song's
  volume, mutes, solos, repeat, the pattern loop, the parts of its synth
  patches, and its place, from the top of the row it was on; and it plays
  on if it was playing. (A paused song comes back stopped at its place.)

Either way `synths_song_position` goes on giving the order, pattern, row
and tick, and `synths_song_info` and the other readers describe the new
song. With no song loaded it is `synths_song_load`.

### Instruments that are synth patches

In one of our own songs an instrument can be a synth patch in place of
samples: the song gives the patch a name and nothing more. The host owns the
patch. After loading, it asks for each instrument's patch name with
`synths_song_patch_name`, sets a part up as that patch with `param` events
like any other, and says which part it is with a `song` event, `a` 6. A note
of that instrument is then a note on that part (as `note_on` starts one),
with the channel's handle, steered by the song: its pitch slides, its volume
and pan change, and when the song lets the key go the patch's own release
runs. An instrument whose patch has not been given a part is silent.

A song can also carry the host's own description of the sound, under the
instrument's `synth` key (`docs/song-format.md`): a preset's name and
settings, or anything else. The engine does not read it. It keeps it with
the song, through loading, saving and packing, and `synths_song_synth`
hands it back as JSON text, so a host need not find it in the song's JSON
itself.

### How it differs from the trackers

The notes, timing and effects are the trackers' own, tested against the
OpenMPT player tests. The sound of the voices is this engine's:

- Volume and pan changes are slid as FastTracker 2 slid them: over a whole
  tick, or over 5 milliseconds for a sudden one (a new volume, a cut). A
  ProTracker song's are at once, as on the Amiga, which here means within a
  control period.
- A FastTracker 2 song's parts are given that tracker's pan law (`pan.law`
  1); a ProTracker song's channels are hard left and right.
- The Amiga's "LED" filter (effect `E0x`) is stood in for by each channel
  part's own filter, as a low-pass at 3300 Hz.
- ProTracker's invert loop (`EFx`) is not played.

## The live input

A host can hand the engine sound as it renders: a microphone, a line in.
It renders with `synths_render_io` or `synths_render_io_into`, giving each
block the frames that came in, mono or stereo, and the engine does the
rest.

The input plays through one part, the one `input.part` names (31 to begin
with, the last of the parts a song never takes). It enters that part as if
it were the part's one voice, so everything a part does to its voices it
does to the input:

- the part's **filter** is the input's tone control: `filter.type`,
  `filter.slope`, `filter.cutoff` and `filter.resonance`, with the cutoff
  as it is set (there is no note, envelope or LFO to move it). A band-pass
  is a telephone;
- the part's **`pan`** (and `pan.law`): a mono input is placed like a mono
  voice, 0.7071 of it on each side in the middle, and a stereo input like
  a stereo sample, both sides at full in the middle;
- the part's **[voice effects](#voice-effects)**, **`fx.drive`** and the
  **bit crusher**, its **`volume`** and its **sends** to the echo, the
  reverb and the chorus;
- the part's **scope and meter** show it, the limiter holds it, and it can
  be [recorded](#recording).

Notes can be played on the same part at the same time; they share its
effects. The input does not use a voice.

Before the part, three things happen to the input, in this order:

1. **Cleaning.** A number that is infinite or not a number is taken as 0.
   The input is multiplied by `input.gain` and then kept within -4..4.
   Anything smaller than 0.000 000 000 1 (200 dB down) is taken as exactly
   0.
2. **The gate**, if `input.gate` is above -100: see below.
3. **`input.monitor`.** Off, the input goes no further: it is not heard,
   and can still be recorded. On (as it is to begin with), it goes into its
   part.

With every default, then, a mono input comes out of both sides at
`0.7071 * 0.35 = 0.25` of its level, and nothing else is done to it: each
number that comes out is the number that went in, times the pan's gain,
times `master.volume`. A stereo input, with `master.volume` 1 and the pan
in the middle, comes out number for number.

`input.part` and `input.monitor` change on the exact frame of their event.
A change to the part's filter or pan is taken up at the start of the next
control period, within 16 frames, and the cutoff and the pan slide to
their new values over that period.

### The gate

A microphone hisses when nobody is speaking. The gate lets the input
through only while it is louder than `input.gate`, a level in dB below
full scale (-45 is a good place to start; -100, the default, is no gate,
and the input is then never touched).

- The gate measures loudness as the largest sample lately seen: it jumps
  up to each new peak at once and falls back to 37% of it in 30
  milliseconds, so it rides over the gaps between the cycles of a voice.
- It **opens** on the first frame whose sample reaches the level, and
  takes 2 milliseconds to open, in a straight line, so the start of a word
  does not click.
- It **closes** when the measured loudness has fallen to half the level (6
  dB under it, so that a sound hovering at the level does not make it
  flutter), and takes `input.gate_release` seconds to close, in a straight
  line. If the sound comes back before it has closed, it opens again from
  where it had got to.
- Closed, the input is exactly nothing: its part makes no sound and does no
  work, and the engine can sleep.

### What it costs

Silence coming in costs almost nothing: one look at each frame, about 20
microseconds for each second of sound. Sound coming in costs what one
voice's share of the mix does. The input, and anything it wakes, start on
the exact frame the sound does, and the result does not depend on the
block size, as with everything else.

## Voice effects

Every part has three effects that come before its distortion: a **pitch
shifter**, a **ring modulator** and a **comb**. They are there to change a
voice (a chipmunk, a monster, a robot) but they belong to the part, not to
the input: a synth or a sample on any part can use them.

They always come in this order, and then the rest of the part follows:

```
pitch shifter ─ ring modulator ─ comb ─ drive ─ bit crusher ─ volume ─ sends
```

So a shifted voice is what the ring modulator multiplies, the comb rings
with what comes out of that, the distortion and the bit crusher roughen the
result, and the echo, reverb and chorus are fed with the whole of it. Each
is off until its parameter is moved from its default, and one that is off
is not there at all: it adds no delay and changes no number.

### The pitch shifter

`fx.pitch` moves everything on the part up or down by that many semitones,
-12 to 12, without making it faster or slower. It moves the whole sound,
resonances and all, as speeding a tape up does: up is small and squeaky,
down is big and slow-sounding.

- **How it works.** It keeps the last few hundredths of a second of the
  part's sound and reads them back faster or slower than they came in.
  Reading faster catches up with the present and reading slower falls
  behind, so every so often the reader jumps: back, to go over a stretch
  again, or forward, to skip one. Before each jump it compares what it is
  playing with every place it could jump to, between 2.5 and 14
  milliseconds away, and picks the one that looks most alike: for a voice
  or a single note, that is a whole number of cycles of the wave. It then
  crossfades to it over at most 4 milliseconds. Because the two sides of a
  jump match, there is none of the warble of a shifter that jumps blindly.
- **Delay.** There is no fixed delay. The shifted sound is between 1 frame
  and `0.4 + 14 + 4 * |r - 1|` milliseconds late, where `r` is the speed
  (2 an octave up, 0.5 an octave down): at most 18.4 ms an octave up and
  16.4 ms an octave down, at 48 kHz, and on average about half that. After
  a silence it starts again at the least delay it can.
- **How good it is.** On a steady sine, at every semitone from -12 to 12,
  the pitch is within 0.02 of a cent of what was asked for, the level
  within 0.01 dB, and everything that is not the tone is at least 49 dB
  below it (usually 80). On a voice-like sound at 120 Hz the pitch is
  within 0.02 of a cent, the level within 0.02 dB, and each cycle is 99.8%
  like the one before it. `cargo test` measures these.
- **What it is not good at.** It follows one pitch at a time, between
  about 71 and 400 Hz (and whole multiples of a cycle above that). Chords,
  and notes below 71 Hz, have no single cycle to jump by, and come out
  with a slight roughness. A sharp click can be heard twice, a few
  milliseconds apart, when a jump goes back over it. Shifting up by `r`
  folds over anything above `1 / (2 r)` of the sample rate, which for a
  voice is nothing that matters.
- A glide of `fx.pitch` (a `param` event with a time) slides the pitch
  smoothly. 0 is off: moving to or from exactly 0 while sound is passing
  can click, so slide to 0.01 if the shifter must stay in.

### The ring modulator

`fx.ring` multiplies the part's sound by a wave: 0 is off, 1 is the product
alone, and in between is a mix of the two. `fx.ring_freq` is how fast the
wave goes, and `fx.ring_wave` what it is:

- **sine** (0). A tone at `f` Hz times a sine at `fx.ring_freq` becomes two
  tones, at `f + fx.ring_freq` and `f - fx.ring_freq`, each at half the
  level, and the tone itself is gone. At a few Hz that is a tremolo; at 30
  to 80 Hz it is the voice of a film robot; at hundreds of Hz it is
  bell-like and strange, because the new tones are no longer in tune with
  each other.
- **noise** (1). The wave is noise with everything above `fx.ring_freq`
  taken off: a random rumble. It smears each harmonic of a voice into a
  band of noise that wide, which leaves the vowels where they were and
  takes the pitch away: a hoarse whisper. It is as loud as the sine's
  product. Around 300 Hz suits a voice.

### The comb

`fx.comb` adds a copy of the sound from a moment ago, fed back round and
round, so the part rings at one note: `fx.comb_freq`, 50 to 5000 Hz, and
every whole multiple of it. 0 is off, 1 is the ringing copy alone.

- `fx.comb_feedback` is how long it rings: at 0.7 a few cycles, at 0.97 a
  long metallic drone. Below 0 it rings at the odd multiples of half
  `fx.comb_freq`: an octave lower and hollow.
- `fx.comb_depth` above 0 sweeps the note up and down by up to 90%,
  `fx.comb_rate` times a second: a flanger.
- The ringing copy is turned down as the feedback goes up, so that the
  part stays about as loud as it was.
- A low note with long feedback is a voice in a pipe, and every word comes
  out on the pipe's note; that and the ring modulator are the two halves
  of a robot.

### How many parts at once

The ring modulator is there for all 64 parts. The pitch shifter and the
comb each remember a stretch of sound, so the engine makes 8 of them when
it is created, and a part takes one for as long as its `fx.pitch` is not 0
or its `fx.comb` is above 0. If more than 8 parts ask at once, the ninth
goes without until another lets go: its sound passes unshifted and
uncombed.

After a part's last voice has ended, its shifter and comb go on until
what they still hold has come out and they have been silent for 60
milliseconds; then they stop, and cost nothing until the part sounds again.

### Voice changers

Seven voices to begin with. Each is a list of parameter values: those of a
part are for the input's part, and those of the echo and the reverb are
the engine's. To put one on, first give every parameter named anywhere in
this table its default, then set the ones in its row; the plain voice is
the first half alone. The same list is in the
[parameter table as JSON](#the-table-as-json), as `voice_presets`, so a
host need not copy it from here.

| name | values | what it is |
|---|---|---|
| `robot` | `fx.ring` 1, `fx.ring_freq` 50, `fx.comb` 0.45, `fx.comb_freq` 110, `fx.comb_feedback` 0.8, `fx.bits` 10 | A machine: the voice multiplied by a 50 Hz wave, ringing in a metal pipe. |
| `chipmunk` | `fx.pitch` 7 | Small and squeaky: everything seven semitones higher. |
| `monster` | `fx.pitch` -7, `fx.drive` 0.25, `send.reverb` 0.3, `reverb.size` 0.7, `reverb.damping` 0.6 | Huge and deep: seven semitones lower, with a growl and a little room. |
| `cave` | `send.delay` 0.5, `send.reverb` 0.6, `delay.time` 0.38, `delay.feedback` 0.55, `delay.tone` 2500, `reverb.type` 3, `reverb.size` 0.4, `reverb.damping` 0.55 | Far inside a cave: long echoes and an enormous, dark space. |
| `telephone` | `filter.type` 3, `filter.slope` 1, `filter.cutoff` 1200, `filter.resonance` 0.2, `fx.drive` 0.2, `fx.bits` 8, `fx.downsample` 6 | Down an old phone line: only the middle of the voice, coarse and a little crushed. |
| `alien` | `fx.pitch` 4, `fx.ring` 0.8, `fx.ring_freq` 180, `fx.comb` 0.4, `fx.comb_freq` 600, `fx.comb_feedback` 0.75, `fx.comb_rate` 0.4, `fx.comb_depth` 0.5, `send.chorus` 0.4 | Not from here: higher, multiplied by a 180 Hz wave, with a slow metallic sweep. |
| `whisper` | `filter.type` 2, `filter.cutoff` 500, `filter.resonance` 0, `fx.ring` 1, `fx.ring_wave` 1, `fx.ring_freq` 350 | Hoarse and breathy: the voice's pitch blurred into noise, with its low end taken off. |

With a real microphone, set `input.gate` to about -45 as well.
`cargo run --release --example voice` renders a made-up voice through each
to `target/demo/voice-dry.wav`, `voice-robot.wav` and so on.

### What is not here

- **Moving the resonances of a voice without its pitch** (a formant
  shift: a child's voice at a man's pitch). The shifter moves both
  together, which is what a chipmunk and a monster are. Moving them apart
  needs the voice cut into single cycles at the right instants, and a way
  of knowing when there are no cycles to cut; done cheaply it warbles on
  exactly the sounds a microphone picks up, so it has been left out rather
  than done badly.
- **A vocoder** (a synth that speaks with the input's vowels). It needs
  about 16 bands, each with a filter on the voice, a filter on the synth
  and a measure of loudness between them: roughly the cost of ten voices
  for the one part, and one part's sound would have to be made before
  another's, which the engine does not arrange yet.

## The reverb

There is one reverb, and every part sends to it with `send.reverb`. What it
adds goes straight to the mix: there is no dry/wet control, the send is it.
`reverb.type` chooses what kind of space it is:

| value | name | what it is | its tail at size 0.5 | at size 0 to 1 | its own pre-delay |
|---|---|---|---|---|---|
| 0 | `room` | A small room: a short soft tail. The one to begin with. | 1 s | 0.33 to 3 s | 4 ms |
| 1 | `hall` | A concert hall: big, smooth and warm. | 2.6 s | 0.87 to 7.8 s | 18 ms |
| 2 | `plate` | A big sheet of metal, as studios used: bright, and thick from the first moment. Good on drums and voices. | 1.8 s | 0.6 to 5.4 s | none |
| 3 | `cathedral` | Enormous: it swells slowly and goes on and on. | 6.5 s | 2.17 to 19.5 s | 30 ms |
| 4 | `spring` | The spring in a guitar amp: it twangs, and boings on anything sudden. | 2 s | 0.67 to 6 s | none |

The room is the one to begin with because the reverb is shared. A beginner
turns one dial, the send, and a whole tune may be going through it: a room
gets louder as the dial is turned and never longer, so quick notes and drums
stay clear, where a hall's 2.6 seconds pile up under them.

### How it is made

```
send.reverb ─ low cut ─ pre-delay ─┬─ 6 early reflections a side ──────────────────────────────┐
                                    └─ 4 all-pass filters a side ─┬─ a little, straight out ─────┤
                                                                  └─ 8 delay lines in a loop ───┴─ mix
```

The first four types are one structure with different numbers: a feedback
delay network. Eight delay lines, of lengths that share no simple ratio,
feed each other through a Hadamard matrix, which loses nothing and sends
every line into every other, so that the echoes multiply into a smooth
wash. On its way round, each line has a filter that makes the treble (and,
by its own amount, the bass) die away at its own rate, and an all-pass
filter that thickens the echoes. Every other line's length drifts slowly by
a few hundredths of a millisecond, which keeps any one pitch from ringing
on, and is too little to hear as a chorus: the reverb of a held note is a
steady note, within 3 cents of its pitch. Left and right hear the same
eight lines added up two different ways, so they are unlike each other,
and their sum (on one loudspeaker) loses nothing.

The spring is something else, and honest about it: two springs, each a
delay in a loop with a chain of 30 all-pass filters that hold the high part
of a sound back longer than the low part, up to about 4 kHz, above which a
spring carries nothing. Every time round, a click comes out more of a
"boing". It is not smooth, and is not meant to be.

### Its parameters

- **`reverb.size`** (0 to 1, 0.5 to begin with) is how big the space is.
  Every length in it, the early reflections' and the type's own pre-delay
  included, is 0.6 times its own at 0 and 1.4 times at 1.
- **`reverb.decay`** is how long the tail takes to fall by 60 dB, in
  seconds, from 0.1 to 30. At 0, where it begins, it is left to the type
  and the size: the type's own time at size 0.5, three times shorter at
  size 0 and three times longer at size 1. In one sentence: **size makes
  the room bigger and its tail longer with it, unless `reverb.decay` says
  in seconds exactly how long the tail is.** What is asked for is what is
  got, within a tenth, from 1 to 30 seconds for every type at every size
  (and from 0.3 seconds for the room and the plate; a hall or a cathedral
  cannot be much shorter than half a second, since its echoes are too far
  apart). That is measured between 500 and 1000 Hz.
- **`reverb.damping`** (0 to 1) is how much sooner the treble dies away
  than the rest: the treble lasts `1 / (1 + 7.5 d²)` as long, which is all
  of the time at 0 (a bright tail), under half of it at 0.4, where it
  begins, and an eighth at 1 (a dull one). "Treble" is above 5 kHz in the
  room, 4 kHz in the hall, 9 kHz in the plate and 3 kHz in the cathedral
  and the spring. The bass, below 180 Hz, lasts 1.15 times as long as the
  middle in the hall, 1.25 times in the cathedral, 0.9 times in the plate,
  and as long in the others.
- **`reverb.predelay`** (0 to 0.2 seconds) is a gap before the reverb
  starts, added to the type's own. It keeps a sound clear of its own
  reverb.
- **`reverb.low_cut`** (20 to 1000 Hz, 120 to begin with) keeps bass below
  it out of the reverb (6 dB an octave), where it would turn to mud. It
  also means nothing steady ever builds up inside.

A longer tail is turned down a little and a bigger room up a little (by
the fourth root of the decay and the square root of the size), so that the
reverb of a held chord grows only gently with its length; and the types
are matched, so that the same send gives about as much of each.

### Changing it while it rings

Nothing clicks, and nothing can make it grow.

- A change of **size** slides: the echoes move apart or together, taking
  a third to two thirds of a second from one end to the other (longest
  for the biggest types), and what is ringing bends in pitch as they go,
  by 15% at the very most. `reverb.predelay` slides the same way.
- A change of **type** fades the old type out over 20 milliseconds, empties
  it, and fades the sound into the new one over the next 20: the old tail
  is lost.
- `reverb.decay` and `reverb.damping` ease to their new values over about
  a tenth of a second.

### What it costs

When nothing is coming in and what comes out has been under a millionth
(0.000 001, -120 dB) for as long as its longest echo, the reverb empties
itself and stops: from then on it puts out exactly nothing and does no
work at all, until the first frame that is not silence wakes it. A reverb
that no part sends to costs nothing.

Working, it takes about 1.9 milliseconds for each second of sound on a
fast laptop as native code (1.5 for the plate, 2.6 for the spring), and
about 2.5 as WebAssembly (3.6 for the spring): as much as two or three
voices. Its memory is made with the engine, for the biggest size of every
type: 440 KB at 48 kHz.

## Recording

The engine can record into a sample slot, and what it records is from
then on a sample like any other: playable at any pitch, forwards or
backwards, looped, on any part.

1. **Make somewhere to record**, before the recording:
   `synths_record_buffer(engine, slot, frames, channels)`. `frames` is the
   longest the recording can be.
2. **Start**, with a `record` event (type 16) whose `a` is 1, `x` the slot
   and `y` what to record:

   | `y` | what is recorded |
   |---|---|
   | 0 | The live input, after `input.gain` and the gate and before its part: the plain voice, whether or not `input.monitor` lets it be heard. |
   | 1 | A part, `part`: after its voice effects, distortion, bit crusher and volume, before its sends. For the input's part, the changed voice. Any part can be recorded, not only the input's. |
   | 2 | The whole output, exactly as the render gives it: every part, the echo, the reverb and the chorus, after the limiter. This is "save what I just played". |

3. **Stop**, with a `record` event whose `a` is 0. A recording also stops
   by itself when the slot is full, and `synths_reset` stops one.
4. **Use it.** `synths_sample_frames` says how long it is,
   `synths_sample_read` copies it out to be saved, `synths_sample_config`
   gives it a loop or another base note, and a part plays it with
   `osc1.wave` 4 and `osc1.sample`.

In JavaScript, with the `event` function of [A host loop](#a-host-loop),
to record three seconds of the changed voice into slot 9:

```js
x.synths_record_buffer(engine, 9, 3 * sampleRate, 1);           // once, beforehand
event(now,                  16 /* record */, 31, 1 /* start */, 9, 1 /* a part: 31 */);
event(now + 3 * sampleRate, 16 /* record */, 0,  0 /* stop  */, 0, 0);
```

How it behaves:

- **Exact.** The recording holds exactly the frames from the start
  event's frame up to, not including, the stop event's: an event at time
  1000 and one at 5000 record 4000 frames, whatever the block size, and
  the same events and input always record the same numbers. The first
  frame recorded is frame 0 of the sample.
- **One at a time.** A start while another recording is running moves the
  recorder to the new slot and source on that frame; the first recording
  keeps what it had. A start into the slot being recorded begins it again.
- **Starting empties the slot's sample** (its memory stays): the sample has
  no frames, no loop, and grows as the recording goes. A voice may play the
  slot while it is being recorded into, and plays what has been recorded
  so far.
- **Channels.** Into a slot with two channels go left and right. Into a
  slot with one goes their average, which for a mono input (source 0) is
  the input itself, exactly. A mono input recorded from its part in the
  middle (source 1) is 0.7071 of its level on each side, as it is heard.
- **Silence is recorded as silence.** A part that is making no sound, an
  input with its gate shut, a render with no input, and an engine that has
  gone to sleep all record zeros; the length is always the time between
  the two events.
- **Nothing to record into.** A start is ignored if the slot is empty or
  does not exist, or `y` is not 0, 1 or 2, or (for `y` 1) the part does
  not exist. (A slot holding a loaded sample can be recorded over as well,
  for as many frames as that sample has.) If the slot is freed or loaded with another sample while it
  is being recorded into, the recording stops.
- The recording is at the engine's sample rate and its base note is 60, so
  played at note 60 it comes back exactly as it went in.
- `synths_record_read` gives a display the state of the recorder at any
  time: see [its description](#the-live-input-and-recording).

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
- A voice ends, and is free again, at the end of the control period in
  which its amplitude envelope finishes its release, or its decay finishes
  with a sustain level of 0, or it has nothing left to play (see one-shots),
  or a fade reaches nothing.

## The path of the sound

```
voice:   oscillator 1 ─┐
         oscillator 2 ─┼─ filter ─ amplifier (envelope, velocity, voice volume, tremolo) ─ pan ─┐
         sub, noise   ─┘                                                                        │
input:   gain ─ gate ─ monitor ─ the filter of its part ─ the pan of its part ─────────────────┤
                                                                                                │
part:    all its voices, and the input if it is this part's ────────────────────────────────────┘
           └─ pitch shifter ─ ring modulator ─ comb ─ drive ─ bit crusher ─ volume ─┬─────────── mix
                                                                                    ├─ send.delay  ─ echo   ─ mix
                                                                                    ├─ send.reverb ─ reverb ─ mix
                                                                                    └─ send.chorus ─ chorus ─ mix
engine:  mix ─ master.volume ─ limiter ─ out
```

- Levels multiply straight through. A single voice at full velocity with
  every default, on a part panned to the middle, peaks at about
  `0.7071 * 0.35 = 0.25` on each side.
- The effects of a part always come in the order shown; each is skipped
  while it is off. The first three are the [voice effects](#voice-effects).
- The sends are taken after the part's volume. The effects add their sound
  to the mix; there is no dry/wet control, the send level is it.
- The part's scope and meter, and a recording of a part, are taken after
  its volume and before its sends. A recording of the input is taken after
  its gate and before `input.monitor`; a recording of the output is taken
  at `out`.
- The echo, the [reverb](#the-reverb) and the chorus are each one effect
  that every part shares.
- When no voice or input has sounded and no echo or reverb has rung for 2.5
  seconds, the engine goes to sleep: it writes exact zeros and does almost
  no work, until the next event, or the next sound coming in, wakes it. The
  reverb does not wait for that: it stops working by itself as soon as its
  own tail is over.

## Threads

The engine has no locks, and none of its functions may run at the same time
as another on the same engine. In practice:

- **In a browser** the module lives in the `AudioWorkletProcessor`, so every
  call is on the audio thread and the rule keeps itself. Messages from the
  page (events to push, samples to load) arrive between calls to `process`.
- **In a native app** call `synths_render` or `synths_render_into` (or
  `synths_render_io`, `synths_render_io_into`) and
  `synths_push_events` from the audio callback only. To send events from
  another thread, put the 32-byte records in a lock-free queue of your own
  and, at the top of the audio callback, copy them into the event buffer and
  push them.
- `synths_sample_load`, `synths_sample_config`, `synths_sample_free`,
  `synths_record_buffer`, `synths_reset`, `synths_create` and
  `synths_destroy` allocate or free
  memory or take a while. They must not overlap a render either. Call them
  before audio starts, or while the audio callback is known not to be
  running; a native host that must load a sample while sound plays should
  stop calling render for that moment and put out silence.
- `synths_alloc` and `synths_free` touch only the memory allocator and are
  safe from any thread in a native build.
- `synths_scope_read`, `synths_levels_read`, `synths_notes_read` and
  `synths_record_read` are the
  exception to the rule: they may be called from any one thread at any time,
  even during a render. (Not during `synths_destroy`.)
- `synths_sample_frames` and `synths_sample_read` only read, but they read
  what a render may be writing: call them from the audio callback, or when
  the recording has stopped and no render is running. A host that wants to
  show a recording's length as it grows uses `synths_record_read`.
- A native host's microphone often arrives on a different callback from the
  one that asks for output. Carry the input across in a lock-free ring of
  your own, and give `synths_render_io_into` the frames that match.
- `synths_macro_set` allocates nothing and is quick, but it changes the
  engine, so it must not overlap a render: in a native app call it from the
  audio callback, between renders, like `synths_push_events`.
- `synths_abi_version`, `synths_version`, `synths_param_table`,
  `synths_param_table_len`, `synths_output_capacity` and
  `synths_event_capacity` need no engine and are safe from any thread.

Inside `synths_render`, `synths_render_into`, `synths_render_io`,
`synths_render_io_into` and `synths_push_events` the
engine does not allocate, lock, wait, make a system call or loop without
bound. (A test counts the calls to the allocator while everything is going,
and there are none.)

## WebAssembly

`scripts/build-wasm.sh` builds `target/wasm/synths_core.wasm` for the
`wasm32-wasip1` target, then loads it in node (`scripts/check-wasm.mjs`),
plays a note through it, reads the taps, sets a macro, feeds it a live
input and times it. The
module is about 354 KB (140 KB gzipped), of which the song loaders and
player are about 150 KB, the JSON song reader 42 KB of that, and the live
input, voice effects and recorder 29 KB. Two smaller
modules can be built: `scripts/build-wasm.sh --no-json` leaves out the JSON
reader (312 KB, 125 KB gzipped), and `--no-songs` the whole song player
(204 KB, 80 KB gzipped). See
[Which kinds of song a build reads](#which-kinds-of-song-a-build-reads).

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
`synths_sample_load`, `synths_record_buffer` and the first
`synths_param_table`; it never grows in
`synths_render`, `synths_render_io` or `synths_push_events`. So make the
views over the output, input
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

To feed a microphone in, the page gets one with `getUserMedia`, connects it
to the node (made with `numberOfInputs: 1`), and step 2 becomes:

```js
    // 2. Copy in what the microphone delivered, render one block, copy it out.
    const mic = inputs[0];                                 // [] until the microphone is connected
    const channels = Math.min(mic.length, 2);
    const out = outputs[0];
    const frames = out[0].length;
    if (channels > 0) new Float32Array(x.memory.buffer, x.synths_input_left(engine), frames).set(mic[0]);
    if (channels > 1) new Float32Array(x.memory.buffer, x.synths_input_right(engine), frames).set(mic[1]);
    const n = x.synths_render_io(engine, frames, channels);
    out[0].set(new Float32Array(x.memory.buffer, x.synths_output_left(engine), n));
    if (out[1]) out[1].set(new Float32Array(x.memory.buffer, x.synths_output_right(engine), n));
    return true;
```

That is all a host does each block: the input's frames in, the same number
of frames out. Everything else is parameters and events, sent as ever:
`input.part`, `input.gain`, `input.gate` and `input.monitor`, the voice
effects of the input's part, and `record` events. On the page:

```js
const stream = await navigator.mediaDevices.getUserMedia({
  audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
});
audioContext.createMediaStreamSource(stream).connect(synthsNode);
```

(With loudspeakers and no echo cancellation the microphone hears the
engine and howls: use headphones, or leave `echoCancellation` on, or keep
`input.monitor` off and only record. A browser gives the microphone at the
context's sample rate, which is the engine's, so nothing needs converting.)

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

With a microphone, a native host renders with `synths_render_io_into`. If
the system delivers input and asks for output in one callback (a duplex
stream), pass its buffers straight in; `in_right` is `NULL` for a mono
microphone:

```c
void duplex_callback(const float *mic, float *left, float *right, uint32_t frames) {
    /* 1. events, as above */
    synths_render_io_into(engine, mic, NULL, left, right, frames);
}
```

If input and output come on separate callbacks, the input callback writes
into a lock-free ring and the output callback reads `frames` frames from it
(silence if it has run dry) before rendering. The input must be at the
engine's sample rate: open the microphone at that rate, or convert before
handing it over.

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
- `Engine::render_io(in_left, in_right, left, right)` is `render` with a
  live input (`in_right` is `None` for a mono one). `record_buffer`,
  `recording`, `sample_frames`, `sample_channels` and `read_sample` are the
  recording functions above, and `Event::record_start` and
  `Event::record_stop` make its events.
- `synths_core::params` has a constant for every parameter id (`CUTOFF`,
  `OSC1_WAVE`, ...) and the table itself (`PARAMS`).
- `synths_core::presets` has the voice changers (`VOICE_PRESETS`) and
  `apply_voice_preset(&mut engine, part, Some(preset))`.

### A sequencer inside the engine

Something that plays music by itself (a tracker song player) lives inside
the engine and is called from within `render`, on exact frames, instead of
pushing events block by block as a host must. It implements one trait:

```rust
use synths_core::sequencer::{Sequencer, Trigger, Voices};

struct Player { /* the song, where it has got to */ }

impl Sequencer for Player {
    fn advance(&mut self, voices: &mut Voices<'_>) -> u32 {
        // One tick of the song: start, steer and stop voices.
        voices.trigger(0, 1, &Trigger { slot: 3, position: 0.0, rate: 8363.0, volume: 1.0, pan: -0.5 });
        voices.macro_tick(synths_core::event::ALL_PARTS);
        882 // call me again in 882 frames
    }
}

engine.set_sequencer(Some(Box::new(Player { /* ... */ })));
```

- `Engine::set_sequencer(Option<Box<dyn Sequencer>>)` installs one (or takes
  it away) and hands back the one that was there. There is one slot. It is
  for between renders, not during one; what is handed back should be dropped
  away from the audio thread.
- `advance` is first called on the next frame rendered, and then on the
  frame it asks for each time: it returns how many frames from now (0 counts
  as 1). `render` splits its block there, so what `advance` does starts on
  that exact frame wherever it falls in a block, and the sound is the same
  whatever the block size. A tick that is not a whole number of frames is
  the sequencer's to manage: return the whole part and carry the fraction.
- On a frame that also has events from the host, the sequencer goes first
  and the host's events after, so the host has the last word.
- `advance` runs on the audio path: no allocating, locking or waiting.
- `Sequencer::reset` is called by `Engine::reset` (`synths_reset`), after
  every voice has been stopped; `advance` is then called on the next frame.
- A `song` event from the host is handed to the sequencer on its frame:
  `fn event(&mut self, event: &Event, voices: &mut Voices<'_>) -> bool`,
  which does nothing unless it is written. Returning true has `advance`
  called at once, on that same frame, whenever it was next due.
- The engine's own song player is such a sequencer:
  `synths_core::song_player::install(&mut engine, song, FIRST_PART, first_slot, raw)`
  is what `synths_song_load` does once it has read the file.
- A sequencer keeps being called while the engine sleeps, and wakes it by
  starting a voice.

`Voices` is the engine as the sequencer sees it during `advance`:

| method | what it does |
|---|---|
| `time()`, `sample_rate()` | The frame this call is for, and the engine's rate. |
| `trigger(part, handle, &Trigger) -> bool` | Start a sample as the part's only voice with that handle: a voice the handle already has there is cut, without a click, on the same frame. `Trigger` gives the sample `slot`, the `position` to start at in frames, the `rate` in frames of the sample per second, and the voice's `volume` (0..4) and `pan` (-1..1). The voice uses the part's patch (envelopes, filter, macro, effects) but plays that sample as oscillator 1 whatever the patch says, so one part can play a different sample on every note. False if the slot is empty, the part does not exist or the rate is not above 0. |
| `set_rate(part, handle, rate, glide)` | Play the handle's voices at `rate` frames of their sample per second, sliding there over `glide` seconds. A tracker turns its period into this. A rate of 0 holds the sample still where it is, until a rate above 0 lets it go on. |
| `set_pitch(part, handle, note, glide)`, `set_freq(part, handle, hz, glide)` | The `voice_pitch` and `voice_freq` events. |
| `set_volume(part, handle, volume)`, `set_pan(part, handle, pan)`, `set_position(part, handle, frames)` | The `voice_volume`, `voice_pan` and `voice_position` events. |
| `ramp_volume(part, handle, volume, frames)`, `ramp_pan(part, handle, pan, frames)` | Take the volume (0..4) or pan (-1..1) of the handle's voices there in a straight line over `frames` frames, starting on this frame. With 0 frames it is there at once (within the control period), as `set_volume` and `set_pan` are; and for a voice started on this same frame it is that voice's starting value, with no slide at all. A later set or ramp takes over from wherever it has got to. FastTracker 2 slid every volume and pan over a whole tick, and its song player here does the same. |
| `kill(part, handle)` | The `voice_kill` event: cut at once, without a click. |
| `fade(part, handle, seconds)` | Fade the handle's voices to nothing in a straight line over `seconds`, starting on this frame, and end them there. 0 seconds is a cut. |
| `queue_sample(part, handle, slot)` | When the sample the handle's voices are playing next reaches the end of its loop, or its end, carry on with the sample in `slot` (`Some(slot)`): from the start of that sample's loop, round and round, at the same pitch. With `None`, or a slot that is empty or has no loop, the voices end there instead. This is how a ProTracker song swaps samples in mid-note. It happens within a control period (16 frames) of the moment, and the same way whatever the block size. A voice whose one-shot has already ended is gone, and there is nothing to queue on: `sounding` says so, and `trigger` starts the new sample at its loop instead. |
| `arp(part, handle, rate, order, offsets)` | The `arp` event (and `arp_more`, when there are more than four offsets): give the handle's voices up to 8 semitone offsets to step through, `rate` steps a second (0: 50; below 0: one step for each `macro_tick`), in an `order` (0 up, 1 down, 2 up and down, 3 as given, 4 random). No offsets stops it. With handle 0 it is for the part's next `note_on`. |
| `sounding(part, handle) -> bool` | Whether any voice with that handle is still sounding on the part. |
| `note_on(part, handle, note, velocity)`, `note_off(part, handle)` | The `note_on` and `note_off` events, for parts that play synth patches. |
| `set_param(part, id, value, glide)` | The `param` event. |
| `macro_tick(part)` | The `macro_tick` event: step the macros of parts with `macro.sync` on. A tracker calls it once on each of its ticks. |
| `event(&Event)` | Carry out any event a host could send, now (its time is ignored). This is how a sequencer sends an event that has no method of its own here, such as one the engine gains later. |

`part` may be `ALL_PARTS` wherever the matching event allows it, and in
`set_rate`, `queue_sample`, `fade` and `sounding`. A handle of 0 addresses nothing, except
that `trigger` and `note_on` accept it for a voice nothing will steer.
