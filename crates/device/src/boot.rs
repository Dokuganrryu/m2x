use dsp::SampleRate;

use crate::adc::{AdcAddress, AdcKind};
use crate::breadcrumb::{At, Place, Weight};
use crate::command::Command;
use crate::dac::{Attenuation, DacWrite, VolumeRegister};
use crate::fault::{FaultCode, I2cPeer};
use crate::output_pin::OutputPin;
use crate::outputs::{Filler, OutputList};
use crate::phantom::{self, Milliseconds, PinLevel};
use crate::pll::{CORE_PLL_STABILISATION, audio_pll_stabilisation};
use crate::safe_init::{SAFE_DAC_INIT, adc_init};
use crate::steps::{
    Attempt, Countdown, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23,
    PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23, ScriptStep, count_down, fully_attenuate,
    later, reached, start_transport,
};
use crate::supervisor;

pub const MOST_OUTPUTS_PER_EVENT: usize = 13;
pub const BOOT_RATE: SampleRate = SampleRate::Hz48000;
pub const RAIL_STAGGER: Milliseconds = Milliseconds(5);
pub const RAIL_SETTLE: Milliseconds = Milliseconds(100);
pub const ADC_RESET_PULSE: Milliseconds = Milliseconds(5);
pub const ADC_WAKE: Milliseconds = Milliseconds(100);
pub const PROBE_SPACING: Milliseconds = Milliseconds(1);

pub const RAMPED_REGISTERS: [VolumeRegister; 6] = [
    VolumeRegister::FactoryZeroA,
    VolumeRegister::FactoryZeroB,
    VolumeRegister::MainA,
    VolumeRegister::MainB,
    VolumeRegister::HeadphoneA,
    VolumeRegister::HeadphoneB,
];

const SILENCED_AUDIO: [Command; 3] = [Command::StopTickTimer, Command::StopDma, Command::StopI2s];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeTargets {
    pub main: Attenuation,
    pub headphone: Attenuation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    StepDone,
    I2cAcked,
    I2cNacked,
    StatusRead(u8),
    ClockInputRead(ClockInput),
    PllLockRead(Pll, PllLock),
    CommandRefused,
    Elapsed(Milliseconds),
    ControlPassElapsed(VolumeTargets),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockInput {
    Ready,
    NotReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pll {
    Core,
    Audio,
    UsbPhy,
}

impl Pll {
    pub const fn unlocked(self) -> FaultCode {
        match self {
            Pll::Core => FaultCode::CorePllUnlocked,
            Pll::Audio => FaultCode::AudioPllUnlocked,
            Pll::UsbPhy => FaultCode::UsbPhyPllUnlocked,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PllLock {
    Locked,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootRecord {
    DacStatus(u8),
    AdcDetected(AdcKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Command(Command),
    Phantom(phantom::Event),
    Supervisor(supervisor::Event),
    Log(BootRecord),
    StartLcd,
}

pub type Outputs = OutputList<Output, MOST_OUTPUTS_PER_EVENT>;

impl Filler for Output {
    const FILLER: Self = Output::Log(BootRecord::DacStatus(0));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Quiesce,
    Clocks,
    UsbAttach,
    AnalogOff,
    Rails,
    StartLcd,
    AudioClocks,
    DacAttenuate,
    AdcProbe(AdcKind),
    TransportStart(AdcKind),
    DacInit(AdcKind),
    OutputConnect(AdcKind),
    AdcInit(AdcKind),
    Panel(AdcKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageClass {
    AudioPath,
    Clock,
    Panel,
    Usb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subsystem {
    CorePll,
    AnalogChain,
    Rails,
    Audio,
    Panel,
    Usb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subsystems(u8);

impl Subsystem {
    const fn mask(self) -> u8 {
        match self {
            Subsystem::CorePll => 1 << 0,
            Subsystem::Rails => 1 << 1,
            Subsystem::Audio => 1 << 2,
            Subsystem::Panel => 1 << 3,
            Subsystem::Usb => 1 << 4,
            Subsystem::AnalogChain => 1 << 5,
        }
    }
}

impl Subsystems {
    pub const NONE: Self = Self(0);

    pub const fn with(self, subsystem: Subsystem) -> Self {
        Self(self.0 | subsystem.mask())
    }

    pub const fn without(self, subsystem: Subsystem) -> Self {
        Self(self.0 & !subsystem.mask())
    }

    pub const fn contains(self, subsystem: Subsystem) -> bool {
        self.0 & subsystem.mask() != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Course {
    Whole,
    FaultLadder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Faults(u32);

impl Faults {
    pub const NONE: Self = Self(0);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub const fn contains(self, code: FaultCode) -> bool {
        self.0 & (1 << code.number()) != 0
    }

    const fn with(self, code: FaultCode) -> Self {
        Self(self.0 | (1 << code.number()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub faults: Faults,
    pub running: Subsystems,
    pub course: Course,
}

impl Outcome {
    pub const START: Self = Self {
        faults: Faults::NONE,
        running: Subsystems::NONE,
        course: Course::Whole,
    };

    const fn with(self, code: FaultCode) -> Self {
        Self {
            faults: self.faults.with(code),
            ..self
        }
    }

    const fn running(self, subsystem: Subsystem) -> Self {
        Self {
            running: self.running.with(subsystem),
            ..self
        }
    }

    const fn on_the_fault_ladder(self) -> Self {
        Self {
            running: self.running.without(Subsystem::Audio),
            course: Course::FaultLadder,
            ..self
        }
    }

    const fn locked(self, pll: Pll) -> Self {
        match pll {
            Pll::Core => self.running(Subsystem::CorePll),
            Pll::Audio | Pll::UsbPhy => self,
        }
    }

    const fn awaits_the_audio(self) -> bool {
        matches!(self.course, Course::Whole) && !self.runs(Subsystem::Audio)
    }

    pub const fn runs(self, subsystem: Subsystem) -> bool {
        self.running.contains(subsystem)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    item: usize,
    wait: Wait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wait {
    StepDone {
        waited: Milliseconds,
    },
    Answer {
        transfer: Transfer,
        attempt: Attempt,
        waited: Milliseconds,
    },
    Hold {
        remaining: Milliseconds,
    },
    ClockInput,
    PllLock {
        pll: Pll,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Transfer {
    command: Command,
    kind: TransferKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransferKind {
    Write(I2cPeer),
    StatusRead,
    Probe(AdcAddress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RampLevels([Attenuation; 6]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ramp {
    written: RampLevels,
    phase: RampPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RampPhase {
    BetweenPasses,
    Writing {
        planned: RampLevels,
        register: usize,
        attempt: Attempt,
        waited: Milliseconds,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Scripted(Stage, Cursor, Outcome),
    VolumeRamp(AdcKind, Ramp, Outcome),
    Running(AdcKind, Outcome),
    Degraded(Outcome),
}

#[derive(Clone, Copy)]
enum Script {
    Outputs(&'static [ScriptStep<Output>]),
    Commands(&'static [ScriptStep<Command>]),
}

enum Step {
    Plain(Output),
    Hold(Milliseconds),
    Transfer(Transfer),
    ClockInputRead(Command),
    PllLockRead(Command, Pll),
}

enum Reply {
    Acked,
    Unanswered,
    Status(u8),
}

enum OnExhausted {
    Skip,
    Fail,
}

const fn command(command: Command) -> ScriptStep<Output> {
    ScriptStep::Emit(Output::Command(command))
}

const fn drive(pin: OutputPin, level: PinLevel) -> ScriptStep<Output> {
    command(Command::DrivePin(pin, level))
}

const fn hold(duration: Milliseconds) -> ScriptStep<Output> {
    ScriptStep::Hold(duration)
}

const RESET_ANALOG_CHAIN: ScriptStep<Output> =
    ScriptStep::Emit(Output::Phantom(phantom::Event::Boot));

const QUIESCE: [ScriptStep<Output>; 12] = [
    drive(OutputPin::TransportActive, PinLevel::Low),
    drive(OutputPin::SecondBringUp, PinLevel::Low),
    drive(OutputPin::FirstBringUp, PinLevel::Low),
    drive(OutputPin::ThirdBringUp, PinLevel::Low),
    command(Command::DisableInterrupts),
    command(Command::StopDma),
    command(Command::StopTickTimer),
    command(Command::StopI2s),
    command(Command::DetachUsb),
    command(Command::FeedWatchdog),
    command(Command::FenceFlashController),
    command(Command::ArmWatchdog),
];

const ANALOG_OFF: [ScriptStep<Output>; 2] =
    [command(Command::ConfigureAnalogWordBus), RESET_ANALOG_CHAIN];

const RAILS: [ScriptStep<Output>; 10] = [
    drive(OutputPin::ConverterEnable, PinLevel::Low),
    drive(OutputPin::AdcEnable, PinLevel::Low),
    drive(OutputPin::FirstBringUp, PinLevel::High),
    hold(RAIL_STAGGER),
    drive(OutputPin::SecondBringUp, PinLevel::High),
    hold(RAIL_STAGGER),
    drive(OutputPin::ThirdBringUp, PinLevel::High),
    hold(RAIL_SETTLE),
    RESET_ANALOG_CHAIN,
    command(Command::ConfigureConverterBus),
];

const CLOCKS: [ScriptStep<Output>; 5] = [
    command(Command::StartClockInput),
    command(Command::ReadClockInput),
    command(Command::StartCorePll),
    hold(CORE_PLL_STABILISATION),
    command(Command::SelectCorePllAsMainClock),
];

const AUDIO_CLOCKS: [ScriptStep<Output>; 3] = [
    command(Command::SwitchPll0Family(BOOT_RATE.family())),
    hold(audio_pll_stabilisation(BOOT_RATE.family())),
    command(Command::EnableMasterClockOutput),
];

const DAC_ATTENUATE: [ScriptStep<Output>; 8] = [
    command(fully_attenuate(VolumeRegister::FactoryZeroA)),
    command(fully_attenuate(VolumeRegister::FactoryZeroB)),
    command(fully_attenuate(VolumeRegister::MainA)),
    command(fully_attenuate(VolumeRegister::MainB)),
    command(fully_attenuate(VolumeRegister::HeadphoneA)),
    command(fully_attenuate(VolumeRegister::HeadphoneB)),
    command(fully_attenuate(VolumeRegister::FactoryHeldA)),
    command(fully_attenuate(VolumeRegister::FactoryHeldB)),
];

const ADC_PROBE: [ScriptStep<Output>; 8] = [
    drive(OutputPin::AdcEnable, PinLevel::Low),
    hold(ADC_RESET_PULSE),
    drive(OutputPin::ConverterEnable, PinLevel::High),
    drive(OutputPin::AdcEnable, PinLevel::High),
    hold(ADC_WAKE),
    command(Command::ProbeAdc(AdcAddress::At13)),
    hold(PROBE_SPACING),
    command(Command::ProbeAdc(AdcAddress::At4F)),
];

const BOOT_TRANSPORT: [Command; 4] = start_transport(BOOT_RATE.frames_per_tick());

const TRANSPORT_START: [ScriptStep<Output>; 5] = {
    let [dma, i2s, tick_timer, _] = BOOT_TRANSPORT;
    [
        command(Command::ZeroTransmitBuffers),
        command(dma),
        command(i2s),
        command(tick_timer),
        ScriptStep::Emit(Output::Supervisor(supervisor::Event::TransportStarted)),
    ]
};

const OUTPUT_CONNECT: [ScriptStep<Output>; 1] = {
    let [_, _, _, connect_outputs] = BOOT_TRANSPORT;
    [command(connect_outputs)]
};

const PANEL: [ScriptStep<Output>; 2] = [
    command(Command::WriteExpanderDirection),
    command(Command::StartKnobScan),
];

const START_LCD: [ScriptStep<Output>; 1] = [ScriptStep::Emit(Output::StartLcd)];

const USB_ATTACH: [ScriptStep<Output>; 1] = [command(Command::AttachUsb)];

const fn script(stage: Stage) -> Script {
    match stage {
        Stage::Quiesce => Script::Outputs(&QUIESCE_OF_THE_BUILD),
        Stage::Clocks => Script::Outputs(&CLOCKS_OF_THE_BUILD),
        Stage::UsbAttach => Script::Outputs(&USB_ATTACH),
        Stage::AnalogOff => Script::Outputs(&ANALOG_OFF),
        Stage::Rails => Script::Outputs(&RAILS),
        Stage::StartLcd => Script::Outputs(&START_LCD_OF_THE_BUILD),
        Stage::AudioClocks => Script::Outputs(&AUDIO_CLOCKS_OF_THE_BUILD),
        Stage::DacAttenuate => Script::Outputs(&DAC_ATTENUATE_OF_THE_BUILD),
        Stage::AdcProbe(_) => Script::Outputs(&ADC_PROBE_OF_THE_BUILD),
        Stage::TransportStart(_) => Script::Outputs(&TRANSPORT_START),
        Stage::DacInit(_) => Script::Commands(&SAFE_DAC_INIT),
        Stage::OutputConnect(_) => Script::Outputs(&OUTPUT_CONNECT),
        Stage::AdcInit(kind) => Script::Commands(adc_init(kind)),
        Stage::Panel(_) => Script::Outputs(&PANEL),
    }
}

impl Script {
    const fn len(self) -> usize {
        match self {
            Script::Outputs(outputs) => outputs.len(),
            Script::Commands(commands) => commands.len(),
        }
    }

    const fn step(self, index: usize) -> Option<Step> {
        if index >= self.len() {
            return None;
        }
        Some(match self {
            Script::Outputs(outputs) => match outputs[index] {
                ScriptStep::Emit(Output::Command(command)) => step_of(command),
                ScriptStep::Emit(output) => Step::Plain(output),
                ScriptStep::Hold(duration) => Step::Hold(duration),
            },
            Script::Commands(commands) => match commands[index] {
                ScriptStep::Emit(command) => step_of(command),
                ScriptStep::Hold(duration) => Step::Hold(duration),
            },
        })
    }
}

const fn step_of(command: Command) -> Step {
    let kind = match command {
        Command::WriteDac(_) => TransferKind::Write(I2cPeer::Dac),
        Command::WriteAdc(_) => TransferKind::Write(I2cPeer::Adc),
        Command::WriteExpanderDirection | Command::WriteExpanderOutputs(_) => {
            TransferKind::Write(I2cPeer::Expander)
        }
        Command::ReadExpanderInputs => TransferKind::StatusRead,
        Command::ReadDac(_) => TransferKind::StatusRead,
        Command::ProbeAdc(address) => TransferKind::Probe(address),
        Command::ReadClockInput => return Step::ClockInputRead(command),
        Command::SelectCorePllAsMainClock => return Step::PllLockRead(command, Pll::Core),
        Command::SwitchPll0Family(_) => return Step::PllLockRead(command, Pll::Audio),
        Command::AttachUsb => return Step::PllLockRead(command, Pll::UsbPhy),
        Command::DrivePin(..)
        | Command::SendAnalogWord(_)
        | Command::StartI2s(_)
        | Command::StopI2s
        | Command::StartDma
        | Command::StopDma
        | Command::StartTickTimer(_)
        | Command::StopTickTimer
        | Command::MuteInput(_)
        | Command::UnmuteInput(_)
        | Command::DisableInterrupts
        | Command::DetachUsb
        | Command::FeedWatchdog
        | Command::ArmWatchdog
        | Command::FenceFlashController
        | Command::ConfigureAnalogWordBus
        | Command::StartClockInput
        | Command::StartCorePll
        | Command::EnableMasterClockOutput
        | Command::ConfigureConverterBus
        | Command::StartKnobScan
        | Command::ZeroTransmitBuffers
        | Command::StopTransmitDma
        | Command::LatchPlaybackSilence
        | Command::RecordFault(..) => return Step::Plain(Output::Command(command)),
    };
    Step::Transfer(Transfer { command, kind })
}

const fn longest_plain_run(script: Script) -> usize {
    let mut longest = 0;
    let mut run = 0;
    let mut index = 0;
    while index < script.len() {
        run = match script.step(index) {
            Some(Step::Plain(_)) => run + 1,
            Some(
                Step::Hold(_) | Step::Transfer(_) | Step::ClockInputRead(_) | Step::PllLockRead(..),
            )
            | None => 0,
        };
        if run > longest {
            longest = run;
        }
        index += 1;
    }
    longest
}

const fn longest_plain_run_of_any_stage() -> usize {
    let stages = [
        Stage::Quiesce,
        Stage::Clocks,
        Stage::UsbAttach,
        Stage::AnalogOff,
        Stage::Rails,
        Stage::StartLcd,
        Stage::AudioClocks,
        Stage::DacAttenuate,
        Stage::AdcProbe(AdcKind::Unanswered),
        Stage::TransportStart(AdcKind::Unanswered),
        Stage::DacInit(AdcKind::Unanswered),
        Stage::OutputConnect(AdcKind::Unanswered),
        Stage::AdcInit(AdcKind::Unanswered),
        Stage::AdcInit(AdcKind::Answered(AdcAddress::At13)),
        Stage::AdcInit(AdcKind::Answered(AdcAddress::At4F)),
        Stage::Panel(AdcKind::Unanswered),
    ];
    let mut longest = 0;
    let mut index = 0;
    while index < stages.len() {
        assert!(
            script(stages[index]).len() <= u8::MAX as usize,
            "a step index fits the breadcrumb's step byte"
        );
        let run = longest_plain_run(script(stages[index]));
        if run > longest {
            longest = run;
        }
        index += 1;
    }
    longest
}

const RECORDS_BEFORE_A_RUN: usize = 1;

const VOLUME_RAMP: Place = Place { stage: 15, step: 0 };
pub(crate) const RUNNING: Place = Place { stage: 16, step: 0 };
const DEGRADED: Place = Place { stage: 17, step: 0 };

const _: () =
    assert!(longest_plain_run_of_any_stage() + RECORDS_BEFORE_A_RUN <= MOST_OUTPUTS_PER_EVENT);

impl RampLevels {
    const FULLY_ATTENUATED: Self = Self([Attenuation::FULL; 6]);

    const fn targets(targets: VolumeTargets) -> Self {
        Self([
            Attenuation::ZERO_DB,
            Attenuation::ZERO_DB,
            targets.main,
            targets.main,
            targets.headphone,
            targets.headphone,
        ])
    }

    fn stepped_toward(self, targets: VolumeTargets) -> Self {
        let goal = Self::targets(targets);
        let mut stepped = self;
        for (level, goal) in stepped.0.iter_mut().zip(goal.0) {
            let (now, wanted) = (level.half_decibel_steps(), goal.half_decibel_steps());
            *level = Attenuation::from_half_decibel_steps(match now.cmp(&wanted) {
                core::cmp::Ordering::Less => now + 1,
                core::cmp::Ordering::Equal => now,
                core::cmp::Ordering::Greater => now - 1,
            });
        }
        stepped
    }

    fn with(self, register: usize, level: Attenuation) -> Self {
        let mut changed = self;
        changed.0[register] = level;
        changed
    }
}

impl Stage {
    pub const fn number(self) -> u8 {
        match self {
            Stage::Quiesce => 1,
            Stage::Clocks => 2,
            Stage::UsbAttach => 3,
            Stage::AnalogOff => 4,
            Stage::Rails => 5,
            Stage::StartLcd => 6,
            Stage::AudioClocks => 7,
            Stage::DacAttenuate => 8,
            Stage::AdcProbe(_) => 9,
            Stage::TransportStart(_) => 10,
            Stage::DacInit(_) => 11,
            Stage::OutputConnect(_) => 12,
            Stage::AdcInit(_) => 13,
            Stage::Panel(_) => 14,
        }
    }

    const fn place(self, item: usize) -> Place {
        Place {
            stage: self.number(),
            step: item as u8,
        }
    }

    pub const fn class(self) -> StageClass {
        match self {
            Stage::Quiesce
            | Stage::AnalogOff
            | Stage::Rails
            | Stage::DacAttenuate
            | Stage::AdcProbe(_)
            | Stage::TransportStart(_)
            | Stage::DacInit(_)
            | Stage::OutputConnect(_)
            | Stage::AdcInit(_) => StageClass::AudioPath,
            Stage::Clocks | Stage::AudioClocks => StageClass::Clock,
            Stage::Panel(_) | Stage::StartLcd => StageClass::Panel,
            Stage::UsbAttach => StageClass::Usb,
        }
    }

    const fn adc_kind(self) -> AdcKind {
        match self {
            Stage::Quiesce
            | Stage::Clocks
            | Stage::UsbAttach
            | Stage::AnalogOff
            | Stage::Rails
            | Stage::StartLcd
            | Stage::AudioClocks
            | Stage::DacAttenuate => AdcKind::Unanswered,
            Stage::AdcProbe(kind)
            | Stage::TransportStart(kind)
            | Stage::DacInit(kind)
            | Stage::OutputConnect(kind)
            | Stage::AdcInit(kind)
            | Stage::Panel(kind) => kind,
        }
    }

    const fn when_exhausted(self) -> OnExhausted {
        match self {
            Stage::DacAttenuate => OnExhausted::Skip,
            Stage::Quiesce
            | Stage::Clocks
            | Stage::UsbAttach
            | Stage::AnalogOff
            | Stage::Rails
            | Stage::StartLcd
            | Stage::AudioClocks
            | Stage::AdcProbe(_)
            | Stage::TransportStart(_)
            | Stage::DacInit(_)
            | Stage::OutputConnect(_)
            | Stage::AdcInit(_)
            | Stage::Panel(_) => OnExhausted::Fail,
        }
    }

    const fn probe_acked(self, address: AdcAddress) -> Self {
        match self {
            Stage::AdcProbe(AdcKind::Unanswered) => Stage::AdcProbe(AdcKind::Answered(address)),
            Stage::AdcProbe(AdcKind::Answered(_))
            | Stage::Quiesce
            | Stage::Clocks
            | Stage::UsbAttach
            | Stage::AnalogOff
            | Stage::Rails
            | Stage::StartLcd
            | Stage::AudioClocks
            | Stage::DacAttenuate
            | Stage::TransportStart(_)
            | Stage::DacInit(_)
            | Stage::OutputConnect(_)
            | Stage::AdcInit(_)
            | Stage::Panel(_) => self,
        }
    }

    #[inline(never)]
    const fn next_in_boot_order(self) -> Option<Self> {
        Some(match self {
            Stage::Quiesce => Stage::Clocks,
            Stage::Clocks => Stage::UsbAttach,
            Stage::UsbAttach => Stage::AnalogOff,
            Stage::AnalogOff => Stage::Rails,
            Stage::Rails => Stage::StartLcd,
            Stage::StartLcd => Stage::AudioClocks,
            Stage::AudioClocks => Stage::DacAttenuate,
            Stage::DacAttenuate => Stage::AdcProbe(AdcKind::Unanswered),
            Stage::AdcProbe(kind) => Stage::TransportStart(kind),
            Stage::TransportStart(kind) => Stage::DacInit(kind),
            Stage::DacInit(kind) => Stage::OutputConnect(kind),
            Stage::OutputConnect(kind) => Stage::AdcInit(kind),
            Stage::AdcInit(kind) => Stage::Panel(kind),
            Stage::Panel(_) => return None,
        })
    }

    #[inline(never)]
    const fn runs_after(self, outcome: Outcome) -> bool {
        match self {
            Stage::AnalogOff => true,
            Stage::UsbAttach => USB_ATTACHES_ON_A_CLOCK_FAULT || outcome.runs(Subsystem::CorePll),
            Stage::Rails => outcome.runs(Subsystem::AnalogChain),
            Stage::StartLcd | Stage::Panel(_) => outcome.runs(Subsystem::Rails),
            Stage::Quiesce
            | Stage::Clocks
            | Stage::AudioClocks
            | Stage::DacAttenuate
            | Stage::AdcProbe(_)
            | Stage::TransportStart(_)
            | Stage::DacInit(_)
            | Stage::OutputConnect(_)
            | Stage::AdcInit(_) => matches!(outcome.course, Course::Whole),
        }
    }

    fn completed(self, outcome: Outcome, outputs: &mut Outputs) -> Outcome {
        match self {
            Stage::UsbAttach => outcome.running(Subsystem::Usb),
            Stage::AnalogOff => outcome.running(Subsystem::AnalogChain),
            Stage::Rails => outcome.running(Subsystem::Rails),
            Stage::StartLcd => outcome.running(Subsystem::Panel),
            Stage::AdcProbe(kind) => {
                outputs.push(Output::Log(BootRecord::AdcDetected(kind)));
                outcome
            }
            Stage::OutputConnect(_) => outcome.running(Subsystem::Audio),
            Stage::Quiesce
            | Stage::Clocks
            | Stage::AudioClocks
            | Stage::DacAttenuate
            | Stage::TransportStart(_)
            | Stage::DacInit(_)
            | Stage::AdcInit(_)
            | Stage::Panel(_) => outcome,
        }
    }
}

fn next_stage(stage: Stage, outcome: Outcome) -> Option<Stage> {
    let mut next = stage.next_in_boot_order()?;
    while !next.runs_after(outcome) {
        next = next.next_in_boot_order()?;
    }
    Some(next)
}

fn finished(stage: Stage, outcome: Outcome) -> State {
    if outcome.runs(Subsystem::Audio) {
        State::VolumeRamp(stage.adc_kind(), Ramp::START, outcome)
    } else {
        State::Degraded(outcome)
    }
}

impl State {
    pub const fn place(self) -> Place {
        match self {
            State::Scripted(stage, cursor, _) => stage.place(cursor.item),
            State::VolumeRamp(..) => VOLUME_RAMP,
            State::Running(..) => RUNNING,
            State::Degraded(_) => DEGRADED,
        }
    }

    pub fn awaits_expander_direction(self) -> bool {
        match self {
            State::Scripted(
                _,
                Cursor {
                    wait: Wait::Answer { transfer, .. },
                    ..
                },
                _,
            ) => transfer.command == Command::WriteExpanderDirection,
            State::Scripted(..)
            | State::VolumeRamp(..)
            | State::Running(..)
            | State::Degraded(_) => false,
        }
    }
}

pub fn begin() -> (State, Outputs) {
    let mut outputs = Outputs::NONE;
    let state = advance(Stage::Quiesce, Outcome::START, 0, &mut outputs);
    (state, outputs)
}

pub fn transition(state: State, event: Event) -> (State, Outputs) {
    let mut outputs = Outputs::NONE;
    let next = match state {
        State::Scripted(stage, cursor, outcome) => {
            scripted(stage, cursor, outcome, event, &mut outputs)
        }
        State::VolumeRamp(kind, ramp, outcome) => ramping(kind, ramp, outcome, event, &mut outputs),
        State::Running(_, _) | State::Degraded(_) => state,
    };
    (next, outputs)
}

fn advance(
    mut stage: Stage,
    mut outcome: Outcome,
    mut item: usize,
    outputs: &mut Outputs,
) -> State {
    loop {
        let script = script(stage);
        let mut ran_plain = false;
        while let Some(Step::Plain(output)) = script.step(item) {
            outputs.push(output);
            ran_plain = true;
            item += 1;
        }
        if ran_plain {
            return waiting(
                stage,
                outcome,
                item,
                Wait::StepDone {
                    waited: Milliseconds(0),
                },
            );
        }
        match script.step(item) {
            Some(Step::Plain(_)) | None => {
                outcome = stage.completed(outcome, outputs);
                match next_stage(stage, outcome) {
                    Some(next) => {
                        stage = next;
                        item = 0;
                    }
                    None => return finished(stage, outcome),
                }
            }
            Some(Step::Hold(remaining)) => {
                return waiting(stage, outcome, item, Wait::Hold { remaining });
            }
            Some(Step::ClockInputRead(command)) => {
                outputs.push(Output::Command(command));
                return waiting(stage, outcome, item, Wait::ClockInput);
            }
            Some(Step::PllLockRead(command, pll)) => {
                outputs.push(Output::Command(command));
                return waiting(stage, outcome, item, Wait::PllLock { pll });
            }
            Some(Step::Transfer(transfer)) => {
                outputs.push(Output::Command(transfer.command));
                return waiting(
                    stage,
                    outcome,
                    item,
                    Wait::Answer {
                        transfer,
                        attempt: Attempt::FIRST,
                        waited: Milliseconds(0),
                    },
                );
            }
        }
    }
}

const fn waiting(stage: Stage, outcome: Outcome, item: usize, wait: Wait) -> State {
    State::Scripted(stage, Cursor { item, wait }, outcome)
}

fn failed(
    stage: Stage,
    outcome: Outcome,
    item: usize,
    code: FaultCode,
    outputs: &mut Outputs,
) -> State {
    let outcome = outcome.with(code);
    let keeps_the_audio = match stage.class() {
        StageClass::AudioPath | StageClass::Clock => false,
        StageClass::Panel | StageClass::Usb => {
            !(matches!(code, FaultCode::RefusedCommand) && outcome.awaits_the_audio())
        }
    };
    let at = At::Step(stage.place(item));
    if keeps_the_audio {
        outputs.push(Output::Command(Command::RecordFault(
            code,
            at,
            Weight::Noted,
        )));
        return advance(stage, outcome, item + 1, outputs);
    }
    announce(code, at, outputs);
    let outcome = outcome.on_the_fault_ladder();
    match next_stage(stage, outcome) {
        Some(next) => advance(next, outcome, 0, outputs),
        None => finished(stage, outcome),
    }
}

fn announce(code: FaultCode, at: At, outputs: &mut Outputs) {
    for command in SILENCED_AUDIO {
        outputs.push(Output::Command(command));
    }
    outputs.push(Output::Supervisor(supervisor::Event::FaultRaised(code, at)));
}

fn scripted(
    stage: Stage,
    cursor: Cursor,
    outcome: Outcome,
    event: Event,
    outputs: &mut Outputs,
) -> State {
    if let Event::CommandRefused = event {
        return failed(
            stage,
            outcome,
            cursor.item,
            FaultCode::RefusedCommand,
            outputs,
        );
    }
    let unchanged = waiting(stage, outcome, cursor.item, cursor.wait);
    match cursor.wait {
        Wait::StepDone { waited } => match event {
            Event::StepDone => advance(stage, outcome, cursor.item, outputs),
            Event::Elapsed(elapsed) => {
                let waited = later(waited, elapsed);
                if reached(waited, PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23) {
                    failed(
                        stage,
                        outcome,
                        cursor.item,
                        FaultCode::BoardStalled,
                        outputs,
                    )
                } else {
                    waiting(stage, outcome, cursor.item, Wait::StepDone { waited })
                }
            }
            Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::ClockInputRead(_)
            | Event::PllLockRead(..)
            | Event::CommandRefused
            | Event::ControlPassElapsed(_) => unchanged,
        },
        Wait::Hold { remaining } => match event {
            Event::Elapsed(elapsed) => match count_down(remaining, elapsed) {
                Countdown::Over => advance(stage, outcome, cursor.item + 1, outputs),
                Countdown::Left(remaining) => {
                    waiting(stage, outcome, cursor.item, Wait::Hold { remaining })
                }
            },
            Event::StepDone
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::ClockInputRead(_)
            | Event::PllLockRead(..)
            | Event::CommandRefused
            | Event::ControlPassElapsed(_) => unchanged,
        },
        Wait::ClockInput => match event {
            Event::ClockInputRead(ClockInput::Ready) => {
                advance(stage, outcome, cursor.item + 1, outputs)
            }
            Event::ClockInputRead(ClockInput::NotReady) => failed(
                stage,
                outcome,
                cursor.item,
                FaultCode::CrystalNotReady,
                outputs,
            ),
            Event::StepDone
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::PllLockRead(..)
            | Event::CommandRefused
            | Event::Elapsed(_)
            | Event::ControlPassElapsed(_) => unchanged,
        },
        Wait::PllLock { pll } => match event {
            Event::PllLockRead(read, PllLock::Locked) if read == pll => {
                advance(stage, outcome.locked(pll), cursor.item + 1, outputs)
            }
            Event::PllLockRead(read, PllLock::TimedOut) if read == pll => {
                failed(stage, outcome, cursor.item, pll.unlocked(), outputs)
            }
            Event::PllLockRead(..)
            | Event::StepDone
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::ClockInputRead(_)
            | Event::CommandRefused
            | Event::Elapsed(_)
            | Event::ControlPassElapsed(_) => unchanged,
        },
        Wait::Answer {
            transfer,
            attempt,
            waited,
        } => {
            let reply = match event {
                Event::I2cAcked => Reply::Acked,
                Event::I2cNacked => Reply::Unanswered,
                Event::StatusRead(byte) => Reply::Status(byte),
                Event::Elapsed(elapsed) => {
                    let waited = later(waited, elapsed);
                    if reached(waited, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23) {
                        Reply::Unanswered
                    } else {
                        return waiting(
                            stage,
                            outcome,
                            cursor.item,
                            Wait::Answer {
                                transfer,
                                attempt,
                                waited,
                            },
                        );
                    }
                }
                Event::StepDone
                | Event::ControlPassElapsed(_)
                | Event::ClockInputRead(_)
                | Event::PllLockRead(..)
                | Event::CommandRefused => {
                    return unchanged;
                }
            };
            answered(
                stage,
                outcome,
                cursor.item,
                transfer,
                attempt,
                reply,
                outputs,
            )
        }
    }
}

fn answered(
    stage: Stage,
    outcome: Outcome,
    item: usize,
    transfer: Transfer,
    attempt: Attempt,
    reply: Reply,
    outputs: &mut Outputs,
) -> State {
    let peer = match (transfer.kind, reply) {
        (TransferKind::Probe(address), Reply::Acked) => {
            return advance(stage.probe_acked(address), outcome, item + 1, outputs);
        }
        (TransferKind::Probe(_), Reply::Unanswered | Reply::Status(_)) => {
            return advance(stage, outcome, item + 1, outputs);
        }
        (TransferKind::Write(_), Reply::Acked) => {
            return advance(stage, outcome, item + 1, outputs);
        }
        (TransferKind::StatusRead, Reply::Status(byte)) => {
            outputs.push(Output::Log(BootRecord::DacStatus(byte)));
            return advance(stage, outcome, item + 1, outputs);
        }
        (TransferKind::Write(peer), Reply::Unanswered | Reply::Status(_)) => peer,
        (TransferKind::StatusRead, Reply::Acked | Reply::Unanswered) => I2cPeer::Dac,
    };
    match attempt.next() {
        Some(attempt) => {
            outputs.push(Output::Command(transfer.command));
            waiting(
                stage,
                outcome,
                item,
                Wait::Answer {
                    transfer,
                    attempt,
                    waited: Milliseconds(0),
                },
            )
        }
        None => match stage.when_exhausted() {
            OnExhausted::Skip => {
                let code = FaultCode::Unanswered(peer);
                outputs.push(Output::Command(Command::RecordFault(
                    code,
                    At::Step(stage.place(item)),
                    Weight::Noted,
                )));
                advance(stage, outcome.with(code), item + 1, outputs)
            }
            OnExhausted::Fail => failed(stage, outcome, item, FaultCode::Unanswered(peer), outputs),
        },
    }
}

impl Ramp {
    const START: Self = Self {
        written: RampLevels::FULLY_ATTENUATED,
        phase: RampPhase::BetweenPasses,
    };
}

fn ramping(
    kind: AdcKind,
    ramp: Ramp,
    outcome: Outcome,
    event: Event,
    outputs: &mut Outputs,
) -> State {
    let unchanged = State::VolumeRamp(kind, ramp, outcome);
    match ramp.phase {
        RampPhase::BetweenPasses => match event {
            Event::ControlPassElapsed(targets) => {
                let planned = ramp.written.stepped_toward(targets);
                if planned == ramp.written {
                    outputs.push(Output::Phantom(phantom::Event::BootReachedRunning));
                    State::Running(kind, outcome)
                } else {
                    write_next_ramp_register(kind, ramp.written, planned, 0, outcome, outputs)
                }
            }
            Event::CommandRefused => stop_the_audio(outcome, FaultCode::RefusedCommand, outputs),
            Event::StepDone
            | Event::I2cAcked
            | Event::I2cNacked
            | Event::StatusRead(_)
            | Event::ClockInputRead(_)
            | Event::PllLockRead(..)
            | Event::Elapsed(_) => unchanged,
        },
        RampPhase::Writing {
            planned,
            register,
            attempt,
            waited,
        } => {
            let acked = match event {
                Event::I2cAcked => true,
                Event::I2cNacked | Event::StatusRead(_) | Event::ClockInputRead(_) => false,
                Event::Elapsed(elapsed) => {
                    let waited = later(waited, elapsed);
                    if reached(waited, PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23) {
                        false
                    } else {
                        return State::VolumeRamp(
                            kind,
                            Ramp {
                                written: ramp.written,
                                phase: RampPhase::Writing {
                                    planned,
                                    register,
                                    attempt,
                                    waited,
                                },
                            },
                            outcome,
                        );
                    }
                }
                Event::StepDone | Event::ControlPassElapsed(_) | Event::PllLockRead(..) => {
                    return unchanged;
                }
                Event::CommandRefused => {
                    return stop_the_audio(outcome, FaultCode::RefusedCommand, outputs);
                }
            };
            if acked {
                let written = ramp.written.with(register, planned.0[register]);
                return write_next_ramp_register(
                    kind,
                    written,
                    planned,
                    register + 1,
                    outcome,
                    outputs,
                );
            }
            match attempt.next() {
                Some(attempt) => {
                    outputs.push(ramp_write(register, planned));
                    State::VolumeRamp(
                        kind,
                        Ramp {
                            written: ramp.written,
                            phase: RampPhase::Writing {
                                planned,
                                register,
                                attempt,
                                waited: Milliseconds(0),
                            },
                        },
                        outcome,
                    )
                }
                None => stop_the_audio(outcome, FaultCode::Unanswered(I2cPeer::Dac), outputs),
            }
        }
    }
}

fn stop_the_audio(outcome: Outcome, code: FaultCode, outputs: &mut Outputs) -> State {
    announce(code, At::Step(VOLUME_RAMP), outputs);
    State::Degraded(outcome.with(code).on_the_fault_ladder())
}

fn ramp_write(register: usize, planned: RampLevels) -> Output {
    Output::Command(Command::WriteDac(DacWrite::Volume(
        RAMPED_REGISTERS[register],
        planned.0[register],
    )))
}

fn write_next_ramp_register(
    kind: AdcKind,
    written: RampLevels,
    planned: RampLevels,
    from: usize,
    outcome: Outcome,
    outputs: &mut Outputs,
) -> State {
    let next =
        (from..RAMPED_REGISTERS.len()).find(|&register| written.0[register] != planned.0[register]);
    match next {
        Some(register) => {
            outputs.push(ramp_write(register, planned));
            State::VolumeRamp(
                kind,
                Ramp {
                    written,
                    phase: RampPhase::Writing {
                        planned,
                        register,
                        attempt: Attempt::FIRST,
                        waited: Milliseconds(0),
                    },
                },
                outcome,
            )
        }
        None => State::VolumeRamp(
            kind,
            Ramp {
                written,
                phase: RampPhase::BetweenPasses,
            },
            outcome,
        ),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    Full,
    QuiesceOnly,
    Milestone,
}

impl Image {
    pub const fn runs_boot_stage(self, state: State) -> bool {
        match self {
            Image::Full => true,
            Image::QuiesceOnly => !matches!(state, State::Scripted(Stage::Clocks, ..)),
            Image::Milestone => matches!(
                state,
                State::Scripted(
                    Stage::Quiesce
                        | Stage::Clocks
                        | Stage::UsbAttach
                        | Stage::AnalogOff
                        | Stage::Rails
                        | Stage::StartLcd,
                    ..
                )
            ),
        }
    }
}

#[cfg(not(feature = "milestone2"))]
const QUIESCE_OF_THE_BUILD: [ScriptStep<Output>; 12] = QUIESCE;
#[cfg(not(feature = "milestone2"))]
const CLOCKS_OF_THE_BUILD: [ScriptStep<Output>; 5] = CLOCKS;
#[cfg(not(feature = "milestone2"))]
const USB_ATTACHES_ON_A_CLOCK_FAULT: bool = false;

// WHY: Milestone2 runs no flash code, so the MPU fence guards nothing and is one more untested step before USB; USB RAM stays Device memory, which its AtomicU32 word accesses allow
#[cfg(feature = "milestone2")]
const QUIESCE_OF_THE_BUILD: [ScriptStep<Output>; 11] = {
    let [a, b, c, d, e, f, g, h, i, j, fence, k] = QUIESCE;
    assert!(matches!(
        fence,
        ScriptStep::Emit(Output::Command(Command::FenceFlashController))
    ));
    [a, b, c, d, e, f, g, h, i, j, k]
};
// WHY: The core keeps the clock it was handed; the crystal is all the USB PHY PLL needs, and a clock fault never skips UsbAttach
#[cfg(feature = "milestone2")]
const CLOCKS_OF_THE_BUILD: [ScriptStep<Output>; 2] = {
    let [start_clock_input, read_clock_input, ..] = CLOCKS;
    [start_clock_input, read_clock_input]
};
#[cfg(feature = "milestone2")]
const USB_ATTACHES_ON_A_CLOCK_FAULT: bool = true;

#[cfg(not(feature = "milestone7"))]
const AUDIO_CLOCKS_OF_THE_BUILD: [ScriptStep<Output>; 3] = AUDIO_CLOCKS;

// WHY: PLL1 is raised only after UsbAttach and StartLcd, so a core clock that never locks leaves USB, the console and the screen alive to report stage 7 and the fault code; PLL1 starts before PLL0
#[cfg(feature = "milestone7")]
const AUDIO_CLOCKS_OF_THE_BUILD: [ScriptStep<Output>; 6] = {
    let [
        start_clock_input,
        read_clock_input,
        start_core_pll,
        settle,
        select_core_pll,
    ] = CLOCKS;
    assert!(matches!(
        start_clock_input,
        ScriptStep::Emit(Output::Command(Command::StartClockInput))
    ));
    assert!(matches!(
        read_clock_input,
        ScriptStep::Emit(Output::Command(Command::ReadClockInput))
    ));
    let [switch_pll0, pll0_settle, master_clock] = AUDIO_CLOCKS;
    [
        start_core_pll,
        settle,
        select_core_pll,
        switch_pll0,
        pll0_settle,
        master_clock,
    ]
};

#[cfg(not(feature = "milestone7"))]
const START_LCD_OF_THE_BUILD: [ScriptStep<Output>; 1] = START_LCD;

// WHY: DISPON reaches the panel 329 ms after StartLcd, 300 ms of it the panel's SWRESET and SLPOUT holds; the boot holds until the screen is lit, so a later stage that hangs the core leaves the screen showing its stage; a hold, not a wait on DISPON, so a stalled FC4 never stops the boot
#[cfg(feature = "milestone7")]
pub const SCREEN_LIT_BEFORE_THE_CLOCKS: Milliseconds = Milliseconds(500);

#[cfg(feature = "milestone7")]
const START_LCD_OF_THE_BUILD: [ScriptStep<Output>; 2] = {
    let [start_lcd] = START_LCD;
    [start_lcd, hold(SCREEN_LIT_BEFORE_THE_CLOCKS)]
};

#[cfg(not(feature = "sound1"))]
const DAC_ATTENUATE_OF_THE_BUILD: [ScriptStep<Output>; 8] = DAC_ATTENUATE;
#[cfg(not(feature = "sound1"))]
const ADC_PROBE_OF_THE_BUILD: [ScriptStep<Output>; 8] = ADC_PROBE;

// WHY: The converters need P1_29 low for 5 ms, then P1_3 and P1_29 high and 100 ms before the first converter bus transfer, and the ES9026 batch only after the probes; with P1_3 low every attenuation write goes unanswered, so the converters are powered first, still with P0_28 low
#[cfg(feature = "sound1")]
const DAC_ATTENUATE_OF_THE_BUILD: [ScriptStep<Output>; 13] = {
    let [adc_low, reset_pulse, converter_high, adc_high, wake, ..] = ADC_PROBE;
    let [a, b, c, d, e, f, g, h] = DAC_ATTENUATE;
    [
        adc_low,
        reset_pulse,
        converter_high,
        adc_high,
        wake,
        a,
        b,
        c,
        d,
        e,
        f,
        g,
        h,
    ]
};

#[cfg(feature = "sound1")]
const ADC_PROBE_OF_THE_BUILD: [ScriptStep<Output>; 3] = {
    let [.., probe_13, spacing, probe_4f] = ADC_PROBE;
    [probe_13, spacing, probe_4f]
};
