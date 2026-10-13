# The song format

A song is one JSON document. It is meant to be read and changed by hand in a text editor, compared line by line between two versions, and written by a program without a library. Samples that are too big to write as numbers live beside it as ordinary WAV files.

The code is `src/song/native.rs`; the model it loads into is `src/song/mod.rs`. MOD and XM files load into the same model, so anything they can say, a song can say.

This is version 1.

## A whole song

```json
{
  "format": "synths-song",
  "version": 1,
  "title": "Three Notes",
  "rules": "ft2",
  "slides": "linear",
  "speed": 6,
  "tempo": 125,
  "channels": 2,
  "order": [0, 0],
  "patterns": [
    {"rows": 16, "events": [
      {"row": 0, "ch": 0, "note": "C-4", "ins": 1},
      {"row": 4, "ch": 0, "note": "E-4", "ins": 1, "vol": 32},
      {"row": 8, "ch": 0, "note": "G-4", "ins": 1, "fx": "A04"},
      {"row": 8, "ch": 1, "note": "C-3", "ins": 2},
      {"row": 12, "ch": 0, "note": "off"}
    ]}
  ],
  "instruments": [
    {"name": "beep", "sample": 0},
    {"name": "bass", "patch": "wobble-bass"}
  ],
  "samples": [
    {"name": "square", "wave": [100, 100, 100, 100, -100, -100, -100, -100], "loop": {"start": 0, "length": 8}}
  ]
}
```

That is a complete, playable song with no other files: the one sample is eight numbers.

## Rules for readers and writers

- `format` must be `"synths-song"`. `version` is a whole number; a reader refuses a version higher than it knows and says so.
- A key a reader does not know is ignored, so a newer writer can add keys without breaking an older reader. A change that an older reader would get wrong raises `version`.
- A key that is left out takes the default given below. A writer leaves out what is at its default.
- Numbers are whole numbers unless said otherwise. Counting starts at 0 for rows, channels, patterns, orders and samples, and at 1 for instruments (so that 0 can mean "no instrument", as trackers have it).
- A value that is out of range, or of the wrong kind, is an error that names the key. Nothing is silently clamped.

## The top level

| Key | Meaning | Default |
|---|---|---|
| `title` | The song's name. | `""` |
| `rules` | Whose behaviour the song is played by: `"ft2"` (FastTracker 2) or `"protracker"`. The two differ in many small ways, and a song sounds as written only under the rules it was written with. | `"ft2"` |
| `slides` | `"linear"`: pitch slides move evenly everywhere on the keyboard. `"amiga"`: they move the Amiga's period, so they are faster on high notes. | `"linear"` |
| `speed` | Ticks in a row, 1 to 31. | 6 |
| `tempo` | Beats a minute, 32 to 255. A tick lasts 2.5 / tempo seconds, so at speed 6 and tempo 125 there are 4 rows to a beat. | 125 |
| `channels` | How many channels, 1 to 64. | 4 |
| `pan` | Each channel's panning to begin with: a list of numbers, 0 (left) to 255 (right), 128 the middle. May be shorter than `channels`. | all 128 |
| `order` | The patterns to play, in order. A pattern can appear many times. | `[0]` |
| `restart` | The place in `order` to go back to when the song runs off its end. | 0 |
| `patterns` | The patterns. | none |
| `instruments` | The instruments. | none |
| `samples` | The samples. | none |

## Patterns

A pattern is `{"rows": n, "events": [...]}` with 1 to 256 rows (64 if left out). Its events are the cells of the tracker grid that have something in them, and nothing else: an empty pattern is `{"rows": 64, "events": []}`.

A writer puts one event on a line, in order of row and then channel. A reader takes them in any order.

An event:

| Key | Meaning |
|---|---|
| `row` | The row, from 0. |
| `ch` | The channel, from 0. |
| `note` | A note name from `"C-0"` to `"B-7"` (`"C#4"`, `"Db4"` and `"C4"` are read too), or `"off"` to let go of the key. A number is also read: 1 is C-0, 49 is C-4, 97 is off. |
| `ins` | The instrument, from 1. |
| `vol` | A number 0 to 64 sets the volume. A two-character text is a volume-column command, below. |
| `fx` | An effect: a letter or digit and two hexadecimal digits, as a tracker shows it (`"A04"`, `"F7D"`, `"R23"`). `[10, 4]` is read too. |
| `chord` | A chord for the note to play as a fast arpeggio; see [Chords](#chords). |
| `chordSpeed`, `chordHz`, `chordOrder` | How fast and in what order the chord's notes go by. |

C-4 plays a sample at 8363 frames a second (unless the sample is tuned otherwise), and is middle C for a synth patch.

### Chords

One channel can sound like a chord by stepping through the chord's notes very quickly, the way Commodore 64 music does. In this format that is not an effect code: the note's event says which chord.

```json
{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "min"},
{"row": 4, "ch": 0, "note": "F-4", "ins": 1, "chord": [0, 4, 7, 12], "chordOrder": "updown"},
{"row": 8, "ch": 0, "ins": 1, "chord": "Gm7", "chordHz": 60},
{"row": 12, "ch": 0, "chord": "off"}
```

`chord` is one of:

- A kind of chord by name, built on the event's note: `"maj"`, `"min"` (or `"m"`), `"dim"`, `"aug"`, `"sus2"`, `"sus4"`, `"5"`, `"6"`, `"m6"`, `"7"`, `"maj7"`, `"m7"`, `"dim7"`, `"m7b5"`, `"9"`, `"maj9"`, `"m9"`, `"add9"`, `"oct"`.
- A chord with its root, as on a song sheet: `"C"`, `"Cm7"`, `"F#maj7"`, `"Bb7"`. If the event has no `note`, the root in octave 4 becomes its note; if it has one, the root must be that note.
- A list of 1 to 8 numbers: each note of the chord in semitones from the event's note. The note itself is 0, and is only played if it is in the list.
- A list of 1 to 8 note names, which needs the event to have a `note` to measure them from.
- `"off"`, to stop the channel's chord and leave its note sounding plain.

A chord carries on, row after row, until the channel's next note. That note plays plain unless it has a chord of its own. An event with a chord and no note changes the chord of the note that is sounding.

| Key | Meaning | Default |
|---|---|---|
| `chordSpeed` | Steps in each tick, so the chord keeps time with the song. May be a fraction. | 1 |
| `chordHz` | Steps a second, whatever the tempo. 50 is the classic rate. Give this or `chordSpeed`, not both. | |
| `chordOrder` | `"up"` (lowest to highest, round and round), `"down"`, `"updown"`, `"written"` (as listed) or `"random"`. | `"up"` |

A writer always writes a chord as its list of numbers, so a name survives loading and saving as the chord it meant, not as the name.

The player hands a chord to the voice as one instruction, and the voice does the stepping, so the steps do not have to fall on ticks. This is separate from the trackers' arpeggio effect `0xy`, which is kept for songs that came from MOD and XM files and behaves exactly as those trackers did: three notes, a step a tick, restarting on every row.

### Volume-column commands

The second character is a hexadecimal digit.

| Text | Does |
|---|---|
| `"-x"`, `"+x"` | Slide the volume down or up by x on every tick but the first. |
| `"Dx"`, `"Ux"` | Move the volume down or up by x once. |
| `"Sx"` | Set the vibrato's speed. |
| `"Vx"` | Vibrato with depth x. |
| `"Px"` | Set the panning: 0 is left, F nearly right. |
| `"Lx"`, `"Rx"` | Slide the panning left or right. |
| `"Mx"` | Glide to the note (tone portamento) at speed x. |

### Effects

These are FastTracker 2's, of which ProTracker's (0 to F) are the first sixteen. The full behaviour of each is in the replayer, `src/replayer/`.

| Effect | Does |
|---|---|
| `0xy` | Arpeggio: cycle through the note, x semitones up and y semitones up. |
| `1xx`, `2xx` | Slide the pitch up or down. |
| `3xx` | Glide to the note in this cell. |
| `4xy` | Vibrato: speed x, depth y. |
| `5xy`, `6xy` | Carry on the glide or the vibrato, with a volume slide. |
| `7xy` | Tremolo: speed x, depth y. |
| `8xx` | Set the panning, 00 to FF. |
| `9xx` | Start the sample xx times 256 frames in. |
| `Axy` | Slide the volume up by x or down by y each tick. |
| `Bxx` | Jump to place xx in the order. |
| `Cxx` | Set the volume, 00 to 40. |
| `Dxx` | Go to the next pattern, at row xx (written in decimal). |
| `E1x`, `E2x` | Move the pitch up or down once. |
| `E3x`, `E4x`, `E7x` | Glissando; the vibrato's shape; the tremolo's shape. |
| `E5x` | Set the note's finetune. |
| `E6x` | Pattern loop: `E60` marks the start, `E6x` goes back to it x times. |
| `E9x` | Start the note again every x ticks. |
| `EAx`, `EBx` | Move the volume up or down once. |
| `ECx`, `EDx` | Cut the note at tick x; delay the note until tick x. |
| `EEx` | Hold the row for x more rows' time. |
| `Fxx` | Below 20: ticks in a row. From 20: beats a minute. |
| `Gxx`, `Hxy` | Set the song's overall volume; slide it. |
| `Kxx` | Let go of the key at tick xx. |
| `Lxx` | Jump to tick xx of the envelope. |
| `Pxy` | Slide the panning right by x or left by y. |
| `Rxy` | Start the note again every y ticks, changing its volume by rule x. |
| `Txy` | Tremor: on for x+1 ticks, off for y+1. |
| `X1x`, `X2x` | Move the pitch up or down once, a quarter as far as `E1x`. |

## Instruments

An instrument is what a cell's `ins` names. It is either samples or a synth patch.

| Key | Meaning | Default |
|---|---|---|
| `name` | A name to show. | `""` |
| `sample` | The sample every key plays. | |
| `zones` | For an instrument with several samples: a list of `{"from": "C-0", "to": "B-3", "sample": 0}`, each giving the sample for a run of keys. Keys in no zone play nothing. | |
| `patch` | The name of a synth patch to play in place of samples. The player does not look inside it: it hands the name to whatever makes the sound. | |
| `volumeEnvelope`, `panEnvelope` | Envelopes, below. | none |
| `vibrato` | The instrument's own vibrato: `{"wave": "sine", "sweep": 0, "depth": 4, "rate": 20}`. `wave` is `"sine"`, `"square"`, `"ramp-up"` or `"ramp-down"`; `sweep` is how many ticks it takes to reach full depth. | none |
| `fadeout` | How fast the note fades after key off, 0 to 65535. 0 never fades; 32768 fades in one tick. | 0 |

An envelope is `{"points": [[0, 64], [8, 32], [20, 0]], "sustain": 1, "loop": [0, 2]}`: up to 12 points of `[tick, value]`, the value from 0 to 64 (for panning, 32 is the middle). `sustain` is the point the envelope waits at until the key is let go, and `loop` the first and last points it repeats between; both can be left out. `"on": false` keeps an envelope's points but does not use it.

An instrument with no envelope is silenced at once by key off. A synth patch is also told when the key is let go, so its own release can run.

## Samples

| Key | Meaning | Default |
|---|---|---|
| `name` | A name to show. | `""` |
| `wave` | The sound, as a list of numbers from -128 to 127: for small hand-drawn waves. | |
| `file` | The sound, as the path of a WAV file beside the song. | |
| `volume` | 0 to 64. | 64 |
| `pan` | 0 (left) to 255 (right). Left out, the sample plays where its channel is panned. | the channel's |
| `loop` | `{"start": 0, "length": 32, "kind": "forward"}`, in frames; `kind` is `"forward"` or `"pingpong"` (there and back). `length` left out means to the end. | no loop |
| `transpose` | Semitones to add to every note. | see below |
| `finetune` | Finer tuning: 128 to the semitone, -128 to 127. | see below |

A sample has one of `wave` and `file`. A WAV file can be 8, 16, 24 or 32 bits or 32-bit floating point; if it has more than one channel, the first is used.

If neither `transpose` nor `finetune` is given, a `file` sample is tuned so that C-4 plays it at the speed it was recorded at: drop in any WAV and it sounds right on C-4. A `wave` sample with neither plays at 8363 frames a second on C-4, so a wave of 32 numbers sounds at about 261 Hz, middle C.

### Sample files

A writer names a sample file after what is in it: `samples/` and the first 16 hexadecimal digits of the SHA-256 of the WAV file, then `.wav`. The same sound used twice, or in two songs in one folder, is then one file, and a file's name changes exactly when its sound does. A writer puts a sample in the JSON as `wave` when it is 8-bit and 256 frames or fewer, and in a file otherwise.

A reader takes any path; it does not check the name against the contents, so a sample edited by hand still loads.

To share a song, zip the JSON with its `samples` folder.

## The packed form

The same song as one run of bytes, samples included: what a program hands the engine when the engine was built without its JSON reader (the WebAssembly module is), and what the engine hands back when asked to turn a MOD or an XM into one of our songs (`synths_song_pack`). It says nothing the JSON cannot and leaves nothing out, so the two convert both ways without loss. The code is `src/song/binary.rs`.

All numbers are little-endian. Text is a `u16` length and then that many bytes of UTF-8, with no zero byte.

**Header**

| bytes | |
|---|---|
| 4 | The letters `SYSG`. |
| `u16` | Version: 1. A reader refuses a higher one. |
| `u16` | Flags: bit 0 set for ProTracker rules (clear for FastTracker 2); bit 1 set for Amiga slides (clear for linear). |
| `u8`, `u8` | Speed (ticks in a row) and tempo (beats a minute). |
| `u16` | Restart: the place in the order to go back to. |
| `u16` | Channels, 1 to 64. |
| `u16` × 4 | How many orders, patterns, instruments and samples follow. |
| text | The title. |
| `u8` × channels | Each channel's panning, 0 to 255. |
| `u16` × orders | The order list. |

**Then each pattern**

| bytes | |
|---|---|
| `u16` | Rows. |
| `u32` | How many events follow. |

and each of its events, in order of row and then channel:

| bytes | |
|---|---|
| `u16` | Row. |
| `u8` | Channel. |
| `u8` | 1 if a chord follows this event, else 0. |
| `u8` × 5 | Note (0 none, 1 to 96 for C-0 to B-7, 97 off), instrument (0 none), the volume column, the effect, the effect's number. |

The volume column is as FastTracker 2 stores it: 0 for nothing, `0x10` to `0x50` for a volume of 0 to 64, and from `0x60` a command in the high four bits (`6` `-`, `7` `+`, `8` `D`, `9` `U`, `A` `S`, `B` `V`, `C` `P`, `D` `L`, `E` `R`, `F` `M`, in the JSON's letters) with its number in the low four. The effect is 0 to 35: the JSON's digit or letter as a base-36 digit.

A chord, when the event has one, is 16 bytes:

| bytes | |
|---|---|
| `u8` | How many offsets are used, 0 to 8; 0 is "chord off". |
| `u8` | Order: 0 up, 1 down, 2 updown, 3 written, 4 random. |
| `u8` | 0 if the rate is steps a tick (`chordSpeed`), 1 if steps a second (`chordHz`). |
| `u8` | 0. |
| `f32` | The rate. |
| `i8` × 8 | The offsets; those not used are 0. |

**Then each instrument**

| bytes | |
|---|---|
| text | Name. |
| text | The synth patch's name; a length of `0xFFFF` (and no bytes) for an instrument that is not a patch. |
| `u8` | Its samples: 0 for none; 1, followed by one `u16`, for one sample on every key; 2, followed by 96 `u16`, for a sample for each key from C-0, where `0xFFFF` is a key with none. |
| envelope × 2 | The volume envelope, then the panning envelope. |
| `u8` × 4 | Vibrato: wave (0 sine, 1 square, 2 ramp-up, 3 ramp-down), sweep, depth, rate. |
| `u16` | Fadeout. |

An envelope is `u8` flags (bit 0 on, bit 1 has a sustain point, bit 2 has a loop), `u8` how many points (0 to 12), `u8` the sustain point, `u8` and `u8` the loop's first and last points, and then each point as two `u16`: tick and value.

**Then each sample**

| bytes | |
|---|---|
| text | Name. |
| `u8` | Flags: bit 0 set for 16-bit sound (clear for 8-bit); bits 1 and 2 the loop (0 none, 1 forward, 2 pingpong); bit 3 set if the sample has its own panning. |
| `u8`, `i8`, `i8`, `u8` | Volume (0 to 64), finetune, transpose, panning. |
| `u8` × 3 | 0. |
| `u32` × 3 | Frames of sound, the loop's start, the loop's length. |
| sound | One signed number for each frame: a byte each, or two for 16-bit. |

A reader takes a file that ends early as far as it goes, treats counts and lengths that the bytes cannot back up as smaller, and never fails on a damaged one: what does not make sense is left out.

## What is not in version 1

- Stereo samples (the first channel is taken).
- Mixer and effect settings beyond panning.
- A separate order list for each channel.
- A packed binary form. The JSON is made so that one can be derived from it mechanically: notes, instruments and effects all have a number form as well as a text one.
