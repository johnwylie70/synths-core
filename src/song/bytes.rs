//! Reading numbers out of a file's bytes without ever stepping outside them.
//!
//! Song files come from anywhere and are often damaged, so every read here
//! is checked: past the end, a number reads as 0 and a slice comes back
//! short.

/// A place in a file.
#[derive(Clone, Copy)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Moves to a place in the file; past the end is the end.
    pub fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    pub fn skip(&mut self, n: usize) {
        self.seek(self.pos.saturating_add(n));
    }

    /// How many bytes are left.
    pub fn left(&self) -> usize {
        self.data.len() - self.pos
    }

    /// The next `n` bytes, or as many as there are.
    pub fn take(&mut self, n: usize) -> &'a [u8] {
        let n = n.min(self.left());
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        s
    }

    pub fn u8(&mut self) -> u8 {
        let v = self.data.get(self.pos).copied().unwrap_or(0);
        self.skip(1);
        v
    }

    fn array<const N: usize>(&mut self) -> [u8; N] {
        let mut a = [0u8; N];
        let s = self.take(N);
        a[..s.len()].copy_from_slice(s);
        a
    }

    pub fn u16_le(&mut self) -> u16 {
        u16::from_le_bytes(self.array())
    }

    pub fn u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.array())
    }

    pub fn u32_le(&mut self) -> u32 {
        u32::from_le_bytes(self.array())
    }

    /// Text of a fixed width: stops at the first zero byte, drops spaces at
    /// the end, and turns anything that is not plain ASCII into a space.
    pub fn text(&mut self, n: usize) -> String {
        let s = self.take(n);
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        let t: String = s[..end].iter().map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { ' ' }).collect();
        t.trim_end().to_string()
    }
}
