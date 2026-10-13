//! The engine's live input and its recorder: `render_io`, and the places
//! where `render` reaches for them. The sound itself is worked out in
//! `crate::input` and `crate::voicefx`.

use super::*;
use crate::input::{Source, RECORD_SIZE};

/// The live input for a render: the left (or only) channel, and the right
/// one if there is one. `None` is a render with no input.
pub(super) type Frames<'a> = Option<(&'a [f32], Option<&'a [f32]>)>;

/// How many frames a render makes: no more than there are of input.
pub(super) fn length(input: &Frames<'_>, frames: usize) -> usize {
    match input {
        Some((l, Some(r))) => frames.min(l.len()).min(r.len()),
        Some((l, None)) => frames.min(l.len()),
        None => frames,
    }
}

impl Engine {
    /// `render`, with a live input: the frames a microphone has just
    /// delivered, mono (`in_right` is `None`) or stereo. The input plays
    /// through the part `input.part` names, and can be recorded. As many
    /// frames are rendered as the shortest of the slices holds.
    pub fn render_io(&mut self, in_left: &[f32], in_right: Option<&[f32]>, left: &mut [f32], right: &mut [f32]) {
        self.render_with(Some((in_left, in_right)), left, right);
    }

    /// Take in the input for the next chunk, which starts `done` frames
    /// into the block. Returns how long the chunk is to be: `n`, or fewer
    /// if the input starts to sound part way through.
    pub(super) fn stage_input(&mut self, input: &Frames<'_>, done: usize, n: usize) -> usize {
        let (left, right) = match input {
            Some((l, r)) => (Some(&l[done..]), r.map(|r| &r[done..])),
            None => (None, None),
        };
        let (n, staged) = self.input.stage(left, right, n, self.tick_pos == 0, &self.globals, self.sample_rate);
        if self.input.sounding && self.globals[INPUT_MONITOR - GLOBAL_BASE] >= 0.5 {
            // Something to hear: the engine may not sleep.
            self.quiet = 0;
            self.asleep = false;
        }
        if self.recorder.source() == Some(Source::Input) {
            if staged {
                self.recorder.write(&mut self.samples, &self.input.pre_l[..n], &self.input.pre_r[..n]);
            } else {
                self.recorder.write_silence(&mut self.samples, n);
            }
        }
        n
    }

    /// Add the input to its part, if it is sounding and to be heard.
    /// Returns 1 if it was added.
    pub(super) fn monitor_input(&mut self, n: usize) -> usize {
        if !self.input.sounding {
            return 0;
        }
        let part = self.globals[INPUT_PART - GLOBAL_BASE] as usize;
        let part = match self.parts.get_mut(part) {
            Some(p) if self.globals[INPUT_MONITOR - GLOBAL_BASE] >= 0.5 => p,
            _ => {
                self.input.unheard();
                return 0;
            }
        };
        if !part.in_use {
            part.in_use = true;
            part.bus_l[..n].fill(0.0);
            part.bus_r[..n].fill(0.0);
        }
        self.input.monitor(&part.p, &mut part.bus_l[..n], &mut part.bus_r[..n], self.tick_pos, self.sample_rate);
        1
    }

    /// Is the live input sounding through this part? (Then a change of the
    /// part's volume slides, as it does while a voice sounds.)
    pub(super) fn input_sounds_on(&self, part: u16) -> bool {
        self.input.sounding
            && self.globals[INPUT_MONITOR - GLOBAL_BASE] >= 0.5
            && self.globals[INPUT_PART - GLOBAL_BASE] as u16 == part
    }

    /// A `record` event: start or stop recording, on this frame.
    pub(super) fn record_event(&mut self, e: &Event) {
        if e.a == 0 {
            self.recorder.stop();
            return;
        }
        if !(e.x >= 0.0 && e.y >= 0.0) || e.x >= MAX_SAMPLES as f32 {
            return;
        }
        let source = match e.y as u32 {
            RECORD_INPUT => Source::Input,
            RECORD_PART if (e.part as usize) < NUM_PARTS => Source::Part(e.part),
            RECORD_MASTER => Source::Master,
            _ => return,
        };
        self.recorder.start(&mut self.samples, e.x as usize, source);
    }

    /// Frames in which the engine slept: a recording of a part or of the
    /// output gets silence.
    pub(super) fn record_silence(&mut self, n: usize) {
        if matches!(self.recorder.source(), Some(Source::Part(_) | Source::Master)) {
            self.recorder.write_silence(&mut self.samples, n);
        }
    }

    /// Frames in which the part being recorded made no sound.
    pub(super) fn record_silence_of_part(&mut self, n: usize) {
        if matches!(self.recorder.source(), Some(Source::Part(_))) {
            self.recorder.write_silence(&mut self.samples, n);
        }
    }

    pub(super) fn reset_io(&mut self) {
        self.input.reset();
        self.vfx.clear();
        self.recorder.stop();
    }

    /// Make a sample slot to record into: room for `frames` frames of 1 or
    /// 2 channels at the engine's sample rate, empty to begin with. This is
    /// where a recording's memory comes from, so it is for before the
    /// recording, not during a render.
    pub fn record_buffer(&mut self, slot: usize, frames: usize, channels: u32) -> Result<(), SampleError> {
        if slot >= MAX_SAMPLES {
            return Err(SampleError::BadSlot);
        }
        if !(channels == 1 || channels == 2) || frames == 0 || frames > u32::MAX as usize / 4 {
            return Err(SampleError::BadFormat);
        }
        let len = frames * channels as usize;
        let mut data: Vec<f32> = Vec::new();
        if data.try_reserve_exact(len).is_err() {
            return Err(SampleError::BadFormat);
        }
        data.resize(len, 0.0);
        self.samples[slot] = Some(Sample {
            data: data.into_boxed_slice(),
            frames: 0,
            channels,
            rate: self.sample_rate,
            base_note: 60.0,
            loop_mode: 0,
            loop_start: 0,
            loop_end: 0,
        });
        Ok(())
    }

    /// The recorder as a display reads it: recording or not (1 or 0), the
    /// slot, the frames recorded so far and the frames there is room for.
    pub fn recording(&self) -> [u32; RECORD_SIZE] {
        self.recorder.read()
    }

    /// How many frames the sample in a slot has (0 for an empty slot).
    pub fn sample_frames(&self, slot: usize) -> usize {
        match self.samples.get(slot) {
            Some(Some(s)) => s.frames as usize,
            _ => 0,
        }
    }

    /// How many channels the sample in a slot has: 1 or 2, or 0 for an empty slot.
    pub fn sample_channels(&self, slot: usize) -> usize {
        match self.samples.get(slot) {
            Some(Some(s)) => s.channels as usize,
            _ => 0,
        }
    }

    /// Copy frames of the sample in a slot into `out`, from frame `start`
    /// on: one number a frame for a mono sample, two (left, right) for a
    /// stereo one. Returns how many frames were copied.
    pub fn read_sample(&self, slot: usize, start: usize, out: &mut [f32]) -> usize {
        let s = match self.samples.get(slot) {
            Some(Some(s)) => s,
            _ => return 0,
        };
        let channels = s.channels.max(1) as usize;
        let have = (s.frames as usize).min(s.data.len() / channels);
        let n = have.saturating_sub(start).min(out.len() / channels);
        if n > 0 {
            out[..n * channels].copy_from_slice(&s.data[start * channels..(start + n) * channels]);
        }
        n
    }
}
