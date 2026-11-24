use core::marker::PhantomData;

use crate::gain::{GAIN_FRACTION_BITS, Gain};
use crate::limiter::DspBlock;
use crate::sample::{DspSample, Frame, I2sWord, StreamSample};
use crate::tick::MAX_FRAMES;

pub(crate) type StreamFrames = [Frame<StreamSample>; MAX_FRAMES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorMix {
    pub playback: Gain,
    pub inputs: [Gain; 2],
}

impl MonitorMix {
    pub const OFF: Self = Self {
        playback: Gain::UNITY,
        inputs: [Gain::ZERO; 2],
    };

    pub const fn monitors_an_input(&self) -> bool {
        self.inputs[0].to_q5_27() != 0 || self.inputs[1].to_q5_27() != 0
    }
}

pub(crate) trait PlaybackMix: Copy {
    type LineWhenPeqSkipped<'a>: LineSource;

    fn mixed(self, playback: Frame<DspSample>, inputs: Frame<StreamSample>) -> Frame<DspSample>;

    fn bypass_words(
        self,
        playback: Frame<StreamSample>,
        inputs: Frame<StreamSample>,
    ) -> Frame<I2sWord>;

    fn line_when_peq_skipped<'a>(
        self,
        input: &'a StreamFrames,
        monitored: &'a StreamFrames,
        mixed_intake: &'a DspBlock,
    ) -> Self::LineWhenPeqSkipped<'a>;
}

pub(crate) trait LineSource: Copy {
    fn words(self) -> impl Iterator<Item = Frame<I2sWord>>;

    fn words_at(self, frame: usize) -> Frame<I2sWord>;
}

#[derive(Clone, Copy)]
pub(crate) struct MixedFromStream<'a, M> {
    pub(crate) input: &'a StreamFrames,
    pub(crate) monitored: &'a StreamFrames,
    pub(crate) mixer: M,
}

#[derive(Clone, Copy)]
pub(crate) struct MixedInIntake<'a, M> {
    mixed_intake: &'a DspBlock,
    mixer: PhantomData<M>,
}

#[derive(Clone, Copy)]
pub(crate) struct PlaybackAlone;

#[derive(Clone, Copy)]
pub(crate) struct WithMonitoredInputs {
    playback: i32,
    inputs: [i32; 2],
}

const ROUNDING_HALF: i64 = 1 << (GAIN_FRACTION_BITS - 1);
const SUM_HIGH_WORD_SHIFT: u32 = 32;
const SUM_HIGH_WORD_LEFT_SHIFT: u32 = SUM_HIGH_WORD_SHIFT - GAIN_FRACTION_BITS;
const SUM_HIGH_WORD_LIMIT: i32 = 1 << (i32::BITS - SUM_HIGH_WORD_LEFT_SHIFT - 1);

impl<M: PlaybackMix> LineSource for MixedFromStream<'_, M> {
    #[inline(always)]
    fn words(self) -> impl Iterator<Item = Frame<I2sWord>> {
        self.input
            .iter()
            .zip(self.monitored)
            .map(move |(input, monitored)| self.mixer.bypass_words(*input, *monitored))
    }

    #[inline(always)]
    fn words_at(self, frame: usize) -> Frame<I2sWord> {
        self.mixer
            .bypass_words(self.input[frame], self.monitored[frame])
    }
}

impl<M: Copy> LineSource for MixedInIntake<'_, M> {
    #[inline(always)]
    fn words(self) -> impl Iterator<Item = Frame<I2sWord>> {
        self.mixed_intake
            .iter()
            .map(|frame| frame.map(I2sWord::saturating_from_dsp))
    }

    #[inline(always)]
    fn words_at(self, frame: usize) -> Frame<I2sWord> {
        self.mixed_intake[frame].map(I2sWord::saturating_from_dsp)
    }
}

impl PlaybackMix for PlaybackAlone {
    type LineWhenPeqSkipped<'a> = MixedFromStream<'a, Self>;

    #[inline(always)]
    fn line_when_peq_skipped<'a>(
        self,
        input: &'a StreamFrames,
        monitored: &'a StreamFrames,
        _: &'a DspBlock,
    ) -> Self::LineWhenPeqSkipped<'a> {
        MixedFromStream {
            input,
            monitored,
            mixer: self,
        }
    }

    #[inline(always)]
    fn mixed(self, playback: Frame<DspSample>, _: Frame<StreamSample>) -> Frame<DspSample> {
        playback
    }

    #[inline(always)]
    fn bypass_words(self, playback: Frame<StreamSample>, _: Frame<StreamSample>) -> Frame<I2sWord> {
        playback.map(I2sWord::exact_from_stream)
    }
}

impl WithMonitoredInputs {
    pub(crate) const fn of(mix: &MonitorMix) -> Self {
        Self {
            playback: mix.playback.to_q5_27(),
            inputs: [mix.inputs[0].to_q5_27(), mix.inputs[1].to_q5_27()],
        }
    }

    #[inline(always)]
    fn side(self, playback: DspSample, monitor: i64) -> DspSample {
        let sum = wide_product(playback.to_q2_29(), self.playback) + monitor;
        let high_word = (sum >> SUM_HIGH_WORD_SHIFT) as i32;
        let low_word = sum as u32;
        let saturated_high = high_word.clamp(-SUM_HIGH_WORD_LIMIT, SUM_HIGH_WORD_LIMIT - 1);
        DspSample::from_q2_29(
            (saturated_high << SUM_HIGH_WORD_LEFT_SHIFT) | (low_word >> GAIN_FRACTION_BITS) as i32,
        )
    }
}

impl PlaybackMix for WithMonitoredInputs {
    type LineWhenPeqSkipped<'a> = MixedInIntake<'a, Self>;

    #[inline(always)]
    fn line_when_peq_skipped<'a>(
        self,
        _: &'a StreamFrames,
        _: &'a StreamFrames,
        mixed_intake: &'a DspBlock,
    ) -> Self::LineWhenPeqSkipped<'a> {
        MixedInIntake {
            mixed_intake,
            mixer: PhantomData,
        }
    }

    #[inline(always)]
    fn mixed(self, playback: Frame<DspSample>, inputs: Frame<StreamSample>) -> Frame<DspSample> {
        let [first, second] = inputs.map(|input| DspSample::exact_from_stream(input).to_q2_29());
        let monitor = wide_product(first, self.inputs[0])
            + wide_product(second, self.inputs[1])
            + ROUNDING_HALF;
        [
            self.side(playback[0], monitor),
            self.side(playback[1], monitor),
        ]
    }

    #[inline(always)]
    fn bypass_words(
        self,
        playback: Frame<StreamSample>,
        inputs: Frame<StreamSample>,
    ) -> Frame<I2sWord> {
        self.mixed(playback.map(DspSample::exact_from_stream), inputs)
            .map(I2sWord::saturating_from_dsp)
    }
}

#[inline(always)]
fn wide_product(sample: i32, gain: i32) -> i64 {
    i64::from(sample) * i64::from(gain)
}
