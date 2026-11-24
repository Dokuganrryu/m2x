use core::f64::consts::{LN_2, LN_10};

use dsp::Gain;

use crate::analog_word::PreampCode;
use crate::boot::VolumeTargets;
use crate::dac::Attenuation;
use crate::knob::{Knob, KnobPosition, Knobs};
use crate::phantom::Jack;

const POSITIONS: usize = KnobPosition::HIGHEST.value() as usize + 1;

const LAST_FINE_TRIM_POSITION: u32 = 204;
const FINE_TRIM_STEPS_PER_HUNDRED_POSITIONS: u32 = 46;
const COARSE_TRIM_STEPS_PER_TEN_POSITIONS: u32 = 45;
const COARSE_TRIM_OFFSET_STEPS: u32 = 57;

const PREAMP_DECIBELS_WITH_NO_JACK: u8 = 6;
const PREAMP_DECIBELS_WITH_A_JACK: u8 = 9;
const MOST_PREAMP_DECIBELS: u8 = 66;
const PREAMP_TOTALS: usize = MOST_PREAMP_DECIBELS as usize + 1;
const POSITIONS_PER_PREAMP_DECIBEL_SHIFT: u32 = 2;
const DECIBELS_PER_PREAMP_CODE: u8 = 3;
const MOST_DIGITAL_PREAMP_DECIBELS: usize = 24;

const LAST_STEEP_BLEND_POSITION: usize = 110;
const STEEP_BLEND_DECIBELS_PER_POSITION: f64 = 0.515;
const STEEP_BLEND_DECIBELS_AT_ZERO: f64 = -60.0;
const GENTLE_BLEND_DECIBELS_PER_POSITION: f64 = 0.025;
const GENTLE_BLEND_DECIBELS_AT_ZERO: f64 = -6.0;

const Q5_27_UNITY: f64 = (1u64 << 27) as f64;
const DECIBELS_PER_DECADE: f64 = 20.0;
const EXPONENTIAL_SERIES_TERMS: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreampGain {
    pub code: PreampCode,
    pub digital: Gain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorBlend {
    pub inputs: Gain,
    pub playback: Gain,
}

static TRIM_BY_POSITION: [Attenuation; POSITIONS] = trim_table();
static PREAMP_BY_TOTAL_DECIBELS: [PreampGain; PREAMP_TOTALS] = preamp_table();
static BLEND_BY_POSITION: [Gain; POSITIONS] = blend_table();

pub fn volume_attenuation(position: KnobPosition) -> Attenuation {
    TRIM_BY_POSITION[position.index()]
}

pub fn volume_targets(knobs: Knobs) -> VolumeTargets {
    VolumeTargets {
        main: volume_attenuation(knobs.position(Knob::Main)),
        headphone: volume_attenuation(knobs.position(Knob::Headphone)),
    }
}

pub fn preamp_gain(position: KnobPosition, jack: Jack) -> PreampGain {
    let floor = match jack {
        Jack::Removed => PREAMP_DECIBELS_WITH_NO_JACK,
        Jack::Inserted => PREAMP_DECIBELS_WITH_A_JACK,
    };
    let from_knob = position.mirrored().value() >> POSITIONS_PER_PREAMP_DECIBEL_SHIFT;
    let total = (from_knob + floor).min(MOST_PREAMP_DECIBELS);
    PREAMP_BY_TOTAL_DECIBELS[usize::from(total)]
}

pub fn monitor_blend(position: KnobPosition) -> MonitorBlend {
    MonitorBlend {
        inputs: BLEND_BY_POSITION[position.mirrored().index()],
        playback: BLEND_BY_POSITION[position.index()],
    }
}

const fn trim_table() -> [Attenuation; POSITIONS] {
    let mut table = [Attenuation::FULL; POSITIONS];
    let mut position = 0;
    while position < POSITIONS {
        let k = position as u32;
        let steps = if k <= LAST_FINE_TRIM_POSITION {
            k * FINE_TRIM_STEPS_PER_HUNDRED_POSITIONS / 100
        } else {
            k * COARSE_TRIM_STEPS_PER_TEN_POSITIONS / 10 - COARSE_TRIM_OFFSET_STEPS
        };
        table[position] = Attenuation::from_half_decibel_steps(steps as u8);
        position += 1;
    }
    table
}

const fn preamp_table() -> [PreampGain; PREAMP_TOTALS] {
    let mut digital = [Gain::UNITY; MOST_DIGITAL_PREAMP_DECIBELS + 1];
    let mut decibels = 0;
    while decibels <= MOST_DIGITAL_PREAMP_DECIBELS {
        digital[decibels] = gain_of_decibels(decibels as f64);
        decibels += 1;
    }
    let lowest = PreampGain {
        code: PreampCode::MINIMUM,
        digital: Gain::UNITY,
    };
    let mut table = [lowest; PREAMP_TOTALS];
    let mut total = 0;
    while total < PREAMP_TOTALS {
        let analog = total as u8 / DECIBELS_PER_PREAMP_CODE;
        let code = if analog < PreampCode::MAXIMUM.value() {
            analog
        } else {
            PreampCode::MAXIMUM.value()
        };
        let remainder = total - (code * DECIBELS_PER_PREAMP_CODE) as usize;
        table[total] = PreampGain {
            code: PreampCode::new(code).unwrap(),
            digital: digital[remainder],
        };
        total += 1;
    }
    table
}

const fn blend_table() -> [Gain; POSITIONS] {
    let mut table = [Gain::ZERO; POSITIONS];
    let mut position = 1;
    while position < POSITIONS - 1 {
        let p = position as f64;
        let decibels = if position <= LAST_STEEP_BLEND_POSITION {
            STEEP_BLEND_DECIBELS_PER_POSITION * p + STEEP_BLEND_DECIBELS_AT_ZERO
        } else {
            GENTLE_BLEND_DECIBELS_PER_POSITION * p + GENTLE_BLEND_DECIBELS_AT_ZERO
        };
        table[position] = gain_of_decibels(decibels);
        position += 1;
    }
    table[POSITIONS - 1] = Gain::UNITY;
    table
}

const fn gain_of_decibels(decibels: f64) -> Gain {
    let ratio = power_of_ten(decibels / DECIBELS_PER_DECADE);
    Gain::from_q5_27((ratio * Q5_27_UNITY + 0.5) as i32)
}

const fn power_of_ten(exponent: f64) -> f64 {
    let natural = exponent * LN_10;
    let octaves = (natural / LN_2) as i32;
    let remainder = natural - octaves as f64 * LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut order = 1;
    while order <= EXPONENTIAL_SERIES_TERMS {
        term = term * remainder / order as f64;
        sum += term;
        order += 1;
    }
    let mut scaled = sum;
    let mut octave = 0;
    while octave < octaves {
        scaled *= 2.0;
        octave += 1;
    }
    while octave > octaves {
        scaled /= 2.0;
        octave -= 1;
    }
    scaled
}
