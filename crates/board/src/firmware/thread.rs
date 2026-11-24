use device::analog_settings::AnalogSettings;
use device::boot::{self, BOOT_RATE, Pll, PllLock, VolumeTargets};
use device::breadcrumb::{At, Weight};
use device::command::Command;
use device::console::{BusHealth, PanelFacts};
use device::control_pass::{CONTROL_PERIOD, ControlPass, Monitoring, PanelReadings};
use device::expander::ExpanderOutputs;
use device::fault::FaultCode;
use device::i2c::Answer;
use device::input::Input;
use device::knob::Knobs;
use device::knob_law::volume_targets;
use device::lcd::screen::ClockLock;
use device::output_pin::OutputPin;
use device::panel_bus::PanelTransfer;
use device::phantom::{Jack, Milliseconds, PinLevel};
use device::pll::Readiness;
use device::stage_batch::{Refusal, run_stage_batch};
use device::usb::handover::NoticesSeen;
use device::usb::service::Notice;
use device::{phantom, rate_change, supervisor};
use dsp::{CoefficientPublisher, DspModeSwitch, MeterReader};
use stream::{PlaybackCounterReader, PlaybackGate};

use super::audio_dma;
use super::console::{Console, ConsoleFacts, ConsoleReach};
use super::executor::{BoardEvent, Completion, Executor};
use super::fault;
use super::halt;
use super::handovers;
use super::image::IMAGE;
use super::interrupts;
use super::panel::{BusNews, Panel, StatusFacts};
use super::pins;

#[expect(dead_code, reason = "the tick, USB, panel and console produce these")]
#[derive(Clone, Copy)]
pub enum ThreadEvent {
    RateChange(rate_change::Event),
    Phantom(phantom::Event),
    Supervisor(supervisor::Event),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Batch {
    Empty,
    Ran,
    AwaitingBus,
    Refused,
}

impl Batch {
    const fn of(completion: Completion) -> Self {
        match completion {
            Completion::Done => Batch::Ran,
            Completion::AwaitingBus => Batch::AwaitingBus,
            Completion::Refused => Batch::Refused,
            Completion::Reported => Batch::Empty,
        }
    }

    const fn then(self, step: Batch) -> Self {
        match (self, step) {
            (Batch::Refused, _) | (_, Batch::Refused) => Batch::Refused,
            (Batch::AwaitingBus, _) | (_, Batch::AwaitingBus) => Batch::AwaitingBus,
            (Batch::Ran, Batch::Ran | Batch::Empty) | (Batch::Empty, Batch::Ran) => Batch::Ran,
            (Batch::Empty, Batch::Empty) => Batch::Empty,
        }
    }

    const fn refusal(self) -> Refusal {
        match self {
            Batch::Refused => Refusal::Refused,
            Batch::Empty | Batch::Ran | Batch::AwaitingBus => Refusal::NotRefused,
        }
    }
}

#[derive(Clone, Copy)]
enum Volume {
    FollowsTheKnobs,
    HeldAtFullAttenuation,
}

#[derive(Clone, Copy)]
enum Stage {
    Booting(boot::State),
    Running {
        rate_change: rate_change::State,
        control: ControlPass,
        volume: Volume,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SequencerBus {
    Idle,
    Awaiting,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AudioClock {
    Unlocked,
    Locked,
}

pub struct ThreadContext {
    stage: Stage,
    phantom: phantom::State,
    supervisor: supervisor::State,
    audio_clock: AudioClock,
    usb_notices: NoticesSeen,
    console: Console,
    panel: Panel,
    sequencer_bus: SequencerBus,
    boot_knobs: Option<Knobs>,
    analog: AnalogSettings,
    since_feed_window: Milliseconds,
    ticks_seen: u16,
    usb_interrupts_seen: u16,
    playback_gate: PlaybackGate<'static>,
    reports: Reports,
}

pub struct ThreadPlayback {
    pub counters: PlaybackCounterReader<'static>,
    pub gate: PlaybackGate<'static>,
}

struct Reports {
    meters: MeterReader<'static>,
    playback_counters: PlaybackCounterReader<'static>,
    coefficients: CoefficientPublisher<'static>,
    dsp_mode: &'static DspModeSwitch,
}

fn advance<State: Copy, Event, Outputs>(
    state: &mut State,
    transition: fn(State, Event) -> (State, Outputs),
    event: Event,
) -> Outputs {
    let (next, outputs) = transition(*state, event);
    *state = next;
    outputs
}

impl ThreadContext {
    pub fn start(
        coefficients: CoefficientPublisher<'static>,
        meters: MeterReader<'static>,
        playback: ThreadPlayback,
        dsp_mode: &'static DspModeSwitch,
        console: Console,
        executor: &mut Executor,
    ) -> Self {
        let (boot, first) = boot::begin();
        fault::mark(boot.place());
        let panel = Panel::claim(console.previous_boot()).unwrap_or_else(|| halt::forever());
        let mut thread = Self {
            stage: Stage::Booting(boot),
            phantom: phantom::State::Disarmed,
            supervisor: supervisor::State::START,
            audio_clock: AudioClock::Unlocked,
            usb_notices: NoticesSeen::START,
            console,
            panel,
            sequencer_bus: SequencerBus::Idle,
            boot_knobs: None,
            analog: AnalogSettings::AT_BOOT,
            since_feed_window: Milliseconds(0),
            ticks_seen: audio_dma::ticks_served(),
            usb_interrupts_seen: interrupts::usb_interrupts_served(),
            playback_gate: playback.gate,
            reports: Reports {
                meters,
                playback_counters: playback.counters,
                coefficients,
                dsp_mode,
            },
        };
        thread.run_boot_outputs(first.as_slice(), executor);
        thread
    }

    pub fn control_pass(&mut self, executor: &mut Executor) {
        executor.poll();
        while let Some(event) = executor.next_event() {
            self.on_board(event, executor);
        }
        self.take_usb_notices(executor);
        self.serve_panel(executor);
        self.serve_console(executor.bus_health());
        self.pass_ticks(executor);
        self.reports.coefficients.poll();
    }

    fn serve_console(&mut self, bus: BusHealth) {
        let status = self.status_facts();
        let facts = ConsoleFacts {
            rate: status.rate,
            running: matches!(self.stage, Stage::Running { .. }),
            knobs: match self.stage {
                Stage::Running { control, .. } => Some(control.knobs()),
                Stage::Booting(_) => self.boot_knobs,
            },
            panel: PanelFacts {
                buttons: self.panel.buttons().0,
                leds: ExpanderOutputs::showing(status.grant, status.monitoring).bits(),
                phantom: Input::BOTH.map(|input| status.grant.power(input)),
                monitoring: status.monitoring,
                phantom_rail: pins::level(OutputPin::PhantomActive.pin()),
                output_connect: pins::level(OutputPin::TransportActive.pin()),
            },
            bus,
        };
        let Reports {
            meters,
            playback_counters,
            coefficients,
            dsp_mode,
        } = &mut self.reports;
        self.console.serve(
            &facts,
            &mut ConsoleReach {
                meters,
                playback_counters,
                coefficients,
                dsp_mode,
            },
        );
    }

    fn serve_panel(&mut self, executor: &mut Executor) {
        let facts = self.status_facts();
        self.panel.poll(facts, &mut self.reports.meters);
        if self.sequencer_bus == SequencerBus::Idle
            && self.panel.is_lit()
            && let Some(command) = self.panel.take_free_bus()
            && executor.run(command) == Completion::Refused
        {
            executor.push(BoardEvent::PanelRefused);
        }
    }

    fn status_facts(&self) -> StatusFacts {
        let (rate, clock, monitoring) = match self.stage {
            Stage::Booting(_) => (BOOT_RATE, ClockLock::Changing, [Monitoring::Off; 2]),
            Stage::Running {
                rate_change,
                control,
                ..
            } => (
                rate_change.reported_rate(),
                if matches!(rate_change, rate_change::State::Steady { .. }) {
                    ClockLock::Locked
                } else {
                    ClockLock::Changing
                },
                control.monitoring(),
            ),
        };
        StatusFacts {
            rate,
            clock,
            grant: self.phantom.grant(),
            dsp: self.dsp_badge(),
            monitoring,
        }
    }

    #[cfg(feature = "sound2")]
    fn dsp_badge(&self) -> device::lcd::screen::DspBadge {
        self.console.badge()
    }

    #[cfg(not(feature = "sound2"))]
    fn dsp_badge(&self) -> device::lcd::screen::DspBadge {
        self.reports.dsp_mode.requested().into()
    }

    fn audio_may_run(&self) -> bool {
        self.audio_clock == AudioClock::Locked
            && !matches!(self.supervisor, supervisor::State::Faulted { .. })
    }

    fn audio_command_refused(&self, command: Command) -> bool {
        !self.audio_may_run()
            && matches!(
                command,
                Command::StartDma
                    | Command::StartI2s(_)
                    | Command::StartTickTimer(_)
                    | Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
            )
    }

    fn run_sequenced(&mut self, command: Command, executor: &mut Executor) -> Batch {
        let completion = if self.audio_command_refused(command) {
            Completion::Refused
        } else {
            executor.run(command)
        };
        match completion {
            Completion::AwaitingBus => {
                self.sequencer_bus = SequencerBus::Awaiting;
                self.panel.bus_preempted();
            }
            Completion::Done => {
                if command == Command::StartKnobScan {
                    self.panel.knob_scan_started();
                }
            }
            Completion::Refused | Completion::Reported => {}
        }
        Batch::of(completion)
    }

    fn route_answer(&mut self, answer: Answer, executor: &mut Executor) {
        if self.sequencer_bus == SequencerBus::Idle && self.panel.holds_bus() {
            match self.panel.answered(answer) {
                BusNews::Nothing => {}
                BusNews::Buttons(buttons) => {
                    for input in Input::BOTH {
                        self.step_phantom_noted(buttons.phantom_button_event(input), executor);
                    }
                }
                BusNews::Fault(code) => {
                    self.step_supervisor(supervisor::Event::FaultRaised(code, At::Here), executor);
                }
            }
            return;
        }
        self.sequencer_bus = SequencerBus::Idle;
        match answer {
            Answer::Acked => self.reply(
                boot::Event::I2cAcked,
                Some(rate_change::Event::I2cAcked),
                executor,
            ),
            Answer::Nacked => self.reply(
                boot::Event::I2cNacked,
                Some(rate_change::Event::I2cNacked),
                executor,
            ),
            Answer::Read(byte) => self.reply(boot::Event::StatusRead(byte), None, executor),
        }
    }

    fn run_panel_period(&mut self, executor: &mut Executor) {
        if !self.panel.is_lit() {
            return;
        }
        self.panel.want(PanelTransfer::Buttons);
        for (input, jack) in Input::BOTH.into_iter().zip(self.panel.jacks()) {
            let pin_byte = match jack {
                Jack::Inserted => 1,
                Jack::Removed => 0,
            };
            self.step_phantom_noted(
                phantom::Event::from_jack_pin_byte(input, pin_byte),
                executor,
            );
        }
        let Some(readings) = self.panel.readings() else {
            return;
        };
        match self.stage {
            Stage::Booting(boot::State::VolumeRamp(..)) => {
                let knobs = match self.boot_knobs {
                    Some(knobs) => knobs.after_scan(readings.knobs),
                    None => Knobs::first_scan(readings.knobs),
                };
                self.boot_knobs = Some(knobs);
                let targets = volume_targets(knobs);
                self.step_boot(boot::Event::ControlPassElapsed(targets), executor);
                self.finish_boot(knobs, targets);
            }
            Stage::Booting(
                boot::State::Scripted(..) | boot::State::Running(..) | boot::State::Degraded(_),
            ) => {}
            Stage::Running { .. } => {
                if self.audio_may_run() {
                    self.run_control_period(readings, executor);
                }
            }
        }
    }

    #[inline(never)]
    fn on_board(&mut self, event: BoardEvent, executor: &mut Executor) {
        match event {
            BoardEvent::StepDone => self.reply(
                boot::Event::StepDone,
                Some(rate_change::Event::StepDone),
                executor,
            ),
            BoardEvent::Bus(answer) => self.route_answer(answer, executor),
            BoardEvent::ClockInput(input) => {
                self.reply(boot::Event::ClockInputRead(input), None, executor);
            }
            BoardEvent::PllLock(pll, readiness) => self.on_pll_lock(pll, readiness, executor),
            BoardEvent::Elapsed(elapsed) => self.pass_time(elapsed, executor),
            BoardEvent::CommandRefused => match self.stage {
                Stage::Booting(_) => {
                    self.step_boot(boot::Event::CommandRefused, executor);
                }
                Stage::Running { .. } => {
                    self.step_supervisor(
                        supervisor::Event::FaultRaised(FaultCode::RefusedCommand, At::Here),
                        executor,
                    );
                }
            },
            BoardEvent::PanelRefused => {
                executor.run(Command::RecordFault(
                    FaultCode::RefusedCommand,
                    At::Here,
                    Weight::Noted,
                ));
            }
        }
    }

    fn on_pll_lock(&mut self, pll: Pll, readiness: Readiness, executor: &mut Executor) {
        let lock = match readiness {
            Readiness::Reached => PllLock::Locked,
            Readiness::TimedOut => PllLock::TimedOut,
        };
        if pll == Pll::Audio {
            self.audio_clock = match lock {
                PllLock::Locked => AudioClock::Locked,
                PllLock::TimedOut => AudioClock::Unlocked,
            };
        }
        match self.stage {
            Stage::Booting(_) => {
                self.step_boot(boot::Event::PllLockRead(pll, lock), executor);
            }
            Stage::Running { .. } => {
                if lock == PllLock::TimedOut {
                    self.step_rate_change(rate_change::Event::PllLockTimedOut, executor);
                    self.step_supervisor(
                        supervisor::Event::FaultRaised(pll.unlocked(), At::Here),
                        executor,
                    );
                }
            }
        }
    }

    fn take_usb_notices(&mut self, executor: &mut Executor) {
        if let Some(usb) = self.usb_notices.bus_changed(&handovers::USB_NOTICES) {
            self.step_supervisor(supervisor::Event::UsbBus(usb), executor);
        }
        let running = matches!(self.stage, Stage::Running { .. });
        let notices = self.usb_notices.drain(&handovers::USB_NOTICES, running);
        for notice in notices.as_slice() {
            self.on_notice(*notice, executor);
        }
    }

    fn on_notice(&mut self, notice: Notice, executor: &mut Executor) {
        match notice {
            Notice::RateChange(event) => self.step(ThreadEvent::RateChange(event), executor),
            Notice::FadeOutputsToSilence => self.set_volume(Volume::HeldAtFullAttenuation),
            Notice::ReleaseOutputSilence => self.release_usb_silence(),
            Notice::SuspendPhantom => {
                self.step(ThreadEvent::Phantom(phantom::Event::Suspend), executor);
            }
            Notice::ResumePhantom => {
                self.step(ThreadEvent::Phantom(phantom::Event::Resume), executor);
            }
            Notice::DataTerminal(terminal) => self.console.on_data_terminal(terminal),
        }
    }

    fn release_usb_silence(&mut self) {
        if let Stage::Running {
            rate_change: rate_change::State::Steady { .. },
            ..
        } = self.stage
        {
            self.set_volume(Volume::FollowsTheKnobs);
        }
    }

    fn step(&mut self, event: ThreadEvent, executor: &mut Executor) {
        match event {
            ThreadEvent::RateChange(event) => self.step_rate_change(event, executor),
            ThreadEvent::Phantom(event) => {
                self.step_phantom_noted(event, executor);
            }
            ThreadEvent::Supervisor(event) => {
                self.step_supervisor(event, executor);
            }
        }
    }

    #[inline(never)]
    fn reply(
        &mut self,
        to_boot: boot::Event,
        to_rate_change: Option<rate_change::Event>,
        executor: &mut Executor,
    ) {
        match self.stage {
            Stage::Booting(_) => self.step_boot(to_boot, executor),
            Stage::Running { .. } => {
                if let Some(event) = to_rate_change {
                    self.step_rate_change(event, executor);
                }
            }
        }
    }

    fn pass_time(&mut self, elapsed: Milliseconds, executor: &mut Executor) {
        match self.stage {
            Stage::Booting(_) => self.step_boot(boot::Event::Elapsed(elapsed), executor),
            Stage::Running { .. } => {
                self.step_rate_change(rate_change::Event::Elapsed(elapsed), executor);
            }
        }
        self.step_phantom_noted(phantom::Event::Elapsed(elapsed), executor);
        self.panel.elapsed(elapsed);
        self.since_feed_window = Milliseconds(self.since_feed_window.0.saturating_add(elapsed.0));
        if self.since_feed_window.0 >= CONTROL_PERIOD.0 {
            self.since_feed_window = Milliseconds(self.since_feed_window.0 - CONTROL_PERIOD.0);
            self.step_supervisor(supervisor::Event::ControlPassDone, executor);
            self.step_supervisor(supervisor::Event::FeedWindowEnded, executor);
            self.run_panel_period(executor);
        }
    }

    fn pass_ticks(&mut self, executor: &mut Executor) {
        let served = audio_dma::ticks_served();
        let ticks = served
            .wrapping_sub(self.ticks_seen)
            .min(u16::from(rate_change::PLAYBACK_FADE.count()));
        self.ticks_seen = served;
        if ticks > 0 {
            self.step_supervisor(supervisor::Event::TickAdvanced, executor);
        }
        let usb_served = interrupts::usb_interrupts_served();
        if usb_served != self.usb_interrupts_seen {
            self.usb_interrupts_seen = usb_served;
            self.step_supervisor(supervisor::Event::UsbServiced, executor);
        }
        for _ in 0..ticks {
            self.step_rate_change(rate_change::Event::Ticked, executor);
        }
    }

    fn finish_issued_batch(batch: Batch, refused_by_the_stage: Refusal, executor: &mut Executor) {
        match (batch, refused_by_the_stage) {
            (Batch::Empty | Batch::AwaitingBus, Refusal::Refused | Refusal::NotRefused)
            | (Batch::Refused, Refusal::NotRefused) => {}
            (Batch::Ran, Refusal::Refused | Refusal::NotRefused) => {
                executor.push(BoardEvent::StepDone)
            }
            (Batch::Refused, Refusal::Refused) => executor.push(BoardEvent::CommandRefused),
        }
    }

    #[inline(never)]
    fn step_boot(&mut self, event: boot::Event, executor: &mut Executor) {
        match &mut self.stage {
            Stage::Booting(state) => {
                if !IMAGE.runs_boot_stage(*state) {
                    return;
                }
                let writing_direction = state.awaits_expander_direction();
                let outputs = advance(state, boot::transition, event);
                fault::mark(state.place());
                if writing_direction && !state.awaits_expander_direction() {
                    self.panel.expander_ready();
                }
                if matches!(state, boot::State::VolumeRamp(..)) {
                    self.playback_gate.open();
                }
                if IMAGE.runs_boot_stage(*state) {
                    self.run_boot_outputs(outputs.as_slice(), executor);
                }
            }
            Stage::Running { .. } => {}
        }
    }

    fn run_boot_outputs(&mut self, outputs: &[boot::Output], executor: &mut Executor) {
        let mut batch = Batch::Empty;
        let refused_by_the_stage = run_stage_batch(outputs, |output| {
            let step = match output {
                boot::Output::Command(command) => self.run_sequenced(command, executor),
                boot::Output::Phantom(event) => self.step_phantom(event, executor),
                boot::Output::Supervisor(event) => self.step_supervisor(event, executor),
                boot::Output::Log(_) => Batch::Empty,
                boot::Output::StartLcd => {
                    self.panel.start();
                    Batch::Ran
                }
            };
            batch = batch.then(step);
            step.refusal()
        });
        Self::finish_issued_batch(batch, refused_by_the_stage, executor);
    }

    fn step_phantom(&mut self, event: phantom::Event, executor: &mut Executor) -> Batch {
        let outputs = advance(&mut self.phantom, phantom::transition, event);
        let mut batch = Batch::Empty;
        for output in outputs.as_slice() {
            let command = Command::from_phantom(*output, self.analog);
            batch = batch.then(Batch::of(executor.run(command)));
        }
        batch
    }

    fn step_phantom_noted(&mut self, event: phantom::Event, executor: &mut Executor) {
        if self.step_phantom(event, executor) == Batch::Refused {
            executor.push(BoardEvent::CommandRefused);
        }
    }

    fn step_supervisor(&mut self, event: supervisor::Event, executor: &mut Executor) -> Batch {
        let outputs = advance(&mut self.supervisor, supervisor::transition, event);
        let mut batch = Batch::Empty;
        for output in outputs.as_slice() {
            let step = match *output {
                supervisor::Output::Command(command) => self.run_sequenced(command, executor),
                supervisor::Output::FaultRaised(code) => {
                    self.panel.fault(code);
                    self.step_phantom(phantom::Event::FaultRaised, executor)
                }
                supervisor::Output::Nothing => Batch::Empty,
            };
            batch = batch.then(step);
        }
        batch
    }

    #[inline(never)]
    fn finish_boot(&mut self, knobs: Knobs, ramp_end: VolumeTargets) {
        match self.stage {
            Stage::Booting(boot::State::Running(adc, _)) => {
                self.stage = Stage::Running {
                    rate_change: rate_change::State::Steady {
                        rate: BOOT_RATE,
                        adc,
                    },
                    control: ControlPass::after_boot(knobs, ramp_end),
                    volume: Volume::FollowsTheKnobs,
                };
            }
            Stage::Booting(
                boot::State::Scripted(..) | boot::State::VolumeRamp(..) | boot::State::Degraded(_),
            )
            | Stage::Running { .. } => {}
        }
    }

    fn step_rate_change(&mut self, event: rate_change::Event, executor: &mut Executor) {
        let Stage::Running { rate_change, .. } = &mut self.stage else {
            return;
        };
        let outputs = advance(rate_change, rate_change::transition, event);
        handovers::report_clock(rate_change.report());
        let mut batch = Batch::Empty;
        let refused_by_the_stage = run_stage_batch(outputs.as_slice(), |output| {
            let step = match output {
                rate_change::Output::Board(command) => self.run_sequenced(command, executor),
                rate_change::Output::RaiseFault(code) => {
                    self.step_supervisor(supervisor::Event::FaultRaised(code, At::Here), executor)
                }
                rate_change::Output::ResetStreams(rate) => {
                    handovers::switch_rate(rate, self.console.limiter());
                    Batch::Ran
                }
                rate_change::Output::SwapCoefficients(rate) => {
                    self.reports.coefficients.change_rate(rate);
                    Batch::Ran
                }
                rate_change::Output::ClearDspState => {
                    handovers::clear_dsp_state();
                    Batch::Ran
                }
                rate_change::Output::HoldVolumeAtFull => {
                    self.set_volume(Volume::HeldAtFullAttenuation);
                    Batch::Ran
                }
                rate_change::Output::RampVolumeFromFull => {
                    self.set_volume(Volume::FollowsTheKnobs);
                    Batch::Ran
                }
                rate_change::Output::SendAnalogWordAtSpeed(speed) => {
                    self.analog.speed = speed;
                    let word = self.analog.word(self.phantom.grant());
                    self.run_sequenced(Command::SendAnalogWord(word), executor)
                }
                rate_change::Output::ClockValidity(_) => Batch::Empty,
            };
            batch = batch.then(step);
            step.refusal()
        });
        Self::finish_issued_batch(batch, refused_by_the_stage, executor);
    }

    fn set_volume(&mut self, next: Volume) {
        let may_run = self.audio_may_run();
        if let Stage::Running {
            control, volume, ..
        } = &mut self.stage
        {
            match (*volume, next) {
                (Volume::HeldAtFullAttenuation, Volume::FollowsTheKnobs) => {
                    if may_run {
                        *control = control.with_volume_from_full_attenuation();
                        *volume = next;
                    }
                }
                (
                    Volume::FollowsTheKnobs | Volume::HeldAtFullAttenuation,
                    Volume::HeldAtFullAttenuation,
                ) => {
                    *volume = next;
                }
                (Volume::FollowsTheKnobs, Volume::FollowsTheKnobs) => {}
            }
        }
    }

    fn run_control_period(&mut self, readings: PanelReadings, executor: &mut Executor) {
        #[cfg(feature = "sound2")]
        let rate = self.status_facts().rate;
        match &mut self.stage {
            Stage::Booting(_) => {}
            Stage::Running {
                control, volume, ..
            } => {
                let (next, outputs) = control.run(readings, self.analog, self.phantom.grant());
                *control = next;
                self.analog = outputs.analog;
                handovers::publish_tick_gains(outputs.tick.capture, outputs.tick.monitor);
                #[cfg(feature = "sound2")]
                if outputs.long_presses[1] {
                    self.console.toggle_preset(
                        &mut self.reports.coefficients,
                        self.reports.dsp_mode,
                        rate,
                        true,
                    );
                }
                let outputs = match volume {
                    Volume::FollowsTheKnobs => outputs,
                    Volume::HeldAtFullAttenuation => outputs.holding_volume_at_full(),
                };
                let mut batch = Batch::Empty;
                for command in outputs.commands.as_slice() {
                    match PanelTransfer::of_pass_command(*command) {
                        Some(transfer) => self.panel.want(transfer),
                        None => batch = batch.then(Batch::of(executor.run(*command))),
                    }
                }
                if batch == Batch::Refused {
                    executor.push(BoardEvent::CommandRefused);
                }
            }
        }
    }
}
