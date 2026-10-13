//! Building songs in a test, and writing them out as MOD and XM files so
//! that the loaders are part of what is tested.

use synths_core::song::*;

/// A cell from text. MOD style is "C-2 01 A04": note, instrument (hex),
/// effect. XM style has the volume column (hex) third: "C-4 01 40 A04".
/// Dots or dashes mean nothing there; "===" is key off.
pub fn cell(text: &str) -> Cell {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let blank = |s: &str| s.chars().all(|c| c == '.' || c == '-');
    let mut c = Cell::default();
    let note = parts.first().copied().unwrap_or("...");
    if !blank(note) {
        c.note = parse_note(note).unwrap_or_else(|| panic!("bad note {note:?}"));
    }
    if let Some(ins) = parts.get(1).filter(|s| !blank(s)) {
        c.instrument = u8::from_str_radix(ins, 16).unwrap();
    }
    let effect = if parts.len() >= 4 {
        if !blank(parts[2]) {
            c.volume = u8::from_str_radix(parts[2], 16).unwrap();
        }
        parts.get(3)
    } else {
        parts.get(2)
    };
    if let Some(e) = effect.filter(|s| !blank(s)) {
        c.effect = (e.as_bytes()[0] as char).to_digit(36).unwrap() as u8;
        c.param = u8::from_str_radix(&e[1..], 16).unwrap();
    }
    c
}

/// A pattern from rows of text, each row its channels' cells split by "|".
/// Rows not given are empty.
pub fn pattern(rows: u16, lines: &[&str]) -> Pattern {
    let mut p = Pattern::new(rows);
    for (row, line) in lines.iter().enumerate() {
        for (ch, text) in line.split('|').enumerate() {
            p.set(row as u16, ch as u8, cell(text));
        }
    }
    p
}

/// A sample of a square wave, `period` frames to the cycle, looped whole.
pub fn square(period: usize, cycles: usize) -> Sample {
    let data: Vec<i8> = (0..period * cycles).map(|i| if i % period < period / 2 { 100 } else { -100 }).collect();
    let len = data.len() as u32;
    Sample {
        name: "square".to_string(),
        data: SampleData::Bits8(data),
        loop_start: 0,
        loop_len: len,
        loop_kind: LoopKind::Forward,
        volume: 64,
        ..Default::default()
    }
}

/// A sample whose frame `i` holds `i` (wrapping at 128), so the output shows
/// which frame is playing. Not looped.
pub fn ramp(frames: usize) -> Sample {
    Sample {
        name: "ramp".to_string(),
        data: SampleData::Bits8((0..frames).map(|i| (i % 128) as i8).collect()),
        volume: 64,
        ..Default::default()
    }
}

/// A sample that is one steady value throughout; looped if asked.
pub fn steady(value: i8, frames: usize, looped: bool) -> Sample {
    Sample {
        name: "steady".to_string(),
        data: SampleData::Bits8(vec![value; frames]),
        loop_len: if looped { frames as u32 } else { 0 },
        loop_kind: if looped { LoopKind::Forward } else { LoopKind::Off },
        volume: 64,
        ..Default::default()
    }
}

/// A song with the given samples, one instrument for each, and patterns
/// played in order.
pub fn song(rules: Rules, channels: usize, samples: Vec<Sample>, patterns: Vec<Pattern>) -> Song {
    let mut s = Song {
        title: "test".to_string(),
        rules,
        frequency_table: if rules == Rules::ProTracker { FrequencyTable::Amiga } else { FrequencyTable::Linear },
        channel_pan: vec![128; channels],
        orders: (0..patterns.len() as u16).collect(),
        patterns,
        ..Default::default()
    };
    if rules == Rules::ProTracker {
        s.channel_pan = (0..channels).map(|c| if matches!(c % 4, 0 | 3) { 0 } else { 255 }).collect();
    }
    for (i, mut sample) in samples.into_iter().enumerate() {
        if rules == Rules::FastTracker2 && sample.pan.is_none() {
            sample.pan = Some(128);
        }
        s.instruments.push(Instrument::of_sample(&sample.name, i as u16));
        s.samples.push(sample);
    }
    s.tidy();
    s
}

/// The period ProTracker writes for a note.
fn mod_period(note: u8) -> u16 {
    const BASE: [u16; 12] = [856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453];
    if note == 0 {
        return 0;
    }
    let n = note as i32 - 37;
    let (octave, key) = (n.div_euclid(12), n.rem_euclid(12) as usize);
    if octave >= 0 {
        // The table's own rounding, octave by octave, as trackers have it.
        const TABLE: [u16; 36] = [
            856, 808, 762, 720, 678, 640, 604, 570, 538, 508, 480, 453, 428, 404, 381, 360, 339, 320, 302, 285, 269,
            254, 240, 226, 214, 202, 190, 180, 170, 160, 151, 143, 135, 127, 120, 113,
        ];
        TABLE.get(n as usize).copied().unwrap_or(BASE[key] >> octave)
    } else {
        BASE[key] << -octave
    }
}

/// The song as a MOD file. It must have at most 31 samples, all 8-bit, and
/// 64-row patterns.
pub fn write_mod(song: &Song) -> Vec<u8> {
    let channels = song.channels();
    let mut out = vec![0u8; 1084];
    out[..song.title.len().min(20)].copy_from_slice(&song.title.as_bytes()[..song.title.len().min(20)]);
    for (i, s) in song.samples.iter().enumerate().take(31) {
        let h = &mut out[20 + i * 30..50 + i * 30];
        h[..s.name.len().min(22)].copy_from_slice(&s.name.as_bytes()[..s.name.len().min(22)]);
        h[22..24].copy_from_slice(&((s.data.len() / 2) as u16).to_be_bytes());
        h[24] = ((s.finetune >> 4) & 15) as u8;
        h[25] = s.volume;
        h[26..28].copy_from_slice(&((s.loop_start / 2) as u16).to_be_bytes());
        let replen = if s.loop_kind == LoopKind::Off { 1 } else { (s.loop_len / 2) as u16 };
        h[28..30].copy_from_slice(&replen.to_be_bytes());
    }
    for i in song.samples.len()..31 {
        out[20 + i * 30 + 29] = 1;
    }
    out[950] = song.orders.len() as u8;
    out[951] = 127;
    for (i, &o) in song.orders.iter().enumerate() {
        out[952 + i] = o as u8;
    }
    let tag = match channels {
        4 => "M.K.".to_string(),
        n if n < 10 => format!("{n}CHN"),
        n => format!("{n}CH"),
    };
    out[1080..1084].copy_from_slice(tag.as_bytes());
    for p in &song.patterns {
        assert_eq!(p.rows, 64, "a MOD pattern has 64 rows");
        for row in 0..64 {
            for ch in 0..channels {
                let c = p.cell(row, ch as u8);
                let period = mod_period(c.note);
                out.push((c.instrument & 0xF0) | (period >> 8) as u8);
                out.push(period as u8);
                out.push((c.instrument << 4) | (c.effect & 15));
                out.push(c.param);
            }
        }
    }
    for s in song.samples.iter().take(31) {
        match &s.data {
            SampleData::Bits8(v) => out.extend(v[..v.len() & !1].iter().map(|&x| x as u8)),
            SampleData::Bits16(_) => panic!("a MOD sample is 8-bit"),
        }
    }
    out
}

fn put_text(out: &mut Vec<u8>, text: &str, width: usize) {
    let mut b = text.as_bytes()[..text.len().min(width)].to_vec();
    b.resize(width, 0);
    out.extend_from_slice(&b);
}

/// The song as an XM file.
pub fn write_xm(song: &Song) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"Extended Module: ");
    put_text(&mut out, &song.title, 20);
    out.push(0x1A);
    put_text(&mut out, "synths-core tests", 20);
    out.extend_from_slice(&0x0104u16.to_le_bytes());
    out.extend_from_slice(&276u32.to_le_bytes());
    for n in [
        song.orders.len() as u16,
        song.restart,
        song.channels() as u16,
        song.patterns.len() as u16,
        song.instruments.len() as u16,
        (song.frequency_table == FrequencyTable::Linear) as u16,
        song.speed as u16,
        song.tempo,
    ] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    let mut orders: Vec<u8> = song.orders.iter().map(|&o| o as u8).collect();
    orders.resize(256, 0);
    out.extend_from_slice(&orders);

    for p in &song.patterns {
        let mut packed = Vec::new();
        for row in 0..p.rows {
            for ch in 0..song.channels() {
                let c = p.cell(row, ch as u8);
                let fields = [c.note, c.instrument, c.volume, c.effect, c.param];
                let mask = fields.iter().enumerate().fold(0x80u8, |m, (i, &f)| if f != 0 { m | 1 << i } else { m });
                packed.push(mask);
                packed.extend(fields.iter().filter(|&&f| f != 0));
            }
        }
        out.extend_from_slice(&9u32.to_le_bytes());
        out.push(0);
        out.extend_from_slice(&p.rows.to_le_bytes());
        out.extend_from_slice(&(packed.len() as u16).to_le_bytes());
        out.extend_from_slice(&packed);
    }

    for ins in &song.instruments {
        // The instrument's samples, in the order its keys first use them.
        let mut used: Vec<u16> = Vec::new();
        for &k in &ins.keymap {
            if k != NO_SAMPLE && !used.contains(&k) {
                used.push(k);
            }
        }
        let size: u32 = if used.is_empty() { 29 } else { 263 };
        out.extend_from_slice(&size.to_le_bytes());
        put_text(&mut out, &ins.name, 22);
        out.push(0);
        out.extend_from_slice(&(used.len() as u16).to_le_bytes());
        if used.is_empty() {
            continue;
        }
        out.extend_from_slice(&40u32.to_le_bytes());
        for k in 0..96 {
            let slot = ins.keymap.get(k).and_then(|s| used.iter().position(|u| u == s));
            out.push(slot.map_or(used.len() as u8, |s| s as u8));
        }
        for e in [&ins.volume_envelope, &ins.pan_envelope] {
            for i in 0..12 {
                let (x, y) = e.points.get(i).copied().unwrap_or((0, 0));
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
            }
        }
        let (v, p) = (&ins.volume_envelope, &ins.pan_envelope);
        out.push(v.points.len() as u8);
        out.push(p.points.len() as u8);
        for e in [v, p] {
            out.push(e.sustain.unwrap_or(0));
            out.push(e.loop_points.map_or(0, |l| l.0));
            out.push(e.loop_points.map_or(0, |l| l.1));
        }
        for e in [v, p] {
            out.push(e.enabled as u8 | (e.sustain.is_some() as u8) << 1 | (e.loop_points.is_some() as u8) << 2);
        }
        out.extend_from_slice(&[ins.vibrato.waveform, ins.vibrato.sweep, ins.vibrato.depth, ins.vibrato.rate]);
        out.extend_from_slice(&ins.fadeout.to_le_bytes());
        out.extend_from_slice(&[0u8; 22]);
        assert_eq!(size, 263);

        for &u in &used {
            let s = &song.samples[u as usize];
            let wide = matches!(s.data, SampleData::Bits16(_));
            let width = if wide { 2 } else { 1 };
            out.extend_from_slice(&((s.data.len() * width) as u32).to_le_bytes());
            out.extend_from_slice(&(s.loop_start * width as u32).to_le_bytes());
            out.extend_from_slice(&(s.loop_len * width as u32).to_le_bytes());
            out.push(s.volume);
            out.push(s.finetune as u8);
            let kind = match s.loop_kind {
                LoopKind::Off => 0,
                LoopKind::Forward => 1,
                LoopKind::PingPong => 2,
            };
            out.push(kind | if wide { 0x10 } else { 0 });
            out.push(s.pan.unwrap_or(128));
            out.push(s.relative_note as u8);
            out.push(0);
            put_text(&mut out, &s.name, 22);
        }
        for &u in &used {
            match &song.samples[u as usize].data {
                SampleData::Bits8(v) => {
                    let mut last = 0i8;
                    for &x in v {
                        out.push(x.wrapping_sub(last) as u8);
                        last = x;
                    }
                }
                SampleData::Bits16(v) => {
                    let mut last = 0i16;
                    for &x in v {
                        out.extend_from_slice(&x.wrapping_sub(last).to_le_bytes());
                        last = x;
                    }
                }
            }
        }
    }
    out
}
