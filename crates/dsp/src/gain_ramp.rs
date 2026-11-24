use crate::gain::Gain;
use crate::tick::MAX_FRAMES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GainStep(i32);

pub(crate) struct FrameGains {
    gain: Gain,
    step_per_frame: GainStep,
}

const FINEST_FRAMES_PER_TICK: u32 = MAX_FRAMES as u32;

impl GainStep {
    #[inline(always)]
    fn per_finest_frame_truncated_toward_zero(from: Gain, to: Gain) -> Self {
        let change = i64::from(to.to_q5_27()) - i64::from(from.to_q5_27());
        let magnitude = (change.unsigned_abs() as u32 / FINEST_FRAMES_PER_TICK) as i32;
        Self(if change < 0 { -magnitude } else { magnitude })
    }

    #[inline(always)]
    const fn times(self, frames: i32) -> Self {
        Self(self.0.wrapping_mul(frames))
    }
}

impl Gain {
    #[inline(always)]
    const fn raised_by(self, step: GainStep) -> Self {
        Self::from_q5_27(self.to_q5_27().wrapping_add(step.0))
    }

    #[inline(always)]
    const fn lowered_by(self, step: GainStep) -> Self {
        Self::from_q5_27(self.to_q5_27().wrapping_sub(step.0))
    }
}

impl FrameGains {
    #[inline(always)]
    pub(crate) fn ramping<const FRAMES: usize>(applied: Gain, target: Gain) -> Self {
        let step = GainStep::per_finest_frame_truncated_toward_zero(applied, target);
        Self {
            gain: target.lowered_by(step.times(FINEST_FRAMES_PER_TICK as i32)),
            step_per_frame: step.times((MAX_FRAMES / FRAMES) as i32),
        }
    }

    #[inline(always)]
    pub(crate) fn next_frame(&mut self) -> Gain {
        self.gain = self.gain.raised_by(self.step_per_frame);
        self.gain
    }
}
