use crate::gain::Gain;
use crate::gain_ramp::FrameGains;
use crate::meter::{MeterTick, PeakLevel, StereoPeak};
use crate::sample::StreamSample;
use crate::tick::{CaptureBlock, FrameCountKernel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputMute {
    Live,
    Muted,
}

pub(crate) struct Capture<'a, 'm> {
    pub(crate) applied_gains: &'a mut [Gain; 2],
    pub(crate) target_gains: [Gain; 2],
    pub(crate) inputs: [InputMute; 2],
    pub(crate) block: &'a mut CaptureBlock,
    pub(crate) meters: &'a mut MeterTick<'m>,
}

const INPUT_LANES: core::ops::Range<usize> = 0..2;

impl InputMute {
    const fn kept_bits(self) -> i32 {
        match self {
            Self::Live => !0,
            Self::Muted => 0,
        }
    }
}

#[inline(always)]
fn capture_lane<const FRAMES: usize>(
    lane: usize,
    mut gains: FrameGains,
    mute: InputMute,
    block: &mut CaptureBlock,
) -> PeakLevel {
    let kept_bits = mute.kept_bits();
    let mut peak = PeakLevel::SILENCE;
    let frames = block.input.iter().zip(block.output.iter_mut());
    for (input, output) in frames.take(FRAMES) {
        let scaled = gains
            .next_frame()
            .scale_rounding_half_up_to_stream(input[lane]);
        let captured = StreamSample::from_bits(scaled.to_bits() & kept_bits);
        peak = peak.max(PeakLevel::of_word(captured));
        output[lane] = captured;
    }
    peak
}

impl FrameCountKernel for Capture<'_, '_> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        let mut peaks = [PeakLevel::SILENCE; 2];
        for lane in INPUT_LANES {
            peaks[lane] = capture_lane::<FRAMES>(
                lane,
                FrameGains::ramping::<FRAMES>(self.applied_gains[lane], self.target_gains[lane]),
                self.inputs[lane],
                self.block,
            );
        }
        *self.applied_gains = self.target_gains;
        self.meters.raise_capture(StereoPeak::of_inputs(peaks));
    }
}
