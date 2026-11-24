use crate::limiter::DspBlock;
use crate::peq::{BandCoefficients, CoefficientFormat, CoefficientSet, PEQ_BANDS, QuantisedBiquad};
use crate::sample::DspSample;
use crate::tick::{FrameCountKernel, FramesPerTick};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RoundingResidue(i32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ChannelHistory {
    x1: i32,
    x2: i32,
    y1: i32,
    y2: i32,
    residue: RoundingResidue,
}

type BandHistory = [ChannelHistory; 2];

#[derive(Clone, Copy, Debug)]
enum HistoryStart {
    Carried,
    Silent,
}

#[derive(Clone, Debug)]
pub struct Peq {
    history: [BandHistory; PEQ_BANDS],
    start: HistoryStart,
}

struct FilterCascade<'a> {
    set: &'a CoefficientSet,
    history: &'a mut [BandHistory; PEQ_BANDS],
    start: &'a mut HistoryStart,
    block: &'a mut DspBlock,
}

#[derive(Clone, Copy)]
struct CoefficientWords {
    b0: i32,
    b1: i32,
    b2: i32,
    f1: i32,
    f2: i32,
}

const SHORTEST_TICK_FRAMES: usize = FramesPerTick::Six.count();
const Q2_30_FRACTION_BITS: u32 = CoefficientFormat::Q2_30.fraction_bits();
const Q3_29_FRACTION_BITS: u32 = CoefficientFormat::Q3_29.fraction_bits();
const Q4_28_FRACTION_BITS: u32 = CoefficientFormat::Q4_28.fraction_bits();
const SILENT_CHANNEL: ChannelHistory = ChannelHistory {
    x1: 0,
    x2: 0,
    y1: 0,
    y2: 0,
    residue: RoundingResidue(0),
};
const SILENT_HISTORY: [BandHistory; PEQ_BANDS] = [[SILENT_CHANNEL; 2]; PEQ_BANDS];

impl Peq {
    pub const fn new() -> Self {
        Self {
            history: SILENT_HISTORY,
            start: HistoryStart::Carried,
        }
    }

    pub(crate) fn start_silent(&mut self) {
        self.start = HistoryStart::Silent;
    }

    pub fn filter(&mut self, set: &CoefficientSet, frames: FramesPerTick, block: &mut DspBlock) {
        frames.process(FilterCascade {
            set,
            history: &mut self.history,
            start: &mut self.start,
            block,
        });
    }

    #[inline(always)]
    pub(crate) fn filter_frames<const FRAMES: usize>(
        &mut self,
        set: &CoefficientSet,
        block: &mut DspBlock,
    ) {
        FilterCascade {
            set,
            history: &mut self.history,
            start: &mut self.start,
            block,
        }
        .process::<FRAMES>();
    }
}

impl Default for Peq {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCountKernel for FilterCascade<'_> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        if let HistoryStart::Silent = *self.start {
            *self.history = SILENT_HISTORY;
            *self.start = HistoryStart::Carried;
        }
        for (coefficients, history) in self.set.bands().iter().zip(self.history.iter_mut()) {
            let BandCoefficients::Filter(biquad) = coefficients else {
                continue;
            };
            match biquad.format() {
                CoefficientFormat::Q2_30 => {
                    filter_band::<FRAMES, Q2_30_FRACTION_BITS>(biquad, history, self.block)
                }
                CoefficientFormat::Q3_29 => {
                    filter_band::<FRAMES, Q3_29_FRACTION_BITS>(biquad, history, self.block)
                }
                CoefficientFormat::Q4_28 => {
                    filter_band::<FRAMES, Q4_28_FRACTION_BITS>(biquad, history, self.block)
                }
            }
        }
    }
}

#[inline(never)]
fn filter_band<const FRAMES: usize, const FRACTION_BITS: u32>(
    biquad: &QuantisedBiquad,
    history: &mut BandHistory,
    block: &mut DspBlock,
) {
    let [b0, b1, b2] = biquad.feedforward();
    let [f1, f2] = biquad.feedback();
    let words = CoefficientWords { b0, b1, b2, f1, f2 };
    let [left, right] = history;
    filter_channel::<FRAMES, FRACTION_BITS, 0>(words, left, block);
    filter_channel::<FRAMES, FRACTION_BITS, 1>(words, right, block);
}

#[inline(always)]
fn filter_channel<const FRAMES: usize, const FRACTION_BITS: u32, const CHANNEL: usize>(
    words: CoefficientWords,
    history: &mut ChannelHistory,
    block: &mut DspBlock,
) {
    let rounding_half: i32 = 1 << (FRACTION_BITS - 1);
    let fraction_mask: i64 = (1 << FRACTION_BITS) - 1;
    let high_ceiling: i32 = (1 << (FRACTION_BITS - 1)) - 1;
    let high_floor: i32 = -(1 << (FRACTION_BITS - 1));
    let ChannelHistory {
        mut x1,
        mut x2,
        mut y1,
        mut y2,
        residue,
    } = *history;
    let mut carried = i64::from(residue.0 + rounding_half);
    for run in block[..FRAMES].chunks_exact_mut(SHORTEST_TICK_FRAMES) {
        for frame in run {
            let oldest_terms = carried
                .wrapping_add(wide_product(words.b2, x2))
                .wrapping_add(wide_product(words.f2, y2));
            let x = frame[CHANNEL].to_q2_29();
            let sum = oldest_terms
                .wrapping_add(wide_product(words.b1, x1))
                .wrapping_add(wide_product(words.f1, y1))
                .wrapping_add(wide_product(words.b0, x));
            let high = ((sum >> 32) as i32).clamp(high_floor, high_ceiling);
            let y = (high << (32 - FRACTION_BITS)) | ((sum as u32) >> FRACTION_BITS) as i32;
            carried = sum & fraction_mask;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
            frame[CHANNEL] = DspSample::from_q2_29(y);
        }
    }
    *history = ChannelHistory {
        x1,
        x2,
        y1,
        y2,
        residue: RoundingResidue(carried as i32 - rounding_half),
    };
}

#[inline(always)]
fn wide_product(coefficient: i32, sample: i32) -> i64 {
    i64::from(coefficient) * i64::from(sample)
}
