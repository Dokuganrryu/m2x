use device::boot::{ClockInput, Pll};
use device::command::Command;
use device::console::{Asked, BusHealth};
use device::fault::FaultCode;
use device::i2c::{self, Answer, Transfer};
use device::phantom::Milliseconds;
use device::pll::Readiness;

use dsp::InputMute;

use super::audio_dma;
use super::clock_gates::{self, BusClock};
use super::clocks;
use super::converter_bus;
use super::elapsed::Millisecond;
use super::fault;
use super::fault_policy::{FAULT_POLICY, FaultPolicy};
use super::handovers;
use super::i2s;
use super::knob_adc;
use super::memory_fence;
use super::nvic;
use super::pins;
use super::polled_bus;
use super::tick_timer;
use super::transport;
use super::usb_connection;
use super::usb_controller;
use super::watchdog::Watchdog;

const QUEUED_EVENTS: usize = 8;
// WHY: Every register-touching command passes Executor::run or the fatal path, so filtering here holds P0_28 and P1_4 low, the DAC volume at 0xFF and the phantom bits clear whatever the boot, rate, control or phantom machines decide
const SILENT: bool = cfg!(feature = "milestone7") && !cfg!(feature = "sound1");
const PHANTOM_HELD: bool = cfg!(feature = "sound1");

pub const fn as_built(command: Command) -> Command {
    if PHANTOM_HELD {
        device::silent::phantom_held(command)
    } else if SILENT {
        device::silent::silenced(command)
    } else {
        command
    }
}

#[derive(Clone, Copy)]
pub enum BoardEvent {
    StepDone,
    Bus(Answer),
    Elapsed(Milliseconds),
    CommandRefused,
    PanelRefused,
    ClockInput(ClockInput),
    PllLock(Pll, Readiness),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Done,
    AwaitingBus,
    Refused,
    Reported,
}

struct EventQueue {
    items: [Option<BoardEvent>; QUEUED_EVENTS],
    first: usize,
    len: usize,
}

impl EventQueue {
    const fn new() -> Self {
        Self {
            items: [None; QUEUED_EVENTS],
            first: 0,
            len: 0,
        }
    }

    fn push(&mut self, event: BoardEvent) {
        if self.len == QUEUED_EVENTS {
            fault::enter(FaultCode::BoardStalled);
        }
        self.items[(self.first + self.len) % QUEUED_EVENTS] = Some(event);
        self.len += 1;
    }

    fn pop(&mut self) -> Option<BoardEvent> {
        if self.len == 0 {
            return None;
        }
        let event = self.items[self.first].take();
        self.first = (self.first + 1) % QUEUED_EVENTS;
        self.len -= 1;
        event
    }
}

pub struct Executor {
    watchdog: Watchdog,
    millisecond: Millisecond,
    bus: i2c::State,
    peer: u8,
    asked: Asked,
    health: BusHealth,
    events: EventQueue,
}

impl Executor {
    pub fn claim() -> Option<Self> {
        Some(Self {
            watchdog: Watchdog::claim()?,
            millisecond: Millisecond::start(),
            bus: i2c::State::Idle,
            peer: 0,
            asked: Asked::Transfer,
            health: BusHealth::START,
            events: EventQueue::new(),
        })
    }

    pub fn run(&mut self, command: Command) -> Completion {
        let command = as_built(command);
        match command {
            Command::DrivePin(pin, level) => {
                clock_gates::enable(&[BusClock::Iocon, BusClock::Gpio0, BusClock::Gpio1]);
                pins::select_factory_gpio_pad(pin);
                pins::drive(pin.pin(), level);
            }
            Command::SendAnalogWord(word) => {
                if polled_bus::send_analog_word_and_wait_until_latched(word) == Readiness::TimedOut
                {
                    return Completion::Refused;
                }
            }
            Command::WriteDac(_)
            | Command::ReadDac(_)
            | Command::ProbeAdc(_)
            | Command::WriteAdc(_)
            | Command::WriteExpanderDirection
            | Command::ReadExpanderInputs
            | Command::WriteExpanderOutputs(_) => return self.begin_transfer(command),
            Command::SwitchPll0Family(family) => {
                let readiness = clocks::switch_pll0_family(family, &mut self.watchdog);
                self.push(BoardEvent::PllLock(Pll::Audio, readiness));
            }
            Command::StopI2s => transport::stop_i2s(),
            Command::StopDma => transport::stop_dma(),
            Command::StopTickTimer => transport::stop_tick_timer(),
            Command::DisableInterrupts => nvic::disable_and_clear_every_device_interrupt(),
            Command::DetachUsb => usb_connection::detach(),
            Command::FeedWatchdog => match FAULT_POLICY {
                FaultPolicy::BringUp => {}
                FaultPolicy::Release => self.watchdog.feed(),
            },
            Command::ArmWatchdog => match FAULT_POLICY {
                FaultPolicy::BringUp => {}
                FaultPolicy::Release => self.watchdog.arm(),
            },
            Command::FenceFlashController => match memory_fence::raise() {
                Ok(()) => {}
                Err(_) => fault::enter(FaultCode::FenceUnavailable),
            },
            Command::ConfigureAnalogWordBus => polled_bus::configure_analog_word_bus(),
            Command::SelectCorePllAsMainClock => {
                let readiness = clocks::select_core_pll_as_main_clock(&mut self.watchdog);
                self.push(BoardEvent::PllLock(Pll::Core, readiness));
                return Completion::Reported;
            }
            Command::EnableMasterClockOutput => clocks::enable_master_clock_output(),
            Command::ConfigureConverterBus => {
                converter_bus::configure();
                self.bus = i2c::State::Idle;
            }
            Command::StopTransmitDma | Command::LatchPlaybackSilence => {
                transport::stop_transmit_dma();
            }
            Command::RecordFault(code, at, weight) => fault::record(code, at, weight),
            Command::StartClockInput => clocks::start_clock_input(),
            Command::ReadClockInput => {
                let input = clocks::clock_input(&mut self.watchdog);
                self.push(BoardEvent::ClockInput(input));
                return Completion::Reported;
            }
            Command::StartCorePll => clocks::start_core_pll(),
            Command::StartDma => audio_dma::start_controller(),
            Command::StartI2s(frames) => i2s::start(frames),
            Command::StartTickTimer(frames) => tick_timer::start(frames),
            Command::ZeroTransmitBuffers => audio_dma::zero_transmit_buffers(),
            Command::MuteInput(input) => handovers::set_input(input, InputMute::Muted),
            Command::UnmuteInput(input) => handovers::set_input(input, InputMute::Live),
            Command::AttachUsb => {
                let readiness =
                    usb_controller::attach(|| clocks::usb_phy_pll_lock(&mut self.watchdog));
                // WHY: A hold counts one millisecond more than its length because the first can come at once; one that elapsed during the lock wait would take that margin and cut the first rail stagger to 4.957 ms (board-run scenario usb-phy-pll-lock-never-rises)
                let _elapsed_during_the_wait = self.millisecond.passed();
                self.push(BoardEvent::PllLock(Pll::UsbPhy, readiness));
                return Completion::Reported;
            }
            Command::StartKnobScan => knob_adc::start_with_offset_calibration(),
        }
        Completion::Done
    }

    fn begin_transfer(&mut self, command: Command) -> Completion {
        if !converter_bus::configured() {
            return Completion::Refused;
        }
        match Transfer::of(command) {
            Some(transfer) => {
                self.peer = transfer.peer_address();
                self.asked = match transfer {
                    Transfer::Probe(_) => Asked::Probe,
                    Transfer::Writes(_) | Transfer::Read(_) => Asked::Transfer,
                };
                self.step_bus(i2c::Event::Begin(transfer));
                Completion::AwaitingBus
            }
            None => Completion::Refused,
        }
    }

    fn step_bus(&mut self, event: i2c::Event) {
        let (next, outputs) = i2c::transition(self.bus, event);
        self.bus = next;
        for output in outputs.as_slice() {
            match *output {
                i2c::Output::Act(action) => converter_bus::act(action),
                i2c::Output::Answered(answer) => {
                    self.health = self.health.after(self.peer, self.asked, answer);
                    self.events.push(BoardEvent::Bus(answer));
                }
            }
        }
    }

    pub fn poll(&mut self) {
        if let Some(elapsed) = self.millisecond.passed() {
            match FAULT_POLICY {
                FaultPolicy::BringUp => self.watchdog.feed(),
                FaultPolicy::Release => {}
            }
            self.events.push(BoardEvent::Elapsed(elapsed));
        }
        match self.bus {
            i2c::State::Idle => {}
            i2c::State::Writing(_) | i2c::State::Reading(_) | i2c::State::Probing(_) => {
                self.step_bus(i2c::Event::Polled(converter_bus::status()));
            }
        }
    }

    pub const fn bus_health(&self) -> BusHealth {
        self.health
    }

    pub fn push(&mut self, event: BoardEvent) {
        self.events.push(event);
    }

    pub fn next_event(&mut self) -> Option<BoardEvent> {
        self.events.pop()
    }
}
