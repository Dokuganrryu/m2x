use crate::breadcrumb::{At, Weight};
use crate::command::Command;
use crate::fault::FaultCode;
use crate::output_pin::OutputPin;
use crate::outputs::{Filler, OutputList};
use crate::phantom::{Milliseconds, PinLevel};

pub const MOST_OUTPUTS_PER_EVENT: usize = 5;
pub const WATCHDOG_TIMEOUT: Milliseconds = Milliseconds(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact {
    TickAdvanced,
    ControlPassDone,
    UsbServiced,
}

impl Fact {
    const fn mask(self) -> u8 {
        match self {
            Fact::TickAdvanced => 1 << 0,
            Fact::ControlPassDone => 1 << 1,
            Fact::UsbServiced => 1 << 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress(u8);

impl Progress {
    pub const NONE: Self = Self(0);

    pub const fn with(self, fact: Fact) -> Self {
        Self(self.0 | fact.mask())
    }

    pub const fn contains(self, fact: Fact) -> bool {
        self.0 & fact.mask() != 0
    }

    pub const fn covers(self, required: Progress) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usb {
    Silent,
    Serving,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Stopped,
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Booting {
        usb: Usb,
        seen: Progress,
    },
    Healthy {
        usb: Usb,
        seen: Progress,
    },
    Faulted {
        code: FaultCode,
        transport: Transport,
        usb: Usb,
        seen: Progress,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    TickAdvanced,
    ControlPassDone,
    UsbServiced,
    UsbBus(Usb),
    TransportStarted,
    FaultRaised(FaultCode, At),
    FeedWindowEnded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Command(Command),
    FaultRaised(FaultCode),
    Nothing,
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::Nothing;
}

impl State {
    pub const START: Self = State::Booting {
        usb: Usb::Silent,
        seen: Progress::NONE,
    };

    pub const fn required(self) -> Progress {
        let (transport, usb) = match self {
            State::Booting { usb, .. } => (Transport::Stopped, usb),
            State::Healthy { usb, .. } => (Transport::Running, usb),
            State::Faulted { transport, usb, .. } => (transport, usb),
        };
        let with_tick = match transport {
            Transport::Stopped => Progress::NONE.with(Fact::ControlPassDone),
            Transport::Running => Progress::NONE
                .with(Fact::ControlPassDone)
                .with(Fact::TickAdvanced),
        };
        match usb {
            Usb::Silent => with_tick,
            Usb::Serving => with_tick.with(Fact::UsbServiced),
        }
    }

    const fn seen(self) -> Progress {
        match self {
            State::Booting { seen, .. }
            | State::Healthy { seen, .. }
            | State::Faulted { seen, .. } => seen,
        }
    }

    const fn with_seen(self, seen: Progress) -> Self {
        match self {
            State::Booting { usb, .. } => State::Booting { usb, seen },
            State::Healthy { usb, .. } => State::Healthy { usb, seen },
            State::Faulted {
                code,
                transport,
                usb,
                ..
            } => State::Faulted {
                code,
                transport,
                usb,
                seen,
            },
        }
    }

    const fn with_usb(self, usb: Usb) -> Self {
        match self {
            State::Booting { seen, .. } => State::Booting { usb, seen },
            State::Healthy { seen, .. } => State::Healthy { usb, seen },
            State::Faulted {
                code,
                transport,
                seen,
                ..
            } => State::Faulted {
                code,
                transport,
                usb,
                seen,
            },
        }
    }

    const fn observed(self, fact: Fact) -> Self {
        self.with_seen(self.seen().with(fact))
    }
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    let mut outputs = Outputs::NONE;
    let next = match event {
        Event::TickAdvanced => state.observed(Fact::TickAdvanced),
        Event::ControlPassDone => state.observed(Fact::ControlPassDone),
        Event::UsbServiced => state.observed(Fact::UsbServiced),
        Event::UsbBus(usb) => state.with_usb(usb),
        Event::TransportStarted => match state {
            State::Booting { usb, seen } | State::Healthy { usb, seen } => {
                State::Healthy { usb, seen }
            }
            State::Faulted {
                code, usb, seen, ..
            } => State::Faulted {
                code,
                transport: Transport::Running,
                usb,
                seen,
            },
        },
        Event::FaultRaised(code, at) => match state {
            State::Booting { usb, seen } | State::Healthy { usb, seen } => {
                announce_fault(&mut outputs, code, at);
                State::Faulted {
                    code,
                    transport: Transport::Stopped,
                    usb,
                    seen,
                }
            }
            State::Faulted { .. } => state,
        },
        Event::FeedWindowEnded => {
            if state.seen().covers(state.required()) {
                outputs.push(Output::Command(Command::FeedWatchdog));
                state.with_seen(Progress::NONE)
            } else {
                state
            }
        }
    };
    (next, outputs)
}

fn announce_fault(outputs: &mut Outputs, code: FaultCode, at: At) {
    outputs.push(Output::Command(Command::LatchPlaybackSilence));
    outputs.push(Output::Command(Command::DrivePin(
        OutputPin::TransportActive,
        PinLevel::Low,
    )));
    outputs.push(Output::Command(Command::StopTickTimer));
    outputs.push(Output::Command(Command::RecordFault(
        code,
        at,
        Weight::Raised,
    )));
    outputs.push(Output::FaultRaised(code));
}
