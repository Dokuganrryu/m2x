pub type Frame<S> = [S; 2];

pub trait Sample: Copy {
    const SILENCE: Self;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct StreamSample(i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct I2sWord(i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct DspSample(i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attenuation(i32);

const STREAM_CODE_BITS: u32 = 24;
const STREAM_PADDING_BITS: u32 = 32 - STREAM_CODE_BITS;
const STREAM_PADDING_MASK: i32 = (1 << STREAM_PADDING_BITS) - 1;
const ATTENUATION_FRACTION_BITS: u32 = 30;
const DSP_HEADROOM_BITS: u32 = 2;
const DSP_FRACTION_BITS: u32 = 31 - DSP_HEADROOM_BITS;

impl StreamSample {
    pub(crate) const PADDING_BITS: u32 = STREAM_PADDING_BITS;

    pub const fn from_bits(left_justified_code: i32) -> Self {
        Self(left_justified_code)
    }

    pub const fn to_bits(self) -> i32 {
        self.0
    }

    #[inline(always)]
    pub(crate) fn saturating_from_code(code: i32) -> Self {
        let largest = (1 << (STREAM_CODE_BITS - 1)) - 1;
        Self(code.clamp(-largest - 1, largest) << STREAM_PADDING_BITS)
    }
}

impl Attenuation {
    pub const fn from_ratio(numerator: u32, denominator: u32) -> Self {
        Self((((numerator as i64) << ATTENUATION_FRACTION_BITS) / denominator as i64) as i32)
    }

    #[inline(always)]
    pub fn attenuate(self, sample: StreamSample) -> StreamSample {
        let product = sample.0 as i64 * self.0 as i64;
        StreamSample((product >> ATTENUATION_FRACTION_BITS) as i32 & !STREAM_PADDING_MASK)
    }
}

impl I2sWord {
    pub const fn from_bits(q0_31: i32) -> Self {
        Self(q0_31)
    }

    pub const fn to_bits(self) -> i32 {
        self.0
    }

    #[inline(always)]
    pub(crate) const fn exact_from_stream(sample: StreamSample) -> Self {
        Self(sample.0)
    }

    #[inline(always)]
    pub(crate) fn saturating_from_dsp(sample: DspSample) -> Self {
        let largest = (1 << DSP_FRACTION_BITS) - 1;
        Self(sample.0.clamp(-largest - 1, largest) << DSP_HEADROOM_BITS)
    }
}

impl DspSample {
    pub const FULL_SCALE: Self = Self(1 << DSP_FRACTION_BITS);

    pub const fn from_q2_29(raw: i32) -> Self {
        Self(raw)
    }

    pub const fn to_q2_29(self) -> i32 {
        self.0
    }

    #[inline(always)]
    pub(crate) const fn exact_from_stream(sample: StreamSample) -> Self {
        Self(sample.0 >> DSP_HEADROOM_BITS)
    }

    #[inline(always)]
    pub(crate) const fn exact_from_output(word: I2sWord) -> Self {
        Self(word.0 >> DSP_HEADROOM_BITS)
    }
}

impl Sample for StreamSample {
    const SILENCE: Self = Self(0);
}

impl Sample for I2sWord {
    const SILENCE: Self = Self(0);
}

impl Sample for DspSample {
    const SILENCE: Self = Self(0);
}
