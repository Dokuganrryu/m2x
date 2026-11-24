#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use crate::fault::FaultCode;
use crate::lcd::panel::{INIT_STEPS, InitAction, InitStep};
use crate::lcd::screen::{BACKGROUND, FAULT_BACKGROUND, Region, RegionSet, Rgb565};
use crate::outputs::{Filler, OutputList};
use crate::phantom::Milliseconds;

pub const MOST_READY_TE_EDGES_BEFORE_DRAW: usize = Region::ALL.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cursor {
    Acting(u8),
    Holding { step: u8, remaining: Milliseconds },
    Clearing(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitCursor(Cursor);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRegions(RegionSet);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultProgress {
    FinishingInit(InitCursor),
    AwaitingTransfer,
    AwaitingTeToClear,
    Clearing,
    AwaitingTeToWriteCode,
    WritingCode,
    Shown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Initialising(InitCursor),
    Idle {
        next: Region,
    },
    AwaitingTe {
        dirty: DirtyRegions,
        next: Region,
    },
    Sending {
        region: Region,
        dirty: RegionSet,
    },
    FaultScreen {
        code: FaultCode,
        progress: FaultProgress,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    StepDone,
    Elapsed(Milliseconds),
    RegionsDirty(RegionSet),
    TeEdge,
    TransferDone,
    FaultRaised(FaultCode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Init(InitAction),
    ClearScreen(Rgb565),
    SendRegion(Region),
    #[cfg(feature = "screen2")]
    SendBatch(RegionSet),
    SendFaultCode(FaultCode),
}

pub type Outputs = OutputList<Output, 1>;

impl Filler for Output {
    const FILLER: Self = Output::ClearScreen(BACKGROUND);
}

enum InitProgress {
    Continuing(InitCursor, Option<Output>),
    Finished,
}

impl InitCursor {
    pub fn at(step: usize) -> Option<Self> {
        let index = u8::try_from(step).ok()?;
        INIT_STEPS
            .get(step)
            .map(|init| Self::entering(index, *init).0)
    }

    pub const fn step(self) -> usize {
        match self.0 {
            Cursor::Acting(step) | Cursor::Holding { step, .. } | Cursor::Clearing(step) => {
                step as usize
            }
        }
    }

    pub const fn remaining_hold(self) -> Option<Milliseconds> {
        match self.0 {
            Cursor::Holding { remaining, .. } => Some(remaining),
            Cursor::Acting(_) | Cursor::Clearing(_) => None,
        }
    }

    const fn entering(index: u8, step: InitStep) -> (Self, Option<Output>) {
        match step {
            InitStep::Act(action) => (Self(Cursor::Acting(index)), Some(Output::Init(action))),
            InitStep::Hold(duration) => (
                Self(Cursor::Holding {
                    step: index,
                    remaining: duration,
                }),
                None,
            ),
            InitStep::ClearScreen => (
                Self(Cursor::Clearing(index)),
                Some(Output::ClearScreen(BACKGROUND)),
            ),
        }
    }

    fn after(index: u8) -> InitProgress {
        let next = index + 1;
        match INIT_STEPS.get(next as usize) {
            Some(step) => {
                let (cursor, output) = Self::entering(next, *step);
                InitProgress::Continuing(cursor, output)
            }
            None => InitProgress::Finished,
        }
    }

    fn advance(self, event: Event) -> InitProgress {
        match (self.0, event) {
            (Cursor::Acting(index), Event::StepDone)
            | (Cursor::Clearing(index), Event::TransferDone) => Self::after(index),
            (
                Cursor::Holding {
                    step,
                    remaining: Milliseconds(left),
                },
                Event::Elapsed(Milliseconds(passed)),
            ) => match left.saturating_sub(passed) {
                0 => Self::after(step),
                still => InitProgress::Continuing(
                    Self(Cursor::Holding {
                        step,
                        remaining: Milliseconds(still),
                    }),
                    None,
                ),
            },
            (
                Cursor::Acting(_),
                Event::Elapsed(_)
                | Event::RegionsDirty(_)
                | Event::TeEdge
                | Event::TransferDone
                | Event::FaultRaised(_),
            )
            | (
                Cursor::Holding { .. },
                Event::StepDone
                | Event::RegionsDirty(_)
                | Event::TeEdge
                | Event::TransferDone
                | Event::FaultRaised(_),
            )
            | (
                Cursor::Clearing(_),
                Event::StepDone
                | Event::Elapsed(_)
                | Event::RegionsDirty(_)
                | Event::TeEdge
                | Event::FaultRaised(_),
            ) => InitProgress::Continuing(self, None),
        }
    }
}

impl DirtyRegions {
    pub fn of(regions: RegionSet) -> Option<Self> {
        (!regions.is_empty()).then_some(Self(regions))
    }

    pub const fn regions(self) -> RegionSet {
        self.0
    }

    #[cfg(not(feature = "screen2"))]
    fn take_from(self, start: Region) -> (Region, RegionSet) {
        let region = self.0.first_from(start).unwrap_or(start);
        (region, self.0.without(region))
    }
}

pub fn start() -> (State, Outputs) {
    let (cursor, output) = InitCursor::entering(0, INIT_STEPS[0]);
    with_output(State::Initialising(cursor), output)
}

const FIRST_REGION: Region = Region::ALL[0];

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    match state {
        State::Initialising(cursor) => initialising(cursor, event),
        State::Idle { next } => idle(next, event),
        State::AwaitingTe { dirty, next } => awaiting_te(dirty, next, event),
        State::Sending { region, dirty } => sending(region, dirty, event),
        State::FaultScreen { code, progress } => fault_screen(code, progress, event),
    }
}

fn quiet(state: State) -> (State, Outputs) {
    (state, Outputs::NONE)
}

fn emitting(state: State, output: Output) -> (State, Outputs) {
    (state, Outputs::of([output]))
}

fn with_output(state: State, output: Option<Output>) -> (State, Outputs) {
    match output {
        Some(output) => emitting(state, output),
        None => quiet(state),
    }
}

fn showing_fault(code: FaultCode, progress: FaultProgress) -> State {
    State::FaultScreen { code, progress }
}

fn initialising(cursor: InitCursor, event: Event) -> (State, Outputs) {
    if let Event::FaultRaised(code) = event {
        return quiet(showing_fault(code, FaultProgress::FinishingInit(cursor)));
    }
    match cursor.advance(event) {
        InitProgress::Continuing(cursor, output) => {
            with_output(State::Initialising(cursor), output)
        }
        InitProgress::Finished => quiet(State::AwaitingTe {
            dirty: DirtyRegions(RegionSet::ALL),
            next: FIRST_REGION,
        }),
    }
}

fn idle(next: Region, event: Event) -> (State, Outputs) {
    match event {
        Event::RegionsDirty(regions) => match DirtyRegions::of(regions) {
            Some(dirty) => quiet(State::AwaitingTe { dirty, next }),
            None => quiet(State::Idle { next }),
        },
        Event::FaultRaised(code) => quiet(showing_fault(code, FaultProgress::AwaitingTeToClear)),
        Event::StepDone | Event::Elapsed(_) | Event::TeEdge | Event::TransferDone => {
            quiet(State::Idle { next })
        }
    }
}

fn awaiting_te(dirty: DirtyRegions, next: Region, event: Event) -> (State, Outputs) {
    match event {
        Event::RegionsDirty(regions) => quiet(State::AwaitingTe {
            dirty: DirtyRegions(dirty.0.union(regions)),
            next,
        }),
        // WHY: Every dirty region goes out in one DMA chain started on the TE edge, so the whole change lands inside one V-blank
        #[cfg(feature = "screen2")]
        Event::TeEdge => emitting(
            State::Sending {
                region: next,
                dirty: RegionSet::EMPTY,
            },
            Output::SendBatch(dirty.0),
        ),
        #[cfg(not(feature = "screen2"))]
        Event::TeEdge => {
            let (region, rest) = dirty.take_from(next);
            emitting(
                State::Sending {
                    region,
                    dirty: rest,
                },
                Output::SendRegion(region),
            )
        }
        Event::FaultRaised(code) => quiet(showing_fault(code, FaultProgress::AwaitingTeToClear)),
        Event::StepDone | Event::Elapsed(_) | Event::TransferDone => {
            quiet(State::AwaitingTe { dirty, next })
        }
    }
}

fn sending(region: Region, dirty: RegionSet, event: Event) -> (State, Outputs) {
    match event {
        Event::RegionsDirty(regions) => quiet(State::Sending {
            region,
            dirty: dirty.union(regions),
        }),
        Event::TransferDone => {
            let next = region.following();
            match DirtyRegions::of(dirty) {
                Some(dirty) => quiet(State::AwaitingTe { dirty, next }),
                None => quiet(State::Idle { next }),
            }
        }
        Event::FaultRaised(code) => quiet(showing_fault(code, FaultProgress::AwaitingTransfer)),
        Event::StepDone | Event::Elapsed(_) | Event::TeEdge => {
            quiet(State::Sending { region, dirty })
        }
    }
}

fn fault_screen(code: FaultCode, progress: FaultProgress, event: Event) -> (State, Outputs) {
    match (progress, event) {
        (
            FaultProgress::FinishingInit(cursor),
            Event::StepDone
            | Event::Elapsed(_)
            | Event::RegionsDirty(_)
            | Event::TeEdge
            | Event::TransferDone
            | Event::FaultRaised(_),
        ) => match cursor.advance(event) {
            InitProgress::Continuing(cursor, output) => with_output(
                showing_fault(code, FaultProgress::FinishingInit(cursor)),
                output,
            ),
            InitProgress::Finished => quiet(showing_fault(code, FaultProgress::AwaitingTeToClear)),
        },
        (FaultProgress::AwaitingTransfer, Event::TransferDone) => {
            quiet(showing_fault(code, FaultProgress::AwaitingTeToClear))
        }
        (FaultProgress::AwaitingTeToClear, Event::TeEdge) => emitting(
            showing_fault(code, FaultProgress::Clearing),
            Output::ClearScreen(FAULT_BACKGROUND),
        ),
        (FaultProgress::Clearing, Event::TransferDone) => {
            quiet(showing_fault(code, FaultProgress::AwaitingTeToWriteCode))
        }
        (FaultProgress::AwaitingTeToWriteCode, Event::TeEdge) => emitting(
            showing_fault(code, FaultProgress::WritingCode),
            Output::SendFaultCode(code),
        ),
        (FaultProgress::WritingCode, Event::TransferDone) => {
            quiet(showing_fault(code, FaultProgress::Shown))
        }
        (
            FaultProgress::AwaitingTransfer,
            Event::StepDone
            | Event::Elapsed(_)
            | Event::RegionsDirty(_)
            | Event::TeEdge
            | Event::FaultRaised(_),
        )
        | (
            FaultProgress::AwaitingTeToClear | FaultProgress::AwaitingTeToWriteCode,
            Event::StepDone
            | Event::Elapsed(_)
            | Event::RegionsDirty(_)
            | Event::TransferDone
            | Event::FaultRaised(_),
        )
        | (
            FaultProgress::Clearing | FaultProgress::WritingCode,
            Event::StepDone
            | Event::Elapsed(_)
            | Event::RegionsDirty(_)
            | Event::TeEdge
            | Event::FaultRaised(_),
        )
        | (
            FaultProgress::Shown,
            Event::StepDone
            | Event::Elapsed(_)
            | Event::RegionsDirty(_)
            | Event::TeEdge
            | Event::TransferDone
            | Event::FaultRaised(_),
        ) => quiet(showing_fault(code, progress)),
    }
}
