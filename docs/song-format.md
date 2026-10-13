# The song format

A song is one JSON document. It is meant to be read and changed by hand in a text editor, compared line by line between two versions, and written by a program without a library. Samples that are too big to write as numbers live beside it as ordinary WAV files, or inside it as base64 text when the song has to be one file.

The code is `src/song/native.rs`; the model it loads into is `src/song/mod.rs`. MOD and XM files load into the same model, so anything they can say, a song can say.

This is version 2. Version 1 had no way to say a chord whose notes sound together: its `chord` was always a fast one-voice arpeggio. Version 2 tells the two apart ([Chords and arpeggios](#chords-and-arpeggios)), and still reads every version 1 song as it was meant ([Songs from before](#songs-from-before)).

## A whole song

```json
{
  "format": "synths-song",
  "version": 2,
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
      {"row": 8, "ch": 1, "note": "C-3", "ins": 2, "arp": {"chord": "min", "rate": "1/16"}},
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

- `format` must be `"synths-song"`. `version` is a whole number, 1 if it is left out; a reader refuses a version higher than it knows and says so. A writer writes 2 for a song with a chord or an arpeggio in it and 1 for a song without, which says nothing a version 1 reader would get wrong.
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
| `chord` | A chord on the note: several notes sounding together. See [Chords and arpeggios](#chords-and-arpeggios). |
| `arp` | An arpeggio on the note: the notes of a chord one at a time. |
| `glide` | The note does not start afresh: the channel's sounding note slides to it. See [Glides, bends and vibrato](#glides-bends-and-vibrato). |
| `bend`, `slide` | A bend of the note's pitch, and a slide from wherever the pitch is. |
| `vibrato` | A vibrato on the note. |
| `gate` | Puts a [trance gate](#the-trance-gate) on the channel, changes it or takes it off. |
| `chordSpeed`, `chordHz`, `chordOrder` | Only in [songs from before](#songs-from-before). |

C-4 plays a sample at 8363 frames a second (unless the sample is tuned otherwise), and is middle C for a synth patch.

### Chords and arpeggios

A **chord** is several notes sounding together. An **arpeggio** is the notes of a chord played one at a time, in a pattern, at a speed. An event has one or the other.

```json
{"row": 0, "ch": 0, "note": "C-4", "ins": 1, "chord": "m7"},
{"row": 16, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "order": "up", "octaves": 2, "gate": 0.5}},
{"row": 32, "ch": 0, "note": "C-4", "ins": 1, "arp": {"chord": "maj", "hz": 50}},
{"row": 48, "ch": 0, "arp": "off"}
```

The first is the four notes of C minor seventh together. The second is a trance arpeggio: those notes one after another in sixteenths, up through two octaves, each held for half its step. The third is the chord of a Commodore 64 tune: one voice stepping through C major 50 times a second, so fast that the ear hears the notes together.

**The notes** are given the same way for both, as `chord` (on the event for notes together, inside `arp` for an arpeggio):

- A kind of chord by name, built on the event's note: `"maj"`, `"min"` (or `"m"`), `"dim"`, `"aug"`, `"sus2"`, `"sus4"`, `"5"`, `"6"`, `"m6"`, `"7"`, `"maj7"`, `"m7"`, `"dim7"`, `"m7b5"`, `"9"`, `"maj9"`, `"m9"`, `"add9"`, `"oct"`.
- A chord with its root, as on a song sheet: `"C"`, `"Cm7"`, `"F#maj7"`, `"Bb7"`. If the event has no `note`, the root in octave 4 becomes its note; if it has one, the root must be that note.
- A list of 1 to 8 numbers: each note of the chord in semitones from the event's note. The note itself is 0, and is only played if it is in the list.
- A list of 1 to 8 note names, which needs the event to have a `note` to measure them from.

**`chord`**, on an event, plays those notes together, all starting on the row. They are one thing on the channel: the channel's volume, panning, slides and effects move them all, key off lets them all go, and the channel's next note takes their place. It takes a voice for each note.

**`arp`** is `{"chord": ..., ...}` with any of:

| Key | Meaning | Default |
|---|---|---|
| `chord` | The notes, as above. It must be there. | |
| `rate` | How long each step is, as a note length: `"1/1"`, `"1/2"`, `"1/4"`, `"1/8"`, `"1/16"`, `"1/32"` or `"1/64"`; with a `t` for one of a triplet (`"1/8t"`, two thirds as long) or a dot for a dotted note (`"1/8."`, half as long again). | `"1/16"` |
| `perRow` | Or: steps in each row of the pattern. May be a fraction (0.5 is a step every two rows). | |
| `perTick` | Or: steps in each tick. | |
| `hz` | Or: steps a second, whatever the song's tempo. 50 is the classic rate of a chip's chord. | |
| `order` | `"up"` (lowest to highest, round and round), `"down"`, `"updown"` (up and back down, the top and bottom played once), `"updown2"` (the top and bottom played twice), `"written"` (as listed), `"random"`, or `"together"` (every note at once on each step: a chord stabbed in time). | `"up"` |
| `style` | `"strike"`: each step is a new note, its sample or its envelopes starting again. `"slide"`: one voice, struck once, whose pitch jumps from note to note. | `"strike"` with `rate` or `perRow`; `"slide"` with `perTick` or `hz` |
| `gate` | Struck notes: how much of its step each is held before it is let go, 0.05 to 1. At 1 each is held until the next is struck. | 0.5 |
| `octaves` | 1 to 4: the pattern runs through the chord, then the chord an octave up, and so on. | 1 |
| `swing` | 0 to 0.75: every second step is this part of a step late. | 0 |
| `accent` | Struck notes: how hard each step is struck, a list of up to 8 levels that goes round, each 0 (a rest) to 15 (full). The last must not be 0. | every step full |
| `pattern` | A pattern of steps to follow in place of `order` and `accent`: see [Arp patterns](#arp-patterns). | none |

An `arp` has one of `rate`, `perRow`, `perTick` and `hz`.

**Note lengths count four rows to the beat**, as trackers do: a sixteenth is a row, an eighth two rows, a quarter four, whatever the song's `speed`. So `"1/16"` puts a step on every row, `"1/8t"` three steps in every four rows, and `"1/32"` two in a row. (At speed 6 a row's six ticks give `"1/32"` and the triplets whole ticks; at other speeds a step may fall between two ticks, and does.) A change of tempo or speed in the song is taken up at the arpeggio's next step.

**How long it lasts.** A chord or an arpeggio carries on, row after row, until the channel's next note; that note plays plain unless it has one of its own. An event with an `arp` or a `chord` and no note changes the one that is running: an arpeggio takes its new notes, rate and style at its next step and keeps its place in the pattern, so a held arpeggio can follow a song's chords; a chord is struck anew on that row. `"arp": "off"` (or `"chord": "off"`) ends an arpeggio: a voice whose pitch was jumping plays on at the note's own pitch, and a struck arpeggio strikes no more, the note in hand finishing its gate. Key off lets go of a chord's notes and ends a struck arpeggio.

**What is struck.** With an instrument that is samples, each struck note starts the sample again (from where an offset effect put the start of the note); with a synth patch it is a new note of the patch. The instrument's volume and panning envelopes, its fade and its vibrato are the channel's and run across the whole arpeggio as across one note. A short gate cuts a sample short, as the cut effect does; with samples that should ring, use a gate of 1.

A writer writes the notes as their list of numbers, so a name survives loading and saving as the chord it meant, not as the name, and leaves out what is at its default.

This is all separate from the trackers' arpeggio effect `0xy`, which is kept for songs that came from MOD and XM files and behaves exactly as those trackers did: three notes, a step a tick, restarting on every row.

### Arp patterns

An arpeggio can follow a pattern of its own in place of one of the orders: up to 32 steps that go round, each a note of the chord, a rest or a tie. The pattern is as long as it is, whatever the chord's size, and the rhythm is in it:

```json
{"row": 0, "ch": 0, "note": "A-3", "ins": 1, "arp": {"chord": "m7", "rate": "1/16", "pattern": "1 - 3 2 _ 1 3 5^"}},
{"row": 16, "ch": 0, "arp": {"chord": [-4, 0, 3, 8], "rate": "1/16", "pattern": "1 - 3 2 _ 1 3 5^"}},
{"row": 32, "ch": 0, "note": "A-3", "ins": 1, "arp": {"chord": "min", "pattern": "trance"}}
```

The steps are written with spaces between them:

| Written | Means |
|---|---|
| `1`, `2` ... `32` | That note of the chord, counted from its lowest. Past the chord's last note the count carries on an octave up: on a three-note chord, `4` is the root an octave higher and `5` the next note above that. |
| `-` (or `.`) | A rest. |
| `_` | A tie: the last note is held through this step. |
| `'` after a note | An octave up (`1''` is two octaves up). |
| `,` after a note | An octave down. |
| `^` after a note | An accent. When a pattern has accents, the accented steps are struck at full and the rest at two thirds. |
| `:1` to `:9` after a note | That step's own gate, in tenths of a step, in place of the arp's `gate`. |
| `~` before a note | Slide into it: the note is not struck; the one sounding slides to it, as a bass line does on a 303. |

So `"1 - 3 2 _ 1 3 5^"` is: the root, a rest, the third note, the second note held for two steps, the root, the third, and the fifth note accented.

In place of the steps a pattern can be the name of one that is built in: `"bounce"` (`1 2 3 2`), `"octaves"` (`1 1' 1 1'`), `"root-fifth"` (`1 3 1' 3`), `"thumb"` (`1 2 1 3 1 4 1 3`), `"converge"` (`1 4 2 3`), `"diverge"` (`2 3 1 4`), `"offbeat"` (`- 1 - 2 - 3 - 2`), `"gallop"` (`1^ - 1 1 2^ - 2 2`), `"trance"` (`1^ - 3 2 - 1^ 3 -`) and `"acid"` (`1^ 1 ~3 - 1' ~1 2^ -`).

It can also be a list, a step each, for a program that would sooner write numbers and for the few things text cannot say: a number is that note of the chord (0 a rest; below 0 counts down from the chord an octave below, -1 its top note), `"-"` a rest, `"_"` a tie, and `{"note": 3, "octave": 1, "level": 12, "gate": 0.3, "slide": true, "tie": false}` a step in full, where `level` is 1 to 15 (how hard it is struck, of 15) and `gate` 0.1 to 1.

`rate`, `gate`, `octaves` (each time round the pattern is an octave higher), `swing` and `style` are the arp's, as above; with `"style": "slide"` the one voice's pitch follows the pattern's notes, and rests and ties leave it where it is. A change of chord under a running pattern (an event with the arp and no note, as on row 16 above) keeps the pattern's place. A writer writes a pattern as its steps (a name is not kept), as text when text can say it.

### Glides, bends and vibrato

The pitch of a note can be moved by keys that say what they do. They work on sample instruments and on synth patches alike, and their times are counted in the song's own time: a note length (`"1/16"`, `"1/8t"`, `"1/4."`, four rows to the beat as for an arp), or `{"rows": 2}`, `{"ticks": 3}` or `{"seconds": 0.25}`.

| What you want to hear | What to write |
|---|---|
| This note slides up or down to the next one | `"glide": "1/16"` on the next note |
| The same, the tracker's way (a speed, not a time) | `"glide": true`, or `"glide": 24` |
| A quick rise into the note from below | `"bend": "scoop"` |
| The note drooping away at its end | `"bend": "fall"` on an event with no note, where the droop begins |
| A dip down and back | `"bend": "dip"` |
| The note bending up two semitones and staying there | `"bend": {"to": 2, "over": "1/8"}` |
| A bend to a named note | `"bend": {"to": "E-4", "over": "1/8"}` |
| A long note that begins to sing after a moment | `"vibrato": true` |
| A wide, slow vibrato straight away | `"vibrato": {"depth": 0.6, "rate": 4, "delay": {"ticks": 0}}` |
| A siren, or a laser drop | `"slide": 12` on one row and `"slide": -12` on the next; `"slide": {"to": -24, "over": "1/4"}` |

```json
{"row": 0, "ch": 0, "note": "A-4", "ins": 1, "bend": "scoop"},
{"row": 4, "ch": 0, "note": "C-5", "ins": 1, "glide": "1/16"},
{"row": 8, "ch": 0, "note": "E-5", "ins": 1, "glide": "1/8", "vibrato": true},
{"row": 20, "ch": 0, "bend": "fall"},
{"row": 24, "ch": 0, "note": "A-3", "ins": 1},
{"row": 25, "ch": 0, "slide": {"to": 24, "over": "1/4"}}
```

**`glide`** goes on the note that is glided *to*. That note is not struck: the channel's sounding note slides to it, in a straight line in pitch, over the time given, and carries on. With nothing sounding on the channel, the note simply starts. `"glide": true` and `"glide": 24` (1 to 255) are the trackers' own glide, the effect `3xx` (true is `300`: at the speed the channel last had), and they are exactly that effect: an event has one of them or an `fx`, not both. A time is a different thing: it always takes that long, however far apart the notes are.

**`bend`** is `{"to": ..., "over": ...}`: `to` is how far, in semitones (fractions allowed, 96 either way), or a note name, which needs the event to have a `note` to measure from; `over` is how long it takes (an eighth if left out). The pitch goes there in a straight line and stays. On an event with a note it begins with the note; on an event with no note it bends the note that is sounding, from that row. Three shapes have names, and `"bend": "scoop"` and so on use them as they are:

| Shape | Does | As it is by default |
|---|---|---|
| `"scoop"` | The note starts `to` away from its pitch and comes to it. | from 2 semitones below, over a sixteenth |
| `"fall"` | The pitch goes `to` away and stays: a bend, with a size of its own. | 5 semitones down, over an eighth |
| `"dip"` | The pitch goes `to` away and comes back, half the time each way. | a semitone down and back, over an eighth |

`{"shape": "scoop", "to": -5, "over": "1/8"}` is a scoop of another size; leave out what is to stay as it is.

**`slide`** moves the pitch from wherever it is now, where a bend moves it from the note: `"slide": 12` is an octave up over one row, and `{"to": -12, "over": "1/4"}` an octave down over a quarter. Slides add up, so a row up and a row down is a siren. An event has a `bend` or a `slide`, not both.

**`vibrato`** is `{"depth": 0.3, "rate": 6, "delay": "1/8"}`: semitones each way (up to 12), times a second (0.1 to 40), and how long the note sounds before it begins; `true` is exactly those numbers, and what is left out of the braces is as there. It lasts for the note. `"vibrato": false` on a later event stops it, and `"vibrato"` on an event with no note gives one to the note that is sounding. (An instrument's own `vibrato`, under [Instruments](#instruments), is the tracker's: every note of the instrument has it.)

How they go together, and what is not done:

- A new note (one with no `glide`) starts at its own pitch: whatever bend or slide the last note had is gone, and so is its vibrato.
- A glide to a note with a `chord`, or onto a channel playing one, moves the whole chord, each note keeping its place. With an `arp`, the glide moves the note the arpeggio is built on: the notes that are sounding slide, and the steps that follow are struck from where it is going. A bend, a slide and a vibrato act on the notes that are sounding when they begin; notes an arpeggio strikes afterwards are struck at the bent pitch, without vibrato.
- A bend and the trackers' pitch effects (`1xx`, `2xx`, `4xy` and the rest) can be on the same note: the effect moves the pitch the song gives the note, and the bend stays on top of it. But an effect that sets the pitch while a bend is still on its way ends the bend's movement there, at where it was going.
- A glide over a time keeps the instrument's envelopes going as the trackers' glide does, and keeps the sample that is playing: it does not change to the new note's sample.
- Seeking stops every channel, so a bend begun before the place sought is not picked up. A song put in the place of the playing one leaves a bend that is under way to finish.
- Every `fx` and volume-column code goes on working and is written back as it was, except `3xx`, which is written as `glide`. A MOD or an XM that is loaded keeps its effects as they are.

### The trance gate

A gate chops a sustained sound in a rhythm: a pattern of steps, each open, closed or part open, in time with the song. It is put on an instrument, so that every note of it is gated, or switched on and off by events, so that a pad is gated for some bars only.

```json
"instruments": [{"name": "pad", "patch": "pad", "gate": "x.x.xx.x x.x.x.xx"}]
```

```json
{"row": 0, "ch": 1, "note": "A-3", "ins": 2, "chord": "min", "gate": {"pattern": "trance", "rate": "1/16", "depth": 0.9}},
{"row": 32, "ch": 1, "gate": "off"}
```

A gate is its pattern alone, or `{"pattern": ..., ...}` with any of:

| Key | Meaning | Default |
|---|---|---|
| `pattern` | One character a step, up to 32: `x` open, `.` closed, `0` to `9` a level between (in ninths: `9` is open). Spaces are passed over. Or a built-in pattern's name: `"eighths"`, `"offbeat"`, `"gallop"`, `"trance"`, `"stutter"`, `"pump"`, `"triplets"` (for a rate of `"1/16t"`), `"dotted"`. Or a list of levels from 0 to 1. | |
| `rate` | How long a step is, as a note length. | `"1/16"` |
| `depth` | How far a closed step closes, 0 to 1: at 1 it is silence. | 1 |
| `smooth` | How long the gate takes to open or close, in seconds, 0.001 to 0.5; or `"hard"`, the least, which is still a ramp and does not click. | 0.008 |
| `swing` | 0 to 0.75: every second step is this part of a step late. | 0 |
| `sends` | `true`: the echo and the reverb are sent the chopped sound, and ring through the gaps. `false`: they are sent the sound from before the gate. | `true` |
| `stereo` | `true`: the steps open on the left and the right by turns. | `false` |

The steps keep to the song: the pattern starts at the top of each pattern of the song (row 0 is its first step when the pattern's length divides the rows into whole bars), a sixteenth is a row, and a change of tempo or speed, a pattern jump or a seek leaves it on the rows.

On an event, `gate` is a gate as above, which stays on the channel from that row, through its later notes, until another event changes it; `"off"`, which takes any gate off the channel, the instrument's own included; or `"instrument"`, which goes back to whatever the instrument of the channel's note has.

The gate is on the part the channel's note plays through: the channel's own part for an instrument that is samples, and for a synth patch the part the host gave the patch, where it chops everything that part is playing and is left open again when the song stops. Two channels playing the same patch share its part, and so its gate.

### Songs from before

In version 1 there was only `chord`, and it always meant the fast one-voice arpeggio, timed by three keys beside it on the event:

| Key | Meaning | Default |
|---|---|---|
| `chordSpeed` | Steps in each tick. May be a fraction. | 1 |
| `chordHz` | Steps a second, whatever the tempo. Give this or `chordSpeed`, not both. | |
| `chordOrder` | `"up"`, `"down"`, `"updown"`, `"written"` or `"random"`. | `"up"` |

A reader still takes them, by this rule:

- In a song whose `version` is 1 (or is left out), every `chord` is that arpeggio, with or without the three keys.
- In a version 2 song, a `chord` with any of `chordSpeed`, `chordHz` and `chordOrder` beside it is that arpeggio too. A `chord` alone is notes together.

Either way it is read as `"arp": {"chord": ..., "perTick": chordSpeed, "order": chordOrder}` (or `"hz": chordHz`), which is one voice (the style `perTick` and `hz` have unless they say otherwise), sounds exactly as it did, and is written back in that shape: a writer never writes the three keys. (`arp` may be used in a version 1 song; its meaning never changed. The three keys beside an `arp` are a mistake.)

One thing about such an arpeggio is as it always was: with `perTick`, one octave and no swing, its speed is worked out from the tempo of the row it begins on, and a later change of tempo does not move it.

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
| `synth` | What a host wants to remember about the synth sound: by convention `{"preset": "name", "set": {"setting": value, ...}}`, a preset to start from and the settings changed from it. It belongs to the host. A reader keeps it, whatever it is, and writes it back (closed up, with no spaces); the engine hands it to the host with `synths_song_synth`. | |
| `volumeEnvelope`, `panEnvelope` | Envelopes, below. | none |
| `vibrato` | The instrument's own vibrato: `{"wave": "sine", "sweep": 0, "depth": 4, "rate": 20}`. `wave` is `"sine"`, `"square"`, `"ramp-up"` or `"ramp-down"`; `sweep` is how many ticks it takes to reach full depth. | none |
| `fadeout` | How fast the note fades after key off, 0 to 65535. 0 never fades; 32768 fades in one tick. | 0 |
| `gate` | A [trance gate](#the-trance-gate) every note of the instrument is played through. | none |

An envelope is `{"points": [[0, 64], [8, 32], [20, 0]], "sustain": 1, "loop": [0, 2]}`: up to 12 points of `[tick, value]`, the value from 0 to 64 (for panning, 32 is the middle). `sustain` is the point the envelope waits at until the key is let go, and `loop` the first and last points it repeats between; both can be left out. `"on": false` keeps an envelope's points but does not use it.

An instrument with no envelope is silenced at once by key off. A synth patch is also told when the key is let go, so its own release can run.

## Samples

| Key | Meaning | Default |
|---|---|---|
| `name` | A name to show. | `""` |
| `wave` | The sound, as a list of numbers from -128 to 127: for small hand-drawn waves. | |
| `file` | The sound, as the path of a WAV file beside the song. | |
| `data` | The sound, as a whole WAV file in base64 text: for a song that is all one file. | |
| `volume` | 0 to 64. | 64 |
| `pan` | 0 (left) to 255 (right). Left out, the sample plays where its channel is panned. | the channel's |
| `loop` | `{"start": 0, "length": 32, "kind": "forward"}`, in frames; `kind` is `"forward"` or `"pingpong"` (there and back). `length` left out means to the end. | no loop |
| `transpose` | Semitones to add to every note. | see below |
| `finetune` | Finer tuning: 128 to the semitone, -128 to 127. | see below |

A sample has one of `wave`, `file` and `data` (a reader that finds more than one takes `file`, then `data`). A WAV file can be 8, 16, 24 or 32 bits or 32-bit floating point; 8 bits stay 8 and the rest become 16. If it has more than one channel they are mixed down to one, each counting the same, because a song's samples have one channel.

`data` is the bytes of the WAV file in base64: the usual alphabet (`+` and `/`) or the web's (`-` and `_`), with or without the `=` signs at the end; spaces and line breaks in it are skipped, and it may begin as a data address does (`data:audio/wav;base64,`). Anything else in it is a mistake, and the song is refused with a message that names `"data"`. It is a third bigger than the file, and it makes the song one thing to keep, send and store.

If neither `transpose` nor `finetune` is given, a `file` or `data` sample is tuned so that C-4 plays it at the speed it was recorded at: drop in any WAV and it sounds right on C-4. A `wave` sample with neither plays at 8363 frames a second on C-4, so a wave of 32 numbers sounds at about 261 Hz, middle C.

### Sample files

A writer names a sample file after what is in it: `samples/` and the first 16 hexadecimal digits of the SHA-256 of the WAV file, then `.wav`. The same sound used twice, or in two songs in one folder, is then one file, and a file's name changes exactly when its sound does. A writer puts a sample in the JSON as `wave` when it is 8-bit and 256 frames or fewer, and in a file otherwise.

A reader takes any path; it does not check the name against the contents, so a sample edited by hand still loads.

To share a song, zip the JSON with its `samples` folder, or write it as one file: `native::save_in_one` writes every sample that would have been a file as `data`, and nothing beside the JSON.

## The packed form

The same song as one run of bytes, samples included: what a program hands the engine when the engine was built without its JSON reader (the WebAssembly module is), and what the engine hands back when asked to turn a MOD or an XM into one of our songs (`synths_song_pack`). It says nothing the JSON cannot and leaves nothing out, so the two convert both ways without loss. The code is `src/song/binary.rs`.

All numbers are little-endian. Text is a `u16` length and then that many bytes of UTF-8, with no zero byte.

**Header**

| bytes | |
|---|---|
| 4 | The letters `SYSG`. |
| `u16` | Version: 1, or 2 if any event is followed by a chord in full (below). A reader refuses a higher one. |
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
| `u8` | What follows this event. The low two bits: 0 no chord, 1 an arpeggio as version 1 packed it (16 bytes), 2 a chord or arpeggio in full (32 bytes). Bit 2 (4): a `u16` comes first, straight after the next five bytes: the number, from 1, of the event's entry in the list of extras (its glide, bend, slide, vibrato or gate). |
| `u8` × 5 | Note (0 none, 1 to 96 for C-0 to B-7, 97 off), instrument (0 none), the volume column, the effect, the effect's number. |

The volume column is as FastTracker 2 stores it: 0 for nothing, `0x10` to `0x50` for a volume of 0 to 64, and from `0x60` a command in the high four bits (`6` `-`, `7` `+`, `8` `D`, `9` `U`, `A` `S`, `B` `V`, `C` `P`, `D` `L`, `E` `R`, `F` `M`, in the JSON's letters) with its number in the low four. The effect is 0 to 35: the JSON's digit or letter as a base-36 digit.

An arpeggio as version 1 packed it is 16 bytes. It is always one voice whose pitch jumps (`"style": "slide"`), one octave, and it is what a writer still uses for an arpeggio that needs no more, and for "off", so that a song with nothing newer in it packs to the same bytes as before:

| bytes | |
|---|---|
| `u8` | How many offsets are used, 0 to 8; 0 is "off". |
| `u8` | Order: 0 up, 1 down, 2 updown, 3 written, 4 random. |
| `u8` | 0 if the rate is steps a tick (`perTick`), 1 if steps a second (`hz`). |
| `u8` | 0. |
| `f32` | The rate. |
| `i8` × 8 | The offsets; those not used are 0. |

A chord or arpeggio in full is 32 bytes:

| bytes | |
|---|---|
| `u8` | How many offsets are used, 1 to 8. |
| `u8` | 0 for a chord (its notes together; the next 22 bytes are then not used), 1 for an arpeggio. |
| `u8` | Order: 0 up, 1 down, 2 updown, 3 written, 4 random, 5 updown2, 6 together; 7 for an arpeggio that follows a pattern. |
| `u8` | Style: 0 slide, 1 strike. |
| `u8` | What the rate is: 0 steps a tick, 1 steps a second, 2 steps a row, 3 a note length. |
| `u8`, `u8` | For a note length: how many make a whole note (1, 2, 4 ... 64), and 0 for plain, 1 for one of a triplet, 2 for dotted. |
| `u8` | Octaves, 1 to 4. |
| `f32` | The rate, when it is not a note length. |
| `f32` | Gate, 0.05 to 1. |
| `f32` | Swing, 0 to 0.75. |
| `u32` | Accents: four bits a step, the first step lowest, each 0 to 15; as long as its last four bits that are not 0; 0 for none. With order 7: the number, from 1, of its pattern in the list of arp patterns. |
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

**Then, only if an instrument has a `synth`**

| bytes | |
|---|---|
| 4 | The letters `SYNT`. |
| `u32`, bytes × instruments | For each instrument in turn, the length of its `synth` as JSON text and then the text; a length of 0 for an instrument with none. |

This comes after everything a reader of the first layout looks at, so such a reader loads the song without it, and a song with no `synth` packs to the same bytes as before.

**Then what version 2 added**, each list begun by its four letters and there only if it has something in it, in this order. (A plain `"glide": true` or a speed is the cell's effect 3, and needs none of this.)

`ARPP`, the arp patterns: a `u16` count, then for each a `u8` number of steps (1 to 32) and four bytes a step: the note (`i8`; 0 a rest), flags (1 a tie, 2 a slide), octaves (`i8`, -3 to 3), and a byte whose low four bits are the level (1 to 15; 0 full) and high four the step's gate in tenths (1 to 10; 0 the arp's).

`XTRA`, the extras: a `u16` count, then for each a `u8` saying what it has (1 a glide, 2 a bend or slide, 4 a vibrato, 8 a gate) and those, in that order:

| | bytes | |
|---|---|---|
| a time | 8 | `u8` kind (0 a note length, 1 rows, 2 ticks, 3 seconds), `u8` and `u8` the note length (how many to a whole note; 0 plain, 1 triplet, 2 dotted), `u8` 0, `f32` the number of rows, ticks or seconds. |
| glide | 8 | Its time. |
| bend | 16 | `u8` shape (0 to and stay, 1 scoop, 2 dip, 3 slide), `u8` × 3 0, `f32` semitones, and its time. |
| vibrato | 16 | `f32` depth, `f32` rate, and the time of its delay. |
| gate | 1, or 17 and its levels | `u8`: 0 off, 1 the instrument's, 2 a gate, which follows. |

A gate is `u8` how many levels (1 to 32), `u8` flags (bit 0 the sends are gated, bit 1 stereo), `u8` and `u8` its rate as a note length, `f32` depth, `f32` smooth, `f32` swing, and an `f32` for each level.

`IGAT`, the instruments' gates: for each instrument in turn a `u8`, 1 if it has a gate, and then the gate.

A song with any of this is version 2.

A reader takes a file that ends early as far as it goes, treats counts and lengths that the bytes cannot back up as smaller, and never fails on a damaged one: what does not make sense is left out.

## What is not in the format

- Stereo samples (the channels are mixed to one).
- Mixer and effect settings beyond panning.
- A separate order list for each channel.
