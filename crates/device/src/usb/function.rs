#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use stream::AlternateSetting;

use super::configuration::BusSpeed;
use super::request::{Address, HaltableEndpoint, InterfaceSelection};
use crate::outputs::{Filler, OutputList};
use crate::supervisor;

pub const MOST_OUTPUTS_PER_EVENT: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    Set,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HaltedEndpoints(u8);

impl HaltedEndpoints {
    const NONE: Self = Self(0);

    const fn bit(endpoint: HaltableEndpoint) -> u8 {
        match endpoint {
            HaltableEndpoint::SerialNotification => 0b001,
            HaltableEndpoint::SerialReceive => 0b010,
            HaltableEndpoint::SerialTransmit => 0b100,
        }
    }

    const fn halt(self, endpoint: HaltableEndpoint) -> Halt {
        if self.0 & Self::bit(endpoint) == 0 {
            Halt::Clear
        } else {
            Halt::Set
        }
    }

    const fn with(self, endpoint: HaltableEndpoint) -> Self {
        Self(self.0 | Self::bit(endpoint))
    }

    const fn without(self, endpoint: HaltableEndpoint) -> Self {
        Self(self.0 & !Self::bit(endpoint))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialFunction {
    halted: HaltedEndpoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioAndSerial {
    playback: AlternateSetting,
    capture: AlternateSetting,
    serial: SerialFunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Configuration {
    HighSpeed(AudioAndSerial),
    FullSpeed(SerialFunction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    Powered,
    Default(BusSpeed),
    Addressed(BusSpeed),
    Configured(Configuration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Detached,
    Awake(Bus),
    Suspended(Bus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Attached,
    Detached,
    BusReset(BusSpeed),
    Suspended,
    Resumed,
    AddressSet(Address),
    ConfigurationSet,
    ConfigurationCleared,
    InterfaceSelected(InterfaceSelection),
    EndpointHalted(HaltableEndpoint),
    EndpointHaltCleared(HaltableEndpoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    SelectPlayback(AlternateSetting),
    SelectCapture(AlternateSetting),
    HaltEndpoint(HaltableEndpoint),
    ClearEndpointHalt(HaltableEndpoint),
    FadeOutputsToSilence,
    ReleaseOutputSilence,
    SuspendPhantom,
    ResumePhantom,
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::ReleaseOutputSilence;
}

const ALL_HALTABLE: [HaltableEndpoint; 3] = [
    HaltableEndpoint::SerialNotification,
    HaltableEndpoint::SerialReceive,
    HaltableEndpoint::SerialTransmit,
];

impl SerialFunction {
    const FRESH: Self = Self {
        halted: HaltedEndpoints::NONE,
    };

    pub const fn halt(self, endpoint: HaltableEndpoint) -> Halt {
        self.halted.halt(endpoint)
    }

    fn clear_all(self, outputs: &mut Outputs) {
        for endpoint in ALL_HALTABLE {
            self.clear_if_halted(endpoint, outputs);
        }
    }

    fn clear_if_halted(self, endpoint: HaltableEndpoint, outputs: &mut Outputs) {
        match self.halted.halt(endpoint) {
            Halt::Set => outputs.push(Output::ClearEndpointHalt(endpoint)),
            Halt::Clear => {}
        }
    }

    fn halted(self, endpoint: HaltableEndpoint, outputs: &mut Outputs) -> Self {
        match self.halted.halt(endpoint) {
            Halt::Set => {}
            Halt::Clear => outputs.push(Output::HaltEndpoint(endpoint)),
        }
        Self {
            halted: self.halted.with(endpoint),
        }
    }

    fn halt_cleared(self, endpoint: HaltableEndpoint, outputs: &mut Outputs) -> Self {
        outputs.push(Output::ClearEndpointHalt(endpoint));
        Self {
            halted: self.halted.without(endpoint),
        }
    }

    fn reselected(self, endpoints: &[HaltableEndpoint], outputs: &mut Outputs) -> Self {
        let mut halted = self.halted;
        for &endpoint in endpoints {
            self.clear_if_halted(endpoint, outputs);
            halted = halted.without(endpoint);
        }
        Self { halted }
    }
}

impl AudioAndSerial {
    const FRESH: Self = Self {
        playback: AlternateSetting::Zero,
        capture: AlternateSetting::Zero,
        serial: SerialFunction::FRESH,
    };

    pub const fn playback(self) -> AlternateSetting {
        self.playback
    }

    pub const fn capture(self) -> AlternateSetting {
        self.capture
    }

    fn stop_streams(self, outputs: &mut Outputs) {
        if self.playback == AlternateSetting::One {
            outputs.push(Output::SelectPlayback(AlternateSetting::Zero));
        }
        if self.capture == AlternateSetting::One {
            outputs.push(Output::SelectCapture(AlternateSetting::Zero));
        }
    }

    fn restart_streams(self, outputs: &mut Outputs) {
        if self.playback == AlternateSetting::One {
            outputs.push(Output::SelectPlayback(AlternateSetting::One));
        }
        if self.capture == AlternateSetting::One {
            outputs.push(Output::SelectCapture(AlternateSetting::One));
        }
    }
}

impl Configuration {
    const fn fresh(speed: BusSpeed) -> Self {
        match speed {
            BusSpeed::High => Self::HighSpeed(AudioAndSerial::FRESH),
            BusSpeed::Full => Self::FullSpeed(SerialFunction::FRESH),
        }
    }

    pub const fn speed(self) -> BusSpeed {
        match self {
            Self::HighSpeed(_) => BusSpeed::High,
            Self::FullSpeed(_) => BusSpeed::Full,
        }
    }

    pub const fn serial(self) -> SerialFunction {
        match self {
            Self::HighSpeed(audio) => audio.serial,
            Self::FullSpeed(serial) => serial,
        }
    }

    const fn with_serial(self, serial: SerialFunction) -> Self {
        match self {
            Self::HighSpeed(audio) => Self::HighSpeed(AudioAndSerial { serial, ..audio }),
            Self::FullSpeed(_) => Self::FullSpeed(serial),
        }
    }

    fn stop_streams(self, outputs: &mut Outputs) {
        match self {
            Self::HighSpeed(audio) => audio.stop_streams(outputs),
            Self::FullSpeed(_) => {}
        }
    }

    fn restart_streams(self, outputs: &mut Outputs) {
        match self {
            Self::HighSpeed(audio) => audio.restart_streams(outputs),
            Self::FullSpeed(_) => {}
        }
    }

    fn released(self, outputs: &mut Outputs) {
        self.stop_streams(outputs);
        self.serial().clear_all(outputs);
    }

    fn select(self, selection: InterfaceSelection, outputs: &mut Outputs) -> Self {
        match (self, selection) {
            (Self::HighSpeed(audio), InterfaceSelection::Playback(playback)) => {
                if audio.playback != playback {
                    outputs.push(Output::SelectPlayback(playback));
                }
                Self::HighSpeed(AudioAndSerial { playback, ..audio })
            }
            (Self::HighSpeed(audio), InterfaceSelection::Capture(capture)) => {
                if audio.capture != capture {
                    outputs.push(Output::SelectCapture(capture));
                }
                Self::HighSpeed(AudioAndSerial { capture, ..audio })
            }
            (
                Self::FullSpeed(_),
                InterfaceSelection::Playback(_)
                | InterfaceSelection::Capture(_)
                | InterfaceSelection::AudioControl,
            )
            | (Self::HighSpeed(_), InterfaceSelection::AudioControl) => self,
            (Self::HighSpeed(_) | Self::FullSpeed(_), InterfaceSelection::SerialControl) => self
                .with_serial(
                    self.serial()
                        .reselected(&[HaltableEndpoint::SerialNotification], outputs),
                ),
            (Self::HighSpeed(_) | Self::FullSpeed(_), InterfaceSelection::SerialData) => self
                .with_serial(self.serial().reselected(
                    &[
                        HaltableEndpoint::SerialReceive,
                        HaltableEndpoint::SerialTransmit,
                    ],
                    outputs,
                )),
        }
    }
}

impl Bus {
    pub const fn speed(self) -> Option<BusSpeed> {
        match self {
            Self::Powered => None,
            Self::Default(speed) | Self::Addressed(speed) => Some(speed),
            Self::Configured(configuration) => Some(configuration.speed()),
        }
    }

    fn stop_streams(self, outputs: &mut Outputs) {
        match self {
            Self::Configured(configuration) => configuration.stop_streams(outputs),
            Self::Powered | Self::Default(_) | Self::Addressed(_) => {}
        }
    }

    fn restart_streams(self, outputs: &mut Outputs) {
        match self {
            Self::Configured(configuration) => configuration.restart_streams(outputs),
            Self::Powered | Self::Default(_) | Self::Addressed(_) => {}
        }
    }
}

impl State {
    pub const INITIAL: Self = Self::Detached;

    pub const fn active_bus(self) -> Option<Bus> {
        match self {
            Self::Awake(bus) => Some(bus),
            Self::Detached | Self::Suspended(_) => None,
        }
    }

    pub const fn supervised(self) -> supervisor::Usb {
        match self {
            Self::Awake(Bus::Configured(_)) => supervisor::Usb::Serving,
            Self::Awake(Bus::Powered | Bus::Default(_) | Bus::Addressed(_))
            | Self::Detached
            | Self::Suspended(_) => supervisor::Usb::Silent,
        }
    }
}

fn leave_suspend(outputs: &mut Outputs) {
    outputs.push(Output::ReleaseOutputSilence);
    outputs.push(Output::ResumePhantom);
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    let mut outputs = Outputs::NONE;
    let next = match state {
        State::Detached => match event {
            Event::Attached => State::Awake(Bus::Powered),
            Event::Detached
            | Event::BusReset(_)
            | Event::Suspended
            | Event::Resumed
            | Event::AddressSet(_)
            | Event::ConfigurationSet
            | Event::ConfigurationCleared
            | Event::InterfaceSelected(_)
            | Event::EndpointHalted(_)
            | Event::EndpointHaltCleared(_) => state,
        },
        State::Suspended(bus) => match event {
            Event::Resumed => {
                leave_suspend(&mut outputs);
                bus.restart_streams(&mut outputs);
                State::Awake(bus)
            }
            Event::BusReset(speed) => {
                leave_suspend(&mut outputs);
                State::Awake(Bus::Default(speed))
            }
            Event::Detached => {
                leave_suspend(&mut outputs);
                State::Detached
            }
            Event::Attached
            | Event::Suspended
            | Event::AddressSet(_)
            | Event::ConfigurationSet
            | Event::ConfigurationCleared
            | Event::InterfaceSelected(_)
            | Event::EndpointHalted(_)
            | Event::EndpointHaltCleared(_) => state,
        },
        State::Awake(bus) => match event {
            Event::Detached => {
                bus.stop_streams(&mut outputs);
                State::Detached
            }
            Event::BusReset(speed) => {
                bus.stop_streams(&mut outputs);
                State::Awake(Bus::Default(speed))
            }
            Event::Suspended => {
                outputs.push(Output::FadeOutputsToSilence);
                bus.stop_streams(&mut outputs);
                outputs.push(Output::SuspendPhantom);
                State::Suspended(bus)
            }
            Event::Attached | Event::Resumed => state,
            Event::AddressSet(_)
            | Event::ConfigurationSet
            | Event::ConfigurationCleared
            | Event::InterfaceSelected(_)
            | Event::EndpointHalted(_)
            | Event::EndpointHaltCleared(_) => State::Awake(awake(bus, event, &mut outputs)),
        },
    };
    (next, outputs)
}

fn awake(bus: Bus, event: Event, outputs: &mut Outputs) -> Bus {
    match bus {
        Bus::Powered => bus,
        Bus::Default(speed) | Bus::Addressed(speed) => match event {
            Event::AddressSet(address) => match address {
                Address::Default => Bus::Default(speed),
                Address::Assigned(_) => Bus::Addressed(speed),
            },
            Event::ConfigurationSet => match bus {
                Bus::Addressed(_) => Bus::Configured(Configuration::fresh(speed)),
                Bus::Powered | Bus::Default(_) | Bus::Configured(_) => bus,
            },
            Event::Attached
            | Event::Detached
            | Event::BusReset(_)
            | Event::Suspended
            | Event::Resumed
            | Event::ConfigurationCleared
            | Event::InterfaceSelected(_)
            | Event::EndpointHalted(_)
            | Event::EndpointHaltCleared(_) => bus,
        },
        Bus::Configured(configuration) => match event {
            Event::ConfigurationSet => {
                configuration.released(outputs);
                Bus::Configured(Configuration::fresh(configuration.speed()))
            }
            Event::ConfigurationCleared => {
                configuration.released(outputs);
                Bus::Addressed(configuration.speed())
            }
            Event::InterfaceSelected(selection) => {
                Bus::Configured(configuration.select(selection, outputs))
            }
            Event::EndpointHalted(endpoint) => Bus::Configured(
                configuration.with_serial(configuration.serial().halted(endpoint, outputs)),
            ),
            Event::EndpointHaltCleared(endpoint) => Bus::Configured(
                configuration.with_serial(configuration.serial().halt_cleared(endpoint, outputs)),
            ),
            Event::Attached
            | Event::Detached
            | Event::BusReset(_)
            | Event::Suspended
            | Event::Resumed
            | Event::AddressSet(_) => bus,
        },
    }
}
