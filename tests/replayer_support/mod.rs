//! What the song and replayer tests share: a reference mixer, and ways to
//! build songs and module files in code.
#![allow(dead_code)]

pub mod build;
pub mod mixer;

pub use build::*;
pub use mixer::*;

/// The output rate the tests render at.
pub const RATE: f64 = 44100.0;

/// A small random number generator with a fixed start, so a test that
/// fails once fails every time.
pub struct Random(pub u64);

impl Random {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number from 0 up to but not including `n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A song that uses a bit of everything, for the loaders to be tried on.
pub fn busy_song(rules: synths_core::song::Rules) -> synths_core::song::Song {
    use synths_core::song::*;
    let xm = rules == Rules::FastTracker2;
    let mut bass = square(32, 8);
    bass.name = "bass".to_string();
    bass.finetune = 32;
    bass.volume = 48;
    let mut hit = ramp(600);
    hit.name = "hit".to_string();
    let mut pad = square(64, 4);
    pad.name = "pad".to_string();
    pad.loop_start = 64;
    pad.loop_len = 128;
    if xm {
        pad.loop_kind = LoopKind::PingPong;
        pad.relative_note = -12;
        pad.pan = Some(200);
        hit.data = SampleData::Bits16((0..600).map(|i| (i * 50 - 15000) as i16).collect());
    }
    let p0 = if xm {
        pattern(
            64,
            &[
                "C-4 01 40 000 | ... .. .. 000 | E-4 03 .. 000 | ... .. .. F04",
                "... .. .. A02 | D-4 02 .. 904 | ... .. C8 000 | ... .. .. 000",
                "=== .. .. 000 | ... .. .. E93 | G-4 03 F2 000 | ... .. .. G20",
                "C-5 01 .. 301 | ... .. .. 000 | ... .. .. R23 | ... .. .. 000",
                "... .. .. X11 | ... .. .. T12 | ... .. .. K03 | ... .. .. D00",
            ],
        )
    } else {
        pattern(
            64,
            &[
                "C-4 01 000 | ... .. 000 | E-4 03 000 | ... .. F04",
                "... .. A02 | D-4 02 904 | ... .. 000 | ... .. 000",
                "... .. 000 | ... .. E93 | G-4 03 000 | ... .. 000",
                "C-5 01 301 | ... .. 000 | ... .. 000 | ... .. 000",
                "... .. E61 | ... .. EE2 | ... .. 47A | ... .. D00",
            ],
        )
    };
    let p1 = pattern(64, &[if xm { "A-4 02 .. 000 | ... .. .. 000 | ... .. .. 000 | ... .. .. B00" } else { "A-4 02 000 | ... .. 000 | ... .. 000 | ... .. B00" }]);
    let mut s = song(rules, 4, vec![bass, hit, pad], vec![p0, p1]);
    s.title = "busy song".to_string();
    if xm {
        let i = &mut s.instruments[0];
        i.volume_envelope = Envelope { enabled: true, points: vec![(0, 64), (8, 32), (20, 0)], sustain: Some(1), loop_points: None };
        i.pan_envelope = Envelope { enabled: true, points: vec![(0, 32), (10, 64), (20, 0)], sustain: None, loop_points: Some((0, 2)) };
        i.vibrato = AutoVibrato { waveform: 0, sweep: 16, depth: 8, rate: 24 };
        i.fadeout = 2048;
    }
    s
}

/// What the replayer asked of the voices on each of the first `n` ticks:
/// the channel and the command, in the order they came.
pub fn run_ticks(song: &synths_core::song::Song, n: usize) -> Vec<Vec<(usize, Command)>> {
    let mut player = synths_core::replayer::Player::new(song, RATE);
    let mut mixer = Mixer::new(song, RATE);
    (0..n)
        .map(|_| {
            let before = mixer.log.len();
            player.tick(song, 0, &mut mixer);
            mixer.log[before..].iter().map(|l| (l.channel, l.command)).collect()
        })
        .collect()
}

/// A channel's (rate, volume, pan) after each tick, carried forward from
/// tick to tick; all 0 before its first note.
pub fn track(ticks: &[Vec<(usize, Command)>], channel: usize) -> Vec<(f64, f32, f32)> {
    let mut now = (0.0, 0.0, 0.0);
    ticks
        .iter()
        .map(|tick| {
            for (c, command) in tick {
                if *c != channel {
                    continue;
                }
                match *command {
                    Command::Play(s) => now = (s.rate, s.volume, s.pan),
                    Command::Rate(r) => now.0 = r,
                    Command::Volume(v, _) => now.1 = v,
                    Command::Pan(p, _) => now.2 = p,
                    Command::Stop => now.1 = 0.0,
                    _ => {}
                }
            }
            now
        })
        .collect()
}

/// The ticks on which a channel started a sample, with the sample and offset.
pub fn plays(ticks: &[Vec<(usize, Command)>], channel: usize) -> Vec<(usize, usize, u32)> {
    let mut out = Vec::new();
    for (t, tick) in ticks.iter().enumerate() {
        for (c, command) in tick {
            if let (true, Command::Play(s)) = (*c == channel, command) {
                if let synths_core::replayer::Source::Sample(i) = s.source {
                    out.push((t, i, s.offset));
                }
            }
        }
    }
    out
}

/// The Amiga period a rate comes from.
pub fn amiga_period(rate: f64) -> u32 {
    (3_546_894.6 / rate).round() as u32
}

/// The FastTracker 2 linear period a rate comes from: 64 to the semitone,
/// falling as the pitch rises, with C-4 (8363 frames a second) at 4608.
pub fn linear_period(rate: f64) -> i32 {
    (4608.0 - 768.0 * (rate / 8363.0).log2()).round() as i32
}

/// A volume from 0 to 1 as the trackers count it, 0 to 64.
pub fn volume64(v: f32) -> i32 {
    (v * 64.0).round() as i32
}
