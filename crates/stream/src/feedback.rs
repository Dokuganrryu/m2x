use dsp::{FramesPerTick, SampleRate};

use crate::{Frames, TRANSMIT_LEAD_BLOCKS};

const ONE_SAMPLE_16_16: u64 = 1 << 16;
const MICROFRAMES_PER_SECOND: u64 = 8_000;
const PARTS_PER_MILLION: u64 = 1_000_000;
const CLAMP_PPM: u64 = 1_000;

const PROPORTIONAL_LSB_PER_FRAME_SHIFT: u32 = 7;
const INTEGRATOR_FRACTION_BITS: u32 = 16;
const INTEGRAL_STEP_PER_FRAME_SHIFT: u32 = 12;
const INTEGRATOR_ROUNDING: i32 = 1 << (INTEGRATOR_FRACTION_BITS - 1);
const LARGEST_FILL_ERROR_FRAMES: u32 = 1 << 12;

impl Frames {
    pub const fn feedback_set_point(frames: FramesPerTick) -> Self {
        Self::new(
            Self::playback_target(frames).count()
                + Self::in_one_tick(frames).count() * TRANSMIT_LEAD_BLOCKS,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedbackValue(u32);

impl FeedbackValue {
    pub const fn nominal(rate: SampleRate) -> Self {
        Self(Window::of(rate).nominal)
    }

    pub const fn lowest(rate: SampleRate) -> Self {
        Self(Window::of(rate).lowest)
    }

    pub const fn highest(rate: SampleRate) -> Self {
        Self(Window::of(rate).highest)
    }

    pub const fn samples_per_microframe_16_16(self) -> u32 {
        self.0
    }

    pub const fn to_wire(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

#[derive(Clone, Copy)]
struct Window {
    nominal: u32,
    lowest: u32,
    highest: u32,
}

impl Window {
    const fn exact_scaled(rate: SampleRate, parts_per_million: u64) -> u64 {
        rate.hz() as u64 * ONE_SAMPLE_16_16 * parts_per_million
    }

    const fn build(rate: SampleRate) -> Self {
        let per_second = MICROFRAMES_PER_SECOND * PARTS_PER_MILLION;
        let nominal = Self::exact_scaled(rate, PARTS_PER_MILLION) + per_second / 2;
        let lowest = Self::exact_scaled(rate, PARTS_PER_MILLION - CLAMP_PPM);
        let highest = Self::exact_scaled(rate, PARTS_PER_MILLION + CLAMP_PPM) + per_second - 1;
        Self {
            nominal: (nominal / per_second) as u32,
            lowest: (lowest / per_second) as u32,
            highest: (highest / per_second) as u32,
        }
    }

    const ALL: [Self; 6] = [
        Self::build(SampleRate::Hz44100),
        Self::build(SampleRate::Hz48000),
        Self::build(SampleRate::Hz88200),
        Self::build(SampleRate::Hz96000),
        Self::build(SampleRate::Hz176400),
        Self::build(SampleRate::Hz192000),
    ];

    const fn of(rate: SampleRate) -> Self {
        Self::ALL[match rate {
            SampleRate::Hz44100 => 0,
            SampleRate::Hz48000 => 1,
            SampleRate::Hz88200 => 2,
            SampleRate::Hz96000 => 3,
            SampleRate::Hz176400 => 4,
            SampleRate::Hz192000 => 5,
        }]
    }

    const fn integrator_floor(self) -> i32 {
        (self.lowest as i32 - self.nominal as i32) << INTEGRATOR_FRACTION_BITS
    }

    const fn integrator_ceiling(self) -> i32 {
        (self.highest as i32 - self.nominal as i32) << INTEGRATOR_FRACTION_BITS
    }

    const fn clamp(self, raw: i32) -> FeedbackValue {
        let floor = self.lowest as i32;
        let ceiling = self.highest as i32;
        let bounded = if raw < floor {
            floor
        } else if raw > ceiling {
            ceiling
        } else {
            raw
        };
        FeedbackValue(bounded as u32)
    }

    const fn with_integral(self, integrator: Integrator, proportional: i32) -> FeedbackValue {
        self.clamp(self.nominal as i32 + proportional + integrator.lsb())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integrator(i32);

impl Integrator {
    pub const ZERO: Self = Self(0);

    const fn lsb(self) -> i32 {
        (self.0 + INTEGRATOR_ROUNDING) >> INTEGRATOR_FRACTION_BITS
    }

    const fn accumulate(self, error_frames: i32, window: Window) -> Self {
        let sum = self.0 + (error_frames << INTEGRAL_STEP_PER_FRAME_SHIFT);
        let floor = window.integrator_floor();
        let ceiling = window.integrator_ceiling();
        Self(if sum < floor {
            floor
        } else if sum > ceiling {
            ceiling
        } else {
            sum
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackState {
    Stopped {
        rate: SampleRate,
    },
    HoldingNominal {
        rate: SampleRate,
    },
    Tracking {
        rate: SampleRate,
        integrator: Integrator,
    },
    Recovering {
        rate: SampleRate,
        integrator: Integrator,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackEvent {
    Started,
    PrimingDone,
    Underran,
    Stopped,
    ResetStream { rate: SampleRate },
    Sof { effective_fill: Frames },
}

const fn fill_error_frames(rate: SampleRate, effective_fill: Frames) -> i32 {
    let fill = if effective_fill.count() > LARGEST_FILL_ERROR_FRAMES {
        LARGEST_FILL_ERROR_FRAMES
    } else {
        effective_fill.count()
    };
    Frames::feedback_set_point(rate.frames_per_tick()).count() as i32 - fill as i32
}

impl FeedbackState {
    #[inline(always)]
    pub const fn step(self, event: FeedbackEvent) -> (Self, FeedbackValue) {
        use FeedbackEvent as E;
        use FeedbackState::*;
        let next = match (self, event) {
            (Stopped { rate }, E::Started)
            | (HoldingNominal { rate }, E::Started)
            | (Tracking { rate, .. }, E::Started)
            | (Recovering { rate, .. }, E::Started) => HoldingNominal { rate },
            (Stopped { rate }, E::Stopped)
            | (HoldingNominal { rate }, E::Stopped)
            | (Tracking { rate, .. }, E::Stopped)
            | (Recovering { rate, .. }, E::Stopped) => Stopped { rate },
            (Stopped { .. }, E::ResetStream { rate }) => Stopped { rate },
            (HoldingNominal { .. }, E::ResetStream { rate })
            | (Tracking { .. }, E::ResetStream { rate })
            | (Recovering { .. }, E::ResetStream { rate }) => HoldingNominal { rate },
            (Stopped { .. }, E::PrimingDone | E::Underran | E::Sof { .. })
            | (HoldingNominal { .. }, E::Underran | E::Sof { .. })
            | (Tracking { .. }, E::PrimingDone)
            | (Recovering { .. }, E::Underran | E::Sof { .. }) => self,
            (HoldingNominal { rate }, E::PrimingDone) => Tracking {
                rate,
                integrator: Integrator::ZERO,
            },
            (Tracking { rate, integrator }, E::Underran) => Recovering { rate, integrator },
            (Recovering { rate, integrator }, E::PrimingDone) => Tracking { rate, integrator },
            (Tracking { rate, integrator }, E::Sof { effective_fill }) => {
                return Self::track(rate, integrator, effective_fill);
            }
        };
        (next, next.resting_value())
    }

    const fn track(
        rate: SampleRate,
        integrator: Integrator,
        effective_fill: Frames,
    ) -> (Self, FeedbackValue) {
        let window = Window::of(rate);
        let error = fill_error_frames(rate, effective_fill);
        let integrator = integrator.accumulate(error, window);
        let proportional = error << PROPORTIONAL_LSB_PER_FRAME_SHIFT;
        (
            Self::Tracking { rate, integrator },
            window.with_integral(integrator, proportional),
        )
    }

    const fn resting_value(self) -> FeedbackValue {
        match self {
            Self::Stopped { rate } | Self::HoldingNominal { rate } => FeedbackValue::nominal(rate),
            Self::Tracking { rate, integrator } | Self::Recovering { rate, integrator } => {
                Window::of(rate).with_integral(integrator, 0)
            }
        }
    }
}
