pub(crate) mod packed;

use super::band::{Band, MOST_BANDS, PEQ_BANDS, PeqPreset, PresetTrim, bands_running_at};
use super::cookbook::{DesignedBiquad, design};
use super::series::ten_to_the_power_of_a_fraction;
use crate::rate::SampleRate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoefficientFormat {
    Q2_30,
    Q3_29,
    Q4_28,
}

impl CoefficientFormat {
    const NARROWEST_FIRST: [Self; 3] = [Self::Q2_30, Self::Q3_29, Self::Q4_28];

    pub const fn post_shift(self) -> u32 {
        match self {
            Self::Q2_30 => 0,
            Self::Q3_29 => 1,
            Self::Q4_28 => 2,
        }
    }

    pub const fn fraction_bits(self) -> u32 {
        30 - self.post_shift()
    }

    const fn unity(self) -> i32 {
        1 << self.fraction_bits()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantisedBiquad {
    feedforward: [i32; 3],
    feedback: [i32; 2],
    format: CoefficientFormat,
}

impl QuantisedBiquad {
    pub const fn feedforward(&self) -> [i32; 3] {
        self.feedforward
    }

    pub const fn feedback(&self) -> [i32; 2] {
        self.feedback
    }

    pub const fn format(&self) -> CoefficientFormat {
        self.format
    }

    fn rounded_to_nearest(designed: &DesignedBiquad) -> Self {
        let format = CoefficientFormat::NARROWEST_FIRST
            .into_iter()
            .find(|format| {
                designed
                    .feedforward
                    .iter()
                    .chain(&designed.feedback)
                    .all(|&value| fits_in_a_word(value, *format))
            })
            .unwrap_or(CoefficientFormat::Q4_28);
        Self {
            feedforward: designed.feedforward.map(|v| word(v, format)),
            feedback: designed.feedback.map(|v| word(v, format)),
            format,
        }
    }

    fn is_flat(&self) -> bool {
        let [b0, b1, b2] = self.feedforward;
        let [f1, f2] = self.feedback;
        b0 == self.format.unity() && b1 == -f1 && b2 == -f2
    }
}

fn scaled_rounded_half_away_from_zero(value: f64, format: CoefficientFormat) -> i64 {
    let scaled = value * f64::from(format.unity());
    if scaled >= 0.0 {
        (scaled + 0.5) as i64
    } else {
        (scaled - 0.5) as i64
    }
}

fn fits_in_a_word(value: f64, format: CoefficientFormat) -> bool {
    i32::try_from(scaled_rounded_half_away_from_zero(value, format)).is_ok()
}

fn word(value: f64, format: CoefficientFormat) -> i32 {
    scaled_rounded_half_away_from_zero(value, format).clamp(i32::MIN.into(), i32::MAX.into()) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandCoefficients {
    Skip,
    Filter(QuantisedBiquad),
}

impl BandCoefficients {
    pub fn design(band: &Band, rate: SampleRate) -> Self {
        Self::of(&design(band, rate))
    }

    fn of(designed: &DesignedBiquad) -> Self {
        let biquad = QuantisedBiquad::rounded_to_nearest(designed);
        if biquad.is_flat() {
            Self::Skip
        } else {
            Self::Filter(biquad)
        }
    }
}

// WHY: A trim is a gain in front of the cascade; scaling the first running band's feedforward by it costs no cycles in the tick, and only a negative trim is accepted, so no coefficient grows
fn trimmed(designed: DesignedBiquad, trim: PresetTrim) -> DesignedBiquad {
    let gain = ten_to_the_power_of_a_fraction(trim.db() / 20.0);
    DesignedBiquad {
        feedforward: designed.feedforward.map(|value| value * gain),
        feedback: designed.feedback,
    }
}

const PURE_GAIN_ONLY: DesignedBiquad = DesignedBiquad {
    feedforward: [1.0, 0.0, 0.0],
    feedback: [0.0, 0.0],
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation(u32);

impl Generation {
    pub const FIRST: Self = Self(0);

    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

const _: () = assert!(MOST_BANDS <= u16::BITS as usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoefficientSet {
    rate: SampleRate,
    bands: [BandCoefficients; PEQ_BANDS],
    enabled_mask: u16,
    generation: Generation,
}

impl CoefficientSet {
    pub fn design(preset: &PeqPreset, rate: SampleRate, generation: Generation) -> Self {
        Self::design_trimmed(preset, PresetTrim::NONE, rate, generation)
    }

    pub fn design_trimmed(
        preset: &PeqPreset,
        trim: PresetTrim,
        rate: SampleRate,
        generation: Generation,
    ) -> Self {
        // WHY: Only the 14-band build trims and caps bands by rate, so the 5-band images keep their old design code and size
        if !cfg!(feature = "bands14") {
            return Self::untrimmed(preset, rate, generation);
        }
        let running = bands_running_at(rate.frames_per_tick());
        let first = preset.bands[..running]
            .iter()
            .position(|slot| slot.enabled)
            .or(if trim == PresetTrim::NONE || running == 0 {
                None
            } else {
                Some(0)
            });
        let mut bands = [BandCoefficients::Skip; PEQ_BANDS];
        for (index, (band, slot)) in bands.iter_mut().zip(&preset.bands).enumerate() {
            let designed = match (index < running, slot.enabled, first == Some(index)) {
                (false, _, _) => continue,
                (true, true, _) => design(&slot.band, rate),
                (true, false, true) => PURE_GAIN_ONLY,
                (true, false, false) => continue,
            };
            *band = if first == Some(index) && trim != PresetTrim::NONE {
                BandCoefficients::of(&trimmed(designed, trim))
            } else {
                BandCoefficients::of(&designed)
            };
        }
        Self::with_mask(rate, bands, generation)
    }

    fn with_mask(
        rate: SampleRate,
        bands: [BandCoefficients; PEQ_BANDS],
        generation: Generation,
    ) -> Self {
        let enabled_mask = bands
            .iter()
            .enumerate()
            .filter(|(_, band)| matches!(band, BandCoefficients::Filter(_)))
            .fold(0, |mask, (index, _)| mask | 1 << index);
        Self {
            rate,
            bands,
            enabled_mask,
            generation,
        }
    }

    fn untrimmed(preset: &PeqPreset, rate: SampleRate, generation: Generation) -> Self {
        let bands = preset.bands.map(|slot| {
            if slot.enabled {
                BandCoefficients::design(&slot.band, rate)
            } else {
                BandCoefficients::Skip
            }
        });
        Self::with_mask(rate, bands, generation)
    }

    pub const fn rate(&self) -> SampleRate {
        self.rate
    }

    pub const fn bands(&self) -> &[BandCoefficients; PEQ_BANDS] {
        &self.bands
    }

    pub const fn enabled_mask(&self) -> u16 {
        self.enabled_mask
    }

    pub const fn generation(&self) -> Generation {
        self.generation
    }
}
