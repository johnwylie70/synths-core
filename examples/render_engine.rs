//! Plays songs through the real engine and writes what comes out, beside
//! the same songs through the plain mixer the replayer's tests use, so the
//! two can be listened to and compared.
//!
//!     cargo run --release --example render_engine -- [song.mod song.xm song.json ...]
//!
//! With no songs named it makes a MOD and an XM of a short tune of its own
//! and plays those. Everything goes in target/demo/: for each song,
//! `name.engine.wav` and `name.mixer.wav`.

#[path = "../tests/replayer_support/build.rs"]
#[allow(dead_code)]
mod build;
#[path = "../tests/replayer_support/mixer.rs"]
#[allow(dead_code)]
mod mixer;

use build::*;
use std::path::{Path, PathBuf};
use synths_core::event::SONG_PLAY;
use synths_core::replayer::Player;
use synths_core::song::{self, Envelope, Rules, SampleData, Song};
use synths_core::song_abi::*;
use synths_core::{Engine, Event};

const RATE: u32 = 44100;
const DEMO: &str = "target/demo";

/// A short tune of our own in four channels, as a MOD or an XM would hold it.
fn tune(rules: Rules) -> Song {
    let xm = rules == Rules::FastTracker2;
    let lead = [
        "C-5", "...", "E-5", "...", "G-5", "...", "E-5", "...", "A-5", "...", "G-5", "...", "E-5", "...", "D-5", "...",
        "C-5", "...", "D-5", "E-5", "F-5", "...", "A-5", "...", "G-5", "...", "...", "...", "E-5", "D-5", "C-5", "...",
    ];
    // Nothing below C-3, the Amiga's lowest note, so the MOD is a true ProTracker one.
    let bass = ["C-4", "C-4", "A-3", "A-3", "F-3", "F-3", "G-3", "G-3"];
    let mut rows = Vec::new();
    for (row, note) in lead.iter().enumerate() {
        let lead_fx = if row % 16 == 12 { "484" } else if *note == "..." && row % 16 == 13 { "484" } else { "000" };
        let lead_cell = if *note == "..." { format!("... .. {lead_fx}") } else { format!("{note} 01 {lead_fx}") };
        let bass_cell = if row % 4 == 0 { format!("{} 02 000", bass[row / 4]) } else if row % 4 == 3 { "... .. A04".to_string() } else { "... .. 000".to_string() };
        let chord_cell = if row % 8 == 0 { format!("{} 01 0{}", ["C-4", "A-3", "F-3", "G-3"][row / 8], ["47", "37", "47", "47"][row / 8]) } else { "... .. 047".to_string() };
        let hat_cell = match row % 4 {
            0 => "C-5 03 C30",
            2 => "B-5 03 C18",
            3 if row % 8 == 7 => "B-5 03 E93",
            _ => "... .. 000",
        };
        let cells = [lead_cell, bass_cell, chord_cell, hat_cell.to_string()];
        rows.push(cells.map(|c| if xm { format!("{} .. {}", &c[..6], &c[7..]) } else { c }).join(" | "));
    }
    rows.push(if xm { "... .. .. D00".to_string() } else { "... .. D00".to_string() });
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    let mut bass_wave = square(128, 1);
    bass_wave.data = SampleData::Bits8((0..128).map(|i| ((i as f32 / 64.0 - 1.0).abs() * 200.0 - 100.0) as i8).collect());
    let mut hat = ramp(600);
    hat.data = SampleData::Bits8((0..600).map(|i| ((((i * 7919 + 13) % 251) as i32 - 125) * (600 - i as i32) / 600) as i8).collect());
    let mut lead_wave = square(32, 1);
    lead_wave.volume = 40;
    let mut s = song(rules, 4, vec![lead_wave, bass_wave, hat], vec![pattern(64, &rows)]);
    s.title = "demo tune".to_string();
    if xm {
        s.channel_pan = vec![128; 4];
        s.samples[0].pan = Some(88);
        s.samples[1].pan = Some(128);
        s.samples[2].pan = Some(176);
        s.instruments[0].volume_envelope = Envelope { enabled: true, points: vec![(0, 64), (5, 40), (40, 24)], sustain: Some(1), loop_points: None };
        s.instruments[0].fadeout = 2048;
    }
    s
}

fn wav(path: &Path, left: &[f32], right: &[f32]) -> std::io::Result<()> {
    std::fs::write(path, mixer::wav_bytes(left, right, RATE))
}

/// How alike two signals' loudness is, a hundredth of a second at a time: 1 is the same shape.
fn likeness(a: &[f32], b: &[f32]) -> f32 {
    let loud = |x: &[f32]| -> Vec<f32> { x.chunks(441).map(|w| (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt()).collect() };
    let (a, b) = (loud(a), loud(b));
    let dot: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
    let (na, nb): (f32, f32) = (a.iter().map(|x| x * x).sum(), b.iter().map(|x| x * x).sum());
    dot / (na * nb).sqrt().max(1e-12)
}

fn render(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

    // The plain mixer, to the song's end (at most five minutes).
    let own = std::str::from_utf8(&data).ok().filter(|t| song::native::is_native(t));
    let dir = path.parent().unwrap_or(Path::new("."));
    let song = match own {
        Some(text) => song::native::load(text, &|file| std::fs::read(dir.join(file)).ok())?,
        None => song::load_module(&data)?,
    };
    let mut player = Player::new(&song, RATE as f64);
    player.set_repeat(false);
    let mut plain = mixer::Mixer::new(&song, RATE as f64);
    while !player.ended() && plain.left.len() < RATE as usize * 300 {
        player.process(&song, 1024, &mut plain);
        plain.render(1024);
    }
    let frames = plain.left.len();

    // The engine, for the same length.
    let mut engine = Engine::new(RATE as f32);
    if own.is_some() {
        // Give the engine the song's sample files first, as a host would.
        if let Some(text) = own {
            for name in text.split("\"file\"").skip(1).filter_map(|rest| rest.split('"').nth(1)) {
                if let Ok(bytes) = std::fs::read(dir.join(name)) {
                    unsafe { synths_song_file(name.as_ptr(), name.len() as u32, bytes.as_ptr(), bytes.len() as u32) };
                }
            }
        }
    }
    let code = unsafe { synths_song_load(&mut *engine, data.as_ptr(), data.len() as u32, 0) };
    if code != SONG_OK {
        let why = unsafe { std::ffi::CStr::from_ptr(synths_song_error().cast()) }.to_string_lossy().to_string();
        return Err(format!("the engine would not load it ({code}): {why}").into());
    }
    engine.push(Event::song(0.0, SONG_PLAY, 0.0, 0.0));
    let (mut left, mut right) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for (l, r) in left.chunks_mut(128).zip(right.chunks_mut(128)) {
        engine.render(l, r);
    }

    let out = |kind: &str| PathBuf::from(DEMO).join(format!("{name}.{kind}.wav"));
    wav(&out("engine"), &left, &right)?;
    wav(&out("mixer"), &plain.left, &plain.right)?;
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    println!(
        "{name}: {:.1} s, {} channels, {:?} rules; loudness alike left {:.3}, right {:.3}; peak engine {:.2}, mixer {:.2}",
        frames as f64 / RATE as f64,
        song.channels(),
        song.rules,
        likeness(&left, &plain.left),
        likeness(&right, &plain.right),
        peak(&left).max(peak(&right)),
        peak(&plain.left).max(peak(&plain.right)),
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(DEMO)?;
    let mut paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        for (file, bytes) in [("demo-tune.mod", write_mod(&tune(Rules::ProTracker))), ("demo-tune.xm", write_xm(&tune(Rules::FastTracker2)))] {
            let path = PathBuf::from(DEMO).join(file);
            std::fs::write(&path, bytes)?;
            paths.push(path);
        }
        paths.push(PathBuf::from("examples/songs/first-steps.json"));
    }
    for path in &paths {
        if let Err(e) = render(path) {
            println!("{}: {e}", path.display());
        }
    }
    println!("written to {DEMO}/");
    Ok(())
}
