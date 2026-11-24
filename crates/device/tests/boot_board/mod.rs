#![allow(dead_code)]

use std::ops::{Deref, DerefMut};

use device::adc::AdcAddress;
use device::analog_settings::AnalogSettings;
use device::boot::{
    self, BootRecord, ClockInput, Event, Image, Outcome, Output, Pll, PllLock, State, VolumeTargets,
};
use device::breadcrumb::{At, Breadcrumb, Weight};
use device::command::Command;
use device::dac::{Attenuation, DacWrite};
use device::fault::FaultCode;
use device::phantom::{self, Milliseconds};
use device::supervisor;

pub const LOOP_STEP: Milliseconds = Milliseconds(1);
pub const CONTROL_PERIOD: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Command(Command),
    Waited(Milliseconds),
    Log(BootRecord),
    StartLcd,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emitted {
    commands: Vec<Command>,
    started_the_lcd: bool,
}

impl Deref for Emitted {
    type Target = Vec<Command>;

    fn deref(&self) -> &Vec<Command> {
        &self.commands
    }
}

impl DerefMut for Emitted {
    fn deref_mut(&mut self) -> &mut Vec<Command> {
        &mut self.commands
    }
}

impl<const N: usize> PartialEq<[Command; N]> for Emitted {
    fn eq(&self, other: &[Command; N]) -> bool {
        self.commands == other
    }
}

impl Emitted {
    pub fn started_the_lcd(&self) -> bool {
        self.started_the_lcd
    }

    fn then(mut self, later: Emitted) -> Self {
        self.commands.extend(later.commands);
        self.started_the_lcd |= later.started_the_lcd;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bench {
    pub acks_at_13: bool,
    pub acks_at_4f: bool,
    pub dac_status: u8,
    pub targets: VolumeTargets,
    pub unanswered_transfer: Option<usize>,
    pub also_unanswered: Option<usize>,
    pub expander_silent: bool,
    pub dac_stops_at_ramp_write: Option<usize>,
    pub clock_input: ClockInput,
    pub core_pll_lock: PllLock,
    pub audio_pll_lock: PllLock,
    pub usb_phy_pll_lock: PllLock,
}

impl Bench {
    pub const ALL_ACK: Self = Self {
        acks_at_13: true,
        acks_at_4f: false,
        dac_status: 0x5A,
        targets: VolumeTargets {
            main: Attenuation::from_half_decibel_steps(0xF0),
            headphone: Attenuation::from_half_decibel_steps(0xF8),
        },
        unanswered_transfer: None,
        also_unanswered: None,
        expander_silent: false,
        dac_stops_at_ramp_write: None,
        clock_input: ClockInput::Ready,
        core_pll_lock: PllLock::Locked,
        audio_pll_lock: PllLock::Locked,
        usb_phy_pll_lock: PllLock::Locked,
    };
}

pub struct Caller {
    image: Image,
    pub boot: State,
    pub phantom: phantom::State,
    pub supervisor: supervisor::State,
    pub crumb: Breadcrumb,
    pub trace: Vec<Seen>,
    pub log: Vec<Seen>,
    pub supervised: Vec<Command>,
    pub reported: Vec<supervisor::Event>,
    pub feeds_at: Vec<u64>,
    pub now: u64,
    idle_since: u64,
}

pub fn raises(seen: Seen, code: FaultCode) -> bool {
    matches!(seen, Seen::Command(Command::RecordFault(recorded, _, Weight::Raised)) if recorded == code)
}

pub fn notes(seen: Seen, code: FaultCode) -> bool {
    matches!(seen, Seen::Command(Command::RecordFault(recorded, _, Weight::Noted)) if recorded == code)
}

pub fn is_transfer(command: Command) -> bool {
    matches!(
        command,
        Command::WriteDac(_)
            | Command::WriteAdc(_)
            | Command::ReadDac(_)
            | Command::ProbeAdc(_)
            | Command::WriteExpanderDirection
    )
}

impl Caller {
    pub fn new() -> (Self, Emitted) {
        Self::for_image(Image::Full)
    }

    pub fn for_image(image: Image) -> (Self, Emitted) {
        let mut caller = Self {
            image,
            boot: State::Degraded(Outcome::START),
            phantom: phantom::State::Disarmed,
            supervisor: supervisor::State::START,
            crumb: Breadcrumb::START,
            trace: Vec::new(),
            log: Vec::new(),
            supervised: Vec::new(),
            reported: Vec::new(),
            feeds_at: Vec::new(),
            now: 0,
            idle_since: 0,
        };
        let (state, outputs) = boot::begin();
        caller.boot = state;
        caller.crumb = caller.crumb.marked(state.place());
        let commands = caller.route(outputs.as_slice());
        (caller, commands)
    }

    pub fn feed(&mut self, event: Event) -> Emitted {
        if !self.image.runs_boot_stage(self.boot) {
            return Emitted::default();
        }
        let (state, outputs) = boot::transition(self.boot, event);
        self.boot = state;
        self.crumb = self.crumb.marked(state.place());
        if !self.image.runs_boot_stage(state) {
            return Emitted::default();
        }
        self.route(outputs.as_slice())
    }

    pub fn supervise(&mut self, event: supervisor::Event) -> Vec<Command> {
        let (state, outputs) = supervisor::transition(self.supervisor, event);
        self.supervisor = state;
        let mut commands = Vec::new();
        for output in outputs.as_slice() {
            match *output {
                supervisor::Output::Command(command) => {
                    if command == Command::FeedWatchdog {
                        self.feeds_at.push(self.now);
                    }
                    self.keep_the_record(command);
                    commands.push(command);
                }
                supervisor::Output::FaultRaised(_) => {
                    commands.extend(self.phantom_event(phantom::Event::FaultRaised));
                }
                supervisor::Output::Nothing => {}
            }
        }
        self.supervised.extend(commands.iter().copied());
        self.log
            .extend(commands.iter().map(|command| Seen::Command(*command)));
        commands
    }

    fn phantom_event(&mut self, event: phantom::Event) -> Vec<Command> {
        let (state, actions) = phantom::transition(self.phantom, event);
        self.phantom = state;
        actions
            .as_slice()
            .iter()
            .map(|action| Command::from_phantom(*action, AnalogSettings::AT_BOOT))
            .collect()
    }

    fn keep_the_record(&mut self, command: Command) {
        if let Command::RecordFault(code, at, weight) = command {
            self.crumb = self.crumb.recorded(code, at, weight);
        }
    }

    pub fn records(&self) -> Vec<(FaultCode, At, Weight)> {
        self.log
            .iter()
            .filter_map(|seen| match seen {
                Seen::Command(Command::RecordFault(code, at, weight)) => {
                    Some((*code, *at, *weight))
                }
                Seen::Command(_) | Seen::Waited(_) | Seen::Log(_) | Seen::StartLcd => None,
            })
            .collect()
    }

    fn record(&mut self, commands: &[Command]) {
        if commands.is_empty() {
            return;
        }
        if self.now > self.idle_since {
            let waited = Seen::Waited(Milliseconds((self.now - self.idle_since) as u32));
            self.trace.push(waited);
            self.log.push(waited);
        }
        for command in commands {
            self.trace.push(Seen::Command(*command));
            self.log.push(Seen::Command(*command));
        }
        self.idle_since = self.now;
    }

    fn route(&mut self, outputs: &[Output]) -> Emitted {
        let mut commands = Vec::new();
        let mut started_the_lcd = false;
        for output in outputs {
            match *output {
                Output::Command(command) => {
                    self.keep_the_record(command);
                    self.record(&[command]);
                    commands.push(command);
                }
                Output::Phantom(event) => {
                    let produced = self.phantom_event(event);
                    self.record(&produced);
                    commands.extend(produced);
                }
                Output::Supervisor(event) => {
                    self.reported.push(event);
                    self.supervise(event);
                }
                Output::Log(record) => {
                    self.trace.push(Seen::Log(record));
                    self.log.push(Seen::Log(record));
                }
                Output::StartLcd => {
                    self.trace.push(Seen::StartLcd);
                    self.log.push(Seen::StartLcd);
                    started_the_lcd = true;
                }
            }
        }
        Emitted {
            commands,
            started_the_lcd,
        }
    }

    pub fn transport_running(&self) -> bool {
        self.supervisor
            .required()
            .contains(supervisor::Fact::TickAdvanced)
    }

    pub fn usb_attached(&self) -> bool {
        self.supervisor
            .required()
            .contains(supervisor::Fact::UsbServiced)
    }

    pub fn main_loop_pass(&mut self, bench: &Bench) -> Emitted {
        self.now += u64::from(LOOP_STEP.0);
        let mut commands = self.feed(Event::Elapsed(LOOP_STEP));
        if self.now % CONTROL_PERIOD != 0 {
            return commands;
        }
        commands = commands.then(self.feed(Event::ControlPassElapsed(bench.targets)));
        self.supervise(supervisor::Event::ControlPassDone);
        if self.transport_running() {
            self.supervise(supervisor::Event::TickAdvanced);
        }
        if self.usb_attached() {
            self.supervise(supervisor::Event::UsbServiced);
        }
        self.supervise(supervisor::Event::FeedWindowEnded);
        commands
    }
}

pub struct Responder {
    bench: Bench,
    transfers: usize,
    ramp_writes: usize,
    last_answer_was_nack: Option<Command>,
    reported: Option<Command>,
}

impl Responder {
    pub fn new(bench: Bench) -> Self {
        Self {
            bench,
            transfers: 0,
            ramp_writes: 0,
            last_answer_was_nack: None,
            reported: None,
        }
    }

    fn ramp_write_stopped_answering(&mut self, command: Command) -> bool {
        if let Command::WriteDac(DacWrite::Volume(_, level)) = command
            && level != Attenuation::FULL
        {
            self.ramp_writes += 1;
        }
        self.bench
            .dac_stops_at_ramp_write
            .is_some_and(|writes| self.ramp_writes >= writes)
    }

    pub fn answer(&mut self, emitted: &Emitted) -> Option<Event> {
        let Some(&last) = emitted.last() else {
            return emitted.started_the_lcd.then_some(Event::StepDone);
        };
        if self.reported == Some(last) {
            return None;
        }
        if matches!(
            last,
            Command::ReadClockInput
                | Command::SelectCorePllAsMainClock
                | Command::SwitchPll0Family(_)
                | Command::AttachUsb
        ) {
            self.reported = Some(last);
            return Some(match last {
                Command::ReadClockInput => Event::ClockInputRead(self.bench.clock_input),
                Command::SelectCorePllAsMainClock => {
                    Event::PllLockRead(Pll::Core, self.bench.core_pll_lock)
                }
                Command::AttachUsb => Event::PllLockRead(Pll::UsbPhy, self.bench.usb_phy_pll_lock),
                _ => Event::PllLockRead(Pll::Audio, self.bench.audio_pll_lock),
            });
        }
        if !is_transfer(last) {
            return Some(Event::StepDone);
        }
        if self.last_answer_was_nack != Some(last) {
            self.transfers += 1;
        }
        let refused = self.bench.unanswered_transfer == Some(self.transfers)
            || self.bench.also_unanswered == Some(self.transfers)
            || (self.bench.expander_silent && is_expander_transfer(last))
            || self.ramp_write_stopped_answering(last);
        let event = match last {
            _ if refused => Event::I2cNacked,
            Command::ProbeAdc(AdcAddress::At13) if !self.bench.acks_at_13 => Event::I2cNacked,
            Command::ProbeAdc(AdcAddress::At4F) if !self.bench.acks_at_4f => Event::I2cNacked,
            Command::ReadDac(_) => Event::StatusRead(self.bench.dac_status),
            _ => Event::I2cAcked,
        };
        self.last_answer_was_nack = match event {
            Event::I2cNacked => Some(last),
            _ => None,
        };
        Some(event)
    }
}

fn is_expander_transfer(command: Command) -> bool {
    matches!(
        command,
        Command::WriteExpanderDirection
            | Command::ReadExpanderInputs
            | Command::WriteExpanderOutputs(_)
    )
}

pub fn clean_boot_transfer_number(wanted: impl Fn(Command) -> bool) -> usize {
    let caller = run_bench(Bench::ALL_ACK, 5_000);
    let scanned = caller
        .trace
        .iter()
        .position(|seen| *seen == Seen::Command(Command::StartKnobScan))
        .expect("the clean boot starts the knob scan");
    caller.trace[..=scanned]
        .iter()
        .filter_map(|seen| match seen {
            Seen::Command(command) if is_transfer(*command) => Some(*command),
            Seen::Command(_) | Seen::Waited(_) | Seen::Log(_) | Seen::StartLcd => None,
        })
        .position(wanted)
        .expect("the clean boot sends the transfer")
        + 1
}

pub fn first_attenuation_write() -> usize {
    clean_boot_transfer_number(|command| matches!(command, Command::WriteDac(DacWrite::Volume(..))))
}

pub fn expander_direction_write() -> usize {
    clean_boot_transfer_number(|command| command == Command::WriteExpanderDirection)
}

pub fn adc_init_write() -> usize {
    clean_boot_transfer_number(|command| matches!(command, Command::WriteAdc(_)))
}

pub fn run_bench(bench: Bench, most_passes: usize) -> Caller {
    run_image(Image::Full, bench, most_passes)
}

pub fn run_image(image: Image, bench: Bench, most_passes: usize) -> Caller {
    let (mut caller, mut emitted) = Caller::for_image(image);
    let mut responder = Responder::new(bench);
    let mut passes = 0;
    while passes < most_passes {
        if matches!(caller.boot, State::Running(..) | State::Degraded(_)) {
            break;
        }
        emitted = match responder.answer(&emitted) {
            Some(event) => caller.feed(event),
            None => {
                passes += 1;
                caller.main_loop_pass(&bench)
            }
        };
    }
    caller
}
