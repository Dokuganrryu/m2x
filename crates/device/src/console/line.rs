pub const LINE_CAPACITY: usize = 64;

const WORD_SEPARATOR: u8 = b' ';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    bytes: [u8; LINE_CAPACITY],
    len: usize,
}

pub(crate) enum Appended {
    Stored(Line),
    Overflowed,
}

impl Line {
    pub(crate) const EMPTY: Self = Self {
        bytes: [0; LINE_CAPACITY],
        len: 0,
    };

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn appended(mut self, text: u8) -> Appended {
        match self.bytes.get_mut(self.len) {
            Some(slot) => {
                *slot = text;
                self.len += 1;
                Appended::Stored(self)
            }
            None => Appended::Overflowed,
        }
    }

    pub(crate) fn words(&self) -> Words<'_> {
        Words {
            rest: self.as_bytes(),
        }
    }
}

pub(crate) struct Words<'line> {
    rest: &'line [u8],
}

impl<'line> Iterator for Words<'line> {
    type Item = &'line [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.rest.iter().position(|&byte| byte != WORD_SEPARATOR)?;
        let after_start = &self.rest[start..];
        let length = after_start
            .iter()
            .position(|&byte| byte == WORD_SEPARATOR)
            .unwrap_or(after_start.len());
        let (word, rest) = after_start.split_at(length);
        self.rest = rest;
        Some(word)
    }
}
