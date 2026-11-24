use core::sync::atomic::{AtomicU32, Ordering};

use crate::sample::{DspSample, Frame, I2sWord, StreamSample};
use crate::tick::{CaptureBlock, FrameCountKernel, FramesPerTick, MAX_FRAMES, PlaybackBlock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeakLevel(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClippedTicks(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterChannel {
    Input1,
    Input2,
    OutputLeft,
    OutputRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterReading {
    pub peak: PeakLevel,
    pub clipped_ticks: ClippedTicks,
}

struct MeterCell {
    peak: AtomicU32,
    clipped_ticks: AtomicU32,
}

pub struct Meters {
    inputs: [MeterCell; 2],
    outputs: [MeterCell; 2],
    #[cfg(feature = "meter-limiting")]
    limited_ticks: AtomicU32,
    #[cfg(feature = "intake-meter")]
    intake: [MeterCell; 2],
}

pub struct MeterTick<'a> {
    meters: &'a Meters,
}

pub struct MeterReader<'a> {
    meters: &'a Meters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StereoPeak([PeakLevel; 2]);

pub(crate) trait SignedWord: Copy {
    fn signed_bits(self) -> i32;
}

struct ScanBlock<'a, W, F> {
    words: &'a [Frame<W>; MAX_FRAMES],
    publish: F,
}

impl PeakLevel {
    pub const SILENCE: Self = Self(0);
    pub const CLIP: Self = Self(0x7FFF_FF00);

    pub const fn from_bits(ones_complement_magnitude: u32) -> Self {
        Self(ones_complement_magnitude)
    }

    pub const fn to_bits(self) -> u32 {
        self.0
    }

    pub const fn is_clip(self) -> bool {
        self.0 >= Self::CLIP.0
    }

    #[inline(always)]
    pub(crate) fn of_word<W: SignedWord>(word: W) -> Self {
        let bits = word.signed_bits();
        Self((bits ^ (bits >> (i32::BITS - 1))) as u32)
    }
}

impl ClippedTicks {
    pub const fn from_count(ticks: u32) -> Self {
        Self(ticks)
    }

    pub const fn count(self) -> u32 {
        self.0
    }
}

impl MeterReading {
    pub const SILENCE: Self = Self {
        peak: PeakLevel::SILENCE,
        clipped_ticks: ClippedTicks(0),
    };
}

impl MeterCell {
    const fn new() -> Self {
        Self {
            peak: AtomicU32::new(PeakLevel::SILENCE.0),
            clipped_ticks: AtomicU32::new(0),
        }
    }

    #[inline(always)]
    fn raise(&self, peak: PeakLevel) {
        self.peak.fetch_max(peak.0, Ordering::Relaxed);
        self.clipped_ticks
            .fetch_add(peak.is_clip() as u32, Ordering::Relaxed);
    }

    fn take(&self) -> MeterReading {
        MeterReading {
            peak: PeakLevel(self.peak.swap(PeakLevel::SILENCE.0, Ordering::Relaxed)),
            clipped_ticks: ClippedTicks(self.clipped_ticks.swap(0, Ordering::Relaxed)),
        }
    }
}

impl Meters {
    pub const fn new() -> Self {
        Self {
            inputs: [MeterCell::new(), MeterCell::new()],
            outputs: [MeterCell::new(), MeterCell::new()],
            #[cfg(feature = "meter-limiting")]
            limited_ticks: AtomicU32::new(0),
            #[cfg(feature = "intake-meter")]
            intake: [MeterCell::new(), MeterCell::new()],
        }
    }

    pub fn split(&mut self) -> (MeterTick<'_>, MeterReader<'_>) {
        (MeterTick { meters: self }, MeterReader { meters: self })
    }
}

impl Default for Meters {
    fn default() -> Self {
        Self::new()
    }
}

impl MeterTick<'_> {
    pub fn record_capture(&mut self, frames: FramesPerTick, block: &CaptureBlock) {
        let inputs = &self.meters.inputs;
        frames.process(ScanBlock {
            words: &block.output,
            publish: |peak: StereoPeak| raise_pair(inputs, peak),
        });
    }

    #[cfg(feature = "intake-meter")]
    pub fn record_intake(&mut self, frames: FramesPerTick, block: &PlaybackBlock) {
        let intake = &self.meters.intake;
        frames.process(ScanBlock {
            words: &block.input,
            publish: |peak: StereoPeak| raise_pair(intake, peak),
        });
    }

    pub fn record_playback(&mut self, frames: FramesPerTick, block: &PlaybackBlock) {
        let outputs = &self.meters.outputs;
        frames.process(ScanBlock {
            words: &block.output,
            publish: |peak: StereoPeak| raise_pair(outputs, peak),
        });
    }
}

impl MeterTick<'_> {
    #[inline(always)]
    pub(crate) fn raise_capture(&mut self, peak: StereoPeak) {
        raise_pair(&self.meters.inputs, peak);
    }

    #[inline(always)]
    pub(crate) fn raise_playback(&mut self, peak: StereoPeak) {
        raise_pair(&self.meters.outputs, peak);
    }

    #[cfg(feature = "meter-limiting")]
    #[inline(always)]
    pub(crate) fn count_limiting(&mut self, limiting: bool) {
        self.meters
            .limited_ticks
            .fetch_add(limiting as u32, Ordering::Relaxed);
    }
}

#[inline(always)]
fn raise_pair(cells: &[MeterCell; 2], peak: StereoPeak) {
    let [left, right] = peak.0;
    cells[0].raise(left);
    cells[1].raise(right);
}

impl MeterReader<'_> {
    #[cfg(feature = "intake-meter")]
    pub fn take_intake(&mut self) -> [MeterReading; 2] {
        let [left, right] = &self.meters.intake;
        [left.take(), right.take()]
    }

    #[cfg(feature = "meter-limiting")]
    pub fn take_limited_ticks(&mut self) -> u32 {
        self.meters.limited_ticks.swap(0, Ordering::Relaxed)
    }

    pub fn take(&mut self, channel: MeterChannel) -> MeterReading {
        let Meters {
            inputs, outputs, ..
        } = self.meters;
        match channel {
            MeterChannel::Input1 => inputs[0].take(),
            MeterChannel::Input2 => inputs[1].take(),
            MeterChannel::OutputLeft => outputs[0].take(),
            MeterChannel::OutputRight => outputs[1].take(),
        }
    }
}

impl StereoPeak {
    pub(crate) const SILENCE: Self = Self([PeakLevel::SILENCE; 2]);

    #[inline(always)]
    pub(crate) const fn of_inputs(peaks: [PeakLevel; 2]) -> Self {
        Self(peaks)
    }

    #[inline(always)]
    pub(crate) fn observe<W: SignedWord>(self, frame: Frame<W>) -> Self {
        let [left, right] = self.0;
        Self([
            left.max(PeakLevel::of_word(frame[0])),
            right.max(PeakLevel::of_word(frame[1])),
        ])
    }

    #[inline(always)]
    pub(crate) fn loudest(self) -> PeakLevel {
        let [left, right] = self.0;
        left.max(right)
    }

    #[inline(always)]
    pub(crate) fn of_block<const FRAMES: usize, W: SignedWord>(
        words: &[Frame<W>; MAX_FRAMES],
    ) -> Self {
        words
            .iter()
            .take(FRAMES)
            .fold(Self::SILENCE, |peak, frame| peak.observe(*frame))
    }
}

impl<W: SignedWord, F: FnOnce(StereoPeak)> FrameCountKernel for ScanBlock<'_, W, F> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        (self.publish)(StereoPeak::of_block::<FRAMES, W>(self.words));
    }
}

impl SignedWord for StreamSample {
    #[inline(always)]
    fn signed_bits(self) -> i32 {
        self.to_bits()
    }
}

impl SignedWord for I2sWord {
    #[inline(always)]
    fn signed_bits(self) -> i32 {
        self.to_bits()
    }
}

impl SignedWord for DspSample {
    #[inline(always)]
    fn signed_bits(self) -> i32 {
        self.to_q2_29()
    }
}
