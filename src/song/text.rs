//! Writing and reading numbers as text, by hand.
//!
//! The standard library's number formatting and its exact decimal reader
//! are general, and large: tens of kilobytes of a WebAssembly module that
//! is itself small. A song file needs whole numbers, a few hex digits and
//! the odd plain decimal, so these do just that.

/// Add a whole number in decimal.
pub fn push_int(out: &mut String, n: i64) {
    if n < 0 {
        out.push('-');
    }
    let mut n = n.unsigned_abs();
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in digits.iter().skip(at) {
        out.push(d as char);
    }
}

/// Add a number in hexadecimal, at least `width` digits wide.
pub fn push_hex(out: &mut String, n: u32, width: usize, upper: bool) {
    let mut started = false;
    for place in (0..8).rev() {
        let digit = (n >> (place * 4)) & 15;
        if digit != 0 || started || place < width {
            started = true;
            let c = if digit < 10 { b'0' + digit as u8 } else if upper { b'A' + digit as u8 - 10 } else { b'a' + digit as u8 - 10 };
            out.push(c as char);
        }
    }
}

/// Add a number that may have a fraction: up to six decimal places, with
/// no zeros left hanging on the end. Numbers beyond a million million or
/// that are not numbers at all are written as 0.
pub fn push_number(out: &mut String, x: f64) {
    if !x.is_finite() || x.abs() >= 1e12 {
        out.push('0');
        return;
    }
    let scaled = (x.abs() * 1e6).round() as u64;
    if x < 0.0 && scaled != 0 {
        out.push('-');
    }
    push_int(out, (scaled / 1_000_000) as i64);
    let mut fraction = scaled % 1_000_000;
    if fraction != 0 {
        out.push('.');
        let mut place = 100_000;
        while fraction != 0 {
            out.push((b'0' + (fraction / place) as u8) as char);
            fraction %= place;
            place /= 10;
        }
    }
}

/// What a digit or letter is worth as a digit in base 36: 0 to 9, then A
/// (or a) to Z as 10 to 35.
pub fn digit36(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'z' => Some(c - b'a' + 10),
        b'A'..=b'Z' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// The number a run of one to eight hexadecimal digits spells.
pub fn hex_value(digits: &[u8]) -> Option<u32> {
    if digits.is_empty() || digits.len() > 8 {
        return None;
    }
    let mut n = 0u32;
    for &d in digits {
        n = n * 16 + digit36(d).filter(|&v| v < 16)? as u32;
    }
    Some(n)
}

/// Text without the spaces, tabs and line ends at either end of it.
pub fn trim(text: &str) -> &str {
    let b = text.as_bytes();
    let space = |c: &u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r');
    let from = b.iter().position(|c| !space(c)).unwrap_or(b.len());
    let to = b.iter().rposition(|c| !space(c)).map_or(from, |i| i + 1);
    text.get(from..to).unwrap_or("")
}

/// Several pieces of text as one.
pub fn join(parts: &[&str]) -> String {
    let mut out = String::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        out.push_str(p);
    }
    out
}

/// A whole number as text.
pub fn int(n: i64) -> String {
    let mut out = String::new();
    push_int(&mut out, n);
    out
}

/// Read a JSON number: an optional minus, digits, an optional fraction and
/// an optional exponent, and nothing else. Whole numbers up to 2 to the 53
/// come out exact; fractions are right to within the last place or two,
/// which is closer than any song needs.
pub fn parse_number(text: &[u8]) -> Option<f64> {
    let mut at = 0;
    let negative = text.first() == Some(&b'-');
    if negative {
        at += 1;
    }
    let mut value = 0.0f64;
    let mut exponent = 0i32;
    let digits_from = at;
    while let Some(d) = text.get(at).filter(|d| d.is_ascii_digit()) {
        value = value * 10.0 + (d - b'0') as f64;
        at += 1;
    }
    if at == digits_from {
        return None;
    }
    if text.get(at) == Some(&b'.') {
        at += 1;
        let fraction_from = at;
        while let Some(d) = text.get(at).filter(|d| d.is_ascii_digit()) {
            value = value * 10.0 + (d - b'0') as f64;
            exponent -= 1;
            at += 1;
        }
        if at == fraction_from {
            return None;
        }
    }
    if matches!(text.get(at), Some(b'e' | b'E')) {
        at += 1;
        let minus = match text.get(at) {
            Some(b'-') => {
                at += 1;
                true
            }
            Some(b'+') => {
                at += 1;
                false
            }
            _ => false,
        };
        let from = at;
        let mut e = 0i32;
        while let Some(d) = text.get(at).filter(|d| d.is_ascii_digit()) {
            e = (e * 10 + (d - b'0') as i32).min(10_000);
            at += 1;
        }
        if at == from {
            return None;
        }
        exponent += if minus { -e } else { e };
    }
    if at != text.len() {
        return None;
    }
    // Scale by the power of ten, a factor of ten at a time in squares.
    let mut scale = 1.0f64;
    let mut base = 10.0f64;
    let mut n = exponent.unsigned_abs().min(400);
    while n > 0 {
        if n & 1 == 1 {
            scale *= base;
        }
        base *= base;
        n >>= 1;
    }
    let value = if exponent < 0 { value / scale } else { value * scale };
    let value = if negative { -value } else { value };
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(x: f64) -> String {
        let mut s = String::new();
        push_number(&mut s, x);
        s
    }

    #[test]
    fn numbers_are_written_plainly() {
        assert_eq!(int(0), "0");
        assert_eq!(int(-1234567890123), "-1234567890123");
        assert_eq!(int(i64::MIN), "-9223372036854775808");
        assert_eq!(number(0.0), "0");
        assert_eq!(number(50.0), "50");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(number(0.1), "0.1");
        assert_eq!(number(1.0 / 3.0), "0.333333");
        assert_eq!(number(12.000001), "12.000001");
        assert_eq!(number(-0.0000001), "0");
        assert_eq!(number(f64::NAN), "0");
        let mut s = String::new();
        push_hex(&mut s, 0x0A, 2, true);
        push_hex(&mut s, 0x1f, 4, false);
        push_hex(&mut s, 0x12345, 2, true);
        assert_eq!(s, "0A001f12345");
        assert_eq!(join(&["a", "", "bc"]), "abc");
        assert_eq!((trim("  a b\t\n"), trim(""), trim(" \n "), trim("x")), ("a b", "", "", "x"));
        assert_eq!((hex_value(b"fF"), hex_value(b"1g"), hex_value(b""), hex_value(b"123456789")), (Some(255), None, None, None));
        assert_eq!((digit36(b'7'), digit36(b'z'), digit36(b'A'), digit36(b'-')), (Some(7), Some(35), Some(10), None));
    }

    #[test]
    fn numbers_are_read_as_json_writes_them() {
        for (text, want) in [("0", 0.0), ("-0", 0.0), ("12", 12.0), ("-2.5", -2.5), ("1e3", 1000.0), ("1.5E-2", 0.015), ("9007199254740992", 9007199254740992.0), ("0.1", 0.1), ("125e+1", 1250.0)] {
            let got = parse_number(text.as_bytes()).unwrap();
            assert!((got - want).abs() <= want.abs() * 1e-15, "{text}: {got}");
        }
        for bad in ["", "-", ".5", "5.", "1e", "1e+", "1.2.3", "0x10", "1 ", "+1", "--1", "1e5000"] {
            assert_eq!(parse_number(bad.as_bytes()), None, "{bad:?}");
        }
        // What is written reads back as itself.
        for x in [0.0, 1.0, -64.0, 0.25, 50.0, 8363.0, 0.333333, 1e-6, 123456.789012] {
            assert_eq!(number(parse_number(number(x).as_bytes()).unwrap()), number(x));
        }
    }
}
