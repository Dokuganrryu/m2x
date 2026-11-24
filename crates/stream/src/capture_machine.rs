use dsp::SampleRate;

use crate::packet_sizer::{PacketAdjustment, PacketSizer};
use crate::{AlternateSetting, Frames};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Idle(SampleRate),
    Filling(PacketSizer),
    Streaming(PacketSizer),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureEvent {
    AltSettingSelected(AlternateSetting),
    Microframe { fill: Frames },
    ResetStream(SampleRate),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureCommand {
    SendNothing,
    DiscardRing,
    SendSilence(Frames),
    SendRingFrames(Frames),
}

impl CaptureState {
    #[inline(always)]
    pub const fn step(self, event: CaptureEvent) -> (Self, CaptureCommand) {
        use CaptureCommand::*;
        use CaptureState::*;
        match (self, event) {
            (Idle(rate), CaptureEvent::AltSettingSelected(AlternateSetting::One)) => {
                (Filling(PacketSizer::starting(rate)), DiscardRing)
            }
            (
                Filling(sizer) | Streaming(sizer),
                CaptureEvent::AltSettingSelected(AlternateSetting::One),
            ) => (Filling(PacketSizer::starting(sizer.rate())), DiscardRing),
            (Idle(rate), CaptureEvent::AltSettingSelected(AlternateSetting::Zero)) => {
                (Idle(rate), SendNothing)
            }
            (
                Filling(sizer) | Streaming(sizer),
                CaptureEvent::AltSettingSelected(AlternateSetting::Zero),
            ) => (Idle(sizer.rate()), SendNothing),
            (Idle(_), CaptureEvent::ResetStream(rate)) => (Idle(rate), SendNothing),
            (Filling(_) | Streaming(_), CaptureEvent::ResetStream(rate)) => {
                (Filling(PacketSizer::starting(rate)), DiscardRing)
            }
            (Idle(rate), CaptureEvent::Microframe { .. }) => (Idle(rate), SendNothing),
            (Filling(sizer), CaptureEvent::Microframe { fill }) => {
                let (next, size) = sizer.next(PacketAdjustment::Nominal);
                if fill.count() >= sizer.target_fill().count() {
                    (Streaming(next), SendRingFrames(size))
                } else {
                    (Filling(next), SendSilence(size))
                }
            }
            (Streaming(sizer), CaptureEvent::Microframe { fill }) => {
                let (next, size) = sizer.next(adjustment_toward_target(sizer, fill));
                if fill.count() < size.count() {
                    (Filling(next), SendRingFrames(size))
                } else {
                    (Streaming(next), SendRingFrames(size))
                }
            }
        }
    }
}

const fn adjustment_toward_target(sizer: PacketSizer, fill: Frames) -> PacketAdjustment {
    let target = sizer.target_fill().count();
    let tolerance = sizer.one_tick().count();
    if fill.count() > target + tolerance {
        PacketAdjustment::OneFrameMore
    } else if fill.count() + tolerance < target {
        PacketAdjustment::OneFrameFewer
    } else {
        PacketAdjustment::Nominal
    }
}
