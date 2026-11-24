use crate::sample::{I2sWord, StreamSample};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gain(i32);

pub(crate) const GAIN_FRACTION_BITS: u32 = 27;
const PRODUCT_HIGH_WORD_SHIFT: u32 = 32;
const STREAM_LSB_IN_HIGH_WORD: u32 =
    StreamSample::PADDING_BITS + GAIN_FRACTION_BITS - PRODUCT_HIGH_WORD_SHIFT;
const HALF_STREAM_LSB_IN_HIGH_WORD: i32 = 1 << (STREAM_LSB_IN_HIGH_WORD - 1);

impl Gain {
    pub const UNITY: Self = Self(1 << GAIN_FRACTION_BITS);
    pub const ZERO: Self = Self(0);

    pub const fn from_q5_27(raw: i32) -> Self {
        Self(raw)
    }

    pub(crate) const fn to_q5_27(self) -> i32 {
        self.0
    }

    #[inline(always)]
    pub(crate) fn scale_rounding_half_up_to_stream(self, word: I2sWord) -> StreamSample {
        let product = word.to_bits() as i64 * self.0 as i64;
        let high_word = (product >> PRODUCT_HIGH_WORD_SHIFT) as i32;
        let code = (high_word + HALF_STREAM_LSB_IN_HIGH_WORD) >> STREAM_LSB_IN_HIGH_WORD;
        StreamSample::saturating_from_code(code)
    }
}
