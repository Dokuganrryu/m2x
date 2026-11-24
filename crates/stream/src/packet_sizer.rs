use dsp::{FramesPerTick, SampleRate};

use crate::Frames;

const MICROFRAMES_PER_SECOND: u32 = 8000;
const RATE_GRANULE_HZ: u32 = 100;
const MICROFRAMES_PER_PHASE_CYCLE: u32 = MICROFRAMES_PER_SECOND / RATE_GRANULE_HZ;
const PHASE_STEPS_PER_FRAME: u8 = MICROFRAMES_PER_PHASE_CYCLE as u8;

const ALTERNATE_ONE_MAX_PACKET_BYTES: usize = 200;
const BYTES_PER_STEREO_SLOT: usize = 8;
pub const MAX_CAPTURE_PACKET_FRAMES: usize = ALTERNATE_ONE_MAX_PACKET_BYTES / BYTES_PER_STEREO_SLOT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NominalPacket {
    whole_frames: u8,
    phase_step: u8,
}

impl NominalPacket {
    const fn at(rate_hz: u32) -> Self {
        assert!(rate_hz % RATE_GRANULE_HZ == 0);
        let frames_per_phase_cycle = rate_hz / RATE_GRANULE_HZ;
        let largest_size = frames_per_phase_cycle / MICROFRAMES_PER_PHASE_CYCLE + 1;
        assert!(largest_size as usize <= MAX_CAPTURE_PACKET_FRAMES);
        Self {
            whole_frames: (frames_per_phase_cycle / MICROFRAMES_PER_PHASE_CYCLE) as u8,
            phase_step: (frames_per_phase_cycle % MICROFRAMES_PER_PHASE_CYCLE) as u8,
        }
    }

    const fn of(rate: SampleRate) -> Self {
        match rate {
            SampleRate::Hz44100 => const { Self::at(SampleRate::Hz44100.hz()) },
            SampleRate::Hz48000 => const { Self::at(SampleRate::Hz48000.hz()) },
            SampleRate::Hz88200 => const { Self::at(SampleRate::Hz88200.hz()) },
            SampleRate::Hz96000 => const { Self::at(SampleRate::Hz96000.hz()) },
            SampleRate::Hz176400 => const { Self::at(SampleRate::Hz176400.hz()) },
            SampleRate::Hz192000 => const { Self::at(SampleRate::Hz192000.hz()) },
        }
    }

    const fn is_whole(self) -> bool {
        self.phase_step == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PacketAdjustment {
    Nominal,
    OneFrameMore,
    OneFrameFewer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketSizer {
    rate: SampleRate,
    phase: u8,
}

impl PacketSizer {
    pub const fn starting(rate: SampleRate) -> Self {
        Self { rate, phase: 0 }
    }

    pub const fn rate(self) -> SampleRate {
        self.rate
    }

    pub(crate) const fn target_fill(self) -> Frames {
        Frames::capture_target(frames_per_tick(self.rate))
    }

    pub(crate) const fn one_tick(self) -> Frames {
        Frames::in_one_tick(frames_per_tick(self.rate))
    }

    pub(crate) const fn next(self, adjustment: PacketAdjustment) -> (Self, Frames) {
        let nominal = NominalPacket::of(self.rate);
        let advanced = self.phase + nominal.phase_step;
        let carries = advanced >= PHASE_STEPS_PER_FRAME;
        let phase = if carries {
            advanced - PHASE_STEPS_PER_FRAME
        } else {
            advanced
        };
        let size = match adjustment {
            PacketAdjustment::Nominal => nominal.whole_frames + carries as u8,
            PacketAdjustment::OneFrameMore => nominal.whole_frames + 1,
            PacketAdjustment::OneFrameFewer => nominal.whole_frames - nominal.is_whole() as u8,
        };
        (
            Self {
                rate: self.rate,
                phase,
            },
            Frames::new(size as u32),
        )
    }
}

const fn frames_per_tick(rate: SampleRate) -> FramesPerTick {
    match rate {
        SampleRate::Hz44100 | SampleRate::Hz48000 => FramesPerTick::Six,
        SampleRate::Hz88200 | SampleRate::Hz96000 => FramesPerTick::Twelve,
        SampleRate::Hz176400 | SampleRate::Hz192000 => FramesPerTick::TwentyFour,
    }
}
