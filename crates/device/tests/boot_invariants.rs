use device::boot::{
    ClockInput, Event, Outcome, Output, Pll, PllLock, Stage, StageClass, State, VolumeTargets,
    begin, transition,
};
use device::breadcrumb::{At, Weight};
use device::command::Command;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::output_pin::OutputPin;
use device::phantom::{self, Milliseconds, PinLevel};
use device::pll::{CORE_PLL_STABILISATION, audio_pll_stabilisation};
use device::supervisor;

const SEQUENCES: u64 = 10_000;
const CLEAN_SEQUENCES: u64 = 2_000;
const CLEAN_EVENTS_PER_SEQUENCE: usize = 12_000;
const EVENTS_PER_SEQUENCE: usize = 3_000;

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn attenuation(&mut self) -> Attenuation {
        Attenuation::from_half_decibel_steps(match self.below(3) {
            0 => 0xFF,
            1 => 0xF0 + self.below(16) as u8,
            _ => self.below(256) as u8,
        })
    }

    fn targets(&mut self) -> VolumeTargets {
        VolumeTargets {
            main: self.attenuation(),
            headphone: self.attenuation(),
        }
    }

    fn lock(&mut self) -> PllLock {
        if self.below(4) == 0 {
            PllLock::TimedOut
        } else {
            PllLock::Locked
        }
    }

    fn any_event(&mut self) -> Event {
        match self.below(9) {
            0 => Event::StepDone,
            6 => Event::ClockInputRead(if self.below(2) == 0 {
                ClockInput::Ready
            } else {
                ClockInput::NotReady
            }),
            7 => Event::PllLockRead(
                match self.below(3) {
                    0 => Pll::Core,
                    1 => Pll::Audio,
                    _ => Pll::UsbPhy,
                },
                self.lock(),
            ),
            8 => Event::CommandRefused,
            1 => Event::I2cAcked,
            2 => Event::I2cNacked,
            3 => Event::StatusRead(self.below(256) as u8),
            4 => Event::Elapsed(Milliseconds(self.below(120) as u32)),
            _ => Event::ControlPassElapsed(self.targets()),
        }
    }
}

struct Board {
    fault_percent: u64,
    nack_percent: u64,
    lock_percent: u64,
    targets: VolumeTargets,
}

impl Board {
    fn next_event(&mut self, rng: &mut XorShift, last: &[Output]) -> Event {
        if rng.below(100) < self.fault_percent {
            return rng.any_event();
        }
        match last.last() {
            Some(Output::Command(Command::ReadClockInput)) => {
                if rng.below(100) < self.nack_percent {
                    Event::ClockInputRead(ClockInput::NotReady)
                } else {
                    Event::ClockInputRead(ClockInput::Ready)
                }
            }
            Some(Output::Command(Command::SelectCorePllAsMainClock)) => Event::PllLockRead(
                Pll::Core,
                if rng.below(100) < self.lock_percent {
                    PllLock::TimedOut
                } else {
                    PllLock::Locked
                },
            ),
            Some(Output::Command(Command::SwitchPll0Family(_))) => Event::PllLockRead(
                Pll::Audio,
                if rng.below(100) < self.lock_percent {
                    PllLock::TimedOut
                } else {
                    PllLock::Locked
                },
            ),
            Some(Output::Command(Command::AttachUsb)) => Event::PllLockRead(
                Pll::UsbPhy,
                if rng.below(100) < self.lock_percent {
                    PllLock::TimedOut
                } else {
                    PllLock::Locked
                },
            ),
            Some(Output::Command(command)) if is_transfer(*command) => {
                if rng.below(100) < self.nack_percent {
                    Event::I2cNacked
                } else {
                    match command {
                        Command::ReadDac(_) => Event::StatusRead(rng.below(256) as u8),
                        Command::ProbeAdc(_) if rng.below(2) == 0 => Event::I2cNacked,
                        _ => Event::I2cAcked,
                    }
                }
            }
            Some(_) => Event::StepDone,
            None => {
                if rng.below(20) == 0 {
                    self.targets = rng.targets();
                }
                match rng.below(3) {
                    0 | 1 => Event::ControlPassElapsed(self.targets),
                    _ => Event::Elapsed(Milliseconds(1 + rng.below(5) as u32)),
                }
            }
        }
    }
}

fn is_transfer(command: Command) -> bool {
    matches!(
        command,
        Command::WriteDac(_)
            | Command::WriteAdc(_)
            | Command::ReadDac(_)
            | Command::ProbeAdc(_)
            | Command::WriteExpanderDirection
    )
}

fn touches_the_analog_chain(command: Command) -> bool {
    matches!(
        command,
        Command::SendAnalogWord(_)
            | Command::ConfigureConverterBus
            | Command::WriteDac(_)
            | Command::ReadDac(_)
            | Command::WriteAdc(_)
            | Command::ProbeAdc(_)
            | Command::DrivePin(OutputPin::ConverterEnable, _)
            | Command::DrivePin(OutputPin::AdcEnable, _)
            | Command::DrivePin(OutputPin::FirstBringUp, PinLevel::High)
            | Command::DrivePin(OutputPin::SecondBringUp, PinLevel::High)
            | Command::DrivePin(OutputPin::ThirdBringUp, PinLevel::High)
            | Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
    )
}

fn speaks_on_the_converter_bus(command: Command) -> bool {
    matches!(
        command,
        Command::WriteDac(_)
            | Command::ReadDac(_)
            | Command::WriteAdc(_)
            | Command::ProbeAdc(_)
            | Command::WriteExpanderDirection
            | Command::ReadExpanderInputs
            | Command::WriteExpanderOutputs(_)
    )
}

fn is_a_start(command: Command) -> bool {
    matches!(
        command,
        Command::StartDma
            | Command::StartI2s(_)
            | Command::StartTickTimer(_)
            | Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
    )
}

fn outcome(state: State) -> Outcome {
    match state {
        State::Scripted(_, _, outcome)
        | State::VolumeRamp(_, _, outcome)
        | State::Running(_, outcome)
        | State::Degraded(outcome) => outcome,
    }
}

fn ramped_index(register: VolumeRegister) -> Option<usize> {
    device::boot::RAMPED_REGISTERS
        .iter()
        .position(|r| *r == register)
}

#[derive(Clone, Copy)]
enum PllWait {
    None,
    Unconfirmed {
        needed: Milliseconds,
        follower: Command,
    },
    Counting {
        needed: Milliseconds,
        counted: u32,
        follower: Command,
        splits: u32,
    },
}

fn volume_index(register: VolumeRegister) -> usize {
    VolumeRegister::ALL
        .iter()
        .position(|r| *r == register)
        .expect("every volume register is listed")
}

struct Observer {
    clock_running: bool,
    converter_bus_configured: bool,
    converter_bus_configurations: u32,
    refused: bool,
    fault_raised: bool,
    awaiting_volume: Option<VolumeRegister>,
    attenuation_acknowledged: [bool; 8],
    outputs_connected: u32,
    pll_wait: PllWait,
    pll_waits_split: u32,
    analog_chain_reset: bool,
    rails_raised: bool,
    level: [u8; 6],
    level_at_pass_start: [u8; 6],
    raised_volume: bool,
    lcd_started: bool,
    usb_attached: bool,
    faults_recorded: u32,
}

impl Observer {
    fn watch_pll(&mut self, seed: u64, command: Command) {
        let started = match command {
            Command::StartCorePll => {
                Some((CORE_PLL_STABILISATION, Command::SelectCorePllAsMainClock))
            }
            Command::SwitchPll0Family(family) => Some((
                audio_pll_stabilisation(family),
                Command::EnableMasterClockOutput,
            )),
            _ => None,
        };
        if let Some((needed, follower)) = started {
            self.pll_wait = PllWait::Unconfirmed { needed, follower };
            return;
        }
        match self.pll_wait {
            PllWait::Unconfirmed { follower, .. } => {
                assert_ne!(
                    command, follower,
                    "seed {seed}: no wait after the PLL start, saw {command:?}"
                );
            }
            PllWait::Counting {
                needed,
                counted,
                follower,
                splits,
            } if command == follower => {
                assert!(
                    counted >= needed.0,
                    "seed {seed}: {follower:?} after {counted} ms of a {} ms PLL wait",
                    needed.0
                );
                if splits > 1 {
                    self.pll_waits_split += 1;
                }
                self.pll_wait = PllWait::None;
            }
            PllWait::Counting { .. } | PllWait::None => {}
        }
    }

    fn new() -> Self {
        Self {
            clock_running: false,
            converter_bus_configured: false,
            converter_bus_configurations: 0,
            refused: false,
            fault_raised: false,
            awaiting_volume: None,
            attenuation_acknowledged: [false; 8],
            outputs_connected: 0,
            pll_wait: PllWait::None,
            pll_waits_split: 0,
            analog_chain_reset: false,
            rails_raised: false,
            level: [0xFF; 6],
            level_at_pass_start: [0xFF; 6],
            raised_volume: false,
            lcd_started: false,
            usb_attached: false,
            faults_recorded: 0,
        }
    }

    fn watch_output_connection(&mut self, seed: u64, command: Command) {
        match command {
            Command::EnableMasterClockOutput => self.clock_running = true,
            Command::DrivePin(OutputPin::ConverterEnable, _) => {
                self.attenuation_acknowledged = [false; 8];
            }
            Command::WriteDac(DacWrite::Volume(register, level)) if level != Attenuation::FULL => {
                self.attenuation_acknowledged[volume_index(register)] = false;
            }
            Command::DrivePin(OutputPin::TransportActive, PinLevel::High) => {
                assert!(
                    self.attenuation_acknowledged.iter().all(|acked| *acked),
                    "seed {seed}: outputs connected before every DAC volume register acknowledged full attenuation with the master clock running, {:?}",
                    self.attenuation_acknowledged
                );
                assert!(
                    !self.fault_raised,
                    "seed {seed}: outputs connected on a path whose audio was abandoned"
                );
                self.outputs_connected += 1;
            }
            Command::DrivePin(OutputPin::TransportActive, PinLevel::Low) => {}
            _ => {}
        }
    }

    fn check(&mut self, seed: u64, before: State, event: Event, after: State, outputs: &[Output]) {
        if outcome(after).runs(device::boot::Subsystem::Rails) {
            self.rails_raised = true;
        }
        if let Event::CommandRefused = event {
            self.refused = true;
        }
        if let (Some(register), Event::I2cAcked, true) =
            (self.awaiting_volume, event, self.clock_running)
        {
            self.attenuation_acknowledged[volume_index(register)] = true;
        }
        self.awaiting_volume =
            match (outputs.last(), event) {
                (
                    Some(Output::Command(Command::WriteDac(DacWrite::Volume(register, level)))),
                    _,
                ) if *level == Attenuation::FULL => Some(*register),
                (Some(_), _)
                | (None, Event::I2cAcked | Event::I2cNacked | Event::StatusRead(_)) => None,
                (None, _) => self.awaiting_volume,
            };
        if let Event::ControlPassElapsed(_) = event {
            self.level_at_pass_start = self.level;
        }
        self.pll_wait = match (self.pll_wait, event) {
            (
                PllWait::Unconfirmed { needed, follower },
                Event::StepDone | Event::PllLockRead(..),
            ) => PllWait::Counting {
                needed,
                counted: 0,
                follower,
                splits: 0,
            },
            (
                PllWait::Counting {
                    needed,
                    counted,
                    follower,
                    splits,
                },
                Event::Elapsed(more),
            ) => PllWait::Counting {
                needed,
                counted: counted.saturating_add(more.0),
                follower,
                splits: splits + 1,
            },
            (wait, _) => wait,
        };
        for output in outputs {
            match *output {
                Output::Phantom(phantom::Event::Boot) => {
                    assert!(
                        !matches!(
                            before,
                            State::VolumeRamp(..) | State::Running(_, _) | State::Degraded(_)
                        ),
                        "seed {seed}: analog reset after bring-up"
                    );
                    self.analog_chain_reset = true;
                }
                Output::Phantom(phantom::Event::BootReachedRunning) => {
                    assert!(
                        matches!(before, State::VolumeRamp(..))
                            && matches!(after, State::Running(_, _)),
                        "seed {seed}: running announced outside the ramp's end"
                    );
                }
                Output::Phantom(other) => panic!("seed {seed}: phantom event {other:?}"),
                Output::Command(command) => {
                    if command == Command::ConfigureConverterBus {
                        self.converter_bus_configurations += 1;
                        assert_eq!(
                            self.converter_bus_configurations, 1,
                            "seed {seed}: a boot configured the converter bus more than once"
                        );
                        self.converter_bus_configured = true;
                    }
                    assert!(
                        self.converter_bus_configured || !speaks_on_the_converter_bus(command),
                        "seed {seed}: {command:?} before the converter bus is configured"
                    );
                    if is_a_start(command) {
                        assert!(
                            !self.fault_raised
                                || command
                                    != Command::DrivePin(
                                        OutputPin::TransportActive,
                                        PinLevel::High
                                    ),
                            "seed {seed}: the outputs connect after a fault"
                        );
                        assert!(!self.refused, "seed {seed}: {command:?} after a refusal");
                    }
                    self.watch_pll(seed, command);
                    self.watch_output_connection(seed, command);
                    assert!(
                        self.analog_chain_reset || !touches_the_analog_chain(command),
                        "seed {seed}: {command:?} before the all-off word"
                    );
                    assert_ne!(
                        command,
                        Command::DrivePin(OutputPin::PhantomActive, PinLevel::High),
                        "seed {seed}"
                    );
                    assert!(
                        !matches!(command, Command::RecordFault(_, _, Weight::Raised)),
                        "seed {seed}: the boot recorded {command:?} past the supervisor"
                    );
                    if let Command::RecordFault(code, at, Weight::Noted) = command {
                        assert_eq!(
                            at,
                            At::Step(before.place()),
                            "seed {seed}: the noted fault is not placed where the boot waited"
                        );
                        self.faults_recorded += 1;
                        assert!(
                            outcome(after).faults.contains(code),
                            "seed {seed}: the noted fault is not in the next state"
                        );
                        assert!(
                            matches!(
                                before,
                                State::Scripted(stage, ..)
                                    if stage == Stage::DacAttenuate
                                        || matches!(stage.class(), StageClass::Panel | StageClass::Usb)
                            ),
                            "seed {seed}: {code:?} noted in {before:?}, a stage whose faults stop the audio"
                        );
                        assert_eq!(
                            outputs.first(),
                            Some(output),
                            "seed {seed}: the note of {code:?} is not the first output of its batch"
                        );
                    }
                    if let Command::WriteDac(write) = command {
                        for t in write.transactions().as_slice() {
                            let number = t.register.number();
                            assert!(
                                number != 0 && number != 62,
                                "seed {seed}: register {number}"
                            );
                        }
                    }
                    if let Command::WriteDac(DacWrite::Volume(register, level)) = command {
                        let steps = level.half_decibel_steps();
                        if !matches!(before, State::VolumeRamp(..)) {
                            assert_eq!(
                                steps, 0xFF,
                                "seed {seed}: {register:?} below full before the ramp"
                            );
                        }
                        if let Some(i) = ramped_index(register) {
                            assert!(
                                u16::from(steps) + 1 >= u16::from(self.level_at_pass_start[i]),
                                "seed {seed}: {register:?} dropped more than one step in a pass"
                            );
                            self.level[i] = steps;
                        } else {
                            assert_eq!(steps, 0xFF, "seed {seed}: {register:?} left full");
                        }
                        if steps < 0xFF {
                            self.raised_volume = true;
                        }
                    }
                }
                Output::Supervisor(supervisor::Event::FaultRaised(code, at)) => {
                    assert_eq!(
                        at,
                        At::Step(before.place()),
                        "seed {seed}: the raised fault is not placed where the boot waited"
                    );
                    self.faults_recorded += 1;
                    self.fault_raised = true;
                    assert!(
                        outcome(after).faults.contains(code),
                        "seed {seed}: the fault is not recorded in the next state"
                    );
                    if matches!(before, State::Scripted(..)) {
                        assert!(
                            !outcome(after).runs(device::boot::Subsystem::Audio),
                            "seed {seed}: the boot still runs audio after {code:?}"
                        );
                    }
                    if !matches!(before, State::VolumeRamp(..)) {
                        assert!(
                            !self.raised_volume,
                            "seed {seed}: volume raised on a failing path"
                        );
                    }
                }
                Output::Supervisor(_) | Output::Log(_) => {}
                Output::StartLcd => self.lcd_started = true,
            }
        }
        if outputs.contains(&Output::Command(Command::AttachUsb)) {
            self.usb_attached = true;
        }
        if matches!(before, State::Running(..) | State::Degraded(_)) {
            assert!(outputs.is_empty(), "seed {seed}: absorbing state emitted");
            assert_eq!(before, after, "seed {seed}: absorbing state left");
        }
        if let State::Degraded(recorded) = after {
            assert!(
                !recorded.runs(device::boot::Subsystem::Audio),
                "seed {seed}: degraded with audio running"
            );
            assert!(
                !recorded.faults.is_empty(),
                "seed {seed}: degraded with no fault"
            );
            assert!(
                self.lcd_started || !self.rails_raised,
                "seed {seed}: degraded before the panel started with the rails up"
            );
            assert!(
                !outcome(before).runs(device::boot::Subsystem::Audio)
                    || matches!(before, State::VolumeRamp(..)),
                "seed {seed}: audio abandoned after it ran"
            );
        }
        if let State::Running(_, recorded) = after {
            assert!(
                recorded.runs(device::boot::Subsystem::Audio),
                "seed {seed}: running without audio"
            );
            if !recorded.faults.is_empty() {
                assert!(self.lcd_started, "seed {seed}: running with a fault");
                assert!(self.usb_attached, "seed {seed}: running with a fault");
            }
        }
    }
}

#[test]
fn every_boot_invariant_holds_over_random_event_sequences_with_injected_faults() {
    let mut reached_running = 0;
    let mut reached_degraded = 0;
    let mut pll_waits_split = 0;
    let mut outputs_connected = 0;
    let mut faults_recorded = 0;
    for seed in 1..=SEQUENCES {
        let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let clean = seed <= CLEAN_SEQUENCES;
        let mut board = Board {
            fault_percent: if clean {
                0
            } else {
                [0, 1, 5, 30][rng.below(4) as usize]
            },
            nack_percent: if clean {
                0
            } else {
                [0, 2, 20][rng.below(3) as usize]
            },
            lock_percent: if clean {
                0
            } else {
                [0, 20, 100][rng.below(3) as usize]
            },
            targets: rng.targets(),
        };
        let mut observer = Observer::new();
        let (mut state, outputs) = begin();
        observer.check(seed, state, Event::StepDone, state, outputs.as_slice());
        let mut last = outputs.as_slice().to_vec();
        let events = if clean {
            CLEAN_EVENTS_PER_SEQUENCE
        } else {
            EVENTS_PER_SEQUENCE
        };
        for _ in 0..events {
            let event = board.next_event(&mut rng, &last);
            let (next, outputs) = transition(state, event);
            observer.check(seed, state, event, next, outputs.as_slice());
            last = outputs.as_slice().to_vec();
            state = next;
        }
        pll_waits_split += observer.pll_waits_split;
        outputs_connected += observer.outputs_connected;
        faults_recorded += observer.faults_recorded;
        if clean {
            assert!(
                matches!(state, State::Running(..)),
                "seed {seed}: a board with no faults must reach Running: {state:?}"
            );
            assert_eq!(outcome(state).faults.len(), 0, "seed {seed}");
            reached_running += 1;
            continue;
        }
        match state {
            State::Running(..) => reached_running += 1,
            State::Degraded(_) => reached_degraded += 1,
            State::Scripted(..) | State::VolumeRamp(..) => {}
        }
    }
    assert!(
        reached_running > SEQUENCES / 20,
        "running {reached_running}"
    );
    assert!(
        reached_degraded > SEQUENCES / 20,
        "degraded {reached_degraded}"
    );
    assert!(
        u64::from(outputs_connected) > SEQUENCES / 20,
        "outputs connected {outputs_connected}"
    );
    assert!(
        u64::from(pll_waits_split) > SEQUENCES / 10,
        "split waits {pll_waits_split}"
    );
    assert!(
        u64::from(faults_recorded) > SEQUENCES / 100,
        "faults {faults_recorded}"
    );
}
