use super::ring::ByteSink;

pub trait ByteOut {
    fn push(&mut self, byte: u8) -> bool;
}

impl<const SLOTS: usize, const WORDS: usize> ByteOut for ByteSink<'_, SLOTS, WORDS> {
    fn push(&mut self, byte: u8) -> bool {
        ByteSink::push(self, byte)
    }
}

const DECIMAL_BASE: u32 = 10;
const MOST_DIGITS: usize = 11;

const LINE_END: &[u8] = b"\r\n";
const FULL_SCALE_LOG2_Q8: i32 = 31 << 8;
const TENTHS_OF_DB_PER_BIT_MILLI: i32 = 60_206;
const Q8_MILLI: i32 = 256_000;
const FRACTION_BITS: u32 = 8;

pub struct Reply<'s, Out: ByteOut> {
    out: &'s mut Out,
    refused: u32,
}

impl<'s, Out: ByteOut> Reply<'s, Out> {
    pub fn new(out: &'s mut Out) -> Self {
        Self { out, refused: 0 }
    }

    pub const fn refused(&self) -> u32 {
        self.refused
    }

    fn byte(&mut self, byte: u8) {
        if !self.out.push(byte) {
            self.refused = self.refused.saturating_add(1);
        }
    }

    #[inline(never)]
    pub fn text(&mut self, text: &[u8]) -> &mut Self {
        for &byte in text {
            self.byte(byte);
        }
        self
    }

    #[inline(never)]
    fn decimal(&mut self, value: u32, with_tenths: bool) -> &mut Self {
        let mut digits = [b'0'; MOST_DIGITS];
        let mut rest = value;
        let mut start = MOST_DIGITS;
        let least = if with_tenths { 2 } else { 1 };
        for digit in digits.iter_mut().rev() {
            *digit = b'0' + (rest % DECIMAL_BASE) as u8;
            rest /= DECIMAL_BASE;
            start -= 1;
            if rest == 0 && MOST_DIGITS - start >= least {
                break;
            }
        }
        let point = MOST_DIGITS - usize::from(with_tenths);
        self.text(digits.get(start..point).unwrap_or_default());
        if with_tenths {
            self.text(b".")
                .text(digits.get(point..).unwrap_or_default());
        }
        self
    }

    pub fn unsigned(&mut self, value: u32) -> &mut Self {
        self.decimal(value, false)
    }

    pub fn signed(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.byte(b'-');
        }
        self.unsigned(value.unsigned_abs())
    }

    pub fn tenths(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.byte(b'-');
        }
        self.decimal(value.unsigned_abs(), true)
    }

    pub fn hex(&mut self, value: u32) -> &mut Self {
        self.text(b"0x");
        for shift in (0..8).rev() {
            let nibble = ((value >> (shift * 4)) & 0xF) as u8;
            self.byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
        self
    }

    #[inline(never)]
    pub fn field(&mut self, name: &[u8], value: u32) -> &mut Self {
        self.text(b" ").text(name).text(b"=").unsigned(value)
    }

    pub fn end(&mut self) {
        self.text(LINE_END);
    }
}

pub fn peak_tenths_of_dbfs(magnitude: u32) -> Option<i32> {
    if magnitude == 0 {
        return None;
    }
    let whole = 31 - magnitude.leading_zeros();
    let mut mantissa = u64::from(magnitude) << (31 - whole);
    let mut fraction = 0;
    for bit in (0..FRACTION_BITS).rev() {
        mantissa = (mantissa * mantissa) >> 31;
        if mantissa >= 1 << 32 {
            mantissa >>= 1;
            fraction |= 1 << bit;
        }
    }
    let log2_q8 = ((whole << FRACTION_BITS) | fraction) as i32;
    Some((log2_q8 - FULL_SCALE_LOG2_Q8) * TENTHS_OF_DB_PER_BIT_MILLI / Q8_MILLI)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    struct Bytes {
        written: Vec<u8>,
        room: usize,
    }

    impl ByteOut for Bytes {
        fn push(&mut self, byte: u8) -> bool {
            if self.written.len() == self.room {
                return false;
            }
            self.written.push(byte);
            true
        }
    }

    fn written(room: usize, write: impl FnOnce(&mut Reply<'_, Bytes>)) -> (Vec<u8>, u32) {
        let mut out = Bytes {
            written: Vec::new(),
            room,
        };
        let refused = {
            let mut reply = Reply::new(&mut out);
            write(&mut reply);
            reply.refused()
        };
        (out.written, refused)
    }

    fn text(write: impl FnOnce(&mut Reply<'_, Bytes>)) -> std::string::String {
        std::string::String::from_utf8(written(usize::MAX, write).0).unwrap_or_default()
    }

    #[test]
    fn unsigned_values_print_in_decimal_without_leading_zeros() {
        for value in [0, 1, 9, 10, 99, 100, 4_096, 1_000_000_007, u32::MAX] {
            assert_eq!(
                text(|reply| {
                    reply.unsigned(value);
                }),
                std::format!("{value}")
            );
        }
    }

    #[test]
    fn signed_values_carry_a_minus_sign_and_reach_both_ends() {
        for value in [0, -1, 1, -40, i32::MIN, i32::MAX] {
            assert_eq!(
                text(|reply| {
                    reply.signed(value);
                }),
                std::format!("{value}")
            );
        }
    }

    #[test]
    fn tenths_print_one_decimal_place() {
        let cases = [
            (0, "0.0"),
            (5, "0.5"),
            (-5, "-0.5"),
            (-35, "-3.5"),
            (120, "12.0"),
            (-1_000, "-100.0"),
        ];
        for (value, expected) in cases {
            assert_eq!(
                text(|reply| {
                    reply.tenths(value);
                }),
                expected
            );
        }
        assert_eq!(
            text(|reply| {
                reply.tenths(i32::MIN);
            }),
            "-214748364.8"
        );
    }

    #[test]
    fn a_report_line_is_a_name_and_space_separated_fields() {
        let line = text(|reply| {
            reply
                .text(b"counters")
                .field(b"underruns", 3)
                .field(b"overruns", 0);
            reply.end();
        });
        assert_eq!(line, "counters underruns=3 overruns=0\r\n");
    }

    #[test]
    fn bytes_the_ring_cannot_take_are_counted_not_lost_silently() {
        let (bytes, refused) = written(4, |reply| {
            reply.text(b"cycles");
        });
        assert_eq!((bytes, refused), (b"cycl".to_vec(), 2));
    }

    #[test]
    fn full_scale_peak_is_zero_dbfs_and_each_halving_is_about_six_db_down() {
        assert_eq!(peak_tenths_of_dbfs(0), None);
        assert_eq!(peak_tenths_of_dbfs(1 << 31), Some(0));
        assert_eq!(peak_tenths_of_dbfs(1 << 30), Some(-60));
        assert_eq!(peak_tenths_of_dbfs(1 << 21), Some(-602));
        assert_eq!(peak_tenths_of_dbfs(1), Some(-1866));
        let three_db_down = peak_tenths_of_dbfs(1_520_301_996).unwrap_or_default();
        assert!((-31..=-29).contains(&three_db_down));
    }
}
