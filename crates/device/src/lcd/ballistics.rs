use dsp::{MeterReading, PeakLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Decibels(i16);

// WHY: IEC 60268-10 Type I release of 20 dB in 1.5 s, constants for the panel's calculated 51.24 Hz TE rate
pub const RELEASE_PER_FRAME: Decibels = Decibels(67);
// WHY: OBS VolumeMeter peak decay 11.76 dB/s (20 dB in 1.7 s); 11.76 x 256 / 51.24 Hz
pub const OBS_RELEASE_PER_FRAME: Decibels = Decibels(59);
const LINEAR_PIXELS_PER_DB_TENTHS: i32 = 12;
const ZONE_HYSTERESIS: i16 = 51;
const YELLOW_FROM: Decibels = Decibels(-20 * Q8 as i16);
const RED_FROM: Decibels = Decibels(-9 * Q8 as i16);
pub const PEAK_HOLD_FRAMES: u8 = 41;
const CAP_GRAVITY: i16 = 15;
const CAP_TERMINAL_SPEED: i16 = 256;
pub const ALERT_HOLD_FRAMES: u8 = 102;
pub const READOUT_LATCH_FRAMES: u8 = 13;
const READOUT_BLOCKS: usize = 8;

const Q8: i32 = 256;
const FULL_SCALE_LOG2_Q8: i32 = 31 * Q8;
const MILLI_DB_PER_BIT: i32 = 6_021;
const LOG2_FRACTION_BITS: u32 = 8;
pub const BAR_PIXELS: u8 = 72;
const KNEE: Decibels = Decibels(-20 * Q8 as i16);
const UPPER_SLOPE_TENTHS: i32 = 22;
const LOWER_SLOPE_TENTHS: i32 = 7;
const KNEE_PIXELS: i32 = 28;

impl Decibels {
    pub const FLOOR: Self = Self(-60 * Q8 as i16);
    pub const FULL_SCALE: Self = Self(0);
    pub const OVER: Self = Self(-26);

    pub const fn from_q8(q8: i16) -> Self {
        Self(q8)
    }

    pub const fn q8(self) -> i16 {
        self.0
    }

    #[inline(never)]
    pub fn of_peak(peak: PeakLevel) -> Self {
        let magnitude = peak.to_bits();
        if magnitude == 0 {
            return Self::FLOOR;
        }
        let whole = 31 - magnitude.leading_zeros();
        let mut mantissa = u64::from(magnitude) << (31 - whole);
        let mut fraction = 0;
        for bit in (0..LOG2_FRACTION_BITS).rev() {
            mantissa = (mantissa * mantissa) >> 31;
            if mantissa >= 1 << 32 {
                mantissa >>= 1;
                fraction |= 1 << bit;
            }
        }
        let log2_q8 = ((whole << LOG2_FRACTION_BITS) | fraction) as i32;
        let q8 = (log2_q8 - FULL_SCALE_LOG2_Q8) * MILLI_DB_PER_BIT / 1_000;
        Self(q8.max(Self::FLOOR.0 as i32) as i16)
    }

    pub const fn tenths(self) -> i16 {
        let scaled = self.0 as i32 * 10;
        let rounded = if scaled < 0 {
            (scaled - Q8 / 2) / Q8
        } else {
            (scaled + Q8 / 2) / Q8
        };
        rounded as i16
    }

    pub const fn bar_pixels(self) -> u8 {
        let db = self.0 as i32;
        let pixels_q8 = if db >= KNEE.0 as i32 {
            BAR_PIXELS as i32 * Q8 + db * UPPER_SLOPE_TENTHS / 10
        } else {
            KNEE_PIXELS * Q8 + (db - KNEE.0 as i32) * LOWER_SLOPE_TENTHS / 10
        };
        let pixels = (pixels_q8 + Q8 / 2) / Q8;
        if pixels < 0 {
            0
        } else if pixels > BAR_PIXELS as i32 {
            BAR_PIXELS
        } else {
            pixels as u8
        }
    }

    // WHY: OBS's linear dB scale, -60 to 0 over the bar's 72 rows at 1.2 px/dB
    pub const fn linear_pixels(self) -> u8 {
        let above_floor = self.0 as i32 - Self::FLOOR.0 as i32;
        let pixels = (above_floor * LINEAR_PIXELS_PER_DB_TENTHS / 10 + Q8 / 2) / Q8;
        if pixels < 0 {
            0
        } else if pixels > BAR_PIXELS as i32 {
            BAR_PIXELS
        } else {
            pixels as u8
        }
    }

    const fn lower(self, by: i16) -> Self {
        Self(self.0.saturating_sub(by))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    level: Decibels,
    peak: Decibels,
    hold: u8,
    speed: i16,
    alert: u8,
}

impl Meter {
    pub const SILENT: Self = Self {
        level: Decibels::FLOOR,
        peak: Decibels::FLOOR,
        hold: 0,
        speed: 0,
        alert: 0,
    };

    pub fn step(self, input: Decibels, alerted: bool) -> Self {
        self.step_with(input, alerted, RELEASE_PER_FRAME)
    }

    #[inline(never)]
    pub fn step_with(self, input: Decibels, alerted: bool, release: Decibels) -> Self {
        let level = if input >= self.level {
            input
        } else {
            self.level.lower(release.0).max(input)
        };
        let (peak, hold, speed) = if input >= self.peak {
            (input, PEAK_HOLD_FRAMES, 0)
        } else if self.hold > 0 {
            (self.peak, self.hold - 1, self.speed)
        } else {
            let speed = (self.speed + CAP_GRAVITY).min(CAP_TERMINAL_SPEED);
            (self.peak.lower(speed).max(level), 0, speed)
        };
        Self {
            level,
            peak,
            hold,
            speed,
            alert: if alerted {
                ALERT_HOLD_FRAMES
            } else {
                self.alert.saturating_sub(1)
            },
        }
    }

    pub const fn level(self) -> Decibels {
        self.level
    }

    pub const fn peak(self) -> Decibels {
        self.peak
    }

    pub const fn alert_lit(self) -> bool {
        self.alert != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Readout {
    blocks: [Decibels; READOUT_BLOCKS],
    block: Decibels,
    frames: u8,
    latched: Decibels,
}

impl Readout {
    const SILENT: Self = Self {
        blocks: [Decibels::FLOOR; READOUT_BLOCKS],
        block: Decibels::FLOOR,
        frames: 0,
        latched: Decibels::FLOOR,
    };

    fn step(self, peak: Decibels) -> Self {
        let block = self.block.max(peak);
        let frames = self.frames + 1;
        if frames < READOUT_LATCH_FRAMES {
            return Self {
                block,
                frames,
                ..self
            };
        }
        let mut blocks = self.blocks;
        blocks.copy_within(1.., 0);
        blocks[READOUT_BLOCKS - 1] = block;
        Self {
            blocks,
            block: Decibels::FLOOR,
            frames: 0,
            latched: blocks.into_iter().fold(Decibels::FLOOR, Decibels::max),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Green,
    Yellow,
    Red,
}

const fn zone_of(level: Decibels) -> Zone {
    if level.0 >= RED_FROM.0 {
        Zone::Red
    } else if level.0 >= YELLOW_FROM.0 {
        Zone::Yellow
    } else {
        Zone::Green
    }
}

// WHY: The latched readout moves up into a zone at its boundary but back down only 0.2 dB below it, so a level hovering on -9 or -20 dB cannot toggle the digits' colour every 250 ms
pub const fn zone_with_hysteresis(level: Decibels, shown: Zone) -> Zone {
    let plain = zone_of(level);
    let held = zone_of(Decibels(level.0.saturating_add(ZONE_HYSTERESIS)));
    match (shown, plain, held) {
        (Zone::Red, _, Zone::Red) => Zone::Red,
        (Zone::Yellow, Zone::Green, Zone::Yellow) => Zone::Yellow,
        _ => plain,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterFrame {
    meters: [Meter; 4],
    limiting: u8,
    readout: Readout,
    readout_zone: Zone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alert {
    Quiet,
    Limiting,
    Over,
}

fn input_clipped(reading: MeterReading) -> bool {
    reading.peak.is_clip() || reading.clipped_ticks.count() != 0
}

fn output_over(reading: MeterReading, level: Decibels) -> bool {
    input_clipped(reading) || level >= Decibels::OVER
}

impl MeterFrame {
    pub const SILENT: Self = Self {
        meters: [Meter::SILENT; 4],
        limiting: 0,
        readout: Readout::SILENT,
        readout_zone: Zone::Green,
    };

    pub fn next_frame(self, readings: [MeterReading; 4], limited_ticks: u32) -> Self {
        let [_, _, out_l, out_r] = readings;
        self.next_frame_tapped(readings, [out_l, out_r], limited_ticks, RELEASE_PER_FRAME)
    }

    // WHY: The output bars and readout follow the profile's tap (the USB intake for obs), while the over and limiter dot always judge the DAC words
    #[inline(never)]
    pub fn next_frame_tapped(
        self,
        bars: [MeterReading; 4],
        dac: [MeterReading; 2],
        limited_ticks: u32,
        release: Decibels,
    ) -> Self {
        let [in_1, in_2, out_l, out_r] = self.meters;
        let [r_in_1, r_in_2, _, _] = bars;
        let levels = bars.map(|reading| Decibels::of_peak(reading.peak));
        let [dac_l, dac_r] = dac;
        let over_l = output_over(dac_l, Decibels::of_peak(dac_l.peak));
        let over_r = output_over(dac_r, Decibels::of_peak(dac_r.peak));
        let readout = self.readout.step(levels[2].max(levels[3]));
        Self {
            meters: [
                in_1.step_with(levels[0], input_clipped(r_in_1), release),
                in_2.step_with(levels[1], input_clipped(r_in_2), release),
                out_l.step_with(levels[2], over_l, release),
                out_r.step_with(levels[3], over_r, release),
            ],
            limiting: if limited_ticks != 0 {
                ALERT_HOLD_FRAMES
            } else {
                self.limiting.saturating_sub(1)
            },
            readout_zone: zone_with_hysteresis(readout.latched, self.readout_zone),
            readout,
        }
    }

    pub const fn readout_zone(self) -> Zone {
        self.readout_zone
    }

    pub const fn meter(self, index: usize) -> Meter {
        self.meters[index]
    }

    pub const fn output_alert(self) -> Alert {
        let [_, _, left, right] = self.meters;
        if left.alert_lit() || right.alert_lit() {
            Alert::Over
        } else if self.limiting != 0 {
            Alert::Limiting
        } else {
            Alert::Quiet
        }
    }

    pub const fn readout(self) -> Decibels {
        self.readout.latched
    }
}
