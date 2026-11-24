use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering, fence};

use crate::capture::InputMute;
use crate::gain::Gain;
use crate::limiter::LimiterCoefficients;
use crate::monitor::MonitorMix;
use crate::rate::SampleRate;

#[derive(Debug)]
pub struct MonitorTargets {
    publishes_begun_and_ended: AtomicU32,
    playback: AtomicI32,
    inputs: [AtomicI32; 2],
}

const PUBLISH_IN_PROGRESS: u32 = 1;

impl MonitorTargets {
    pub const fn new() -> Self {
        Self {
            publishes_begun_and_ended: AtomicU32::new(0),
            playback: AtomicI32::new(MonitorMix::OFF.playback.to_q5_27()),
            inputs: [
                AtomicI32::new(MonitorMix::OFF.inputs[0].to_q5_27()),
                AtomicI32::new(MonitorMix::OFF.inputs[1].to_q5_27()),
            ],
        }
    }

    pub fn publish(&self, mix: MonitorMix) {
        let ended = self.publishes_begun_and_ended.load(Ordering::Relaxed);
        self.publishes_begun_and_ended
            .store(ended.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.playback
            .store(mix.playback.to_q5_27(), Ordering::Relaxed);
        for (input, gain) in self.inputs.iter().zip(mix.inputs) {
            input.store(gain.to_q5_27(), Ordering::Relaxed);
        }
        self.publishes_begun_and_ended
            .store(ended.wrapping_add(2), Ordering::Release);
    }

    pub fn latest_or(&self, held: MonitorMix) -> MonitorMix {
        let before = self.publishes_begun_and_ended.load(Ordering::Acquire);
        let mix = MonitorMix {
            playback: Gain::from_q5_27(self.playback.load(Ordering::Relaxed)),
            inputs: self
                .inputs
                .each_ref()
                .map(|input| Gain::from_q5_27(input.load(Ordering::Relaxed))),
        };
        fence(Ordering::Acquire);
        let after = self.publishes_begun_and_ended.load(Ordering::Relaxed);
        if before == after && before & PUBLISH_IN_PROGRESS == 0 {
            mix
        } else {
            held
        }
    }
}

impl Default for MonitorTargets {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct InputMutes {
    muted: [AtomicBool; 2],
}

impl InputMutes {
    pub const fn new() -> Self {
        Self {
            muted: [AtomicBool::new(false), AtomicBool::new(false)],
        }
    }

    pub fn publish(&self, mutes: [InputMute; 2]) {
        for (cell, mute) in self.muted.iter().zip(mutes) {
            cell.store(mute == InputMute::Muted, Ordering::Relaxed);
        }
    }

    pub fn latest(&self) -> [InputMute; 2] {
        self.muted.each_ref().map(|cell| {
            if cell.load(Ordering::Relaxed) {
                InputMute::Muted
            } else {
                InputMute::Live
            }
        })
    }
}

impl Default for InputMutes {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub struct LimiterTargets {
    publishes_begun_and_ended: AtomicU32,
    ceiling: AtomicI32,
    release: AtomicU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimiterFollower {
    seen: u32,
}

impl LimiterTargets {
    pub const fn new() -> Self {
        Self {
            publishes_begun_and_ended: AtomicU32::new(0),
            ceiling: AtomicI32::new(0),
            release: AtomicU32::new(0),
        }
    }

    pub fn publish(&self, coefficients: LimiterCoefficients) {
        let ended = self.publishes_begun_and_ended.load(Ordering::Relaxed);
        self.publishes_begun_and_ended
            .store(ended.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        let (ceiling, release) = coefficients.to_words();
        self.ceiling.store(ceiling, Ordering::Relaxed);
        self.release.store(release, Ordering::Relaxed);
        self.publishes_begun_and_ended
            .store(ended.wrapping_add(2), Ordering::Release);
    }
}

impl Default for LimiterTargets {
    fn default() -> Self {
        Self::new()
    }
}

impl LimiterFollower {
    pub fn of(targets: &LimiterTargets) -> Self {
        Self {
            seen: targets.publishes_begun_and_ended.load(Ordering::Acquire),
        }
    }

    pub fn changed(&mut self, targets: &LimiterTargets) -> Option<LimiterCoefficients> {
        let before = targets.publishes_begun_and_ended.load(Ordering::Acquire);
        if before == self.seen || before & PUBLISH_IN_PROGRESS != 0 {
            return None;
        }
        let coefficients = LimiterCoefficients::from_words(
            targets.ceiling.load(Ordering::Relaxed),
            targets.release.load(Ordering::Relaxed),
        );
        fence(Ordering::Acquire);
        if targets.publishes_begun_and_ended.load(Ordering::Relaxed) != before {
            return None;
        }
        self.seen = before;
        Some(coefficients)
    }
}

#[derive(Debug)]
pub struct RateSwitch {
    published: AtomicU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateFollower {
    seen: u32,
}

const RATE_CODE_BITS: u32 = 3;
const RATE_CODE_MASK: u32 = (1 << RATE_CODE_BITS) - 1;
const GENERATION_STEP: u32 = 1 << RATE_CODE_BITS;

const fn rate_code(rate: SampleRate) -> u32 {
    match rate {
        SampleRate::Hz44100 => 0,
        SampleRate::Hz48000 => 1,
        SampleRate::Hz88200 => 2,
        SampleRate::Hz96000 => 3,
        SampleRate::Hz176400 => 4,
        SampleRate::Hz192000 => 5,
    }
}

const fn rate_of(word: u32) -> SampleRate {
    match word & RATE_CODE_MASK {
        0 => SampleRate::Hz44100,
        1 => SampleRate::Hz48000,
        2 => SampleRate::Hz88200,
        3 => SampleRate::Hz96000,
        4 => SampleRate::Hz176400,
        _ => SampleRate::Hz192000,
    }
}

impl RateSwitch {
    pub const fn at(rate: SampleRate) -> Self {
        Self {
            published: AtomicU32::new(rate_code(rate)),
        }
    }

    pub fn switch_to(&self, rate: SampleRate) {
        let published = self.published.load(Ordering::Relaxed);
        let generation = (published & !RATE_CODE_MASK).wrapping_add(GENERATION_STEP);
        self.published
            .store(generation | rate_code(rate), Ordering::Release);
    }

    pub fn clear_dsp_state(&self) {
        let published = self.published.load(Ordering::Relaxed);
        self.published
            .store(published.wrapping_add(GENERATION_STEP), Ordering::Release);
    }
}

impl RateFollower {
    pub fn of(switch: &RateSwitch) -> Self {
        Self {
            seen: switch.published.load(Ordering::Acquire),
        }
    }

    pub fn changed(&mut self, switch: &RateSwitch) -> Option<SampleRate> {
        let published = switch.published.load(Ordering::Acquire);
        if published == self.seen {
            return None;
        }
        self.seen = published;
        Some(rate_of(published))
    }
}

#[derive(Debug)]
pub struct PublishCount {
    publishes: AtomicU32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublishesSeen {
    seen: u32,
}

impl PublishCount {
    pub const fn new() -> Self {
        Self {
            publishes: AtomicU32::new(0),
        }
    }

    pub fn published(&self) {
        let publishes = self.publishes.load(Ordering::Relaxed);
        self.publishes
            .store(publishes.wrapping_add(1), Ordering::Release);
    }
}

impl Default for PublishCount {
    fn default() -> Self {
        Self::new()
    }
}

impl PublishesSeen {
    pub fn of(count: &PublishCount) -> Self {
        Self {
            seen: count.publishes.load(Ordering::Acquire),
        }
    }

    #[inline(always)]
    pub fn any_new(&mut self, count: &PublishCount) -> bool {
        let publishes = count.publishes.load(Ordering::Acquire);
        if publishes == self.seen {
            return false;
        }
        self.seen = publishes;
        true
    }
}
