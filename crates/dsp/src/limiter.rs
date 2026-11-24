use core::f64::consts::LN_10;

use crate::meter::StereoPeak;
use crate::rate::RateFamily;
use crate::sample::{DspSample, Frame, I2sWord, Sample};
use crate::tick::{FrameCountKernel, FramesPerTick, MAX_FRAMES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingOutOfRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ceiling(i16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseTime(u16);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LimiterSettings {
    pub ceiling: Ceiling,
    pub release: ReleaseTime,
}

impl LimiterSettings {
    pub const DEFAULT: Self = Self {
        ceiling: Ceiling(DEFAULT_CEILING_TENTHS_OF_DBFS),
        release: ReleaseTime(DEFAULT_RELEASE_MILLIS),
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimiterCoefficients {
    ceiling: DspSample,
    release: ReleaseCoefficient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReleaseCoefficient(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LimiterGain(i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PeakBound(u32);

pub type DspBlock = [Frame<DspSample>; MAX_FRAMES];

#[derive(Clone, Copy, Debug)]
pub(crate) struct LimiterTrack {
    coefficients: LimiterCoefficients,
    gain: LimiterGain,
    held_peak: PeakBound,
    held_safe_gain: LimiterGain,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LimiterRamp {
    gain: LimiterGain,
    step: i32,
}

#[derive(Clone, Debug)]
pub struct Limiter {
    track: LimiterTrack,
    lookahead: LookaheadPair,
}

#[derive(Clone, Debug)]
pub(crate) struct LookaheadPair {
    blocks: [DspBlock; 2],
    intake: BlockSlot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockSlot {
    First,
    Second,
}

struct LimitOneTick<'a> {
    limiter: &'a mut Limiter,
}

const QUIETEST_CEILING_TENTHS_OF_DBFS: i16 = -200;
const LOUDEST_CEILING_TENTHS_OF_DBFS: i16 = 0;
const DEFAULT_CEILING_TENTHS_OF_DBFS: i16 = -10;
const SHORTEST_RELEASE_MILLIS: u16 = 5;
const LONGEST_RELEASE_MILLIS: u16 = 1000;
const DEFAULT_RELEASE_MILLIS: u16 = 50;
const TENTHS_PER_DECIBEL: f64 = 10.0;
const DECIBELS_PER_AMPLITUDE_DECADE: f64 = 20.0;
const MILLIS_PER_SECOND: f64 = 1000.0;
const RELEASE_COEFFICIENT_SCALE: f64 = 4_294_967_296.0;
const EXPONENTIAL_SERIES_TERMS: u32 = 40;
const GAIN_FRACTION_BITS: u32 = 30;
const APPLIED_GAIN_DISCARDED_BITS: u32 = MANTISSA_WORD_BITS - GAIN_FRACTION_BITS;
const APPLIED_GAIN_ROUNDING_HALF: i64 = 1 << (MANTISSA_WORD_BITS - 1);
const APPLIED_GAIN_ROUNDING_HEADROOM: u32 = 1 << (APPLIED_GAIN_DISCARDED_BITS - 1);
const LIMITED_FRACTION_BITS: u32 = 29 - APPLIED_GAIN_DISCARDED_BITS;
const LIMITED_FULL_SCALE: i32 = 1 << LIMITED_FRACTION_BITS;
const LIMITED_TO_OUTPUT_SHIFT: u32 = 31 - LIMITED_FRACTION_BITS;
const RECIPROCAL_FRACTION_BITS: u32 = 30;
const MANTISSA_WORD_BITS: u32 = 32;
const SEED_INDEX_BITS: u32 = 8;
const SEED_INDEX_SHIFT: u32 = MANTISSA_WORD_BITS - 1 - SEED_INDEX_BITS;
const SEED_COUNT: usize = 1 << SEED_INDEX_BITS;
const DOUBLED_BUCKET_MIDPOINT_SCALE_BITS: u32 = SEED_INDEX_BITS + 2;
const RECIPROCAL_SEEDS: [u32; SEED_COUNT] = reciprocal_seeds();

const fn reciprocal_seeds() -> [u32; SEED_COUNT] {
    let mut seeds = [0; SEED_COUNT];
    let mut index = 0;
    while index < SEED_COUNT {
        let doubled_bucket_midpoint = (2 * SEED_COUNT + 2 * index + 1) as u64;
        let numerator = 1u64 << (RECIPROCAL_FRACTION_BITS + DOUBLED_BUCKET_MIDPOINT_SCALE_BITS);
        seeds[index] = (numerator / doubled_bucket_midpoint) as u32;
        index += 1;
    }
    seeds
}

impl Ceiling {
    pub const fn from_tenths_of_dbfs(tenths: i16) -> Result<Self, SettingOutOfRange> {
        if tenths >= QUIETEST_CEILING_TENTHS_OF_DBFS && tenths <= LOUDEST_CEILING_TENTHS_OF_DBFS {
            Ok(Self(tenths))
        } else {
            Err(SettingOutOfRange)
        }
    }

    pub const fn tenths_of_dbfs(self) -> i16 {
        self.0
    }

    const fn level(self) -> DspSample {
        let decibels = self.0 as f64 / TENTHS_PER_DECIBEL;
        let amplitude = exponential(decibels * LN_10 / DECIBELS_PER_AMPLITUDE_DECADE);
        let full_scale = DspSample::FULL_SCALE.to_q2_29() as f64;
        DspSample::from_q2_29((amplitude * full_scale) as i32)
    }
}

impl Default for Ceiling {
    fn default() -> Self {
        Self(DEFAULT_CEILING_TENTHS_OF_DBFS)
    }
}

impl ReleaseTime {
    pub const fn from_millis(millis: u16) -> Result<Self, SettingOutOfRange> {
        if millis >= SHORTEST_RELEASE_MILLIS && millis <= LONGEST_RELEASE_MILLIS {
            Ok(Self(millis))
        } else {
            Err(SettingOutOfRange)
        }
    }

    pub const fn millis(self) -> u16 {
        self.0
    }

    const fn coefficient(self, family: RateFamily) -> ReleaseCoefficient {
        let ticks_per_time_constant =
            self.0 as f64 * family.ticks_per_second() as f64 / MILLIS_PER_SECOND;
        let fraction_closed_per_tick = 1.0 - exponential(-1.0 / ticks_per_time_constant);
        ReleaseCoefficient((fraction_closed_per_tick * RELEASE_COEFFICIENT_SCALE) as u32)
    }
}

impl Default for ReleaseTime {
    fn default() -> Self {
        Self(DEFAULT_RELEASE_MILLIS)
    }
}

impl LimiterCoefficients {
    #[inline(never)]
    pub const fn design(settings: LimiterSettings, family: RateFamily) -> Self {
        Self {
            ceiling: settings.ceiling.level(),
            release: settings.release.coefficient(family),
        }
    }

    pub const fn ceiling(&self) -> DspSample {
        self.ceiling
    }

    pub(crate) fn to_words(self) -> (i32, u32) {
        (self.ceiling.to_q2_29(), self.release.0)
    }

    pub(crate) fn from_words(ceiling: i32, release: u32) -> Self {
        Self {
            ceiling: DspSample::from_q2_29(ceiling),
            release: ReleaseCoefficient(release),
        }
    }
}

impl LimiterGain {
    const UNITY: Self = Self(1 << GAIN_FRACTION_BITS);

    #[inline(always)]
    fn released(self, coefficient: ReleaseCoefficient) -> Self {
        let gap = (Self::UNITY.0 - self.0) as u32 as u64;
        let closed = (gap * coefficient.0 as u64).div_ceil(1 << MANTISSA_WORD_BITS);
        Self(self.0 + closed as i32)
    }

    #[inline(always)]
    fn safe_for(peak: PeakBound, ceiling: DspSample) -> Self {
        let limit = ceiling.to_q2_29() as u32 - APPLIED_GAIN_ROUNDING_HEADROOM;
        let divisor = peak.0.max(limit);
        let normalising_shift = divisor.leading_zeros();
        let mantissa = divisor << normalising_shift;
        let seed = RECIPROCAL_SEEDS[(mantissa >> SEED_INDEX_SHIFT) as u8 as usize];
        let product_rounded_up =
            (mantissa as u64 * seed as u64).div_ceil(1 << MANTISSA_WORD_BITS) as u32;
        let two = 2u32 << RECIPROCAL_FRACTION_BITS;
        let reciprocal =
            (seed as u64 * (two - product_rounded_up) as u64) >> RECIPROCAL_FRACTION_BITS;
        let normalised_limit = limit << normalising_shift;
        let safe = (normalised_limit as u64 * reciprocal) >> MANTISSA_WORD_BITS;
        if peak.0 <= limit {
            Self::UNITY
        } else {
            Self(safe as i32)
        }
    }

    #[inline(always)]
    fn ramp_step_toward<const FRAMES: usize>(self, target: Self) -> i32 {
        let distance = target.0 - self.0;
        let scale_rounded_down = ((1u64 << MANTISSA_WORD_BITS) / FRAMES as u64) as i64;
        let scale_rounded_toward_more_attenuation = scale_rounded_down + (distance < 0) as i64;
        ((distance as i64 * scale_rounded_toward_more_attenuation) >> MANTISSA_WORD_BITS) as i32
    }

    #[inline(always)]
    fn apply(self, sample: DspSample) -> DspSample {
        DspSample::from_q2_29(self.applied_high_word(sample) << APPLIED_GAIN_DISCARDED_BITS)
    }

    #[inline(always)]
    pub(crate) fn apply_onto_output(self, sample: DspSample) -> I2sWord {
        let high_word = self.applied_high_word(sample);
        let saturated = high_word.clamp(-LIMITED_FULL_SCALE, LIMITED_FULL_SCALE - 1);
        I2sWord::from_bits(saturated << LIMITED_TO_OUTPUT_SHIFT)
    }

    #[inline(always)]
    fn applied_high_word(self, sample: DspSample) -> i32 {
        let product = sample.to_q2_29() as i64 * self.0 as i64;
        ((product + APPLIED_GAIN_ROUNDING_HALF) >> MANTISSA_WORD_BITS) as i32
    }
}

impl PeakBound {
    const SILENCE: Self = Self(0);

    #[inline(always)]
    pub(crate) fn of_block<const FRAMES: usize>(block: &DspBlock) -> Self {
        Self::of_stereo_peak(StereoPeak::of_block::<FRAMES, DspSample>(block))
    }

    #[inline(always)]
    pub(crate) fn of_stereo_peak(peak: StereoPeak) -> Self {
        Self(peak.loudest().to_bits() + 1)
    }
}

impl BlockSlot {
    const fn other(self) -> Self {
        match self {
            Self::First => Self::Second,
            Self::Second => Self::First,
        }
    }
}

impl LookaheadPair {
    pub(crate) const fn silent() -> Self {
        Self {
            blocks: [[[DspSample::SILENCE; 2]; MAX_FRAMES]; 2],
            intake: BlockSlot::First,
        }
    }

    #[inline(always)]
    pub(crate) fn intake_and_held(&mut self) -> (&mut DspBlock, &mut DspBlock) {
        let [first, second] = &mut self.blocks;
        match self.intake {
            BlockSlot::First => (first, second),
            BlockSlot::Second => (second, first),
        }
    }

    #[inline(always)]
    pub(crate) fn advance(&mut self) {
        self.intake = self.intake.other();
    }
}

impl LimiterTrack {
    pub(crate) const fn new(coefficients: LimiterCoefficients) -> Self {
        Self {
            coefficients,
            gain: LimiterGain::UNITY,
            held_peak: PeakBound::SILENCE,
            held_safe_gain: LimiterGain::UNITY,
        }
    }

    pub(crate) fn retune(&mut self, coefficients: LimiterCoefficients) {
        self.coefficients = coefficients;
        self.held_safe_gain = LimiterGain::safe_for(self.held_peak, coefficients.ceiling);
        self.gain = self.gain.min(self.held_safe_gain);
    }

    pub(crate) fn restart_holding(&mut self, held_peak: PeakBound) {
        self.held_peak = held_peak;
        self.held_safe_gain = LimiterGain::safe_for(held_peak, self.coefficients.ceiling);
        self.gain = self.held_safe_gain;
    }

    #[inline(always)]
    pub(crate) fn ramp_toward<const FRAMES: usize>(
        &mut self,
        incoming_peak: PeakBound,
    ) -> LimiterRamp {
        let incoming_safe_gain = LimiterGain::safe_for(incoming_peak, self.coefficients.ceiling);
        let target = incoming_safe_gain
            .min(self.held_safe_gain)
            .min(self.gain.released(self.coefficients.release));
        let ramp = self.ramp_onto::<FRAMES>(target);
        self.held_peak = incoming_peak;
        self.held_safe_gain = incoming_safe_gain;
        ramp
    }

    #[inline(always)]
    pub(crate) fn ramp_out<const FRAMES: usize>(&mut self) -> LimiterRamp {
        let target = self
            .held_safe_gain
            .min(self.gain.released(self.coefficients.release));
        self.ramp_onto::<FRAMES>(target)
    }

    #[inline(always)]
    fn ramp_onto<const FRAMES: usize>(&mut self, target: LimiterGain) -> LimiterRamp {
        let ramp = LimiterRamp {
            gain: self.gain,
            step: self.gain.ramp_step_toward::<FRAMES>(target),
        };
        self.gain = target;
        ramp
    }
}

impl LimiterRamp {
    #[cfg(feature = "meter-limiting")]
    #[inline(always)]
    pub(crate) fn limiting(&self) -> bool {
        self.gain.0 < LimiterGain::UNITY.0 || self.step < 0
    }

    #[inline(always)]
    pub(crate) fn next_frame(&mut self) -> LimiterGain {
        self.gain = LimiterGain(self.gain.0 + self.step);
        self.gain
    }
}

impl Limiter {
    pub fn new(coefficients: LimiterCoefficients) -> Self {
        Self {
            track: LimiterTrack::new(coefficients),
            lookahead: LookaheadPair::silent(),
        }
    }

    pub fn retune(&mut self, coefficients: LimiterCoefficients) {
        self.track.retune(coefficients);
    }

    pub fn intake(&mut self) -> &mut DspBlock {
        self.lookahead.intake_and_held().0
    }

    pub fn process(&mut self, frames: FramesPerTick) -> &DspBlock {
        frames.process(LimitOneTick { limiter: self });
        self.lookahead.advance();
        self.intake()
    }
}

impl FrameCountKernel for LimitOneTick<'_> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        let Limiter { track, lookahead } = self.limiter;
        let (incoming, held) = lookahead.intake_and_held();
        let mut ramp = track.ramp_toward::<FRAMES>(PeakBound::of_block::<FRAMES>(incoming));
        for frame in held.iter_mut().take(FRAMES) {
            let gain = ramp.next_frame();
            *frame = [gain.apply(frame[0]), gain.apply(frame[1])];
        }
    }
}

#[inline(never)]
const fn exponential(exponent: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut order = 1;
    loop {
        term *= exponent / order as f64;
        let next = sum + term;
        if next == sum || order == EXPONENTIAL_SERIES_TERMS {
            return next;
        }
        sum = next;
        order += 1;
    }
}
