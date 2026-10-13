//! The parameter table: every value a patch or the engine has, defined once.
//!
//! Each parameter has a number that never changes (its id), a name, a range,
//! a default and a unit. The JavaScript and Python libraries and any editor
//! are generated from this table, which the engine hands out as JSON
//! (`synths_param_table`). To add a parameter, add a line here with an id
//! that has not been used before; never renumber.
//!
//! Ids 0..=127 belong to a part (each part has its own value). Ids 200..=231
//! belong to the whole engine.

/// What sort of value a parameter holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Any number in the range.
    Float,
    /// A whole number in the range.
    Int,
    /// 0 (off) or 1 (on).
    Bool,
    /// A whole number that picks one of `choices`.
    Choice,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Float => "float",
            Kind::Int => "int",
            Kind::Bool => "bool",
            Kind::Choice => "choice",
        }
    }
}

/// One line of the table.
#[derive(Clone, Copy, Debug)]
pub struct ParamInfo {
    pub id: u16,
    pub name: &'static str,
    pub kind: Kind,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    /// For `Kind::Choice`: the names of the choices, in order from 0, separated by `|`.
    pub choices: &'static str,
    pub help: &'static str,
}

impl ParamInfo {
    pub fn is_global(&self) -> bool {
        self.id >= GLOBAL_BASE as u16
    }

    /// Bring a value into range (and to a whole number where the kind needs one).
    pub fn clean(&self, value: f32) -> f32 {
        let v = value.clamp(self.min, self.max);
        match self.kind {
            Kind::Float => v,
            _ => v.round(),
        }
    }
}

/// How many values each part holds (part ids are below this).
pub const PART_PARAMS: usize = 128;
/// The first engine-wide id.
pub const GLOBAL_BASE: usize = 200;
/// How many engine-wide values there are room for.
pub const GLOBAL_PARAMS: usize = 32;

macro_rules! param_table {
    ($( $id:literal, $konst:ident, $name:literal, $kind:ident, $min:expr, $max:expr, $def:expr, $unit:literal, $choices:literal, $help:literal; )*) => {
        $( pub const $konst: usize = $id; )*
        /// Every parameter, in id order.
        pub const PARAMS: &[ParamInfo] = &[
            $( ParamInfo {
                id: $id,
                name: $name,
                kind: Kind::$kind,
                min: $min as f32,
                max: $max as f32,
                default: $def as f32,
                unit: $unit,
                choices: $choices,
                help: $help,
            }, )*
        ];
    };
}

param_table! {
    // id, constant, name, kind, min, max, default, unit, choices, help

    // ---- the part ----
    0,  VOLUME, "volume", Float, 0, 2, 1, "gain", "", "How loud the part is (1 is unchanged).";
    1,  PAN, "pan", Float, -1, 1, 0, "pan", "", "Left (-1) to right (1).";
    2,  TRANSPOSE, "transpose", Float, -48, 48, 0, "semitones", "", "Shifts every note of the part.";
    3,  TUNE, "tune", Float, -100, 100, 0, "cents", "", "Fine tuning of the part.";
    4,  BEND, "bend", Float, -1, 1, 0, "", "", "The pitch bend wheel: -1 to 1 times bend_range.";
    5,  BEND_RANGE, "bend_range", Float, 0, 48, 2, "semitones", "", "How far a full pitch bend moves the pitch.";
    6,  GLIDE, "glide", Float, 0, 10, 0, "seconds", "", "How long the pitch takes to slide from the last note to a new one.";
    7,  MODE, "mode", Choice, 0, 2, 0, "", "poly|mono|legato", "poly: every note gets its own voice. mono: one note at a time. legato: one note at a time, and a new note does not restart the envelopes.";
    8,  UNISON, "unison.voices", Int, 1, 8, 1, "voices", "", "How many detuned voices each note plays.";
    9,  UNISON_DETUNE, "unison.detune", Float, 0, 100, 20, "cents", "", "How far apart the unison voices are tuned.";
    10, UNISON_SPREAD, "unison.spread", Float, 0, 1, 0.5, "", "", "How far apart the unison voices are panned.";
    11, VELOCITY_SENS, "velocity", Float, 0, 1, 1, "", "", "How much a note's velocity changes its loudness (0: not at all).";
    12, PAN_LAW, "pan.law", Choice, 0, 1, 0, "", "equal_power|square_root", "How pan shares a sound between left and right. square_root is FastTracker 2's, for songs written with it.";

    // ---- oscillator 1 ----
    16, OSC1_WAVE, "osc1.wave", Choice, 0, 4, 1, "", "sine|triangle|saw|pulse|sample", "The shape of the wave, or a loaded sample.";
    17, OSC1_LEVEL, "osc1.level", Float, 0, 1, 1, "gain", "", "How loud oscillator 1 is.";
    18, OSC1_COARSE, "osc1.coarse", Float, -48, 48, 0, "semitones", "", "Tuning in semitones.";
    19, OSC1_FINE, "osc1.fine", Float, -100, 100, 0, "cents", "", "Fine tuning.";
    20, OSC1_PW, "osc1.pulse_width", Float, 0, 1, 0.5, "", "", "The width of the pulse wave (0.5 is a square; at 0 and 1 it is silent).";
    21, OSC1_SAMPLE, "osc1.sample", Int, 0, 1023, 0, "slot", "", "Which sample slot the sample wave plays.";
    22, OSC1_START, "osc1.start", Float, 0, 1, 0, "", "", "Where in the sample a note starts (0: the beginning, 1: the end).";
    23, OSC1_KEYTRACK, "osc1.keytrack", Bool, 0, 1, 1, "", "", "On: the pitch follows the note. Off: every note plays at the same pitch (drums).";

    // ---- oscillator 2 ----
    24, OSC2_WAVE, "osc2.wave", Choice, 0, 4, 2, "", "sine|triangle|saw|pulse|sample", "The shape of the wave, or a loaded sample.";
    25, OSC2_LEVEL, "osc2.level", Float, 0, 1, 0, "gain", "", "How loud oscillator 2 is.";
    26, OSC2_COARSE, "osc2.coarse", Float, -48, 48, 0, "semitones", "", "Tuning in semitones.";
    27, OSC2_FINE, "osc2.fine", Float, -100, 100, 0, "cents", "", "Fine tuning.";
    28, OSC2_PW, "osc2.pulse_width", Float, 0, 1, 0.5, "", "", "The width of the pulse wave (0.5 is a square; at 0 and 1 it is silent).";
    29, OSC2_SAMPLE, "osc2.sample", Int, 0, 1023, 0, "slot", "", "Which sample slot the sample wave plays.";
    30, OSC2_START, "osc2.start", Float, 0, 1, 0, "", "", "Where in the sample a note starts.";
    31, OSC2_KEYTRACK, "osc2.keytrack", Bool, 0, 1, 1, "", "", "On: the pitch follows the note.";
    32, OSC2_SYNC, "osc2.sync", Bool, 0, 1, 0, "", "", "Hard sync: oscillator 2 restarts its wave every time oscillator 1 does.";
    33, OSC2_RING, "osc2.ring", Float, 0, 1, 0, "", "", "Ring modulation: how much oscillator 2 is multiplied by oscillator 1.";

    34, OSC1_REVERSE, "osc1.reverse", Bool, 0, 1, 0, "", "", "On: oscillator 1's sample plays backwards.";
    35, OSC2_REVERSE, "osc2.reverse", Bool, 0, 1, 0, "", "", "On: oscillator 2's sample plays backwards.";

    // ---- sub oscillator and noise ----
    36, SUB_LEVEL, "sub.level", Float, 0, 1, 0, "gain", "", "How loud the sub oscillator is.";
    37, SUB_WAVE, "sub.wave", Choice, 0, 2, 0, "", "sine|square|triangle", "The shape of the sub oscillator.";
    38, SUB_OCTAVE, "sub.octave", Int, -2, -1, -1, "octaves", "", "One or two octaves below the note.";
    39, SAMPLE_ANTIALIAS, "sample.antialias", Bool, 0, 1, 1, "", "", "On: a sample played faster than it was recorded is smoothed, so it does not turn harsh. Off: it is left raw, as an old sampler or a tracker of the 1980s would play it.";
    40, NOISE_LEVEL, "noise.level", Float, 0, 1, 0, "gain", "", "How loud the noise is.";
    41, NOISE_TYPE, "noise.type", Choice, 0, 3, 0, "", "white|pink|lfsr|lfsr_short", "white and pink are hiss; lfsr is the pitched noise of an 8-bit sound chip; lfsr_short is its buzzy, metallic version.";
    42, NOISE_PITCH, "noise.pitch", Float, -48, 48, 0, "semitones", "", "Tuning of the lfsr noise, which follows the note.";

    // ---- filter ----
    48, FILTER_TYPE, "filter.type", Choice, 0, 7, 0, "", "off|lowpass|highpass|bandpass|notch|low_band|band_high|all", "What the filter lets through. The last four add outputs together, as a sound chip does: notch is low + high, low_band is low + band, band_high is band + high, all is all three.";
    49, FILTER_SLOPE, "filter.slope", Choice, 0, 1, 0, "", "12db|24db", "How steeply the filter cuts.";
    50, CUTOFF, "filter.cutoff", Float, 20, 20000, 2000, "hz", "", "The frequency where the filter starts to cut.";
    51, RESONANCE, "filter.resonance", Float, 0, 1, 0.15, "", "", "How much the filter emphasises the cutoff frequency.";
    52, FILTER_KEYTRACK, "filter.keytrack", Float, 0, 1, 0, "", "", "How much the cutoff follows the note (1: exactly, around middle C).";
    53, FILTER_ENV, "filter.env", Float, -96, 96, 0, "semitones", "", "How far the filter envelope moves the cutoff.";
    54, FILTER_VELOCITY, "filter.velocity", Float, -96, 96, 0, "semitones", "", "How far a full-velocity note moves the cutoff.";

    // ---- amplitude envelope ----
    56, AMP_ATTACK, "amp.attack", Float, 0, 20, 0.005, "seconds", "", "How long a note takes to reach full loudness.";
    57, AMP_DECAY, "amp.decay", Float, 0, 20, 0.1, "seconds", "", "How long it then takes to fall to the sustain level.";
    58, AMP_SUSTAIN, "amp.sustain", Float, 0, 1, 0.8, "", "", "The loudness while the note is held.";
    59, AMP_RELEASE, "amp.release", Float, 0, 20, 0.15, "seconds", "", "How long the note takes to die away after it is let go.";
    60, AMP_CURVE, "amp.curve", Float, 0, 1, 0.5, "", "", "The shape of the envelope: 0 is straight lines, 1 is strongly curved.";

    // ---- filter envelope ----
    64, FENV_ATTACK, "filter_env.attack", Float, 0, 20, 0.005, "seconds", "", "Attack time of the filter envelope.";
    65, FENV_DECAY, "filter_env.decay", Float, 0, 20, 0.2, "seconds", "", "Decay time of the filter envelope.";
    66, FENV_SUSTAIN, "filter_env.sustain", Float, 0, 1, 0.3, "", "", "Sustain level of the filter envelope.";
    67, FENV_RELEASE, "filter_env.release", Float, 0, 20, 0.2, "seconds", "", "Release time of the filter envelope.";
    68, FENV_CURVE, "filter_env.curve", Float, 0, 1, 0.5, "", "", "The shape of the filter envelope.";

    // ---- modulation envelope ----
    72, MENV_ATTACK, "mod_env.attack", Float, 0, 20, 0.005, "seconds", "", "Attack time of the modulation envelope.";
    73, MENV_DECAY, "mod_env.decay", Float, 0, 20, 0.2, "seconds", "", "Decay time of the modulation envelope.";
    74, MENV_SUSTAIN, "mod_env.sustain", Float, 0, 1, 0, "", "", "Sustain level of the modulation envelope.";
    75, MENV_RELEASE, "mod_env.release", Float, 0, 20, 0.2, "seconds", "", "Release time of the modulation envelope.";
    76, MENV_CURVE, "mod_env.curve", Float, 0, 1, 0.5, "", "", "The shape of the modulation envelope.";
    77, MENV_PITCH, "mod_env.pitch", Float, -48, 48, 0, "semitones", "", "How far the modulation envelope moves the pitch of the whole voice.";
    78, MENV_OSC2_PITCH, "mod_env.osc2_pitch", Float, -48, 48, 0, "semitones", "", "How far it moves the pitch of oscillator 2 only (sweeps a synced oscillator).";
    79, MENV_PW, "mod_env.pulse_width", Float, -1, 1, 0, "", "", "How far it moves the pulse width.";

    // ---- LFO 1 ----
    80, LFO1_WAVE, "lfo1.wave", Choice, 0, 4, 0, "", "sine|triangle|saw|square|random", "The shape of the slow wave. random steps to a new value each cycle.";
    81, LFO1_RATE, "lfo1.rate", Float, 0.01, 50, 5, "hz", "", "How fast it goes.";
    82, LFO1_DELAY, "lfo1.delay", Float, 0, 10, 0, "seconds", "", "How long after the note starts before it begins.";
    83, LFO1_FADE, "lfo1.fade", Float, 0, 10, 0, "seconds", "", "How long it then takes to reach full strength.";
    84, LFO1_SYNC, "lfo1.sync", Bool, 0, 1, 1, "", "", "On: it restarts with every note. Off: all the notes of the part share one.";
    85, LFO1_PITCH, "lfo1.pitch", Float, -48, 48, 0, "semitones", "", "Vibrato: how far it moves the pitch.";
    86, LFO1_PW, "lfo1.pulse_width", Float, -1, 1, 0, "", "", "How far it moves the pulse width.";
    87, LFO1_CUTOFF, "lfo1.cutoff", Float, -96, 96, 0, "semitones", "", "How far it moves the filter cutoff.";
    88, LFO1_AMP, "lfo1.amp", Float, 0, 1, 0, "", "", "Tremolo: how much it changes the loudness.";
    89, LFO1_PAN, "lfo1.pan", Float, 0, 1, 0, "", "", "How far it moves the sound left and right.";

    // ---- LFO 2 ----
    96,  LFO2_WAVE, "lfo2.wave", Choice, 0, 4, 1, "", "sine|triangle|saw|square|random", "The shape of the slow wave.";
    97,  LFO2_RATE, "lfo2.rate", Float, 0.01, 50, 0.5, "hz", "", "How fast it goes.";
    98,  LFO2_DELAY, "lfo2.delay", Float, 0, 10, 0, "seconds", "", "How long after the note starts before it begins.";
    99,  LFO2_FADE, "lfo2.fade", Float, 0, 10, 0, "seconds", "", "How long it then takes to reach full strength.";
    100, LFO2_SYNC, "lfo2.sync", Bool, 0, 1, 0, "", "", "On: it restarts with every note. Off: all the notes of the part share one.";
    101, LFO2_PITCH, "lfo2.pitch", Float, -48, 48, 0, "semitones", "", "How far it moves the pitch.";
    102, LFO2_PW, "lfo2.pulse_width", Float, -1, 1, 0, "", "", "How far it moves the pulse width.";
    103, LFO2_CUTOFF, "lfo2.cutoff", Float, -96, 96, 0, "semitones", "", "How far it moves the filter cutoff.";
    104, LFO2_AMP, "lfo2.amp", Float, 0, 1, 0, "", "", "How much it changes the loudness.";
    105, LFO2_PAN, "lfo2.pan", Float, 0, 1, 0, "", "", "How far it moves the sound left and right.";

    // ---- the part's voice effects: they come before the distortion ----
    106, FX_PITCH, "fx.pitch", Float, -12, 12, 0, "semitones", "", "Pitch shifter: moves the part's whole sound up or down without changing its speed (0 is off). Up is a chipmunk, down is a monster.";
    107, FX_RING, "fx.ring", Float, 0, 1, 0, "", "", "Ring modulator: how much of the sound is multiplied by a steady wave (0 is off). The robot voice.";
    108, FX_RING_FREQ, "fx.ring_freq", Float, 1, 5000, 30, "hz", "", "Ring modulator: how fast the wave goes. Slow is a wobble, 30 to 80 is a robot, hundreds is a bell-like alien.";
    109, FX_RING_WAVE, "fx.ring_wave", Choice, 0, 1, 0, "", "sine|noise", "Ring modulator: sine is the robot. noise multiplies by a rumble as wide as fx.ring_freq, which blurs the pitch of a voice into a hoarse whisper.";
    110, FX_COMB, "fx.comb", Float, 0, 1, 0, "", "", "Comb: how much of the sound is the ringing, metallic copy (0 is off).";
    111, FX_COMB_FREQ, "fx.comb_freq", Float, 50, 5000, 200, "hz", "", "Comb: the note it rings at. Low is a robot in a pipe; high, and swept, is a flanger.";

    // ---- the part's effects ----
    112, DRIVE, "fx.drive", Float, 0, 1, 0, "", "", "Distortion (0 is off).";
    113, CRUSH_BITS, "fx.bits", Float, 1, 16, 16, "bits", "", "Bit crusher: how many bits each sample keeps (16 is off).";
    114, CRUSH_RATE, "fx.downsample", Int, 1, 64, 1, "", "", "Bit crusher: keep only one sample in this many (1 is off).";
    115, SEND_DELAY, "send.delay", Float, 0, 1, 0, "gain", "", "How much of the part goes to the echo.";
    116, SEND_REVERB, "send.reverb", Float, 0, 1, 0, "gain", "", "How much of the part goes to the reverb.";
    117, SEND_CHORUS, "send.chorus", Float, 0, 1, 0, "gain", "", "How much of the part goes to the chorus.";

    // ---- the part's step macro ----
    120, MACRO_RATE, "macro.rate", Float, 1, 1000, 50, "hz", "", "How many steps of the part's macro play each second.";
    121, MACRO_SYNC, "macro.sync", Bool, 0, 1, 0, "", "", "On: the macro steps on each macro_tick event (a tracker's tick) instead of by the clock.";

    // ---- more of the comb ----
    122, FX_COMB_FEEDBACK, "fx.comb_feedback", Float, -0.97, 0.97, 0.7, "", "", "Comb: how long it rings. Below 0 it rings an octave lower and hollow.";
    123, FX_COMB_RATE, "fx.comb_rate", Float, 0.01, 10, 0.3, "hz", "", "Comb: how fast its note sweeps up and down.";
    124, FX_COMB_DEPTH, "fx.comb_depth", Float, 0, 1, 0, "", "", "Comb: how far its note sweeps (0: it stays put).";

    // ---- the trance gate ----
    90, GATE_DEPTH, "gate.depth", Float, 0, 1, 0, "", "", "Trance gate: how far a closed step of its pattern closes (0: no gate; 1: to silence).";
    91, GATE_RATE, "gate.rate", Float, 0.25, 16, 4, "steps", "", "Trance gate: steps to a beat of the tempo (4 is sixteenths, 2 eighths, 8 thirty-seconds, 3 eighth-note triplets).";
    92, GATE_SMOOTH, "gate.smooth", Float, 0.001, 0.5, 0.008, "seconds", "", "Trance gate: how long it takes to open or close. It never clicks; 0.001 is as hard as it goes.";
    93, GATE_SWING, "gate.swing", Float, 0, 0.75, 0, "", "", "Trance gate: how far every second step is put back, as a part of a step.";
    94, GATE_SENDS, "gate.sends", Bool, 0, 1, 1, "", "", "On: the echo, reverb and chorus are sent the gated sound, so their tails ring through the gaps. Off: they are sent the sound as it was before the gate, and only the part's own sound is chopped.";
    95, GATE_STEREO, "gate.stereo", Bool, 0, 1, 0, "", "", "On: the gate's steps open on the left and on the right by turns.";

    // ---- the whole engine ----
    200, MASTER_VOLUME, "master.volume", Float, 0, 2, 0.35, "gain", "", "How loud everything is.";
    201, LIMITER, "master.limiter", Bool, 0, 1, 1, "", "", "Turns loud passages down so they do not distort. The output never goes past -1..1 either way.";
    202, DELAY_TIME, "delay.time", Float, 0.001, 2, 0.3, "seconds", "", "The time between echoes.";
    203, DELAY_FEEDBACK, "delay.feedback", Float, 0, 0.95, 0.4, "", "", "How much of each echo is fed back to make the next.";
    204, DELAY_TONE, "delay.tone", Float, 200, 20000, 6000, "hz", "", "Each echo is duller than the last; this is how much treble survives.";
    205, DELAY_PINGPONG, "delay.pingpong", Bool, 0, 1, 0, "", "", "On: the echoes bounce between left and right.";
    208, REVERB_SIZE, "reverb.size", Float, 0, 1, 0.5, "", "", "How big the room is: a bigger one has its echoes further apart and, unless reverb.decay says otherwise, a longer tail (three times shorter at 0, three times longer at 1).";
    209, REVERB_DAMP, "reverb.damping", Float, 0, 1, 0.4, "", "", "How quickly the treble of the reverb dies away (0: as slowly as the rest, a bright tail; 1: at once, a dull one).";
    210, REVERB_TYPE, "reverb.type", Choice, 0, 4, 0, "", "room|hall|plate|cathedral|spring", "What kind of space it is. room: a small room, a short soft tail. hall: a concert hall, big and smooth. plate: a sheet of metal, bright and thick at once. cathedral: enormous, and it goes on and on. spring: the twangy spring of a guitar amp, which boings.";
    211, REVERB_DECAY, "reverb.decay", Float, 0, 30, 0, "seconds", "", "How long the tail takes to die away (to fall by 60 dB). 0 leaves it to the type and reverb.size.";
    212, CHORUS_RATE, "chorus.rate", Float, 0.05, 10, 0.8, "hz", "", "How fast the chorus wobbles.";
    213, CHORUS_DEPTH, "chorus.depth", Float, 0, 1, 0.5, "", "", "How far the chorus wobbles.";
    216, VOICES, "voices", Int, 1, 64, 64, "voices", "", "The most voices that sound at once; a new note past this takes over the oldest.";

    // ---- the live input (a microphone) ----
    217, INPUT_PART, "input.part", Int, 0, 63, 31, "part", "", "The part the live input plays through: it gets that part's filter, pan, effects, volume and sends.";
    218, INPUT_GAIN, "input.gain", Float, 0, 8, 1, "gain", "", "How much the live input is turned up before anything else (1 is unchanged).";
    219, INPUT_MONITOR, "input.monitor", Bool, 0, 1, 1, "", "", "On: the live input is heard. Off: it can still be recorded, in silence.";
    220, INPUT_GATE, "input.gate", Float, -100, 0, -100, "db", "", "Noise gate: the input is let through only while it is louder than this, which keeps a microphone's hiss out (-100 is off; try -45).";
    221, INPUT_GATE_RELEASE, "input.gate_release", Float, 0.005, 2, 0.15, "seconds", "", "Noise gate: how long it takes to close once the input has gone quiet.";

    // ---- more of the reverb ----
    222, REVERB_PREDELAY, "reverb.predelay", Float, 0, 0.2, 0, "seconds", "", "A gap before the reverb starts, on top of the type's own: it keeps a sound clear of its own reverb.";
    223, REVERB_LOW_CUT, "reverb.low_cut", Float, 20, 1000, 120, "hz", "", "Bass below this is kept out of the reverb, so that it does not turn to mud.";

    // ---- time ----
    224, TEMPO, "tempo", Float, 20, 999, 120, "bpm", "", "Beats a minute, for arpeggios that count their steps in beats. A change is taken up at each arpeggio's next step.";
    225, REVERB_LEVEL, "reverb.level", Float, 0, 2, 1, "gain", "", "How loud the reverb comes back into the mix (1 is unchanged).";
    226, DELAY_LEVEL, "delay.level", Float, 0, 2, 1, "gain", "", "How loud the echoes come back into the mix (1 is unchanged).";
}

// Offsets inside a group of parameters that repeats.
/// Oscillator 2's parameters are oscillator 1's plus this.
pub const OSC_STRIDE: usize = OSC2_WAVE - OSC1_WAVE;
/// LFO 2's parameters are LFO 1's plus this.
pub const LFO_STRIDE: usize = LFO2_WAVE - LFO1_WAVE;

pub const WAVE_SINE: u8 = 0;
pub const WAVE_TRIANGLE: u8 = 1;
pub const WAVE_SAW: u8 = 2;
pub const WAVE_PULSE: u8 = 3;
pub const WAVE_SAMPLE: u8 = 4;

const fn build_index() -> [u16; 256] {
    let mut t = [u16::MAX; 256];
    let mut i = 0;
    while i < PARAMS.len() {
        t[PARAMS[i].id as usize] = i as u16;
        i += 1;
    }
    t
}

const INDEX: [u16; 256] = build_index();

/// Look a parameter up by id.
#[inline]
pub fn info(id: usize) -> Option<&'static ParamInfo> {
    if id >= 256 {
        return None;
    }
    let i = INDEX[id];
    if i == u16::MAX {
        None
    } else {
        PARAMS.get(i as usize)
    }
}

/// Look a parameter up by name.
pub fn by_name(name: &str) -> Option<&'static ParamInfo> {
    PARAMS.iter().find(|p| p.name == name)
}

/// The values a new part starts with.
pub fn part_defaults() -> [f32; PART_PARAMS] {
    let mut v = [0.0; PART_PARAMS];
    for p in PARAMS {
        if !p.is_global() {
            v[p.id as usize] = p.default;
        }
    }
    v
}

/// The values a new engine starts with.
pub fn global_defaults() -> [f32; GLOBAL_PARAMS] {
    let mut v = [0.0; GLOBAL_PARAMS];
    for p in PARAMS {
        if p.is_global() {
            v[p.id as usize - GLOBAL_BASE] = p.default;
        }
    }
    v
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// The whole table as JSON text, with the event types and the engine's limits.
pub fn table_json() -> String {
    use crate::engine as e;
    use core::fmt::Write;
    let mut s = String::with_capacity(16 * 1024);
    let _ = write!(
        s,
        "{{\"version\":\"{}\",\"abi\":{},\"limits\":{{\"voices\":{},\"parts\":{},\"samples\":{},\"event_size\":{},\"event_buffer\":{},\"event_queue\":{},\"output_frames\":{},\"macro_steps\":{},\"scope_master\":{},\"scope_part\":{},\"zone_size\":{},\"zones_part\":{},\"zones_total\":{},\"zone_layers\":{}}},",
        crate::VERSION,
        crate::ABI_VERSION,
        e::MAX_VOICES,
        e::NUM_PARTS,
        e::MAX_SAMPLES,
        crate::event::EVENT_SIZE,
        e::EVENT_BUFFER,
        e::QUEUE_CAPACITY,
        e::OUTPUT_FRAMES,
        crate::macros::MAX_STEPS,
        crate::taps::SCOPE_MASTER,
        crate::taps::SCOPE_PART,
        crate::zones::ZONE_SIZE,
        crate::zones::ZONES_PER_PART,
        crate::zones::ZONES_TOTAL,
        crate::zones::MAX_LAYERS
    );
    s.push_str("\"events\":[");
    for (i, (id, name, help)) in crate::event::EVENT_TYPES.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"type\":{id},\"name\":");
        json_str(&mut s, name);
        s.push_str(",\"help\":");
        json_str(&mut s, help);
        s.push('}');
    }
    s.push_str("],\"params\":[");
    for (i, p) in PARAMS.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{{\"id\":{},\"name\":", p.id);
        json_str(&mut s, p.name);
        let _ = write!(
            s,
            ",\"scope\":\"{}\",\"kind\":\"{}\",\"min\":{},\"max\":{},\"default\":{},\"unit\":",
            if p.is_global() { "engine" } else { "part" },
            p.kind.name(),
            p.min,
            p.max,
            p.default
        );
        json_str(&mut s, p.unit);
        if !p.choices.is_empty() {
            s.push_str(",\"choices\":[");
            for (j, c) in p.choices.split('|').enumerate() {
                if j > 0 {
                    s.push(',');
                }
                json_str(&mut s, c);
            }
            s.push(']');
        }
        s.push_str(",\"help\":");
        json_str(&mut s, p.help);
        s.push('}');
    }
    s.push_str("],\"voice_presets\":[");
    for (i, preset) in crate::presets::VOICE_PRESETS.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str("{\"name\":");
        json_str(&mut s, preset.name);
        s.push_str(",\"help\":");
        json_str(&mut s, preset.help);
        s.push_str(",\"values\":{");
        for (j, (name, value)) in preset.values.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            json_str(&mut s, name);
            let _ = write!(s, ":{value}");
        }
        s.push_str("}}");
    }
    s.push_str("]}");
    s
}

/// A parameter as a row of the table in `docs/engine-api.md`. The document's
/// table is made by `cargo run --example param_table`, and a test checks the
/// two agree.
pub fn doc_row(p: &ParamInfo) -> String {
    let kind = if p.choices.is_empty() {
        p.kind.name().to_string()
    } else {
        let list: Vec<String> = p.choices.split('|').enumerate().map(|(i, c)| format!("{i} {c}")).collect();
        format!("choice: {}", list.join(", "))
    };
    format!(
        "| {} | `{}` | {} | {} | {} | {} | {} | {} |",
        p.id, p.name, kind, p.min, p.max, p.default, p.unit, p.help
    )
}
