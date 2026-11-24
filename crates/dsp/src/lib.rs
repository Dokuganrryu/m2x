#![no_std]
#![forbid(unsafe_code)]

mod biquad;
mod capture;
mod capture_targets;
mod crossfade;
mod gain;
mod gain_ramp;
mod handover;
mod limiter;
mod meter;
mod monitor;
mod peq;
mod pipeline;
mod playback;
mod rate;
mod sample;
mod swap;
mod tick;

pub use biquad::Peq;
pub use capture::InputMute;
pub use capture_targets::CaptureGainTargets;
pub use crossfade::{
    CROSSFADE_TICKS, Crossfade, CrossfadeEvent, CrossfadeGains, CrossfadeState, DspMode,
    DspModeSwitch, FadeGain, FadeStep, OutputWords, TickMix,
};
pub use gain::Gain;
pub use handover::{
    InputMutes, LimiterFollower, LimiterTargets, MonitorTargets, PublishCount, PublishesSeen,
    RateFollower, RateSwitch,
};
pub use limiter::{
    Ceiling, DspBlock, Limiter, LimiterCoefficients, LimiterSettings, ReleaseTime,
    SettingOutOfRange,
};
pub use meter::{
    ClippedTicks, MeterChannel, MeterReader, MeterReading, MeterTick, Meters, PeakLevel,
};
pub use monitor::MonitorMix;
#[cfg(feature = "band-governor")]
pub use peq::lift_bands_at_twelve_frames;
pub use peq::{
    Band, BandCoefficients, BandFrequency, BandGain, BandParameterError, BandQ, CoefficientFormat,
    CoefficientSet, FilterKind, Generation, MOST_BANDS, MOST_FULL_BOOST_BANDS, PEQ_BANDS,
    PeqPreset, PresetBand, PresetTrim, QuantisedBiquad, bands_running_at,
};
pub use pipeline::{PeqAtRate, Pipeline, TickControls};
pub use rate::{RateFamily, SampleRate};
pub use sample::{Attenuation, DspSample, Frame, I2sWord, Sample, StreamSample};
pub use swap::{
    CoefficientExchange, CoefficientPublisher, CoefficientSlot, CoefficientTick, NextFill,
    SwapCommand, SwapEvent, SwapState,
};
pub use tick::{CaptureBlock, FramesPerTick, MAX_FRAMES, PlaybackBlock, TickBlock};
