//! Renders a song to a WAV file for listening, with the plain mixer the
//! tests use (not the engine).
//!
//!     cargo run --release --example render_song -- song.xm [seconds] [--dump]
//!
//! The song can be a MOD, an XM, or one of our own JSON songs (its sample
//! files are looked for beside it). The WAV goes in target/renders/. With
//! --dump, the song's patterns and samples are printed too.

#[path = "../tests/replayer_support/mixer.rs"]
#[allow(dead_code)]
mod mixer;

use std::path::Path;
use synths_core::replayer::Player;
use synths_core::song::{self, note_name, Song};

const RATE: u32 = 44100;

fn load(path: &Path) -> Result<Song, Box<dyn std::error::Error>> {
    let data = std::fs::read(path)?;
    if let Ok(text) = std::str::from_utf8(&data) {
        if song::native::is_native(text) {
            let dir = path.parent().unwrap_or(Path::new("."));
            return Ok(song::native::load(text, &|name| std::fs::read(dir.join(name)).ok())?);
        }
    }
    Ok(song::load_module(&data)?)
}

fn dump(song: &Song) {
    for (i, s) in song.samples.iter().enumerate().filter(|(_, s)| !s.data.is_empty()) {
        println!(
            "sample {i:3} {:22} {:7} frames, loop {:?} {:?}, volume {}, finetune {}, transpose {}, pan {:?}",
            s.name,
            s.data.len(),
            s.loop_kind,
            s.loop_range(),
            s.volume,
            s.finetune,
            s.relative_note,
            s.pan
        );
    }
    let used = |ins: &song::Instrument| ins.keymap.iter().any(|&k| song.samples.get(k as usize).is_some_and(|s| !s.data.is_empty()));
    for (i, ins) in song.instruments.iter().enumerate().filter(|(_, i)| used(i) || i.patch.is_some()) {
        let mut samples: Vec<u16> = ins.keymap.clone();
        samples.dedup();
        println!("instrument {:3} {:22} samples {:?}, fadeout {}", i + 1, ins.name, samples, ins.fadeout);
        if ins.vibrato.depth > 0 {
            println!("    {:?}", ins.vibrato);
        }
        for (name, e) in [("volume", &ins.volume_envelope), ("pan", &ins.pan_envelope)] {
            if !e.points.is_empty() {
                println!("    {name} envelope{}: {:?}, sustain {:?}, loop {:?}", if e.enabled { "" } else { " (off)" }, e.points, e.sustain, e.loop_points);
            }
        }
    }
    for (i, p) in song.patterns.iter().enumerate() {
        println!("pattern {i} ({} rows)", p.rows);
        for row in 0..p.rows {
            if p.row(row).is_empty() {
                continue;
            }
            let cells: Vec<String> = (0..song.channels())
                .map(|ch| {
                    let c = p.cell(row, ch as u8);
                    let note = if c.note == 0 { "...".to_string() } else { format!("{:3}", note_name(c.note)) };
                    let effect = char::from_digit(c.effect as u32, 36).unwrap_or('?').to_ascii_uppercase();
                    format!("{note} {:02X} {:02X} {effect}{:02X}", c.instrument, c.volume, c.param)
                })
                .collect();
            println!("  {row:3} | {}", cells.join(" | "));
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.iter().find(|a| !a.starts_with("--") && a.parse::<f64>().is_err()) else {
        eprintln!("usage: render_song <song.mod | song.xm | song.json> [seconds] [--dump]");
        std::process::exit(2);
    };
    let path = Path::new(path);
    let limit = args.iter().find_map(|a| a.parse::<f64>().ok()).unwrap_or(300.0);
    let song = load(path)?;
    println!(
        "{:?}: \"{}\", {:?} rules, {:?} slides, {} channels, {} orders {:?}, {} patterns, speed {}, tempo {}",
        path.file_name().unwrap_or_default(),
        song.title,
        song.rules,
        song.frequency_table,
        song.channels(),
        song.orders.len(),
        song.orders,
        song.patterns.len(),
        song.speed,
        song.tempo
    );
    if args.iter().any(|a| a == "--dump") {
        dump(&song);
    }

    let mut player = Player::new(&song, RATE as f64);
    player.set_repeat(false);
    let mut mixer = mixer::Mixer::new(&song, RATE as f64);
    // Play to the song's end, then a little more for the last notes to ring.
    let most = (limit * RATE as f64) as usize;
    while !player.ended() && mixer.left.len() < most {
        player.process(&song, 1024, &mut mixer);
        mixer.render(1024);
    }
    let seconds = mixer.left.len() as f64 / RATE as f64;
    let out = Path::new("target/renders").join(path.file_name().unwrap_or_default()).with_extension("wav");
    std::fs::create_dir_all("target/renders")?;
    std::fs::write(&out, mixer::wav_bytes(&mixer.left, &mixer.right, RATE))?;
    let state = if player.ended() { "to its end" } else { "cut at the time limit" };
    println!("{seconds:.1} seconds ({state}) -> {}", out.display());
    Ok(())
}
