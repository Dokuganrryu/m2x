use core::sync::atomic::{AtomicU32, Ordering};

use crate::peq::{CoefficientSet, Generation, PackedCoefficientSet, PeqPreset, PresetTrim};
use crate::rate::SampleRate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoefficientSlot {
    First,
    Second,
}

impl CoefficientSlot {
    pub const fn of(generation: Generation) -> Self {
        if generation.value() & 1 == 0 {
            Self::First
        } else {
            Self::Second
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NextFill {
    NotNeeded,
    Needed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapState {
    Settled(Generation),
    Filling(Generation),
    AwaitingTick {
        published: Generation,
        next_fill: NextFill,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapEvent {
    ParametersChanged,
    DesignDone,
    TickUsedGeneration(Generation),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapCommand {
    Hold,
    FillIdleSlot(Generation),
    Publish(Generation),
}

impl SwapState {
    pub fn step(self, event: SwapEvent) -> (Self, SwapCommand) {
        use SwapEvent::{DesignDone, ParametersChanged, TickUsedGeneration};
        match (self, event) {
            (Self::Settled(active), ParametersChanged) => Self::fill(active.next()),
            (Self::Settled(active), DesignDone | TickUsedGeneration(_)) => {
                (Self::Settled(active), SwapCommand::Hold)
            }
            (Self::Filling(filling), ParametersChanged) => Self::fill(filling),
            (Self::Filling(filling), DesignDone) => (
                Self::AwaitingTick {
                    published: filling,
                    next_fill: NextFill::NotNeeded,
                },
                SwapCommand::Publish(filling),
            ),
            (Self::Filling(filling), TickUsedGeneration(_)) => {
                (Self::Filling(filling), SwapCommand::Hold)
            }
            (Self::AwaitingTick { published, .. }, ParametersChanged) => (
                Self::AwaitingTick {
                    published,
                    next_fill: NextFill::Needed,
                },
                SwapCommand::Hold,
            ),
            (awaiting @ Self::AwaitingTick { .. }, DesignDone) => (awaiting, SwapCommand::Hold),
            (
                awaiting @ Self::AwaitingTick {
                    published,
                    next_fill,
                },
                TickUsedGeneration(used),
            ) => match (used == published, next_fill) {
                (false, NextFill::NotNeeded | NextFill::Needed) => (awaiting, SwapCommand::Hold),
                (true, NextFill::NotNeeded) => (Self::Settled(published), SwapCommand::Hold),
                (true, NextFill::Needed) => Self::fill(published.next()),
            },
        }
    }

    const fn fill(generation: Generation) -> (Self, SwapCommand) {
        (
            Self::Filling(generation),
            SwapCommand::FillIdleSlot(generation),
        )
    }
}

pub struct CoefficientExchange {
    slots: [PackedCoefficientSet; 2],
    published: AtomicU32,
    adopted: AtomicU32,
}

pub struct CoefficientPublisher<'a> {
    exchange: &'a CoefficientExchange,
    state: SwapState,
    preset: PeqPreset,
    high_rate: Option<&'static PeqPreset>,
    trim: PresetTrim,
    rate: SampleRate,
}

pub struct CoefficientTick<'a> {
    exchange: &'a CoefficientExchange,
    in_use: CoefficientSet,
}

impl CoefficientExchange {
    pub const fn flat_at(rate: SampleRate) -> Self {
        Self {
            slots: [
                PackedCoefficientSet::flat_at(rate),
                PackedCoefficientSet::flat_at(rate),
            ],
            published: AtomicU32::new(Generation::FIRST.value()),
            adopted: AtomicU32::new(Generation::FIRST.value()),
        }
    }

    pub fn split(&mut self) -> (CoefficientPublisher<'_>, CoefficientTick<'_>) {
        let exchange = &*self;
        let published = exchange.published_generation();
        let in_use = exchange.slot(published).load(published);
        exchange.adopted.store(published.value(), Ordering::Release);
        (
            CoefficientPublisher {
                exchange,
                state: SwapState::Settled(published),
                preset: PeqPreset::FLAT,
                high_rate: None,
                trim: PresetTrim::NONE,
                rate: in_use.rate(),
            },
            CoefficientTick { exchange, in_use },
        )
    }

    fn slot(&self, generation: Generation) -> &PackedCoefficientSet {
        let [first, second] = &self.slots;
        match CoefficientSlot::of(generation) {
            CoefficientSlot::First => first,
            CoefficientSlot::Second => second,
        }
    }

    fn published_generation(&self) -> Generation {
        Generation::from_published(self.published.load(Ordering::Acquire))
    }
}

impl CoefficientPublisher<'_> {
    pub fn change(&mut self, preset: &PeqPreset, rate: SampleRate) {
        self.preset = *preset;
        self.high_rate = None;
        self.rate = rate;
        self.run(SwapEvent::ParametersChanged);
    }

    pub fn change_trimmed(&mut self, preset: &PeqPreset, trim: PresetTrim, rate: SampleRate) {
        self.preset = *preset;
        self.high_rate = None;
        self.trim = trim;
        self.rate = rate;
        self.run(SwapEvent::ParametersChanged);
    }

    // WHY: The first 7 bands of a 14-band fit miss the curve by 8 dB at 96 kHz because the fitted bands lean on each other, so a preset carries its own 7-band fit for 12 frames per tick
    pub fn change_presets(
        &mut self,
        preset: &PeqPreset,
        high_rate: &'static PeqPreset,
        trim: PresetTrim,
        rate: SampleRate,
    ) {
        self.preset = *preset;
        self.high_rate = Some(high_rate);
        self.trim = trim;
        self.rate = rate;
        self.run(SwapEvent::ParametersChanged);
    }

    pub fn change_trim(&mut self, trim: PresetTrim, rate: SampleRate) {
        self.trim = trim;
        self.rate = rate;
        self.run(SwapEvent::ParametersChanged);
    }

    pub const fn trim(&self) -> PresetTrim {
        self.trim
    }

    pub fn change_rate(&mut self, rate: SampleRate) {
        self.rate = rate;
        self.run(SwapEvent::ParametersChanged);
    }

    pub fn poll(&mut self) {
        let adopted = Generation::from_published(self.exchange.adopted.load(Ordering::Acquire));
        self.run(SwapEvent::TickUsedGeneration(adopted));
    }

    pub const fn state(&self) -> SwapState {
        self.state
    }

    fn run(&mut self, first: SwapEvent) {
        let mut event = first;
        loop {
            let (next, command) = self.state.step(event);
            self.state = next;
            match command {
                SwapCommand::Hold => return,
                SwapCommand::FillIdleSlot(generation) => {
                    let preset = match (self.rate.frames_per_tick(), self.high_rate) {
                        (crate::tick::FramesPerTick::Twelve, Some(high_rate)) => high_rate,
                        _ => &self.preset,
                    };
                    let set =
                        CoefficientSet::design_trimmed(preset, self.trim, self.rate, generation);
                    self.exchange.slot(generation).store(&set);
                    event = SwapEvent::DesignDone;
                }
                SwapCommand::Publish(generation) => {
                    self.exchange
                        .published
                        .store(generation.value(), Ordering::Release);
                    return;
                }
            }
        }
    }
}

impl CoefficientTick<'_> {
    pub fn adopt(&mut self) -> &CoefficientSet {
        let published = self.exchange.published_generation();
        if published != self.in_use.generation() {
            self.take_published(published);
        }
        &self.in_use
    }

    #[cold]
    #[inline(never)]
    fn take_published(&mut self, published: Generation) {
        self.in_use = self.exchange.slot(published).load(published);
        self.exchange
            .adopted
            .store(published.value(), Ordering::Release);
    }
}
