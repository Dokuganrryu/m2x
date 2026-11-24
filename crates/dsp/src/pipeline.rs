use crate::capture::{Capture, InputMute};
use crate::crossfade::TickMix;
use crate::gain::Gain;
use crate::limiter::{LimiterCoefficients, LimiterSettings, LimiterTrack};
use crate::meter::MeterTick;
use crate::monitor::{MonitorMix, PlaybackAlone, WithMonitoredInputs};
use crate::peq::CoefficientSet;
use crate::playback::{PeqUse, PlaybackPath, PlaybackTick};
use crate::rate::{RateFamily, SampleRate};
use crate::tick::{CaptureBlock, FramesPerTick, PlaybackBlock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeqAtRate {
    Runs,
    OffAtThisRate,
}

#[derive(Clone, Copy, Debug)]
pub struct TickControls<'a> {
    pub frames: FramesPerTick,
    pub mix: TickMix,
    pub coefficients: &'a CoefficientSet,
    pub monitor: MonitorMix,
    pub inputs: [InputMute; 2],
    pub capture_gains: [Gain; 2],
}

#[derive(Clone, Debug)]
pub struct Pipeline {
    applied_capture_gains: [Gain; 2],
    playback: PlaybackPath,
}

impl PeqAtRate {
    pub const fn at(rate: SampleRate) -> Self {
        Self::for_frames(rate.frames_per_tick())
    }

    pub const fn for_frames(frames: FramesPerTick) -> Self {
        match frames {
            FramesPerTick::Six | FramesPerTick::Twelve => Self::Runs,
            FramesPerTick::TwentyFour => Self::OffAtThisRate,
        }
    }
}

impl<'a> PeqUse<'a> {
    const fn of(frames: FramesPerTick, set: &'a CoefficientSet) -> Self {
        match (PeqAtRate::for_frames(frames), set.enabled_mask()) {
            (PeqAtRate::OffAtThisRate, _) | (PeqAtRate::Runs, 0) => Self::Skipped,
            (PeqAtRate::Runs, _) => Self::Filtering(set),
        }
    }

    pub(crate) const fn in_kernel<const FRAMES: usize>(self) -> Self {
        let off_at_this_rate = FRAMES == FramesPerTick::TwentyFour.count()
            && matches!(
                PeqAtRate::for_frames(FramesPerTick::TwentyFour),
                PeqAtRate::OffAtThisRate
            );
        if off_at_this_rate {
            Self::Skipped
        } else {
            self
        }
    }
}

impl Pipeline {
    pub const fn new(limiter: LimiterCoefficients) -> Self {
        Self {
            applied_capture_gains: [Gain::UNITY; 2],
            playback: PlaybackPath::new(LimiterTrack::new(limiter)),
        }
    }

    pub fn retune_limiter(&mut self, coefficients: LimiterCoefficients) {
        self.playback.limiter().retune(coefficients);
    }

    pub fn tick(
        &mut self,
        controls: &TickControls<'_>,
        playback: &mut PlaybackBlock,
        capture: &mut CaptureBlock,
        meters: &mut MeterTick<'_>,
    ) {
        let frames = controls.frames;
        // WHY: OBS meters the stream Windows sends, so sound3's obs profile scans the USB intake before any monitor mix, trim, EQ or limiter, one pass of the tick's frames
        #[cfg(feature = "intake-meter")]
        meters.record_intake(frames, playback);
        frames.process(Capture {
            applied_gains: &mut self.applied_capture_gains,
            target_gains: controls.capture_gains,
            inputs: controls.inputs,
            block: capture,
            meters,
        });
        let peq = PeqUse::of(frames, controls.coefficients);
        if controls.monitor.monitors_an_input() {
            frames.process(PlaybackTick {
                path: &mut self.playback,
                mix: controls.mix,
                peq,
                mixer: WithMonitoredInputs::of(&controls.monitor),
                block: playback,
                monitored: &capture.output,
                meters,
            });
        } else {
            frames.process(PlaybackTick {
                path: &mut self.playback,
                mix: controls.mix,
                peq,
                mixer: PlaybackAlone,
                block: playback,
                monitored: &capture.output,
                meters,
            });
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new(LimiterCoefficients::design(
            LimiterSettings::default(),
            RateFamily::FortyEightKilohertz,
        ))
    }
}
