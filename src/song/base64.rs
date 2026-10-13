//! Base64, for a sample's sound written into a song's JSON as text.

/// The value of one base64 character, in the usual alphabet or the one
/// made for web addresses (`-` and `_` for `+` and `/`).
fn value(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' | b'-' => 62,
        b'/' | b'_' => 63,
        _ => return None,
    } as u32)
}

/// The bytes a base64 text stands for. Spaces and line breaks are skipped,
/// the `=` signs at the end may be there or not, and a text that begins as
/// a data address does (`data:audio/wav;base64,`) is read from after its
/// comma. `None` if anything else is in it, or it stops part way through a
/// byte. The answer is never longer than three quarters of the text.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes = text.as_bytes();
    if bytes.starts_with(b"data:") {
        let comma = bytes.iter().position(|&b| b == b',')?;
        bytes = &bytes[comma + 1..];
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 3);
    let (mut bits, mut have, mut ended) = (0u32, 0u32, false);
    for &c in bytes {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            ended = true;
            continue;
        }
        if ended {
            return None;
        }
        bits = bits << 6 | value(c)?;
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
        }
    }
    // Six bits left over would be a character that carried no whole byte.
    if have >= 6 {
        return None;
    }
    Some(out)
}

/// Bytes as base64 text, in the usual alphabet with `=` at the end.
pub fn encode(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (chunk.get(1).copied().unwrap_or(0) as u32) << 8 | chunk.get(2).copied().unwrap_or(0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(LETTERS[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
