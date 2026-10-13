//! A small JSON reader, and the pieces of a writer.
//!
//! The crate takes no dependencies, and a song file needs very little of
//! JSON's machinery, so this is written by hand. It reads any valid JSON
//! text into a tree. Text that is not valid gives an error that says where;
//! it never panics, and nesting is limited so that a hostile file cannot
//! exhaust the stack.

/// One JSON value. An object keeps its keys in the order they were written.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The value under a key, if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The number, if it is a whole one.
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().filter(|n| n.fract() == 0.0 && n.abs() < 9.0e15).map(|n| n as i64)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
}

impl Value {
    /// The value as JSON text again, with no spaces or line breaks.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => match self.as_i64() {
                Some(whole) => out.push_str(&super::text::int(whole)),
                // JSON has no way to write a number that is not finite.
                None if !n.is_finite() => out.push_str("null"),
                // Exactly: this is a host's number, and not ours to round.
                None => out.push_str(&n.to_string()),
            },
            Value::String(s) => out.push_str(&quote(s)),
            Value::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Object(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&quote(k));
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// How deep arrays and objects may nest.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

/// Reads a JSON text. The error says what is wrong and at which byte.
#[cold]
pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser { text: text.as_bytes(), pos: 0 };
    let value = p.value(0)?;
    p.space();
    if p.pos != p.text.len() {
        return Err(p.error("something after the end of the JSON"));
    }
    Ok(value)
}

impl Parser<'_> {
    #[inline(never)]
    fn error(&self, what: &str) -> String {
        super::text::join(&[what, " (at byte ", &super::text::int(self.pos as i64), ")"])
    }

    /// What has not been read yet.
    fn rest(&self) -> &[u8] {
        self.text.get(self.pos..).unwrap_or(&[])
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn word(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.rest().starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error("not a JSON value"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        self.space();
        match self.peek() {
            None => Err(self.error("the JSON ends early")),
            Some(b'n') => self.word("null", Value::Null),
            Some(b't') => self.word("true", Value::Bool(true)),
            Some(b'f') => self.word("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.space();
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(self.error("expected , or ]")),
                    }
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut pairs = Vec::new();
                self.space();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(Value::Object(pairs));
                }
                loop {
                    self.space();
                    if self.peek() != Some(b'"') {
                        return Err(self.error("expected a key in quotes"));
                    }
                    let key = self.string()?;
                    self.space();
                    if self.peek() != Some(b':') {
                        return Err(self.error("expected : after a key"));
                    }
                    self.pos += 1;
                    pairs.push((key, self.value(depth + 1)?));
                    self.space();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Value::Object(pairs));
                        }
                        _ => return Err(self.error("expected , or }")),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => {
                let start = self.pos;
                while matches!(self.peek(), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                    self.pos += 1;
                }
                match super::text::parse_number(self.text.get(start..self.pos).unwrap_or(&[])) {
                    Some(n) => Ok(Value::Number(n)),
                    None => {
                        self.pos = start;
                        Err(self.error("not a number"))
                    }
                }
            }
            Some(_) => Err(self.error("not a JSON value")),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.text.get(self.pos..self.pos + 4);
        let n = digits.and_then(super::text::hex_value).ok_or_else(|| self.error("bad \\u escape"))?;
        self.pos += 4;
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(b) = self.peek() else { return Err(self.error("a string does not end")) };
            self.pos += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(e) = self.peek() else { return Err(self.error("a string does not end")) };
                    self.pos += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut code = self.hex4()?;
                            // Characters outside the first 65536 come as two
                            // escapes, a high half and a low half.
                            if (0xD800..0xDC00).contains(&code) && self.rest().starts_with(b"\\u") {
                                self.pos += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                }
                            }
                            char::from_u32(code).unwrap_or('\u{FFFD}')
                        }
                        _ => return Err(self.error("bad escape in a string")),
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                0..=0x1F => return Err(self.error("a control character in a string")),
                _ => out.push(b),
            }
        }
        // The input was a `str` and escapes were added as UTF-8, so the
        // bytes are valid; a split in the middle of a character cannot happen
        // because quotes and backslashes are never part of one.
        String::from_utf8(out).map_err(|_| self.error("bad text in a string"))
    }
}

/// A string as JSON writes it, quotes and all.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                super::text::push_hex(&mut out, c as u32, 4, false);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_value() {
        let v = parse(r#" {"a": [1, -2.5, 1e3, true, false, null], "b": {"c": "d\n\u00e9\ud83c\udfb5"}} "#).unwrap();
        let a = v.get("a").unwrap().as_array().unwrap();
        assert_eq!(a[0].as_i64(), Some(1));
        assert_eq!(a[1].as_f64(), Some(-2.5));
        assert_eq!(a[1].as_i64(), None);
        assert_eq!(a[2].as_i64(), Some(1000));
        assert_eq!(a[3], Value::Bool(true));
        assert_eq!(a[5], Value::Null);
        assert_eq!(v.get("b").unwrap().get("c").unwrap().as_str(), Some("d\n\u{e9}\u{1F3B5}"));
    }

    #[test]
    fn what_is_quoted_reads_back_the_same() {
        let s = "a \"quoted\" \\ line\nand\ttab \u{1} \u{e9} \u{1F3B5}";
        assert_eq!(parse(&quote(s)).unwrap().as_str(), Some(s));
    }

    #[test]
    fn bad_json_is_an_error_and_not_a_panic() {
        for bad in ["", "{", "[1,", "[1 2]", "{\"a\" 1}", "{a:1}", "\"abc", "\"\\x\"", "\"\\u12\"", "tru", "1.2.3", "[] []", "-"] {
            assert!(parse(bad).is_err(), "{bad:?} should not parse");
        }
        let deep = "[".repeat(100_000);
        assert!(parse(&deep).is_err());
    }
}
