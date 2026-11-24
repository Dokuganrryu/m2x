use crate::sample::{Frame, I2sWord, Sample, StreamSample};

pub const MAX_FRAMES: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramesPerTick {
    Six,
    Twelve,
    TwentyFour,
}

pub(crate) trait FrameCountKernel {
    fn process<const FRAMES: usize>(self);
}

impl FramesPerTick {
    pub const fn count(self) -> usize {
        match self {
            Self::Six => 6,
            Self::Twelve => 12,
            Self::TwentyFour => 24,
        }
    }

    #[inline(always)]
    pub(crate) fn process<K: FrameCountKernel>(self, kernel: K) {
        match self {
            Self::Six => kernel.process::<6>(),
            Self::Twelve => kernel.process::<12>(),
            Self::TwentyFour => kernel.process::<24>(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TickBlock<In, Out> {
    pub input: [Frame<In>; MAX_FRAMES],
    pub output: [Frame<Out>; MAX_FRAMES],
}

pub type PlaybackBlock = TickBlock<StreamSample, I2sWord>;
pub type CaptureBlock = TickBlock<I2sWord, StreamSample>;

impl<In: Sample, Out: Sample> TickBlock<In, Out> {
    pub const fn new() -> Self {
        Self {
            input: [[In::SILENCE; 2]; MAX_FRAMES],
            output: [[Out::SILENCE; 2]; MAX_FRAMES],
        }
    }
}

impl<In: Sample, Out: Sample> Default for TickBlock<In, Out> {
    fn default() -> Self {
        Self::new()
    }
}
