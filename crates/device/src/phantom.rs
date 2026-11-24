use crate::input::Input;
use crate::outputs::{Filler, OutputList};
use crate::steps::{Countdown, count_down};

pub const SHORTEST_COUNTED_PRESS: Milliseconds = Milliseconds(30);
pub const LONGEST_COUNTED_PRESS: Milliseconds = Milliseconds(1000);
pub const MUTE_AFTER_PHANTOM_CHANGE: Milliseconds = Milliseconds(1000);
pub const MOST_OUTPUTS_PER_EVENT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Milliseconds(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhantomPower {
    Off,
    On,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhantomGrant {
    one: PhantomPower,
    two: PhantomPower,
}

impl PhantomGrant {
    pub const NONE: Self = Self {
        one: PhantomPower::Off,
        two: PhantomPower::Off,
    };

    pub const fn power(self, input: Input) -> PhantomPower {
        match input {
            Input::One => self.one,
            Input::Two => self.two,
        }
    }

    const fn active_pin_level(self) -> PinLevel {
        match (self.one, self.two) {
            (PhantomPower::Off, PhantomPower::Off) => PinLevel::Low,
            (PhantomPower::On, PhantomPower::Off)
            | (PhantomPower::Off, PhantomPower::On)
            | (PhantomPower::On, PhantomPower::On) => PinLevel::High,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinLevel {
    Low,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    MuteInput(Input),
    UnmuteInput(Input),
    SendAnalogWord(PhantomGrant),
    DrivePhantomActivePin(PinLevel),
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::SendAnalogWord(PhantomGrant::NONE);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Boot,
    BootReachedRunning,
    ButtonPressed(Input),
    ButtonReleased(Input),
    JackInserted(Input),
    JackRemoved(Input),
    Elapsed(Milliseconds),
    Suspend,
    Resume,
    FaultRaised,
}

impl Event {
    pub const fn from_jack_pin_byte(input: Input, pin_byte: u8) -> Self {
        match pin_byte {
            0 => Event::JackRemoved(input),
            1..=u8::MAX => Event::JackInserted(input),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Jack {
    Inserted,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Released,
    Held { duration: Milliseconds },
    HeldTooLong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settle {
    Settled,
    Muted { remaining: Milliseconds },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Idle {
    jack: Jack,
    settle: Settle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Unpowered {
        jack: Jack,
        button: Button,
        settle: Settle,
    },
    Powered {
        button: Button,
        settle: Settle,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair<T> {
    one: T,
    two: T,
}

impl<T: Copy> Pair<T> {
    const fn both(value: T) -> Self {
        Self {
            one: value,
            two: value,
        }
    }

    const fn get(self, input: Input) -> T {
        match input {
            Input::One => self.one,
            Input::Two => self.two,
        }
    }

    fn with(self, input: Input, value: T) -> Self {
        match input {
            Input::One => Self { one: value, ..self },
            Input::Two => Self { two: value, ..self },
        }
    }

    fn map<U>(self, mut f: impl FnMut(T) -> U) -> Pair<U> {
        Pair {
            one: f(self.one),
            two: f(self.two),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Disarmed,
    Booting(Pair<Idle>),
    Running(Pair<Channel>),
    Suspended(Pair<Idle>),
}

impl State {
    pub const fn grant(self) -> PhantomGrant {
        match self {
            State::Disarmed | State::Booting(_) | State::Suspended(_) => PhantomGrant::NONE,
            State::Running(channels) => PhantomGrant {
                one: channels.one.power(),
                two: channels.two.power(),
            },
        }
    }

    const fn settle(self, input: Input) -> Option<Settle> {
        match self {
            State::Disarmed => None,
            State::Booting(idle) | State::Suspended(idle) => Some(idle.get(input).settle),
            State::Running(channels) => Some(channels.get(input).settle()),
        }
    }
}

impl Channel {
    const fn power(self) -> PhantomPower {
        match self {
            Channel::Unpowered { .. } => PhantomPower::Off,
            Channel::Powered { .. } => PhantomPower::On,
        }
    }

    const fn from_idle(idle: Idle) -> Self {
        Channel::Unpowered {
            jack: idle.jack,
            button: Button::Released,
            settle: idle.settle,
        }
    }

    const fn into_idle(self) -> Idle {
        match self {
            Channel::Unpowered { jack, settle, .. } => Idle { jack, settle },
            Channel::Powered { .. } => Idle {
                jack: Jack::Removed,
                settle: Settle::muted(),
            },
        }
    }

    fn press(self) -> Self {
        match self {
            Channel::Unpowered {
                jack,
                button,
                settle,
            } => Channel::Unpowered {
                jack,
                button: button.press(),
                settle,
            },
            Channel::Powered { button, settle } => Channel::Powered {
                button: button.press(),
                settle,
            },
        }
    }

    fn release(self) -> Self {
        match self {
            Channel::Unpowered {
                jack: Jack::Removed,
                button,
                settle,
            } => match button.release() {
                Release::Counted => Channel::Powered {
                    button: Button::Released,
                    settle: Settle::muted(),
                },
                Release::Ignored => Channel::Unpowered {
                    jack: Jack::Removed,
                    button: Button::Released,
                    settle,
                },
            },
            Channel::Unpowered {
                jack: Jack::Inserted,
                settle,
                ..
            } => Channel::Unpowered {
                jack: Jack::Inserted,
                button: Button::Released,
                settle,
            },
            Channel::Powered { button, settle } => match button.release() {
                Release::Counted => Channel::Unpowered {
                    jack: Jack::Removed,
                    button: Button::Released,
                    settle: Settle::muted(),
                },
                Release::Ignored => Channel::Powered {
                    button: Button::Released,
                    settle,
                },
            },
        }
    }

    const fn jack(self, jack: Jack) -> Self {
        match (self, jack) {
            (
                Channel::Unpowered {
                    jack: Jack::Inserted,
                    button,
                    settle,
                },
                Jack::Inserted,
            )
            | (
                Channel::Unpowered {
                    jack: Jack::Removed,
                    button,
                    settle,
                },
                Jack::Removed,
            ) => Channel::Unpowered {
                jack,
                button,
                settle,
            },
            (
                Channel::Unpowered {
                    jack: Jack::Inserted,
                    button,
                    settle,
                },
                Jack::Removed,
            )
            | (
                Channel::Unpowered {
                    jack: Jack::Removed,
                    button,
                    settle,
                },
                Jack::Inserted,
            ) => Channel::Unpowered {
                jack,
                button: button.cancel(),
                settle,
            },
            (Channel::Powered { button, .. }, Jack::Inserted) => Channel::Unpowered {
                jack: Jack::Inserted,
                button: button.cancel(),
                settle: Settle::muted(),
            },
            (Channel::Powered { button, settle }, Jack::Removed) => {
                Channel::Powered { button, settle }
            }
        }
    }

    const fn elapse(self, elapsed: Milliseconds) -> Self {
        match self {
            Channel::Unpowered {
                jack,
                button,
                settle,
            } => Channel::Unpowered {
                jack,
                button: button.elapse(elapsed),
                settle: settle.elapse(elapsed),
            },
            Channel::Powered { button, settle } => Channel::Powered {
                button: button.elapse(elapsed),
                settle: settle.elapse(elapsed),
            },
        }
    }

    const fn settle(self) -> Settle {
        match self {
            Channel::Unpowered { settle, .. } | Channel::Powered { settle, .. } => settle,
        }
    }
}

pub(crate) enum Release {
    Counted,
    Ignored,
}

impl Button {
    pub(crate) const fn press(self) -> Self {
        match self {
            Button::Released => Button::Held {
                duration: Milliseconds(0),
            },
            Button::Held { duration } => Button::Held { duration },
            Button::HeldTooLong => Button::HeldTooLong,
        }
    }

    pub(crate) const fn release(self) -> Release {
        match self {
            Button::Held { duration } if duration.0 >= SHORTEST_COUNTED_PRESS.0 => Release::Counted,
            Button::Held { .. } | Button::Released | Button::HeldTooLong => Release::Ignored,
        }
    }

    pub(crate) const fn cancel(self) -> Self {
        match self {
            Button::Released => Button::Released,
            Button::Held { .. } | Button::HeldTooLong => Button::HeldTooLong,
        }
    }

    pub(crate) const fn elapse(self, elapsed: Milliseconds) -> Self {
        match self {
            Button::Released => Button::Released,
            Button::HeldTooLong => Button::HeldTooLong,
            Button::Held { duration } => {
                let total = duration.0.saturating_add(elapsed.0);
                if total > LONGEST_COUNTED_PRESS.0 {
                    Button::HeldTooLong
                } else {
                    Button::Held {
                        duration: Milliseconds(total),
                    }
                }
            }
        }
    }
}

impl Settle {
    const fn muted() -> Self {
        Settle::Muted {
            remaining: MUTE_AFTER_PHANTOM_CHANGE,
        }
    }

    const fn elapse(self, elapsed: Milliseconds) -> Self {
        match self {
            Settle::Settled => Settle::Settled,
            Settle::Muted { remaining } => match count_down(remaining, elapsed) {
                Countdown::Over => Settle::Settled,
                Countdown::Left(remaining) => Settle::Muted { remaining },
            },
        }
    }
}

impl Pair<Idle> {
    fn jack(self, input: Input, jack: Jack) -> Self {
        self.with(
            input,
            Idle {
                jack,
                ..self.get(input)
            },
        )
    }
}

impl Idle {
    const fn elapse(self, elapsed: Milliseconds) -> Self {
        Idle {
            settle: self.settle.elapse(elapsed),
            ..self
        }
    }
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    let next = next_state(state, event);
    (next, outputs_between(state, next, event))
}

fn next_state(state: State, event: Event) -> State {
    match state {
        State::Disarmed => match event {
            Event::Boot => booted(),
            Event::BootReachedRunning
            | Event::ButtonPressed(_)
            | Event::ButtonReleased(_)
            | Event::JackInserted(_)
            | Event::JackRemoved(_)
            | Event::Elapsed(_)
            | Event::Suspend
            | Event::Resume
            | Event::FaultRaised => State::Disarmed,
        },
        State::Booting(idle) => match event {
            Event::Boot => booted(),
            Event::BootReachedRunning => State::Running(idle.map(Channel::from_idle)),
            Event::JackInserted(input) => State::Booting(idle.jack(input, Jack::Inserted)),
            Event::JackRemoved(input) => State::Booting(idle.jack(input, Jack::Removed)),
            Event::Elapsed(elapsed) => State::Booting(idle.map(|i| i.elapse(elapsed))),
            Event::FaultRaised => State::Disarmed,
            Event::ButtonPressed(_) | Event::ButtonReleased(_) | Event::Suspend | Event::Resume => {
                State::Booting(idle)
            }
        },
        State::Running(channels) => match event {
            Event::Boot => booted(),
            Event::ButtonPressed(input) => {
                State::Running(channels.with(input, channels.get(input).press()))
            }
            Event::ButtonReleased(input) => {
                State::Running(channels.with(input, channels.get(input).release()))
            }
            Event::JackInserted(input) => {
                State::Running(channels.with(input, channels.get(input).jack(Jack::Inserted)))
            }
            Event::JackRemoved(input) => {
                State::Running(channels.with(input, channels.get(input).jack(Jack::Removed)))
            }
            Event::Elapsed(elapsed) => State::Running(channels.map(|c| c.elapse(elapsed))),
            Event::Suspend => State::Suspended(channels.map(Channel::into_idle)),
            Event::FaultRaised => State::Disarmed,
            Event::BootReachedRunning | Event::Resume => State::Running(channels),
        },
        State::Suspended(idle) => match event {
            Event::Boot => booted(),
            Event::Resume => State::Running(idle.map(Channel::from_idle)),
            Event::JackInserted(input) => State::Suspended(idle.jack(input, Jack::Inserted)),
            Event::JackRemoved(input) => State::Suspended(idle.jack(input, Jack::Removed)),
            Event::Elapsed(elapsed) => State::Suspended(idle.map(|i| i.elapse(elapsed))),
            Event::FaultRaised => State::Disarmed,
            Event::ButtonPressed(_)
            | Event::ButtonReleased(_)
            | Event::BootReachedRunning
            | Event::Suspend => State::Suspended(idle),
        },
    }
}

const fn booted() -> State {
    State::Booting(Pair::both(Idle {
        jack: Jack::Inserted,
        settle: Settle::Settled,
    }))
}

fn outputs_between(before: State, after: State, event: Event) -> Outputs {
    let mut outputs = Outputs::NONE;
    let (before_grant, after_grant) = (before.grant(), after.grant());
    let changed = before_grant != after_grant;
    let (mutes_due, word_due) = match event {
        Event::Boot => (false, true),
        Event::FaultRaised => (changed, before != State::Disarmed),
        Event::BootReachedRunning
        | Event::ButtonPressed(_)
        | Event::ButtonReleased(_)
        | Event::JackInserted(_)
        | Event::JackRemoved(_)
        | Event::Elapsed(_)
        | Event::Suspend
        | Event::Resume => (changed, changed),
    };
    if mutes_due {
        for input in Input::BOTH {
            if before_grant.power(input) != after_grant.power(input) {
                outputs.push(Output::MuteInput(input));
            }
        }
    }
    if word_due {
        outputs.push(Output::SendAnalogWord(after_grant));
        outputs.push(Output::DrivePhantomActivePin(
            after_grant.active_pin_level(),
        ));
    }
    if let Event::Elapsed(_) = event {
        for input in Input::BOTH {
            if let (Some(Settle::Muted { .. }), Some(Settle::Settled)) =
                (before.settle(input), after.settle(input))
            {
                outputs.push(Output::UnmuteInput(input));
            }
        }
    }
    outputs
}
