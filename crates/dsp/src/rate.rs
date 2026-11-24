use crate::tick::FramesPerTick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleRate {
    Hz44100,
    Hz48000,
    Hz88200,
    Hz96000,
    Hz176400,
    Hz192000,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateFamily {
    FortyFourPointOneKilohertz,
    FortyEightKilohertz,
}

impl SampleRate {
    pub const ALL: [Self; 6] = [
        Self::Hz44100,
        Self::Hz48000,
        Self::Hz88200,
        Self::Hz96000,
        Self::Hz176400,
        Self::Hz192000,
    ];

    pub const fn hz(self) -> u32 {
        match self {
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
            Self::Hz88200 => 88_200,
            Self::Hz96000 => 96_000,
            Self::Hz176400 => 176_400,
            Self::Hz192000 => 192_000,
        }
    }

    pub const fn family(self) -> RateFamily {
        match self {
            Self::Hz44100 | Self::Hz88200 | Self::Hz176400 => {
                RateFamily::FortyFourPointOneKilohertz
            }
            Self::Hz48000 | Self::Hz96000 | Self::Hz192000 => RateFamily::FortyEightKilohertz,
        }
    }

    pub const fn frames_per_tick(self) -> FramesPerTick {
        match self {
            Self::Hz44100 | Self::Hz48000 => FramesPerTick::Six,
            Self::Hz88200 | Self::Hz96000 => FramesPerTick::Twelve,
            Self::Hz176400 | Self::Hz192000 => FramesPerTick::TwentyFour,
        }
    }
}

impl RateFamily {
    pub const fn ticks_per_second(self) -> u32 {
        match self {
            Self::FortyFourPointOneKilohertz => 7350,
            Self::FortyEightKilohertz => 8000,
        }
    }
}
