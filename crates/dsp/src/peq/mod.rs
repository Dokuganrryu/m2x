mod band;
mod coefficients;
mod cookbook;
mod series;

#[cfg(feature = "band-governor")]
pub use band::lift_bands_at_twelve_frames;
pub use band::{
    Band, BandFrequency, BandGain, BandParameterError, BandQ, FilterKind, MOST_BANDS,
    MOST_FULL_BOOST_BANDS, PEQ_BANDS, PeqPreset, PresetBand, PresetTrim, bands_running_at,
};
pub(crate) use coefficients::packed::PackedCoefficientSet;
pub use coefficients::{
    BandCoefficients, CoefficientFormat, CoefficientSet, Generation, QuantisedBiquad,
};
