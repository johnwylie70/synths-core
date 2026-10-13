//! WAV files, the form a song's samples are kept in beside its JSON.
//!
//! Written: one channel, 8 bits (unsigned, as WAV has it) or 16. Read: PCM
//! of 8, 16, 24 or 32 bits or 32-bit floats, any number of channels (the
//! first is taken). Anything else in the file is skipped.

use super::bytes::Reader;
use super::{LoadError, SampleData, MAX_SAMPLE_FRAMES};

/// A sample's sound as a WAV file that says it plays at `rate` frames a
/// second.
pub fn write(data: &SampleData, rate: u32) -> Vec<u8> {
    let (width, frames) = match data {
        SampleData::Bits8(v) => (1u16, v.len()),
        SampleData::Bits16(v) => (2u16, v.len()),
    };
    let size = frames * width as usize;
    let padded = size + size % 2;
    let mut out = Vec::with_capacity(44 + padded);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + padded as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * width as u32).to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&(width * 8).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    match data {
        SampleData::Bits8(v) => out.extend(v.iter().map(|&x| (x as u8) ^ 0x80)),
        SampleData::Bits16(v) => out.extend(v.iter().flat_map(|x| x.to_le_bytes())),
    }
    out.resize(44 + padded, 0);
    out
}

/// The sound in a WAV file, and the rate it says it plays at.
pub fn read(file: &[u8]) -> Result<(SampleData, u32), LoadError> {
    let bad = |what: &str| LoadError::Invalid(format!("WAV file: {what}"));
    if file.len() < 12 || &file[..4] != b"RIFF" || &file[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let mut r = Reader::new(file);
    r.seek(12);
    let mut format = None;
    while r.left() >= 8 {
        let id = r.take(4);
        let size = r.u32_le() as usize;
        let body = r.take(size);
        r.skip(size % 2);
        if id == b"fmt " {
            let mut f = Reader::new(body);
            let (kind, channels, rate) = (f.u16_le(), f.u16_le(), f.u32_le());
            f.skip(6);
            let bits = f.u16_le();
            // 0xFFFE is "extensible": the real kind is further in.
            let kind = if kind == 0xFFFE {
                f.skip(8);
                f.u16_le()
            } else {
                kind
            };
            format = Some((kind, channels as usize, rate, bits));
        } else if id == b"data" {
            let (kind, channels, rate, bits) = format.ok_or_else(|| bad("the sound comes before its format"))?;
            let width = bits as usize / 8;
            if channels == 0 || !matches!((kind, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
                return Err(bad("a kind of sound this player does not read"));
            }
            let frames = body.chunks_exact(width * channels).take(MAX_SAMPLE_FRAMES).map(|f| &f[..width]);
            let data = match (kind, bits) {
                (1, 8) => SampleData::Bits8(frames.map(|f| (f[0] ^ 0x80) as i8).collect()),
                (1, _) => SampleData::Bits16(frames.map(|f| i16::from_le_bytes([f[width - 2], f[width - 1]])).collect()),
                _ => SampleData::Bits16(
                    frames
                        .map(|f| {
                            let x = f32::from_le_bytes([f[0], f[1], f[2], f[3]]);
                            (x.clamp(-1.0, 1.0) * 32767.0) as i16
                        })
                        .collect(),
                ),
            };
            return Ok((data, rate));
        }
    }
    Err(bad("no sound in it"))
}
