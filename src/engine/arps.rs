//! Arpeggiators: the arpeggios and chords that take more than one voice
//! stepping its own pitch.
//!
//! The `arp` event has always made a voice step its pitch by itself
//! (`crate::arp::Arp`). An arpeggiator belongs to the engine instead, and
//! plays a note's arpeggio as a player would: at each step it lets go of
//! the last note and strikes the next, so every step has its own attack and
//! its own release, which overlaps the next step's. It also plays a chord's
//! notes all together, runs a pattern up through several octaves, swings,
//! accents, keeps time with the `tempo` parameter or steps when it is told
//! to, and takes a change of chord at its next step without losing its
//! place. An arpeggio is given to an arpeggiator when it asks for any of
//! that (`ArpSpec::wants_arpeggiator`).
//!
//! Its notes are ordinary notes of its part with its handle, so everything
//! that addresses a handle reaches them; the engine tells the arpeggiator
//! too, so that the notes it has yet to strike follow.

use super::*;
use crate::arp::*;

/// No frame: nothing is waiting.
const NEVER: u64 = u64::MAX;

#[derive(Clone, Copy)]
pub(super) struct Arpeggiator {
    live: bool,
    /// A number of its own, which its voices carry (`Arp::owner`).
    id: u16,
    part: u16,
    handle: u32,
    /// The note the arpeggio is on, and how hard it was struck.
    root: f32,
    velocity: f32,
    /// The volume and pan its handle has been given, for notes yet to come.
    volume: f32,
    pan: f32,
    spec: ArpSpec,
    /// How many steps it has begun.
    step: u32,
    /// The time of the next step by its clock, in frames. Not finite when
    /// it has no clock, or has been told to stop.
    due: f64,
    /// How long its steps last, in frames, as far as it knows.
    length: f64,
    /// A step waiting for its swing: its frame, which step it is and how
    /// long it sounds.
    strike_at: u64,
    strike_step: u32,
    strike_length: f64,
    /// The frame at which the sounding step's notes are let go.
    off_at: u64,
    /// It takes no more steps, and ends when the last is let go.
    stopping: bool,
    /// It has its note: the one note of an arpeggio that strikes none
    /// after the first. When that note is gone, so is the arpeggio.
    begun: bool,
    /// The sample slot its notes play as oscillator 1 and where they start
    /// in it, for a song's channel; below 0 for the part's own patch.
    sample: i32,
    position: f64,
    rng: Rng,
}

impl Arpeggiator {
    pub(super) const NONE: Arpeggiator = Arpeggiator {
        live: false,
        id: 0,
        part: 0,
        handle: 0,
        root: 0.0,
        velocity: 1.0,
        volume: 1.0,
        pan: 0.0,
        spec: ArpSpec::NONE,
        step: 0,
        due: f64::INFINITY,
        length: 0.0,
        strike_at: NEVER,
        strike_step: 0,
        strike_length: 0.0,
        off_at: NEVER,
        stopping: false,
        begun: false,
        sample: -1,
        position: 0.0,
        rng: Rng(1),
    };

    /// Is this the arpeggiator a handle on a part means?
    #[inline]
    fn is(&self, part: u16, handle: u32) -> bool {
        self.live && self.handle == handle && (part == ALL_PARTS || self.part == part)
    }

    /// The next frame it has something to do on.
    fn next_frame(&self) -> u64 {
        let due = if self.due.is_finite() { self.due as u64 } else { NEVER };
        due.min(self.strike_at).min(self.off_at)
    }
}

impl Engine {
    /// How many arpeggiators are running.
    pub fn active_arpeggiators(&self) -> usize {
        self.arps.iter().filter(|a| a.live).count()
    }

    fn tempo(&self) -> f32 {
        self.globals[TEMPO - GLOBAL_BASE]
    }

    /// Make an arpeggiator, in a free place or else the place of the one
    /// that has run longest. Its first step is taken on this frame, once
    /// every event of the frame has had its say.
    #[inline(never)]
    fn arp_new(&mut self, part: u16, handle: u32, root: f32, velocity: f32, spec: ArpSpec) -> usize {
        let i = match self.arps.iter().position(|a| !a.live) {
            Some(i) => i,
            None => {
                // The lowest number is the oldest, until the numbers come round.
                let i = (0..MAX_ARPEGGIATORS).min_by_key(|&i| self.arps[i].id).unwrap_or(0);
                self.arp_let_go(self.arps[i].id);
                i
            }
        };
        self.arp_ids = if self.arp_ids == u16::MAX { 1 } else { self.arp_ids + 1 };
        self.arps[i] = Arpeggiator {
            live: true,
            id: self.arp_ids,
            part,
            handle,
            root,
            velocity,
            spec,
            due: self.time as f64,
            rng: Rng::new(self.rng.next_u32()),
            ..Arpeggiator::NONE
        };
        self.arp_at = self.time;
        i
    }

    /// A `note_on` for a part with an arpeggio waiting that an arpeggiator
    /// is to play: the arpeggiator is the note.
    #[inline(never)]
    pub(super) fn arp_begin(&mut self, part: u16, handle: u32, note: f32, velocity: f32) {
        let spec = core::mem::replace(&mut self.parts[part as usize].next_arp, ArpSpec::NONE);
        if handle != 0 {
            self.arp_end(part, handle, true);
        }
        self.arp_new(part, handle, note, velocity, spec);
    }

    /// Let go of the notes an arpeggiator is holding down. One that has
    /// made no sound yet is simply taken away.
    #[inline(never)]
    fn arp_let_go(&mut self, id: u16) {
        for v in self.voices.iter_mut() {
            if v.state == VoiceState::Held && v.arp.owner == id {
                if let Some(p) = self.parts.get_mut(v.part as usize) {
                    p.forget(v.handle, v.key);
                }
                if v.unheard() {
                    Engine::cut(&mut self.parts, v);
                } else {
                    v.release();
                }
            }
        }
    }

    /// Cut the oldest of an arpeggiator's notes that are dying away until
    /// it has no more than `room` voices.
    #[inline(never)]
    fn arp_trim(&mut self, id: u16, room: usize) {
        loop {
            let mut count = 0;
            let mut oldest: Option<usize> = None;
            for (i, v) in self.voices.iter().enumerate() {
                if v.active() && v.arp.owner == id {
                    count += 1;
                    if v.state == VoiceState::Released && oldest.is_none_or(|j| v.born < self.voices[j].born) {
                        oldest = Some(i);
                    }
                }
            }
            match oldest {
                Some(i) if count > room => Engine::cut(&mut self.parts, &mut self.voices[i]),
                _ => return,
            }
        }
    }

    /// Strike one note for an arpeggiator: an ordinary note of its part,
    /// marked as its own.
    #[inline(never)]
    fn arp_note_on(&mut self, a: &Arpeggiator, shift: f32, velocity: f32) {
        let pi = a.part as usize;
        let before = self.born;
        // An arpeggio waiting for the part's next note is not for this one.
        let waiting = core::mem::replace(&mut self.parts[pi].next_arp, ArpSpec::NONE);
        // (Through `apply`, so that there is one copy of the code that starts a note.)
        self.apply(&Event::note_on(0.0, a.part, a.handle, a.root + shift, velocity));
        self.parts[pi].next_arp = waiting;
        // In mono and legato mode the part's one voice has moved to the note.
        let mono = self.parts[pi].p[MODE] as u8 != 0;
        for v in self.voices.iter_mut() {
            if !(v.active() && v.part == a.part) {
                continue;
            }
            if v.born > before {
                if a.sample >= 0 {
                    v.use_sample(a.sample as u16, a.position);
                }
                v.slide_volume(a.volume, 0.0);
                v.slide_pan(a.pan, 0.0);
            } else if !mono {
                continue;
            }
            v.arp.owner = a.id;
            v.arp.shift = shift;
            v.arp.shift_key = v.key;
        }
    }

    /// Begin a step: now, or with swing a little later.
    #[inline(never)]
    fn arp_take_step(&mut self, i: usize, frames: f32) {
        let mut a = self.arps[i];
        if !a.live || a.stopping {
            return;
        }
        // A step still waiting for its swing is not passed over.
        if a.strike_at != NEVER {
            self.arp_strike(i);
            a = self.arps[i];
        }
        let now = self.time;
        let period = a.spec.period(self.sample_rate, self.tempo());
        if period > 0.0 {
            a.length = period;
            a.due = if a.due.is_finite() { a.due + period } else { now as f64 + period };
            if a.due <= now as f64 {
                // Far behind (it should not happen): count again from here.
                a.due = now as f64 + period;
            }
        } else {
            if frames.is_finite() && frames > 0.0 {
                a.length = frames as f64;
            }
            a.due = f64::INFINITY;
        }
        let k = a.step;
        a.step = a.step.wrapping_add(1);
        // Swing puts every second step back, so the step before it is that
        // much longer and it is that much shorter.
        let swing = a.spec.swing as f64;
        let (wait, length) = if k % 2 == 1 { (swing * a.length, a.length * (1.0 - swing)) } else { (0.0, a.length * (1.0 + swing)) };
        a.strike_step = k;
        a.strike_length = length;
        a.strike_at = now.saturating_add(wait as u64);
        self.arps[i] = a;
        if a.strike_at <= now {
            self.arp_strike(i);
        }
    }

    /// Sound the step that is waiting.
    #[inline(never)]
    fn arp_strike(&mut self, i: usize) {
        let mut a = self.arps[i];
        a.strike_at = NEVER;
        let now = self.time;
        let sr = self.sample_rate;
        let mut notes = [0i16; MAX_OFFSETS];
        let n = a.spec.notes(a.strike_step, &mut a.rng, &mut notes);
        self.wake();
        if !a.spec.struck() {
            // One note, whose pitch jumps. It is struck once, on the first step.
            let id = a.id;
            let held = |v: &Voice| v.state == VoiceState::Held && v.arp.owner == id;
            if !self.voices.iter().any(held) {
                if a.begun {
                    // Its note has gone, and the arpeggio with it.
                    self.arps[i].live = false;
                    return;
                }
                self.arp_note_on(&a, 0.0, a.velocity);
            }
            a.begun = true;
            for v in self.voices.iter_mut() {
                if !held(v) {
                    continue;
                }
                if v.arp.shift != 0.0 {
                    // It was a struck note until now.
                    v.arp.shift = 0.0;
                    v.set_pitch(a.root, 0.0, sr);
                }
                v.arp.offset = notes[0] as f32;
                v.pitch_jump = true;
            }
            a.off_at = NEVER;
        } else {
            self.arp_let_go(a.id);
            let level = a.spec.level(a.strike_step);
            if level > 0.0 {
                let unison = (self.parts[a.part as usize].p[UNISON] as usize).clamp(1, 8);
                self.arp_trim(a.id, ARPEGGIATOR_VOICES.saturating_sub(n * unison));
                for &note in &notes[..n] {
                    self.arp_note_on(&a, note as f32, (a.velocity * level).clamp(0.0, 1.0));
                }
                self.arp_trim(a.id, ARPEGGIATOR_VOICES);
            }
            let gate = a.spec.gate as f64;
            a.off_at = if gate > 0.0 && gate < 1.0 && a.strike_length > 0.0 { now.saturating_add(((gate * a.strike_length) as u64).max(1)) } else { NEVER };
        }
        self.arps[i] = a;
    }

    /// Whatever the arpeggiators have to do on this frame. Returns the
    /// next frame any of them has something to do on.
    #[inline(never)]
    pub(super) fn run_arps(&mut self) -> u64 {
        let now = self.time;
        let mut next = NEVER;
        for i in 0..MAX_ARPEGGIATORS {
            if !self.arps[i].live {
                continue;
            }
            if self.arps[i].off_at <= now {
                // The gate closes: the step's notes are let go.
                self.arps[i].off_at = NEVER;
                self.arp_let_go(self.arps[i].id);
                self.wake();
                if self.arps[i].stopping {
                    self.arps[i].live = false;
                    continue;
                }
            }
            if self.arps[i].strike_at <= now {
                self.arp_strike(i);
            }
            if self.arps[i].due.is_finite() && self.arps[i].due as u64 <= now {
                self.arp_take_step(i, 0.0);
            }
            if self.arps[i].live {
                next = next.min(self.arps[i].next_frame());
            }
        }
        next
    }

    /// Stop the arpeggiators of a handle. With `finish`, the step that is
    /// sounding is held for as long as its gate says and then let go (a
    /// note whose pitch was jumping goes back to its own pitch and stays);
    /// without, they are simply gone, and their voices are the caller's
    /// business.
    #[inline(never)]
    pub(super) fn arp_end(&mut self, part: u16, handle: u32, finish: bool) {
        for i in 0..MAX_ARPEGGIATORS {
            let a = self.arps[i];
            if !a.is(part, handle) {
                continue;
            }
            self.arps[i].live = false;
            if !finish {
                continue;
            }
            if !a.spec.struck() {
                for v in self.voices.iter_mut() {
                    if v.active() && v.arp.owner == a.id {
                        v.arp.offset = 0.0;
                        v.pitch_jump = true;
                    }
                }
                self.dirty = true;
            } else if a.off_at != NEVER {
                self.arps[i] = Arpeggiator { stopping: true, due: f64::INFINITY, strike_at: NEVER, ..a };
            }
        }
    }

    /// Stop every arpeggiator of a part (or, with `ALL_PARTS`, of every
    /// part); their voices are the caller's business.
    pub(super) fn arp_end_part(&mut self, part: u16) {
        for a in self.arps.iter_mut() {
            if part == ALL_PARTS || a.part == part {
                a.live = false;
            }
        }
    }

    /// Stop every arpeggiator.
    pub(super) fn arp_reset(&mut self) {
        self.arps.fill(Arpeggiator::NONE);
    }

    /// A `note_off`: the arpeggiators it means end, and their notes are let
    /// go. With no handle, it means those on the note.
    pub(super) fn arp_note_off(&mut self, part: u16, handle: u32, note: f32) {
        for i in 0..MAX_ARPEGGIATORS {
            let a = self.arps[i];
            let hit = a.live && a.part == part && if handle != 0 { a.handle == handle } else { (a.root - note).abs() < 0.005 };
            if hit {
                self.arps[i].live = false;
                self.arp_let_go(a.id);
            }
        }
    }

    /// The handle's notes have been moved to a note: the arpeggio is on
    /// that note from now on. (Its sounding voices are moved by the caller,
    /// each keeping its distance from the note: `Arp::shift`.)
    pub(super) fn arp_root(&mut self, part: u16, handle: u32, note: f32) {
        for a in self.arps.iter_mut().filter(|a| a.is(part, handle)) {
            a.root = note;
        }
    }

    /// The same, from a rate of the sample the arpeggio plays.
    pub(super) fn arp_rate(&mut self, part: u16, handle: u32, rate: f32) {
        for i in 0..MAX_ARPEGGIATORS {
            let a = self.arps[i];
            if !a.is(part, handle) {
                continue;
            }
            let slot = if a.sample >= 0 { a.sample as usize } else { self.parts[a.part as usize].p[OSC1_SAMPLE] as usize };
            if let Some(Some(sample)) = self.samples.get(slot) {
                self.arps[i].root = Engine::note_for_rate(sample, rate);
            }
        }
    }

    /// The handle's notes have been given a volume (or, with `pan`, a pan):
    /// the notes still to come have it too.
    pub(super) fn arp_level(&mut self, part: u16, handle: u32, pan: bool, value: f32) {
        for a in self.arps.iter_mut().filter(|a| a.is(part, handle)) {
            if pan {
                a.pan = value;
            } else {
                a.volume = value;
            }
        }
    }

    /// Have the handle's arpeggiator play a sample slot as oscillator 1 of
    /// every note it strikes, from `position` frames in, whatever the
    /// part's patch says (`None`: as the patch says). See `Voices::arp_sample`.
    pub(crate) fn arp_sample(&mut self, part: u16, handle: u32, slot: Option<u16>, position: f64) {
        for a in self.arps.iter_mut().filter(|a| a.is(part, handle)) {
            a.sample = slot.map_or(-1, |s| s as i32);
            a.position = if position.is_finite() { position.max(0.0) } else { 0.0 };
        }
    }

    /// An event about arpeggios. True if it has been dealt with here;
    /// false if it is for a voice's own arpeggio (`arp` and `arp_more`).
    #[inline(never)]
    pub(super) fn arp_event(&mut self, e: &Event) -> bool {
        let (part, handle) = (e.part, e.a);
        let (now, sr, tempo) = (self.time, self.sample_rate, self.tempo());
        let change = |spec: &mut ArpSpec| match e.kind {
            EV_ARP => *spec = ArpSpec::from_event(e.x, e.y, e.z, e.b),
            EV_ARP_MORE => spec.set_packed(4, e.b),
            EV_ARP_STYLE => spec.set_style(e.x, e.y, e.z, e.b),
            EV_ARP_BEAT => spec.set_beat(e.x),
            EV_ARP_NOTES => spec.set_notes(e.x, e.y, e.z, e.b),
            _ => {}
        };
        if handle == 0 {
            // For the next note of the part.
            if matches!(e.kind, EV_ARP_STYLE | EV_ARP_BEAT | EV_ARP_NOTES) {
                if let Some(p) = self.parts.get_mut(part as usize) {
                    change(&mut p.next_arp);
                }
            }
            return e.kind != EV_ARP && e.kind != EV_ARP_MORE;
        }
        let mut found = false;
        for i in 0..MAX_ARPEGGIATORS {
            let a = self.arps[i];
            if !a.is(part, handle) {
                continue;
            }
            if e.kind == EV_ARP_STEP {
                found = true;
                if a.spec.period(sr, tempo) == 0.0 {
                    self.arp_take_step(i, e.x);
                }
                continue;
            }
            let mut spec = a.spec;
            change(&mut spec);
            if !spec.wants_arpeggiator() {
                // Taken away, or made a voice's own arpeggio again.
                self.arp_end(a.part, handle, true);
                continue;
            }
            found = true;
            let a = &mut self.arps[i];
            a.spec = spec;
            if e.kind == EV_ARP {
                // `arp` starts the pattern again, at once.
                a.step = 0;
                a.strike_at = NEVER;
                a.due = now as f64;
            } else if spec.period(sr, tempo) == 0.0 {
                // No clock. (One that has yet to begin still takes its first step.)
                if a.step > 0 {
                    a.due = f64::INFINITY;
                }
            } else if !a.due.is_finite() {
                // It has been given a clock: the next step is a step from now.
                a.due = now as f64 + spec.period(sr, tempo);
            }
        }
        self.arp_at = self.time;
        if found || e.kind == EV_ARP_STEP {
            return true;
        }
        if e.kind == EV_ARP_MORE {
            return false;
        }
        // No arpeggiator has the handle: one takes over each sounding note
        // that has it, if what is asked is for an arpeggiator to play.
        let mut taken = false;
        for vi in 0..MAX_VOICES {
            let v = self.voices[vi];
            if !(v.active() && v.handle == handle && (part == ALL_PARTS || v.part == part)) || self.arps.iter().any(|a| a.is(v.part, handle)) {
                continue;
            }
            let mut spec = v.arp.spec();
            change(&mut spec);
            if !spec.wants_arpeggiator() {
                continue;
            }
            taken = true;
            let i = self.arp_new(v.part, handle, v.current_note(), v.velocity(), spec);
            let a = &mut self.arps[i];
            a.volume = v.volume;
            a.pan = v.pan;
            let id = a.id;
            // A voice that was stepping by itself hands its place over.
            let (step, due) = if v.arp.active() && e.kind != EV_ARP { self.voices[vi].arp.hand_over() } else { (0, 0.0) };
            for w in self.voices.iter_mut() {
                if w.active() && w.handle == handle && w.part == v.part {
                    if w.arp.active() {
                        w.arp.hand_over();
                    }
                    w.arp.owner = id;
                    w.arp.shift = 0.0;
                }
            }
            if v.arp.active() && e.kind != EV_ARP {
                let a = &mut self.arps[i];
                a.step = step as u32 + 1;
                a.due = if due > 0.0 && spec.period(sr, tempo) > 0.0 { due } else { f64::INFINITY };
                if due == 0.0 && spec.period(sr, tempo) > 0.0 {
                    a.due = now as f64 + spec.period(sr, tempo);
                }
            }
        }
        // `arp` for a voice's own arpeggio is the caller's to carry out.
        taken || e.kind != EV_ARP
    }
}
