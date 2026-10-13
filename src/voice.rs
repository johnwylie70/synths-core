//! A voice: everything that makes one note. Two oscillators (waves or
//! samples), a sub oscillator and noise go through a filter and an amplifier,
//! shaped by three envelopes and two LFOs.

use crate::arp::Arp;
use crate::env::{Env, EnvShape};
use crate::filter::{self, Svf, FILTER_OFF};
use crate::lfo::{FreeLfo, Lfo, LfoShape};
use crate::macros::{Macro, MacroPlayer, STEP_NOISE};
use crate::math::{fast_sin, note_to_hz, semis_to_ratio, Rng, PI};
use crate::osc::{Noise, Osc, NOISE_LFSR};
use crate::params::*;
use crate::sample::{Player, Sample};

/// The engine works out slow-moving values (pitch, filter, loudness) once
/// every this many samples and slides smoothly between them.
pub const CONTROL: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceState {
    Free,
    /// The note is held down.
    Held,
    /// The note has been let go and is dying away.
    Released,
}

/// What a new voice needs to know to start.
pub struct Start {
    pub part: u16,
    pub handle: u32,
    pub note: f32,
    /// The pitch to slide from (the same as `note` for no slide).
    pub from: f32,
    /// How long the slide takes, in seconds.
    pub glide: f32,
    pub velocity: f32,
    /// Unison: this voice's detune in semitones, pan offset and loudness.
    pub detune: f32,
    pub pan: f32,
    pub gain: f32,
    /// Start the oscillators at a random point of their wave.
    pub random_phase: bool,
    pub seed: u32,
    /// A number that goes up with every voice started, to find the oldest.
    pub born: u64,
}

#[derive(Clone, Copy)]
pub struct Voice {
    pub state: VoiceState,
    pub part: u16,
    pub handle: u32,
    /// The note that started the voice, which a note-off must match.
    pub key: f32,
    pub born: u64,
    velocity: f32,
    note: f32,
    note_target: f32,
    /// Semitones per sample while the pitch slides.
    glide_step: f32,
    detune: f32,
    unison_pan: f32,
    unison_gain: f32,
    /// Set directly on a live voice (the tracker does this).
    pub volume: f32,
    pub pan: f32,

    osc: [Osc; 2],
    sub: Osc,
    noise: Noise,
    players: [Player; 2],
    filters: [[Svf; 2]; 2],
    amp_env: Env,
    filter_env: Env,
    mod_env: Env,
    lfos: [Lfo; 2],
    pub macro_player: MacroPlayer,
    pub arp: Arp,
    /// The arpeggio has just stepped: the pitch jumps instead of sliding.
    pub pitch_jump: bool,
    pub rng: Rng,

    // Worked out by `control`, used by `render`.
    amp_shape: EnvShape,
    filter_shape: EnvShape,
    mod_shape: EnvShape,
    waves: [u8; 2],
    slots: [u16; 2],
    levels: [f32; 2],
    sub_wave: u8,
    sub_level: f32,
    noise_type: u8,
    noise_level: f32,
    ring: f32,
    sync: bool,
    stereo: bool,
    filter_type: u8,
    filter_24: bool,
    g: f32,
    dg: f32,
    k: f32,
    gain: [f32; 2],
    dgain: [f32; 2],
    /// Samples rendered since `control` last ran.
    since_control: u32,
    /// No sample has been rendered yet.
    fresh: bool,
    /// The last sample this voice put out, for a click-free cut.
    pub last: [f32; 2],
}

/// Slide `cur` towards `target` over the coming samples (or jump, on a new voice).
#[inline]
fn aim(cur: &mut f32, step: &mut f32, target: f32, jump: bool, inv_len: f32) {
    if jump {
        *cur = target;
        *step = 0.0;
    } else {
        *step = (target - *cur) * inv_len;
    }
}

impl Voice {
    pub fn new() -> Voice {
        Voice {
            state: VoiceState::Free,
            part: 0,
            handle: 0,
            key: 0.0,
            born: 0,
            velocity: 0.0,
            note: 0.0,
            note_target: 0.0,
            glide_step: 0.0,
            detune: 0.0,
            unison_pan: 0.0,
            unison_gain: 1.0,
            volume: 1.0,
            pan: 0.0,
            osc: [Osc::default(); 2],
            sub: Osc::default(),
            noise: Noise::new(1),
            players: [Player::default(); 2],
            filters: [[Svf::default(); 2]; 2],
            amp_env: Env::default(),
            filter_env: Env::default(),
            mod_env: Env::default(),
            lfos: [Lfo::default(); 2],
            macro_player: MacroPlayer::default(),
            arp: Arp::default(),
            pitch_jump: false,
            rng: Rng::new(1),
            amp_shape: EnvShape::default(),
            filter_shape: EnvShape::default(),
            mod_shape: EnvShape::default(),
            waves: [0; 2],
            slots: [0; 2],
            levels: [0.0; 2],
            sub_wave: 0,
            sub_level: 0.0,
            noise_type: 0,
            noise_level: 0.0,
            ring: 0.0,
            sync: false,
            stereo: false,
            filter_type: FILTER_OFF,
            filter_24: false,
            g: 0.0,
            dg: 0.0,
            k: 2.0,
            gain: [0.0; 2],
            dgain: [0.0; 2],
            since_control: 0,
            fresh: true,
            last: [0.0; 2],
        }
    }

    #[inline]
    pub fn active(&self) -> bool {
        self.state != VoiceState::Free
    }

    /// The note the voice is playing (or sliding to), for a keyboard display.
    #[inline]
    pub fn current_note(&self) -> f32 {
        self.note_target
    }

    fn read_shapes(&mut self, p: &[f32; PART_PARAMS], sample_rate: f32) {
        self.amp_shape = EnvShape::new(&p[AMP_ATTACK..AMP_ATTACK + 5], sample_rate);
        self.filter_shape = EnvShape::new(&p[FENV_ATTACK..FENV_ATTACK + 5], sample_rate);
        self.mod_shape = EnvShape::new(&p[MENV_ATTACK..MENV_ATTACK + 5], sample_rate);
    }

    /// Start a new note on this voice.
    pub fn start(&mut self, s: &Start, p: &[f32; PART_PARAMS], samples: &[Option<Sample>], sample_rate: f32) {
        *self = Voice::new();
        self.state = VoiceState::Held;
        self.part = s.part;
        self.handle = s.handle;
        self.key = s.note;
        self.born = s.born;
        self.velocity = s.velocity;
        self.detune = s.detune;
        self.unison_pan = s.pan;
        self.unison_gain = s.gain;
        self.rng = Rng::new(s.seed);
        self.noise = Noise::new(self.rng.next_u32());
        self.note = if s.glide > 0.0 { s.from } else { s.note };
        self.set_pitch(s.note, s.glide, sample_rate);
        for i in 0..2 {
            let wave = p[OSC1_WAVE + i * OSC_STRIDE] as u8;
            let offset = if s.random_phase { self.rng.unipolar() } else { 0.0 };
            self.osc[i].reset(wave, offset);
            let slot = p[OSC1_SAMPLE + i * OSC_STRIDE] as usize;
            let frames = match samples.get(slot) {
                Some(Some(sm)) => sm.frames as f64,
                _ => 0.0,
            };
            self.players[i].start(p[OSC1_START + i * OSC_STRIDE] as f64 * frames);
        }
        self.sub.reset(WAVE_SINE, if s.random_phase { self.rng.unipolar() } else { 0.0 });
        self.read_shapes(p, sample_rate);
        self.amp_env.gate_on(&self.amp_shape);
        self.filter_env.gate_on(&self.filter_shape);
        self.mod_env.gate_on(&self.mod_shape);
        let mut rng = self.rng;
        for l in self.lfos.iter_mut() {
            l.reset(&mut rng);
        }
        self.rng = rng;
    }

    /// Move the voice to a new pitch, sliding over `glide` seconds.
    pub fn set_pitch(&mut self, note: f32, glide: f32, sample_rate: f32) {
        self.note_target = note;
        if glide > 0.0 {
            self.glide_step = (note - self.note).abs() / (glide * sample_rate);
        } else {
            self.note = note;
            self.glide_step = 0.0;
        }
    }

    /// Play a new note on a voice that is already sounding (mono and legato).
    pub fn replay(&mut self, note: f32, velocity: f32, handle: u32, glide: f32, retrigger: bool, sample_rate: f32) {
        self.key = note;
        self.handle = handle;
        self.set_pitch(note, glide, sample_rate);
        if retrigger || self.state == VoiceState::Released {
            self.velocity = velocity;
            self.macro_player.restart();
            self.amp_env.gate_on(&self.amp_shape);
            self.filter_env.gate_on(&self.filter_shape);
            self.mod_env.gate_on(&self.mod_shape);
        }
        self.state = VoiceState::Held;
    }

    /// Let the note go.
    pub fn release(&mut self) {
        if self.state == VoiceState::Held {
            self.state = VoiceState::Released;
            self.macro_player.release();
            self.amp_env.gate_off(&self.amp_shape);
            self.filter_env.gate_off(&self.filter_shape);
            self.mod_env.gate_off(&self.mod_shape);
        }
    }

    /// Jump to a position in the sample, in frames.
    pub fn set_position(&mut self, frames: f32) {
        for pl in self.players.iter_mut() {
            pl.start(frames as f64);
        }
    }

    /// Work out the slow-moving values for the next `len` samples.
    pub fn control(
        &mut self,
        p: &[f32; PART_PARAMS],
        free: &[FreeLfo; 2],
        steps: &Macro,
        samples: &[Option<Sample>],
        sample_rate: f32,
        len: usize,
    ) {
        let elapsed = self.since_control;
        self.since_control = 0;
        let jump = self.fresh;
        let pitch_jump = jump || self.pitch_jump;
        self.pitch_jump = false;
        let inv_len = 1.0 / len.max(1) as f32;
        let inv_sr = 1.0 / sample_rate;

        if self.note != self.note_target {
            let step = self.glide_step * elapsed as f32;
            let d = self.note_target - self.note;
            if self.glide_step <= 0.0 || d.abs() <= step {
                self.note = self.note_target;
            } else {
                self.note += step.copysign(d);
            }
        }

        self.read_shapes(p, sample_rate);
        let fenv = self.filter_env.advance(elapsed, &self.filter_shape);
        let menv = self.mod_env.advance(elapsed, &self.mod_shape);

        let mut lfo = [0.0f32; 2];
        let mut rng = self.rng;
        for i in 0..2 {
            let b = LFO1_WAVE + i * LFO_STRIDE;
            let shape = LfoShape {
                wave: p[b] as u8,
                rate: p[b + 1],
                delay: p[b + 2],
                fade: p[b + 3],
                sync: p[b + 4] >= 0.5,
            };
            lfo[i] = self.lfos[i].tick(elapsed, &shape, &free[i], inv_sr, &mut rng);
        }
        self.rng = rng;
        let l1 = LFO1_WAVE;
        let l2 = LFO2_WAVE;
        let off = |id: usize| id - LFO1_WAVE;
        let lfo_sum = |id: usize| lfo[0] * p[l1 + off(id)] + lfo[1] * p[l2 + off(id)];

        // The part's step macro.
        let step = self.macro_player.advance(steps, elapsed as f32 * p[MACRO_RATE] * inv_sr, p[MACRO_SYNC] >= 0.5);
        let noise_step = step.wave == STEP_NOISE;

        // Pitch. `bend` is everything that moves the pitch away from the note.
        let bend = step.pitch
            + self.arp.offset
            + p[TRANSPOSE]
            + p[TUNE] * 0.01
            + p[BEND] * p[BEND_RANGE]
            + self.detune
            + lfo_sum(LFO1_PITCH)
            + menv * p[MENV_PITCH];
        let base_hz = note_to_hz((self.note + bend).clamp(-60.0, 200.0));
        let pw_mod = 0.5 * (lfo_sum(LFO1_PW) + menv * p[MENV_PW]);
        let top = 0.49;

        self.stereo = false;
        for i in 0..2 {
            let b = i * OSC_STRIDE;
            let mut wave = p[OSC1_WAVE + b] as u8;
            self.levels[i] = p[OSC1_LEVEL + b];
            if i == 0 && step.wave >= 0.0 {
                // A macro step picks the wave; for a step of noise the
                // oscillator falls silent and the noise takes its level.
                wave = if noise_step { WAVE_PULSE } else { step.wave as u8 };
                if noise_step {
                    self.levels[0] = 0.0;
                }
            }
            self.waves[i] = wave;
            let mut semis = p[OSC1_COARSE + b] + p[OSC1_FINE + b] * 0.01;
            if i == 1 {
                semis += menv * p[MENV_OSC2_PITCH];
            }
            let keytrack = p[OSC1_KEYTRACK + b] >= 0.5;
            if wave == WAVE_SAMPLE {
                let slot = p[OSC1_SAMPLE + b] as usize;
                self.slots[i] = slot as u16;
                let rate = match samples.get(slot) {
                    Some(Some(s)) => {
                        if i == 0 && s.channels == 2 {
                            self.stereo = true;
                        }
                        let n = if keytrack { self.note - s.base_note } else { 0.0 };
                        s.rate * inv_sr * semis_to_ratio((n + bend + semis).clamp(-120.0, 120.0))
                    }
                    _ => 0.0,
                };
                let pl = &mut self.players[i];
                aim(&mut pl.inc, &mut pl.dinc, rate.min(1024.0), pitch_jump, inv_len);
            } else {
                let mut hz = if keytrack { base_hz } else { note_to_hz(60.0 + bend) };
                if semis != 0.0 {
                    hz *= semis_to_ratio(semis);
                }
                let o = &mut self.osc[i];
                aim(&mut o.inc, &mut o.dinc, (hz * inv_sr).min(top), pitch_jump, inv_len);
                let width = if step.pulse_width >= 0.0 { step.pulse_width } else { p[OSC1_PW + b] };
                let pw = (width + pw_mod).clamp(0.0, 1.0);
                aim(&mut o.pw, &mut o.dpw, pw, jump, inv_len);
            }
        }
        self.sync = p[OSC2_SYNC] >= 0.5 && self.waves[0] != WAVE_SAMPLE && self.waves[1] != WAVE_SAMPLE;
        self.ring = p[OSC2_RING];

        self.sub_level = p[SUB_LEVEL];
        self.sub_wave = match p[SUB_WAVE] as u8 {
            0 => WAVE_SINE,
            1 => WAVE_PULSE,
            _ => WAVE_TRIANGLE,
        };
        let sub_hz = base_hz * if p[SUB_OCTAVE] <= -1.5 { 0.25 } else { 0.5 };
        aim(&mut self.sub.inc, &mut self.sub.dinc, (sub_hz * inv_sr).min(top), pitch_jump, inv_len);
        self.sub.pw = 0.5;
        self.sub.dpw = 0.0;

        self.noise_level = if noise_step { p[NOISE_LEVEL].max(p[OSC1_LEVEL]) } else { p[NOISE_LEVEL] };
        self.noise_type = p[NOISE_TYPE] as u8;
        if self.noise_level > 0.0 && self.noise_type >= NOISE_LFSR {
            self.noise.clock_inc = base_hz * 16.0 * semis_to_ratio(p[NOISE_PITCH]) * inv_sr;
        }

        // Filter.
        let was_off = self.filter_type == FILTER_OFF;
        self.filter_type = p[FILTER_TYPE] as u8;
        self.filter_24 = p[FILTER_SLOPE] >= 0.5;
        if self.filter_type != FILTER_OFF {
            let semis = p[FILTER_KEYTRACK] * (self.note - 60.0)
                + p[FILTER_ENV] * fenv
                + p[FILTER_VELOCITY] * self.velocity
                + step.cutoff
                + lfo_sum(LFO1_CUTOFF);
            let hz = (p[CUTOFF] * semis_to_ratio(semis.clamp(-120.0, 120.0))).clamp(16.0, sample_rate * 0.45);
            let g = (PI * hz * inv_sr).tan();
            aim(&mut self.g, &mut self.dg, g, jump || was_off, inv_len);
            // Resonance 0..1 is a Q of 0.5 (no peak) up to about 45 (it rings).
            self.k = 2.0 * (-6.5 * p[RESONANCE]).exp2();
            if was_off {
                self.filters = [[Svf::default(); 2]; 2];
            }
            for f in self.filters.iter_mut().flatten() {
                f.tidy();
            }
        }

        // Loudness and pan.
        let sens = p[VELOCITY_SENS];
        let mut gain = (1.0 - sens + sens * self.velocity) * self.volume * self.unison_gain * step.volume;
        gain *= 1.0 - p[LFO1_AMP] * (0.5 - 0.5 * lfo[0]);
        gain *= 1.0 - p[LFO2_AMP] * (0.5 - 0.5 * lfo[1]);
        let pan = (p[PAN] + self.pan + self.unison_pan + lfo_sum(LFO1_PAN)).clamp(-1.0, 1.0);
        let (gl, gr) = if self.stereo {
            // A stereo sample keeps both sides at full until it is panned.
            ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
        } else {
            // Equal power: the same loudness wherever the sound sits.
            let a = (pan + 1.0) * 0.125;
            (fast_sin(a + 0.25), fast_sin(a))
        };
        let [g0, g1] = &mut self.gain;
        let [d0, d1] = &mut self.dgain;
        aim(g0, d0, gain * gl, jump, inv_len);
        aim(g1, d1, gain * gr, jump, inv_len);
    }

    /// Add the next samples of this voice to `out_l` and `out_r`. Returns
    /// false once the voice has finished. At most `CONTROL` samples at a time.
    pub fn render(&mut self, out_l: &mut [f32], out_r: &mut [f32], samples: &[Option<Sample>]) -> bool {
        let n = out_l.len().min(out_r.len()).min(CONTROL);
        let mut buf_a = [0.0f32; CONTROL];
        let mut buf_b = [0.0f32; CONTROL];
        let mut buf_t = [0.0f32; CONTROL];
        let mut buf_s = [0.0f32; CONTROL];
        let a = &mut buf_a[..n];
        let b = &mut buf_b[..n];
        let t = &mut buf_t[..n];
        let sync = &mut buf_s[..n];
        let stereo = self.stereo;
        let mut sounding = false;
        self.fresh = false;

        // Oscillator 1 into `a` (and `b`, for a stereo sample), at full level
        // for now: ring modulation needs it that way.
        let [l1, l2] = self.levels;
        let need1 = l1 > 0.0 || (l2 > 0.0 && (self.ring > 0.0 || self.sync));
        if self.waves[0] == WAVE_SAMPLE {
            match samples.get(self.slots[0] as usize) {
                Some(Some(s)) => {
                    if need1 {
                        self.players[0].render(s, a, if stereo { Some(&mut *b) } else { None });
                        sounding |= l1 > 0.0 && !self.players[0].ended;
                    }
                }
                _ => self.players[0].ended = true,
            }
        } else if need1 {
            if self.sync {
                self.osc[0].wrap_points(sync);
            }
            self.osc[0].render(self.waves[0], a);
            sounding |= l1 > 0.0;
        }

        // Oscillator 2 into `t`, then mix.
        let mut have2 = false;
        if l2 > 0.0 {
            if self.waves[1] == WAVE_SAMPLE {
                match samples.get(self.slots[1] as usize) {
                    Some(Some(s)) => {
                        self.players[1].render(s, t, None);
                        have2 = true;
                        sounding |= !self.players[1].ended;
                    }
                    _ => self.players[1].ended = true,
                }
            } else {
                if self.sync {
                    self.osc[1].render_synced(self.waves[1], t, sync);
                } else {
                    self.osc[1].render(self.waves[1], t);
                }
                have2 = true;
                sounding = true;
            }
        }
        if have2 {
            if self.ring > 0.0 {
                let ring = self.ring;
                for i in 0..n {
                    t[i] *= 1.0 - ring + ring * a[i];
                }
            }
            for i in 0..n {
                a[i] = a[i] * l1 + t[i] * l2;
            }
            if stereo {
                for i in 0..n {
                    b[i] = b[i] * l1 + t[i] * l2;
                }
            }
        } else if l1 != 1.0 {
            for x in a.iter_mut().chain(b.iter_mut()) {
                *x *= l1;
            }
        }

        if self.sub_level > 0.0 {
            self.sub.render(self.sub_wave, t);
            let level = self.sub_level;
            for i in 0..n {
                a[i] += t[i] * level;
            }
            if stereo {
                for i in 0..n {
                    b[i] += t[i] * level;
                }
            }
            sounding = true;
        }
        if self.noise_level > 0.0 {
            if stereo {
                t.fill(0.0);
                self.noise.render_add(self.noise_type, self.noise_level, t);
                for i in 0..n {
                    a[i] += t[i];
                    b[i] += t[i];
                }
            } else {
                self.noise.render_add(self.noise_type, self.noise_level, a);
            }
            sounding = true;
        }

        if self.filter_type != FILTER_OFF {
            let (g, dg, k, kind) = (self.g, self.dg, self.k, filter::outputs(self.filter_type));
            let sides = if stereo { 2 } else { 1 };
            for side in 0..sides {
                let buf: &mut [f32] = if side == 0 { &mut *a } else { &mut *b };
                self.g = self.filters[side][0].process(buf, g, dg, k, kind);
                if self.filter_24 {
                    self.filters[side][1].process(buf, g, dg, core::f32::consts::SQRT_2, kind);
                }
            }
        }

        let [mut gl, mut gr] = self.gain;
        let [dl, dr] = self.dgain;
        let (mut last_l, mut last_r) = (0.0, 0.0);
        for i in 0..n {
            let e = self.amp_env.next(&self.amp_shape);
            gl += dl;
            gr += dr;
            last_l = a[i] * e * gl;
            last_r = (if stereo { b[i] } else { a[i] }) * e * gr;
            out_l[i] += last_l;
            out_r[i] += last_r;
        }
        self.gain = [gl, gr];
        self.last = [last_l, last_r];
        self.since_control += n as u32;
        sounding && !self.amp_env.finished(&self.amp_shape)
    }
}

impl Default for Voice {
    fn default() -> Voice {
        Voice::new()
    }
}
