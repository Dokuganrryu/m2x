// WHY: 5 bands cost 1,701 cycles of a 12,500-cycle tick at 48 kHz on hardware, 340 per band; 14 bands cost about 6,100 cycles steady and 8,250 with the fade and the largest USB span; the default build keeps 5 so the golden vectors and the budget stay pinned
pub const PEQ_BANDS: usize = if cfg!(feature = "bands14") { 14 } else { 5 };
pub const MOST_BANDS: usize = 16;
// WHY: At 14 bands up to 6 stacked +12 dB bands of every kind and Q track the saturating reference at 48 and 96 kHz; deeper stacks of high-Q shelves driven at full scale can leave the reference, so the built-in presets are proven never to clamp inside the cascade (device tests/preset.rs) and custom presets keep the per-band saturation
pub const MOST_FULL_BOOST_BANDS: usize = 6;
const _: () = assert!(PEQ_BANDS <= MOST_BANDS);

// WHY: 340 cycles a band a tick at 6 frames on hardware, and about 505 at 12 frames; so at a 100 MHz core 88.2 and 96 kHz run at most 7 bands, about 67% of the tick with the fade and the largest USB span; at 24 frames PEQ is already off (PeqAtRate), so the full set is designed and skipped there
const MOST_BANDS_AT_TWELVE_FRAMES: usize = 7;

// WHY: At 150 MHz a tick holds 18,750 cycles and 14 bands at 12 frames cost about 9,125 steady, so the board lifts the 12-frame cap once its core runs at 150 MHz; the cap is read where coefficients are designed, never in the tick
#[cfg(feature = "band-governor")]
static BANDS_AT_TWELVE_FRAMES: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(MOST_BANDS_AT_TWELVE_FRAMES);

#[cfg(feature = "band-governor")]
pub fn lift_bands_at_twelve_frames() {
    BANDS_AT_TWELVE_FRAMES.store(PEQ_BANDS, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(feature = "band-governor")]
pub fn bands_running_at(frames: crate::tick::FramesPerTick) -> usize {
    match frames {
        crate::tick::FramesPerTick::Twelve => BANDS_AT_TWELVE_FRAMES
            .load(core::sync::atomic::Ordering::Relaxed)
            .min(PEQ_BANDS),
        crate::tick::FramesPerTick::Six | crate::tick::FramesPerTick::TwentyFour => PEQ_BANDS,
    }
}

#[cfg(not(feature = "band-governor"))]
pub const fn bands_running_at(frames: crate::tick::FramesPerTick) -> usize {
    match frames {
        crate::tick::FramesPerTick::Six => PEQ_BANDS,
        crate::tick::FramesPerTick::Twelve => {
            if PEQ_BANDS < MOST_BANDS_AT_TWELVE_FRAMES {
                PEQ_BANDS
            } else {
                MOST_BANDS_AT_TWELVE_FRAMES
            }
        }
        crate::tick::FramesPerTick::TwentyFour => PEQ_BANDS,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Peaking,
    LowShelf,
    HighShelf,
    HighPass,
    LowPass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandParameterError {
    FrequencyOutsideAudibleBand,
    GainBeyondTwelveDecibels,
    QOutsideSupportedRange,
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct BandFrequency(f64);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct BandGain(f64);

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct BandQ(f64);

const LOWEST_HZ: f64 = 20.0;
const DEEPEST_TRIM_DB: f64 = -18.0;
const HIGHEST_HZ: f64 = 20_000.0;
const LARGEST_GAIN_DB: f64 = 12.0;
const LOWEST_Q: f64 = 0.3;
const HIGHEST_Q: f64 = 10.0;

impl BandFrequency {
    pub const LOWEST: Self = Self(LOWEST_HZ);
    pub const HIGHEST: Self = Self(HIGHEST_HZ);

    pub fn from_hz(hz: f64) -> Result<Self, BandParameterError> {
        (LOWEST_HZ..=HIGHEST_HZ)
            .contains(&hz)
            .then_some(Self(hz))
            .ok_or(BandParameterError::FrequencyOutsideAudibleBand)
    }

    pub const fn hz(self) -> f64 {
        self.0
    }
}

impl BandGain {
    pub const FLAT: Self = Self(0.0);
    pub const LARGEST_CUT: Self = Self(-LARGEST_GAIN_DB);
    pub const LARGEST_BOOST: Self = Self(LARGEST_GAIN_DB);

    pub fn from_db(db: f64) -> Result<Self, BandParameterError> {
        (-LARGEST_GAIN_DB..=LARGEST_GAIN_DB)
            .contains(&db)
            .then_some(Self(db))
            .ok_or(BandParameterError::GainBeyondTwelveDecibels)
    }

    pub const fn db(self) -> f64 {
        self.0
    }
}

impl BandQ {
    pub const LOWEST: Self = Self(LOWEST_Q);
    pub const BUTTERWORTH: Self = Self(core::f64::consts::FRAC_1_SQRT_2);
    pub const HIGHEST: Self = Self(HIGHEST_Q);

    pub fn new(q: f64) -> Result<Self, BandParameterError> {
        (LOWEST_Q..=HIGHEST_Q)
            .contains(&q)
            .then_some(Self(q))
            .ok_or(BandParameterError::QOutsideSupportedRange)
    }

    pub const fn value(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct PresetTrim(f64);

impl PresetTrim {
    pub const NONE: Self = Self(0.0);

    pub fn from_db(db: f64) -> Result<Self, BandParameterError> {
        (DEEPEST_TRIM_DB..=0.0)
            .contains(&db)
            .then_some(Self(db))
            .ok_or(BandParameterError::GainBeyondTwelveDecibels)
    }

    pub const fn db(self) -> f64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub kind: FilterKind,
    pub frequency: BandFrequency,
    pub gain: BandGain,
    pub q: BandQ,
}

impl Band {
    // WHY: Building the built-in presets at run time cost 2.8 KB of code; a const band is checked by the compiler against the same limits and costs only its data
    pub const fn checked(kind: FilterKind, hz: f64, db: f64, q: f64) -> Self {
        assert!(hz >= LOWEST_HZ && hz <= HIGHEST_HZ);
        assert!(db >= -LARGEST_GAIN_DB && db <= LARGEST_GAIN_DB);
        assert!(q >= LOWEST_Q && q <= HIGHEST_Q);
        Self {
            kind,
            frequency: BandFrequency(hz),
            gain: BandGain(db),
            q: BandQ(q),
        }
    }
}

impl PresetTrim {
    pub const fn checked(db: f64) -> Self {
        assert!(db >= DEEPEST_TRIM_DB && db <= 0.0);
        Self(db)
    }
}

impl Band {
    pub const FLAT: Self = Self {
        kind: FilterKind::Peaking,
        frequency: BandFrequency(1_000.0),
        gain: BandGain::FLAT,
        q: BandQ::BUTTERWORTH,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresetBand {
    pub band: Band,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeqPreset {
    pub bands: [PresetBand; PEQ_BANDS],
}

impl PeqPreset {
    pub const FLAT: Self = Self {
        bands: [PresetBand {
            band: Band::FLAT,
            enabled: false,
        }; PEQ_BANDS],
    };
}
