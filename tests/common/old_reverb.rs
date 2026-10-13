//! The reverb the engine had before `synths_core::reverb`: the classic
//! "Freeverb" layout (Jezar at Dreampoint, public domain), eight echoing
//! combs side by side and four all-pass filters in a row. It is kept here,
//! out of the engine, so that the tests and `examples/reverb.rs` can measure
//! and render the new reverb against it.
#![allow(dead_code)]

fn zeros(n: usize) -> Box<[f32]> {
    vec![0.0; n].into_boxed_slice()
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-15 {
        0.0
    } else {
        x
    }
}

struct Comb {
    buf: Box<[f32]>,
    at: usize,
    store: f32,
}

impl Comb {
    #[inline]
    fn process(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let y = self.buf[self.at];
        self.store = flush(y * (1.0 - damp) + self.store * damp);
        self.buf[self.at] = x + self.store * feedback;
        self.at += 1;
        if self.at >= self.buf.len() {
            self.at = 0;
        }
        y
    }
}

struct Allpass {
    buf: Box<[f32]>,
    at: usize,
}

impl Allpass {
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.at];
        self.buf[self.at] = flush(x + b * 0.5);
        self.at += 1;
        if self.at >= self.buf.len() {
            self.at = 0;
        }
        b - x
    }
}

// The lengths, in samples at 44.1 kHz, of the classic "Freeverb" layout
// (Jezar at Dreampoint, public domain): eight echoing combs side by side,
// then four all-pass filters in a row to smear them.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

/// The old reverb.
pub struct OldReverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl OldReverb {
    pub fn new(sample_rate: f32) -> OldReverb {
        let scale = sample_rate / 44100.0;
        let len = |n: usize| ((n as f32 * scale) as usize).max(1);
        let side = |spread: usize| -> (Vec<Comb>, Vec<Allpass>) {
            (
                COMBS.iter().map(|&n| Comb { buf: zeros(len(n + spread)), at: 0, store: 0.0 }).collect(),
                ALLPASSES.iter().map(|&n| Allpass { buf: zeros(len(n + spread)), at: 0 }).collect(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(STEREO_SPREAD);
        OldReverb { combs: [cl, cr], allpasses: [al, ar] }
    }

    pub fn clear(&mut self) {
        for side in self.combs.iter_mut() {
            for c in side.iter_mut() {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
        }
        for side in self.allpasses.iter_mut() {
            for a in side.iter_mut() {
                a.buf.fill(0.0);
            }
        }
    }

    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        size: f32,
        damping: f32,
    ) {
        let feedback = 0.7 + 0.28 * size.clamp(0.0, 1.0);
        let damp = 0.4 * damping.clamp(0.0, 1.0);
        for i in 0..in_l.len() {
            let x = (in_l[i] + in_r[i]) * 0.015;
            for (side, out) in [&mut out_l[i], &mut out_r[i]].into_iter().enumerate() {
                let mut y = 0.0;
                for c in self.combs[side].iter_mut() {
                    y += c.process(x, feedback, damp);
                }
                for a in self.allpasses[side].iter_mut() {
                    y = a.process(y);
                }
                *out += y;
            }
        }
    }
}
