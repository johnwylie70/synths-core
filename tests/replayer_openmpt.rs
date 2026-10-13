//! The OpenMPT player tests for MOD and XM: small modules, each built to
//! show one piece of tracker behaviour.
//!
//! The modules are not in the repository (they carry no licence). Fetch
//! them with `scripts/fetch-openmpt-tests.sh`; without them these tests
//! pass without doing anything.

mod replayer_support;
use replayer_support::*;
use std::path::PathBuf;
use synths_core::replayer::{Player, Source, Start};
use synths_core::song::{load_module, Song};

fn test_dir(kind: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/openmpt-tests").join(kind);
    dir.is_dir().then_some(dir)
}

thread_local! {
    /// Which player the test running on this thread renders with: the plain
    /// mixer, or the real engine.
    static THROUGH_ENGINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The same through the real engine: the song installed as its sequencer on
/// parts 32 up, with the master set so that the level matches the mixer's.
fn render_engine(song: &Song, seconds: f64, heard: impl Fn(usize) -> bool) -> (Vec<f32>, Vec<f32>) {
    use synths_core::event::{SONG_PLAY, SONG_REPEAT};
    use synths_core::params::{LIMITER, MASTER_VOLUME, VOLUME};
    use synths_core::song_player::{install, STATE_PLAYING};
    let mut engine = synths_core::Engine::new(RATE as f32);
    let shared = install(&mut engine, song.clone(), 32, 0, true).unwrap();
    engine.set_param(0, MASTER_VOLUME, 0.5);
    engine.set_param(0, LIMITER, 0.0);
    for c in (0..song.channels()).filter(|&c| !heard(c)) {
        engine.set_param(32 + c as u16, VOLUME, 0.0);
    }
    engine.push(synths_core::Event::song(0.0, SONG_REPEAT, 0.0, 0.0));
    engine.push(synths_core::Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut block = ([0.0f32; 1024], [0.0f32; 1024]);
    loop {
        engine.render(&mut block.0, &mut block.1);
        left.extend_from_slice(&block.0);
        right.extend_from_slice(&block.1);
        if shared.position()[0] != STATE_PLAYING || left.len() as f64 >= seconds * RATE {
            return (left, right);
        }
    }
}

/// Renders a song until it ends, or for at most `seconds`, hearing only
/// the channels `heard` picks.
fn render_some(song: &Song, seconds: f64, heard: impl Fn(usize) -> bool) -> (Vec<f32>, Vec<f32>) {
    if THROUGH_ENGINE.get() {
        return render_engine(song, seconds, heard);
    }
    let mut player = Player::new(song, RATE);
    player.set_repeat(false);
    let mut mixer = Mixer::new(song, RATE);
    mixer.muted = (0..song.channels()).map(|c| !heard(c)).collect();
    let most = (seconds * RATE) as usize;
    while !player.ended() && mixer.left.len() < most {
        play(song, &mut player, &mut mixer, 1024);
    }
    (mixer.left, mixer.right)
}

fn render(song: &Song, seconds: f64) -> (Vec<f32>, Vec<f32>) {
    render_some(song, seconds, |_| true)
}

/// Everything the replayer asks of the voices until the song ends, or for
/// `seconds` if it is to go round and round.
fn instructions(song: &Song, seconds: f64, repeat: bool) -> Vec<Logged> {
    let mut player = Player::new(song, RATE);
    player.set_repeat(repeat);
    let mut mixer = Mixer::new(song, RATE);
    while (repeat || !player.ended()) && (mixer.left.len() as f64) < seconds * RATE {
        play(song, &mut player, &mut mixer, 1024);
    }
    mixer.log
}

/// The samples started, in order, each with the offset it started at.
fn starts(log: &[Logged]) -> Vec<(usize, u32)> {
    log.iter()
        .filter_map(|l| match l.command {
            Command::Play(Start { source: Source::Sample(s), offset, .. }) => Some((s, offset)),
            _ => None,
        })
        .collect()
}

/// The windows the two sides are compared in: `WINDOW` frames long, one
/// starting every `HOP` frames.
const WINDOW: usize = 1024;
const HOP: usize = 256;

fn windows(x: &[f32]) -> impl Iterator<Item = &[f32]> {
    (0..x.len().saturating_sub(WINDOW) / HOP + 1).map(move |i| &x[(i * HOP).min(x.len())..(i * HOP + WINDOW).min(x.len())])
}

/// How loud a signal is in each window.
fn loudness(x: &[f32]) -> Vec<f32> {
    windows(x).map(|w| (w.iter().map(|v| v * v).sum::<f32>() / w.len().max(1) as f32).sqrt()).collect()
}

/// How often a signal rises through its own middle in each window: a rough
/// measure of its pitch.
fn crossings(x: &[f32]) -> Vec<f32> {
    windows(x)
        .map(|w| {
            let mean = w.iter().sum::<f32>() / w.len().max(1) as f32;
            w.windows(2).filter(|p| p[0] < mean && p[1] >= mean).count() as f32
        })
        .collect()
}

/// How far apart the two sides of a render are.
struct Difference {
    /// The share of windows in which the sides differ in loudness or pitch.
    share: f32,
    /// Where the sides differ, in seconds.
    at: Vec<f64>,
    /// How far the right side lags the left, in seconds, at the best fit.
    lag: f64,
}

/// Compares the left of a render (this player) with the right (a recording
/// of the real tracker, played as a sample). The recording may start a
/// little early or late, so the best fit within a tenth of a second is
/// taken; and a window is forgiven if a neighbour fits, because notes on
/// the two sides can start a few milliseconds apart. `pitch` is how far
/// the pitch may differ, as a fraction: drums and noise need a lot of room.
fn compare(l: &[f32], r: &[f32], pitch: f32) -> Difference {
    let (ll, lr) = (loudness(l), loudness(r));
    let (cl, cr) = (crossings(l), crossings(r));
    let top = ll.iter().chain(&lr).fold(0.0f32, |m, &v| m.max(v)).max(1e-9);
    let n = ll.len() as i64;
    let same = |i: i64, j: i64| {
        if i < 0 || j < 0 || i >= n || j >= n {
            return false;
        }
        let (a, b) = (ll[i as usize], lr[j as usize]);
        let quiet = a < top * 0.03 && b < top * 0.03;
        let loud_ok = (a - b).abs() <= 0.3 * a.max(b) + top * 0.03;
        let (ca, cb) = (cl[i as usize], cr[j as usize]);
        // Pitch is only judged where both sides are well clear of silence: a
        // window that a note starts or dies away in has too few crossings.
        let pitch_ok = a.min(b) < top * 0.15 || (ca - cb).abs() <= pitch * ca.max(cb) + 2.0;
        quiet || (loud_ok && pitch_ok)
    };
    let reach = (RATE * 0.1) as i64 / HOP as i64;
    let mut best = Difference { share: f32::INFINITY, at: Vec::new(), lag: 0.0 };
    for lag in -reach..=reach {
        let bad: Vec<i64> = (0..n).filter(|&i| !(-3..=3).any(|near| same(i, i + lag + near))).collect();
        let share = bad.len() as f32 / n.max(1) as f32;
        if std::env::var("LAGS").is_ok() {
            println!("lag {lag:+} share {share:.3}");
        }
        if share < best.share || (share == best.share && lag.abs() < (best.lag * RATE / HOP as f64).abs() as i64) {
            best = Difference {
                share,
                at: bad.iter().map(|&i| i as f64 * HOP as f64 / RATE).collect(),
                lag: lag as f64 * HOP as f64 / RATE,
            };
        }
    }
    best
}

/// How a test module shows whether it was played correctly.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Expect {
    /// This player is on the left and a recording of the real tracker (or a
    /// second channel that must sound the same) is on the right.
    Sides,
    /// As `Sides`, but with pitch compared only roughly, because the sound
    /// is noise, speech or far above the range a crossing count can follow.
    SidesRoughPitch,
    /// The recording is in stereo on its own channels; the channels listed
    /// are this player's and must match it, left for left and right for right.
    Stereo(&'static [usize]),
    /// The module holds recordings of the real tracker's left and right
    /// output as two of its samples (by index), which it does not play; the
    /// render must match them.
    Recording(usize, usize),
    /// Two channels of this player's must sound the same, whatever their panning.
    Channels(usize, usize),
    /// Two channels of this player's must sound the same and be panned the same.
    ChannelsAndPanning(usize, usize),
    /// The module cancels itself out when played correctly. A click where a
    /// volume or panning changes is allowed, as the tests themselves say.
    Silent,
    /// These samples must be started, in this order, and no others. The
    /// modules that say "success" or count aloud are checked this way.
    Starts(&'static [usize]),
    /// As `Starts`, for a module that never ends: its first starts.
    FirstStarts(&'static [usize]),
    /// The samples in the first list are the voice saying "success" and at
    /// least one must be started; those in the second say "fail" and none may be.
    Says(&'static [usize], &'static [usize]),
    /// Samples must be started at exactly these offsets, and no more.
    Offsets(&'static [u32]),
    /// The second channel must always play at this multiple of the first's rate.
    InTune(usize, usize, f64),
    /// The module is judged by listening (a voice says "success", say), or
    /// its recording does not line up; a machine cannot mark it.
    ByEar,
    /// This player gets it wrong; the text says how.
    Fails(&'static str),
}

struct Case {
    name: &'static str,
    expect: Expect,
    /// The part of the render that is compared, in seconds.
    from: f64,
    until: f64,
    /// The most of the render that may differ.
    allow: f32,
}

const fn case(name: &'static str, expect: Expect) -> Case {
    Case { name, expect, from: 0.0, until: 60.0, allow: ALIKE }
}

use Expect::*;

/// The MOD tests, https://wiki.openmpt.org/Development:_Test_Cases/MOD
const MOD_CASES: &[Case] = &[
    // Two channels an octave apart that must stay in tune.
    case("AmigaLimitsFinetune.mod", InTune(1, 3, 2.0)),
    case("ArpWraparound.mod", SidesRoughPitch),
    // The recording is of XMPlay, which leaves out one retrigger (on the
    // first tick of the row with the delay) that ProTracker makes.
    Case { allow: 0.04, ..case("DelayBreak.mod", SidesRoughPitch) },
    case("InstrDelay.mod", Sides),
    case("InstrSwapRetrigger.mod", Sides),
    case("NoteDelay-NextRow.mod", SidesRoughPitch),
    // One channel; the swap must happen at about row 10 and stop at about row 18.
    case("PTInstrSwap.mod", ByEar),
    case("PTInstrVolume.mod", Silent),
    case("PTOffsetRetrigger.mod", Sides),
    case("PTRetrigger.mod", SidesRoughPitch),
    case("PTStoppedSwap.mod", Sides),
    case("PTSwapEmpty.mod", Sides),
    case("PTSwapNoLoop.mod", Sides),
    // The voice starts to say "success" twice and is cut off, then says it.
    case("PatLoop-Break.mod", Starts(&[0, 0, 0])),
    case("PatternDelaysRetrig.mod", Sides),
    case("PatternJump.mod", Says(&[1], &[0])),
    case("PortaSmpChange.mod", Sides),
    // Both sides are this player's and are not meant to match throughout.
    case("PortaSwapPT.mod", ByEar),
    case("PortaTarget.mod", ByEar),
    case("TempoChange.mod", Sides),
    case("VibratoReset.mod", Sides),
    case("finetune.mod", Sides),
    case("ptoffset.mod", Sides),
];

/// The XM tests, https://wiki.openmpt.org/Development:_Test_Cases/XM
const XM_CASES: &[Case] = &[
    case("3xx-no-old-samp.xm", Sides),
    case("3xxins.xm", Sides),
    case("ArpSlide.xm", Sides),
    case("Arpeggio.xm", Sides),
    case("ArpeggioClamp.xm", Sides),
    case("AutoVibratoSweepKeyOff.xm", Sides),
    // A steady buzz that must not phase.
    case("BidiPrecision.xm", ByEar),
    case("DelayCombination.xm", Silent),
    case("DelayVolume.xm", Silent),
    case("E90.xm", Silent),
    case("EnvLoops.xm", Sides),
    case("EnvOff.xm", Sides),
    case("FT2PanLaw.xm", Recording(1, 2)),
    case("FadeoutPickup.xm", Sides),
    case("FineVol-LinkMem.xm", Silent),
    case("Finetune-Precision.xm", Sides),
    case("FreqWraparound.xm", SidesRoughPitch),
    case("Glissando.xm", Sides),
    case("GlobalVolume.xm", Fails("a note left of a global volume command is heard when FT2 keeps it silent")),
    case("KeyOff2.xm", Sides),
    case("NoteLimit.xm", SidesRoughPitch),
    case("NoteLimit2.xm", SidesRoughPitch),
    case("NoteOff.xm", Sides),
    case("NoteOff2.xm", Sides),
    case("NoteOffFadeNoEnv.xm", Sides),
    case("NoteOffInstrChange.xm", Sides),
    case("NoteOffVolume.xm", Sides),
    case("Off-Porta.xm", Sides),
    case("OffDelay.xm", Sides),
    // Only the sample longer than the offset plays, once on each channel.
    case("OffsetRange.xm", Starts(&[0, 0])),
    // The word "correct" is heard once (the first start is at no volume);
    // the later notes are past the end of the sample and start nothing.
    case("OffsetWithoutNote.xm", Offsets(&[0x2200, 0x2200])),
    case("PanMemory.xm", ChannelsAndPanning(0, 1)),
    // One channel's panning jumps (an instrument number) where the other's
    // glides (a panning command), so two ticks of it do not quite cancel.
    Case { allow: 0.05, ..case("PanMemory2.xm", Silent) },
    case("PanOff.xm", Stereo(&[0])),
    case("PanSlideMem.xm", Silent),
    case("PanSlideZero.xm", Stereo(&[0])),
    case("PanSustainRelease.xm", Silent),
    case("PatLoop-Break.xm", ByEar),
    // The recording is looped, starts out of step, and (as FT2 plays it, a
    // touch flat) drifts away from the song; a stretch near the start is compared.
    Case { from: 1.0, until: 15.0, ..case("PatLoop-Infinite.xm", Sides) },
    case("PatLoop-Jumps.xm", Sides),
    // FT2 itself plays this one differently every time.
    case("PatLoop-Nested.xm", ByEar),
    case("PatLoop-Overflow.xm", Says(&[0], &[1])),
    // "Fail" is started three times, each a tick before it is silenced (the
    // "bit of quiet noise" the test describes), and then "success" is said.
    case("PatLoop-Various.xm", ByEar),
    // The voice counts "1 4 2" twice and then "3 4 2" for ever.
    case("PatLoop-Weird.xm", FirstStarts(&[0, 3, 1, 0, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1])),
    case("PatternDelay-NoteDelay.xm", Sides),
    // The second pattern tests a ModPlug effect that FT2 does not have.
    Case { until: 0.95, ..case("PatternDelays.xm", Sides) },
    case("PatternDelaysRetrig.xm", Sides),
    case("Pickup.xm", SidesRoughPitch),
    case("Porta-LinkMem.xm", Channels(0, 1)),
    case("Porta-Pickup.xm", Sides),
    case("PortaDelay.xm", Sides),
    case("PortaJustStoppedNote.xm", Sides),
    case("PortaResetDirection.xm", Silent),
    case("RetrigFade.xm", Sides),
    case("RetrigTick0.xm", SidesRoughPitch),
    case("SampleChange.xm", Sides),
    case("SamplePortaInInstrument.xm", Silent),
    case("SetEnvPos.xm", Silent),
    case("TonePortamentoMemory.xm", Sides),
    case("TremoloVibrato.xm", Sides),
    case("TremoloWaveforms.xm", Sides),
    case("Tremor.xm", Sides),
    case("TremorInstr.xm", Sides),
    case("TremorRecover.xm", Sides),
    case("VibratoCombinations.xm", Sides),
    case("VibratoDouble.xm", Sides),
    case("VibratoWaveforms.xm", Sides),
    case("VolColDelay.xm", Sides),
    case("delay1.xm", SidesRoughPitch),
    case("delay2.xm", Channels(0, 1)),
    case("delay3.xm", Sides),
    case("delaycut.xm", Sides),
    case("envretrig.xm", Sides),
    case("finetune.xm", Sides),
    case("key_off.xm", Sides),
    case("keyoff+instr.xm", Sides),
    case("lowest.xm", Sides),
    // The pattern header is an unusual size; it loads, which is the test.
    case("pathead.xm", ByEar),
    case("porta-delay.xm", Sides),
    case("porta-offset.xm", Sides),
    case("retrig.xm", SidesRoughPitch),
];

/// The most of a render that may differ for the two sides to count as alike.
const ALIKE: f32 = 0.03;
/// As loud as silence may be, where one channel at full volume is 0.25.
const SILENT: f32 = 0.01;

/// What happened when a case was played: `Ok` with a note of how close it
/// was, or `Err` with what went wrong.
fn judge(case: &Case, song: &Song) -> Result<String, String> {
    let cut = |x: Vec<f32>| -> Vec<f32> {
        let (a, b) = ((case.from * RATE) as usize, (case.until * RATE) as usize);
        x[a.min(x.len())..b.min(x.len())].to_vec()
    };
    let alike = |a: &[f32], b: &[f32], pitch: f32| {
        let d = compare(a, b, pitch);
        let note = format!("{:.1}% differs", d.share * 100.0);
        if d.share <= case.allow {
            Ok(note)
        } else {
            let at: Vec<String> = d.at.iter().step_by(4).take(6).map(|t| format!("{:.2}", t + case.from)).collect();
            Err(format!("{note}, at {at:?} seconds"))
        }
    };
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    match case.expect {
        Sides | SidesRoughPitch => {
            let (l, r) = render(song, 60.0);
            if peak(&l) < SILENT {
                return Err("nothing is heard".to_string());
            }
            alike(&cut(l), &cut(r), if case.expect == Sides { 0.1 } else { 0.6 })
        }
        Stereo(ours) => {
            let (l, r) = render_some(song, 60.0, |c| ours.contains(&c));
            let (rl, rr) = render_some(song, 60.0, |c| !ours.contains(&c));
            let left = alike(&cut(l), &cut(rl), 0.1)?;
            let right = alike(&cut(r), &cut(rr), 0.1)?;
            Ok(format!("left {left}, right {right}"))
        }
        Recording(left, right) => {
            let (l, r) = render(song, 60.0);
            let recorded = |i: usize| song.samples[i].data.to_f32().iter().map(|x| x * 0.25).collect::<Vec<f32>>();
            let (rl, rr) = (recorded(left), recorded(right));
            let n = l.len().min(rl.len());
            let left = alike(&l[..n], &rl[..n], 0.1)?;
            let right = alike(&r[..n], &rr[..n], 0.1)?;
            Ok(format!("left {left}, right {right}"))
        }
        Channels(a, b) | ChannelsAndPanning(a, b) => {
            let (al, ar) = render_some(song, 60.0, |c| c == a);
            let (bl, br) = render_some(song, 60.0, |c| c == b);
            if peak(&al).max(peak(&ar)) < SILENT {
                return Err("nothing is heard".to_string());
            }
            if case.expect == ChannelsAndPanning(a, b) {
                let left = alike(&cut(al), &cut(bl), 0.1)?;
                let right = alike(&cut(ar), &cut(br), 0.1)?;
                Ok(format!("left {left}, right {right}"))
            } else {
                let mono = |l: Vec<f32>, r: Vec<f32>| -> Vec<f32> { l.iter().zip(&r).map(|(x, y)| x + y).collect() };
                alike(&cut(mono(al, ar)), &cut(mono(bl, br)), 0.1)
            }
        }
        Silent => {
            let (l, r) = render(song, 60.0);
            let loud: Vec<f32> = loudness(&l).iter().zip(loudness(&r)).map(|(a, b)| a.max(b)).collect();
            let heard = loud.iter().filter(|&&x| x > SILENT).count() as f32 / loud.len().max(1) as f32;
            if heard <= case.allow {
                Ok(format!("peak {:.4}, {:.1}% of it clicks", peak(&l).max(peak(&r)), heard * 100.0))
            } else {
                Err(format!("should be silent, but {:.1}% of it is not", heard * 100.0))
            }
        }
        Starts(want) | FirstStarts(want) => {
            let repeat = matches!(case.expect, FirstStarts(_));
            let got: Vec<usize> = starts(&instructions(song, 20.0, repeat)).iter().map(|s| s.0).collect();
            let seen = if matches!(case.expect, FirstStarts(_)) { &got[..want.len().min(got.len())] } else { &got[..] };
            if seen == want {
                Ok(format!("samples {want:?} in order"))
            } else {
                Err(format!("samples started were {seen:?}, not {want:?}"))
            }
        }
        Says(success, fail) => {
            let got: Vec<usize> = starts(&instructions(song, 20.0, false)).iter().map(|s| s.0).collect();
            if got.iter().any(|s| fail.contains(s)) {
                Err(format!("it says \"fail\": samples started were {got:?}"))
            } else if !got.iter().any(|s| success.contains(s)) {
                Err(format!("it never says \"success\": samples started were {got:?}"))
            } else {
                Ok("says \"success\"".to_string())
            }
        }
        Offsets(want) => {
            let got: Vec<u32> = starts(&instructions(song, 20.0, false)).iter().map(|s| s.1).collect();
            if got == want {
                Ok(format!("offsets {want:?}"))
            } else {
                Err(format!("offsets were {got:?}, not {want:?}"))
            }
        }
        InTune(a, b, ratio) => {
            // Each channel's rate after every frame on which either changed.
            let log = instructions(song, 20.0, false);
            let mut rates = [0.0f64; 64];
            let mut checked = 0;
            for (i, l) in log.iter().enumerate() {
                match l.command {
                    Command::Play(start) => rates[l.channel] = start.rate,
                    Command::Rate(rate) => rates[l.channel] = rate,
                    _ => continue,
                }
                let last_of_frame = log.get(i + 1).is_none_or(|next| next.frame != l.frame);
                if last_of_frame && rates[a] > 0.0 && rates[b] > 0.0 {
                    checked += 1;
                    let got = rates[b] / rates[a];
                    if (got / ratio - 1.0).abs() > 0.005 {
                        return Err(format!("at {:.2} seconds the rates are {got:.4} apart, not {ratio}", l.frame as f64 / RATE));
                    }
                }
            }
            if checked == 0 {
                return Err("the channels never play together".to_string());
            }
            Ok(format!("in tune at {checked} moments"))
        }
        ByEar | Fails(_) => unreachable!(),
    }
}

/// Plays every case of one kind and checks it. Returns (passed, failing as
/// expected, not machine-checkable).
fn run(kind: &str, cases: &[Case], engine: bool) -> Option<(usize, usize, usize)> {
    let dir = test_dir(kind)?;
    THROUGH_ENGINE.set(engine);
    let through = if engine { " through the engine" } else { "" };
    let (mut passed, mut failing, mut by_ear) = (0, 0, 0);
    let mut surprises = Vec::new();
    for case in cases {
        let data = std::fs::read(dir.join(case.name)).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let song = load_module(&data).unwrap_or_else(|e| panic!("{} does not load: {e}", case.name));
        match case.expect {
            ByEar => {
                // It must at least play without trouble.
                render(&song, 10.0);
                by_ear += 1;
                println!("{:28} by ear", case.name);
            }
            Fails(why) => {
                // Checked as a comparison of sides, to notice if it starts passing.
                let now = judge(&Case { expect: Sides, ..*case }, &song);
                failing += 1;
                println!("{:28} FAILS (known): {why}", case.name);
                if now.is_ok() {
                    surprises.push(format!("{} is listed as failing but now passes", case.name));
                }
            }
            // What is checked by the replayer's instructions has no sound to
            // judge, so it says nothing about the engine.
            Starts(_) | FirstStarts(_) | Says(..) | Offsets(_) | InTune(..) if engine => {
                by_ear += 1;
                println!("{:28} not about sound", case.name);
            }
            _ => match judge(case, &song) {
                Ok(note) => {
                    passed += 1;
                    println!("{:28} pass   {note}", case.name);
                }
                Err(why) if engine => {
                    failing += 1;
                    println!("{:28} DIFFERS{through}: {why}", case.name);
                    if !ENGINE_DIFFERS.contains(&case.name) && !matches!(case.expect, Fails(_)) {
                        surprises.push(format!("{}: {why}", case.name));
                    }
                }
                Err(why) => surprises.push(format!("{}: {why}", case.name)),
            },
        }
    }
    println!("{kind}{through}: {passed} pass, {failing} fail, {by_ear} not machine-checkable, of {}", cases.len());
    assert!(surprises.is_empty(), "{kind} tests{through}:\n{}", surprises.join("\n"));
    Some((passed, failing, by_ear))
}

/// The cases whose sound through the engine is known to differ from the
/// recording by more than the comparison allows.
const ENGINE_DIFFERS: &[&str] = &[
    // Periods wrapped round to rates far above what the output can carry:
    // what is heard is all aliasing, and the engine's interpolation aliases
    // differently from FT2's.
    "FreqWraparound.xm",
    // 4 to 5% against the 3% allowed, at the moments a sample is swapped at
    // the end of its loop: the engine makes the swap within 16 frames of the
    // moment and takes the click off it.
    "PTSwapEmpty.mod",
    "PortaSmpChange.mod",
];

#[test]
fn mod_cases_through_the_engine() {
    run("mod", MOD_CASES, true);
}

#[test]
fn xm_cases_through_the_engine() {
    run("xm", XM_CASES, true);
}

#[test]
fn mod_cases() {
    if run("mod", MOD_CASES, false).is_none() {
        println!("the OpenMPT MOD tests are not here; run scripts/fetch-openmpt-tests.sh");
    }
}

#[test]
fn xm_cases() {
    if run("xm", XM_CASES, false).is_none() {
        println!("the OpenMPT XM tests are not here; run scripts/fetch-openmpt-tests.sh");
    }
}

#[test]
fn every_fetched_module_has_a_case() {
    for (kind, cases) in [("mod", MOD_CASES), ("xm", XM_CASES)] {
        let Some(dir) = test_dir(kind) else { continue };
        for entry in std::fs::read_dir(dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(cases.iter().any(|c| c.name == name), "{name} has no case");
        }
    }
}

/// Prints one module's two sides window by window, to see where they part.
/// `DETAIL=mod/ArpWraparound.mod FROM=0 TO=1 cargo test ... detail -- --ignored --nocapture`
/// With `BAD=1`, only the windows that differ are shown.
#[test]
#[ignore]
fn detail() {
    let Ok(names) = std::env::var("DETAIL") else { return };
    let number = |key: &str, default: f64| std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    let (from, to, pitch) = (number("FROM", 0.0), number("TO", 1e9), number("PITCH", 0.1) as f32);
    let only_bad = std::env::var("BAD").is_ok();
    for name in names.split(',') {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/openmpt-tests").join(name);
        let song = load_module(&std::fs::read(path).unwrap()).unwrap();
        let (l, r) = render(&song, 60.0);
        let (ll, lr) = (loudness(&l), loudness(&r));
        let (cl, cr) = (crossings(&l), crossings(&r));
        let d = compare(&l, &r, pitch);
        let hz = RATE as f32 / WINDOW as f32;
        println!("== {name}: tick {:.1} ms, differ {:.3}, lag {:+.3}", 2500.0 / song.tempo as f64, d.share, d.lag);
        let shift = (d.lag * RATE / HOP as f64).round() as i64;
        let mut shown = 0;
        for i in 0..ll.len() {
            let t = i as f64 * HOP as f64 / RATE;
            let bad = d.at.iter().any(|&b| (b - t).abs() < 1e-6);
            let j = (i as i64 + shift).clamp(0, lr.len() as i64 - 1) as usize;
            if t >= from && t <= to && (!only_bad || bad) && shown < 300 {
                shown += 1;
                println!("{t:7.3}{} L {:.4} {:6.0} Hz   R {:.4} {:6.0} Hz", if bad { '*' } else { ' ' }, ll[i], cl[i] * hz, lr[j], cr[j] * hz);
            }
        }
    }
}
