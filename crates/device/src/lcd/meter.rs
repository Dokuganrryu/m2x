use dsp::{MeterChannel, MeterReading, PeakLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MeterLevel(u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClipHoldFrames(u8);

pub const FACTORY_CLIP_HOLD: ClipHoldFrames = ClipHoldFrames(90);

pub const METER_CHANNELS: [MeterChannel; 4] = [
    MeterChannel::Input1,
    MeterChannel::Input2,
    MeterChannel::OutputLeft,
    MeterChannel::OutputRight,
];

const FACTORY_PEAK_DISCARDED_BITS: u32 = 7;
const FACTORY_LOG_KNEE_STEPS_BELOW_TOP: u8 = 24;

const FACTORY_LEVEL_THRESHOLDS: [u32; 76] = [
    0x008000, 0x008D53, 0x009C09, 0x00AC47, 0x00BE36, 0x00D202, 0x00E7DE, 0x010000, 0x011AA6,
    0x013812, 0x01588D, 0x017C6B, 0x01A403, 0x01CFBC, 0x020000, 0x02354C, 0x027023, 0x02B11A,
    0x02F8D5, 0x034806, 0x039F77, 0x040000, 0x046A97, 0x04E046, 0x056234, 0x05F1A9, 0x06900C,
    0x073EED, 0x080000, 0x08D52D, 0x09C08B, 0x0AC468, 0x0BE351, 0x0D2018, 0x0E7DD9, 0x100000,
    0x11AA5A, 0x138115, 0x1588CF, 0x17C6A2, 0x1A4030, 0x1CFBB1, 0x200000, 0x2354B4, 0x270229,
    0x2B119E, 0x2F8D44, 0x348060, 0x39F761, 0x400000, 0x46A968, 0x4E0452, 0x51FA0D, 0x56233B,
    0x5A827A, 0x5F1A88, 0x63EE48, 0x6900C0, 0x6E5521, 0x73EEC1, 0x79D126, 0x800000, 0x867F32,
    0x8D52CF, 0x947F1F, 0x9C08A4, 0xA3F419, 0xAC4676, 0xB504F4, 0xBE3510, 0xC7DC8F, 0xD20180,
    0xDCAA41, 0xE7DD82, 0xF3A24C, 0x1000000,
];

impl MeterLevel {
    pub const SILENT: Self = Self(0);
    pub const TOP: Self = Self(FACTORY_LEVEL_THRESHOLDS.len() as u8);
    pub const FACTORY_KNEE: Self = Self(Self::TOP.0 - FACTORY_LOG_KNEE_STEPS_BELOW_TOP);

    pub fn of_peak(peak: PeakLevel) -> Self {
        let peak_index = peak.to_bits() >> FACTORY_PEAK_DISCARDED_BITS;
        Self(FACTORY_LEVEL_THRESHOLDS.partition_point(|threshold| *threshold <= peak_index) as u8)
    }

    pub const fn steps(self) -> u8 {
        self.0
    }

    const fn one_step_down(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

impl ClipHoldFrames {
    pub const NONE: Self = Self(0);

    pub const fn new(frames: u8) -> Self {
        Self(frames)
    }

    const fn one_frame_less(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterDisplay {
    level: MeterLevel,
    clip_hold: ClipHoldFrames,
}

impl MeterDisplay {
    pub const SILENT: Self = Self {
        level: MeterLevel::SILENT,
        clip_hold: ClipHoldFrames::NONE,
    };

    pub fn next_frame(self, reading: MeterReading) -> Self {
        let clipped = reading.peak.is_clip() || reading.clipped_ticks.count() != 0;
        Self {
            level: MeterLevel::of_peak(reading.peak).max(self.level.one_step_down()),
            clip_hold: if clipped {
                FACTORY_CLIP_HOLD
            } else {
                self.clip_hold.one_frame_less()
            },
        }
    }

    pub const fn level(self) -> MeterLevel {
        self.level
    }

    pub const fn clip_lit(self) -> bool {
        self.clip_hold.0 != ClipHoldFrames::NONE.0
    }

    #[cfg(not(feature = "screen2"))]
    pub(crate) fn looks_like(self, other: Self) -> bool {
        self.level == other.level && self.clip_lit() == other.clip_lit()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterFrame([MeterDisplay; 4]);

impl MeterFrame {
    pub const SILENT: Self = Self([MeterDisplay::SILENT; 4]);

    pub fn next_frame(self, readings: [MeterReading; 4]) -> Self {
        let [a, b, c, d] = self.0;
        let [ra, rb, rc, rd] = readings;
        Self([
            a.next_frame(ra),
            b.next_frame(rb),
            c.next_frame(rc),
            d.next_frame(rd),
        ])
    }

    pub const fn channel(self, channel: MeterChannel) -> MeterDisplay {
        let [input_1, input_2, output_left, output_right] = self.0;
        match channel {
            MeterChannel::Input1 => input_1,
            MeterChannel::Input2 => input_2,
            MeterChannel::OutputLeft => output_left,
            MeterChannel::OutputRight => output_right,
        }
    }

    #[cfg(not(feature = "screen2"))]
    pub(crate) fn looks_like(self, other: Self) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .all(|(mine, theirs)| mine.looks_like(*theirs))
    }
}
