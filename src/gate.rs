//! The trance gate: a part's sound chopped in a pattern of steps that keeps
//! time with the tempo. Each step has a level, the gate moves from one to
//! the next quickly but never at once (so it does not click), and it sits
//! after the part's voices and effects, so that it chops whatever the part
//! is playing: a held chord, a pad, the live input.

/// The most steps a gate's pattern can have.
pub const MAX_GATE_STEPS: usize = 32;

/// The names of the gate patterns that are built in, and what each is.
pub const NAMED_GATES: &[(&str, &str)] = &[
    ("eighths", "x.x.x.x.x.x.x.x."),
    ("offbeat", "..x...x...x...x."),
    ("gallop", "x.xxx.xxx.xxx.xx"),
    ("trance", "x.x.xx.xx.x.xx.x"),
    ("stutter", "xxx.xx.xx.xxx.x."),
    ("pump", "2468246824682468"),
    ("triplets", "x..x..x..x..x..x..x..x.."),
    ("dotted", "x..x..x.x..x..x."),
];

/// What is wrong with a gate pattern's text, in words.
pub const GATE_HELP: &str = "a gate pattern is one character a step: x for open, . for closed, 0 to 9 for a level between (9 is open); spaces are passed over; at most 32 steps; or the name of a built-in pattern";

/// The levels a gate pattern's text means: `"x.x.xx.x"`, or a name.
pub fn parse(text: &str) -> Result<([f32; MAX_GATE_STEPS], usize), &'static str> {
    let text = text.trim();
    let text = NAMED_GATES.iter().find(|(name, _)| *name == text).map_or(text, |(_, steps)| steps);
    let mut levels = [0.0; MAX_GATE_STEPS];
    let mut n = 0;
    for c in text.bytes() {
        let level = match c {
            b' ' | b'|' => continue,
            b'x' | b'X' => 1.0,
            b'.' | b'-' => 0.0,
            b'0'..=b'9' => (c - b'0') as f32 / 9.0,
            _ => return Err(GATE_HELP),
        };
        if n == MAX_GATE_STEPS {
            return Err(GATE_HELP);
        }
        levels[n] = level;
        n += 1;
    }
    if n == 0 {
        return Err(GATE_HELP);
    }
    Ok((levels, n))
}

/// The text for a list of levels, if text can say them: each must be one
/// of the ten levels a character can give.
pub fn to_text(levels: &[f32]) -> Option<String> {
    let mut out = String::new();
    for &level in levels.iter().take(MAX_GATE_STEPS) {
        let ninth = (level * 9.0).round();
        if !(0.0..=9.0).contains(&ninth) || (ninth / 9.0 - level).abs() > 1.0e-6 {
            return None;
        }
        out.push(match ninth as u8 {
            0 => '.',
            9 => 'x',
            n => (b'0' + n) as char,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// How a part's gate is set, as its parameters have it.
#[derive(Clone, Copy)]
pub struct Settings {
    /// How far a closed step closes, 0..1; 0 is no gate.
    pub depth: f32,
    /// Steps to a beat.
    pub rate: f32,
    /// How much of the way to its new level the gate goes each frame.
    pub ease: f32,
    pub swing: f32,
    /// Open steps go left and right by turns.
    pub alternate: bool,
}

/// One part's gate.
#[derive(Clone, Copy)]
pub struct Gate {
    levels: [f32; MAX_GATE_STEPS],
    len: u8,
    /// How open it is now, left and right.
    open: [f32; 2],
}

impl Gate {
    pub const fn new() -> Gate {
        Gate { levels: [0.0; MAX_GATE_STEPS], len: 0, open: [1.0; 2] }
    }

    /// Give it a pattern: a level for each step, 0 (closed) to 1 (open).
    /// None at all leaves it with open and closed by turns.
    pub fn set(&mut self, levels: &[f32]) {
        self.len = levels.len().min(MAX_GATE_STEPS) as u8;
        for (slot, &level) in self.levels.iter_mut().zip(levels) {
            *slot = if level.is_finite() { level.clamp(0.0, 1.0) } else { 0.0 };
        }
    }

    pub fn levels(&self) -> &[f32] {
        &self.levels[..self.len as usize]
    }

    /// Open at once, as an engine that is reset is.
    pub fn clear(&mut self) {
        self.open = [1.0; 2];
    }

    /// Has it anything to do: is it gating, or still opening after being switched off?
    #[inline]
    pub fn live(&self, depth: f32) -> bool {
        depth > 0.0 || self.open != [1.0; 2]
    }

    /// The step the gate is on at a beat, and whether it is an odd one.
    #[inline]
    fn step_at(&self, beat: f64, s: &Settings) -> usize {
        let at = beat * s.rate as f64;
        // Swing: of each two steps the first is longer and the second shorter.
        let pair = (at * 0.5).floor();
        let within = at - 2.0 * pair;
        let step = 2 * pair as i64 + (within >= 1.0 + s.swing as f64) as i64;
        step.rem_euclid(if self.len > 0 { self.len as i64 } else { 2 }) as usize
    }

    /// Gate a part's sound. The first frame is `since` frames after the
    /// frame that was on `beat`, and each frame moves the beat by `per_frame`.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32], beat: f64, since: u64, per_frame: f64, s: &Settings) {
        let [mut gl, mut gr] = self.open;
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let (mut tl, mut tr) = (1.0, 1.0);
            if s.depth > 0.0 {
                let step = self.step_at(beat + (since + i as u64) as f64 * per_frame, s);
                // (With no pattern: open and closed by turns.)
                let level = if self.len > 0 { self.levels[step] } else { (step % 2 == 0) as u8 as f32 };
                let open = 1.0 - s.depth * (1.0 - level);
                let shut = 1.0 - s.depth;
                (tl, tr) = match (s.alternate, step % 2) {
                    (false, _) => (open, open),
                    (true, 0) => (open, shut),
                    (true, _) => (shut, open),
                };
            }
            gl += (tl - gl) * s.ease;
            gr += (tr - gr) * s.ease;
            *l *= gl;
            *r *= gr;
        }
        if s.depth <= 0.0 && (1.0 - gl).abs() < 1.0e-4 && (1.0 - gr).abs() < 1.0e-4 {
            (gl, gr) = (1.0, 1.0);
        }
        self.open = [gl, gr];
    }
}

impl Default for Gate {
    fn default() -> Gate {
        Gate::new()
    }
}
