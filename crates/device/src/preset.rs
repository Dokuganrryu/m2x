mod tables;

use dsp::{Band, DspMode, FilterKind, PEQ_BANDS, PeqPreset, PresetBand, PresetTrim};

use crate::lcd::screen::DspBadge;
use tables::{CLARITY, CLARITY_HIGH_RATE, CLARITY_TRIM_DB, WARM, WARM_HIGH_RATE, WARM_TRIM_DB};
#[cfg(feature = "sound3")]
use tables::{
    CLARITY_HIGH_RATE_14, CLARITY_TRIM_DB_AT_150_MHZ, WARM_HIGH_RATE_14, WARM_TRIM_DB_AT_150_MHZ,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Clarity,
    Warm,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bands {
    pub preset: PeqPreset,
    pub high_rate: PeqPreset,
    pub trim: PresetTrim,
    #[cfg(feature = "sound3")]
    pub full_high_rate: PeqPreset,
    #[cfg(feature = "sound3")]
    pub full_trim: PresetTrim,
}

// WHY: At 150 MHz 88.2 and 96 kHz run the dedicated 14-band fit with the trim that covers it and the 48 kHz table; at 100 MHz they keep sound2's 7-band fit and its trim
#[cfg(feature = "sound3")]
impl Bands {
    pub const fn for_twelve_frame_bands(&self, bands: usize) -> (&PeqPreset, PresetTrim) {
        if bands >= PEQ_BANDS {
            (&self.full_high_rate, self.full_trim)
        } else {
            (&self.high_rate, self.trim)
        }
    }
}

pub(crate) const fn pk(hz: f64, db: f64, q: f64) -> Band {
    Band::checked(FilterKind::Peaking, hz, db, q)
}

pub(crate) const fn lsc(hz: f64, db: f64, q: f64) -> Band {
    Band::checked(FilterKind::LowShelf, hz, db, q)
}

pub(crate) const fn hsc(hz: f64, db: f64, q: f64) -> Band {
    Band::checked(FilterKind::HighShelf, hz, db, q)
}

// WHY: Every band is checked against the firmware's limits when the image is compiled, and the presets are built at compile time, so the image holds them as data
const fn preset_of(table: &[Band]) -> PeqPreset {
    let mut preset = PeqPreset::FLAT;
    let mut index = 0;
    while index < table.len() {
        preset.bands[index] = PresetBand {
            band: table[index],
            enabled: true,
        };
        index += 1;
    }
    preset
}

static CLARITY_BANDS: Bands = Bands {
    preset: preset_of(&CLARITY),
    high_rate: preset_of(&CLARITY_HIGH_RATE),
    trim: PresetTrim::checked(CLARITY_TRIM_DB),
    #[cfg(feature = "sound3")]
    full_high_rate: preset_of(&CLARITY_HIGH_RATE_14),
    #[cfg(feature = "sound3")]
    full_trim: PresetTrim::checked(CLARITY_TRIM_DB_AT_150_MHZ),
};

static WARM_BANDS: Bands = Bands {
    preset: preset_of(&WARM),
    high_rate: preset_of(&WARM_HIGH_RATE),
    trim: PresetTrim::checked(WARM_TRIM_DB),
    #[cfg(feature = "sound3")]
    full_high_rate: preset_of(&WARM_HIGH_RATE_14),
    #[cfg(feature = "sound3")]
    full_trim: PresetTrim::checked(WARM_TRIM_DB_AT_150_MHZ),
};

impl Preset {
    // WHY: Boot bit-perfect, so a new unit plays exactly what the host sends until someone picks a curve with `preset clarity`, `preset warm` or a MON 2 press held past one second
    pub const AT_BOOT: Self = Self::Flat;

    pub fn named(word: &[u8]) -> Option<Self> {
        match word {
            b"clarity" | b"eq1" => Some(Self::Clarity),
            b"warm" | b"eq2" => Some(Self::Warm),
            b"flat" | b"raw" | b"bypass" => Some(Self::Flat),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static [u8] {
        match self {
            Self::Clarity => b"clarity",
            Self::Warm => b"warm",
            Self::Flat => b"flat",
        }
    }

    pub const fn mode(self) -> DspMode {
        match self {
            Self::Clarity | Self::Warm => DspMode::On,
            Self::Flat => DspMode::Bypass,
        }
    }

    pub const fn badge(self) -> DspBadge {
        match self {
            Self::Clarity => DspBadge::Clarity,
            Self::Warm => DspBadge::Warm,
            Self::Flat => DspBadge::Raw,
        }
    }

    pub const fn toggled(self) -> Self {
        match self {
            Self::Clarity => Self::Warm,
            Self::Warm | Self::Flat => Self::Clarity,
        }
    }

    pub fn bands(self) -> Option<&'static Bands> {
        match self {
            Self::Clarity => Some(&CLARITY_BANDS),
            Self::Warm => Some(&WARM_BANDS),
            Self::Flat => None,
        }
    }
}

const _: () = assert!(CLARITY.len() <= PEQ_BANDS && WARM.len() <= PEQ_BANDS);
