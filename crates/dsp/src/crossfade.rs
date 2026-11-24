use core::sync::atomic::{AtomicBool, Ordering};

use crate::sample::{Frame, I2sWord};
use crate::tick::{FrameCountKernel, FramesPerTick, MAX_FRAMES};

pub const CROSSFADE_TICKS: usize = 32;

const _: () = assert!(CROSSFADE_TICKS.is_power_of_two());
const _: () = assert!(CROSSFADE_TICKS <= u8::MAX as usize);

const GAIN_FRACTION_BITS: u32 = 30;
const ROUNDING_HALF: i64 = 1 << (GAIN_FRACTION_BITS - 1);
const STEP_INDEX_MASK: u8 = CROSSFADE_TICKS as u8 - 1;
const FIRST_STEP_AFTER_REVERSAL: u8 = 2;
const REVERSAL_MIRROR: u8 = CROSSFADE_TICKS as u8 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FadeGain(i32);

impl FadeGain {
    pub const ZERO: Self = Self(0);
    pub const UNITY: Self = Self(1 << GAIN_FRACTION_BITS);

    pub const fn to_q1_30(self) -> i32 {
        self.0
    }

    const fn complement(self) -> Self {
        Self(Self::UNITY.0 - self.0)
    }
}

const SMOOTHSTEP_PROCESSED_GAINS: [FadeGain; CROSSFADE_TICKS] = smoothstep_processed_gains();

const fn smoothstep_processed_gains() -> [FadeGain; CROSSFADE_TICKS] {
    let intervals = CROSSFADE_TICKS as u64 + 1;
    let denominator = intervals * intervals * intervals;
    let unity = FadeGain::UNITY.0 as u64;
    let mut gains = [FadeGain::ZERO; CROSSFADE_TICKS];
    let mut step = 0;
    while step < CROSSFADE_TICKS {
        let position = step as u64 + 1;
        let numerator = unity * position * position * (3 * intervals - 2 * position);
        gains[step] = FadeGain(((numerator + denominator / 2) / denominator) as i32);
        step += 1;
    }
    gains
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FadeStep(u8);

impl FadeStep {
    pub const FIRST: Self = Self(0);
    pub const LAST: Self = Self(STEP_INDEX_MASK);

    pub const fn new(index: u8) -> Option<Self> {
        if index <= STEP_INDEX_MASK {
            Some(Self(index))
        } else {
            None
        }
    }

    const fn next(self) -> Option<Self> {
        Self::new(self.0 + 1)
    }

    const fn reversed(self) -> Option<Self> {
        if self.0 < FIRST_STEP_AFTER_REVERSAL {
            None
        } else {
            Some(Self(REVERSAL_MIRROR - self.0))
        }
    }

    const fn rising_gain(self) -> FadeGain {
        SMOOTHSTEP_PROCESSED_GAINS[(self.0 & STEP_INDEX_MASK) as usize]
    }

    const fn falling_gain(self) -> FadeGain {
        SMOOTHSTEP_PROCESSED_GAINS[((STEP_INDEX_MASK - self.0) & STEP_INDEX_MASK) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossfadeState {
    Bypass,
    FadingToOn(FadeStep),
    On,
    FadingToBypass(FadeStep),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrossfadeEvent {
    RequestOn,
    RequestBypass,
    Ticked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrossfadeGains {
    processed: FadeGain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickMix {
    Bypass,
    Blend(CrossfadeGains),
    Processed,
}

impl CrossfadeState {
    pub const fn step(self, event: CrossfadeEvent) -> (Self, Option<TickMix>) {
        use CrossfadeEvent::{RequestBypass, RequestOn, Ticked};
        match (self, event) {
            (Self::Bypass, RequestOn) => (Self::FadingToOn(FadeStep::FIRST), None),
            (Self::Bypass, RequestBypass) => (Self::Bypass, None),
            (Self::Bypass, Ticked) => (Self::Bypass, Some(self.mix())),
            (Self::FadingToOn(step), RequestOn) => (Self::FadingToOn(step), None),
            (Self::FadingToOn(step), RequestBypass) => match step.reversed() {
                Some(reversed) => (Self::FadingToBypass(reversed), None),
                None => (Self::Bypass, None),
            },
            (Self::FadingToOn(step), Ticked) => match step.next() {
                Some(next) => (Self::FadingToOn(next), Some(self.mix())),
                None => (Self::On, Some(self.mix())),
            },
            (Self::On, RequestOn) => (Self::On, None),
            (Self::On, RequestBypass) => (Self::FadingToBypass(FadeStep::FIRST), None),
            (Self::On, Ticked) => (Self::On, Some(self.mix())),
            (Self::FadingToBypass(step), RequestOn) => match step.reversed() {
                Some(reversed) => (Self::FadingToOn(reversed), None),
                None => (Self::On, None),
            },
            (Self::FadingToBypass(step), RequestBypass) => (Self::FadingToBypass(step), None),
            (Self::FadingToBypass(step), Ticked) => match step.next() {
                Some(next) => (Self::FadingToBypass(next), Some(self.mix())),
                None => (Self::Bypass, Some(self.mix())),
            },
        }
    }

    pub const fn mix(self) -> TickMix {
        match self {
            Self::Bypass => TickMix::Bypass,
            Self::FadingToOn(step) => TickMix::blend(step.rising_gain()),
            Self::On => TickMix::Processed,
            Self::FadingToBypass(step) => TickMix::blend(step.falling_gain()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DspMode {
    Bypass,
    On,
}

impl DspMode {
    const fn request(self) -> CrossfadeEvent {
        match self {
            Self::Bypass => CrossfadeEvent::RequestBypass,
            Self::On => CrossfadeEvent::RequestOn,
        }
    }
}

#[derive(Debug)]
pub struct DspModeSwitch {
    on_requested: AtomicBool,
}

impl DspModeSwitch {
    pub const fn new() -> Self {
        Self {
            on_requested: AtomicBool::new(false),
        }
    }

    pub fn request(&self, mode: DspMode) {
        let on_requested = match mode {
            DspMode::Bypass => false,
            DspMode::On => true,
        };
        self.on_requested.store(on_requested, Ordering::Relaxed);
    }

    pub fn requested(&self) -> DspMode {
        if self.on_requested.load(Ordering::Relaxed) {
            DspMode::On
        } else {
            DspMode::Bypass
        }
    }
}

impl Default for DspModeSwitch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossfade {
    state: CrossfadeState,
}

impl Crossfade {
    pub const fn new() -> Self {
        Self {
            state: CrossfadeState::Bypass,
        }
    }

    pub fn tick(&mut self, switch: &DspModeSwitch) -> TickMix {
        let (requested, _) = self.state.step(switch.requested().request());
        let (next, _) = requested.step(CrossfadeEvent::Ticked);
        self.state = next;
        requested.mix()
    }
}

impl Default for Crossfade {
    fn default() -> Self {
        Self::new()
    }
}

pub type OutputWords = [Frame<I2sWord>; MAX_FRAMES];

impl TickMix {
    const fn blend(processed: FadeGain) -> Self {
        Self::Blend(CrossfadeGains { processed })
    }

    pub const fn processed_gain(self) -> FadeGain {
        match self {
            Self::Bypass => FadeGain::ZERO,
            Self::Blend(gains) => gains.processed,
            Self::Processed => FadeGain::UNITY,
        }
    }

    pub const fn bypass_gain(self) -> FadeGain {
        self.processed_gain().complement()
    }

    pub fn apply(self, frames: FramesPerTick, bypass: &OutputWords, output: &mut OutputWords) {
        match self {
            Self::Bypass => frames.process(CopyBypass { bypass, output }),
            Self::Blend(gains) => frames.process(Blend {
                gains,
                bypass,
                output,
            }),
            Self::Processed => {}
        }
    }
}

impl CrossfadeGains {
    #[inline(always)]
    pub(crate) fn mix(self, bypass: I2sWord, processed: I2sWord) -> I2sWord {
        let bypass_part = bypass.to_bits() as i64 * self.processed.complement().0 as i64;
        let processed_part = processed.to_bits() as i64 * self.processed.0 as i64;
        let rounded = (bypass_part + processed_part + ROUNDING_HALF) >> GAIN_FRACTION_BITS;
        I2sWord::from_bits(rounded as i32)
    }
}

struct CopyBypass<'a> {
    bypass: &'a OutputWords,
    output: &'a mut OutputWords,
}

impl FrameCountKernel for CopyBypass<'_> {
    #[inline(always)]
    fn process<const FRAMES: usize>(self) {
        for (output, bypass) in self.output.iter_mut().zip(self.bypass).take(FRAMES) {
            *output = *bypass;
        }
    }
}

struct Blend<'a> {
    gains: CrossfadeGains,
    bypass: &'a OutputWords,
    output: &'a mut OutputWords,
}

impl FrameCountKernel for Blend<'_> {
    #[inline(never)]
    fn process<const FRAMES: usize>(self) {
        for (output, bypass) in self.output.iter_mut().zip(self.bypass).take(FRAMES) {
            *output = [
                self.gains.mix(bypass[0], output[0]),
                self.gains.mix(bypass[1], output[1]),
            ];
        }
    }
}
