mod rate_change_support;

use device::adc::{AdcAddress, AdcKind};
use device::boot::{self, BootRecord};
use device::breadcrumb::At;
use device::command::Command;
use device::fault::FaultCode;
use device::output_pin::OutputPin;
use device::outputs::Filler;
use device::phantom::{self, PinLevel};
use device::rate_change::{self, ClockValidity, Event, State};
use device::stage_batch::{BatchRole, Refusal, StageOutput, run_stage_batch};
use device::supervisor;
use dsp::{FramesPerTick, RateFamily, SampleRate};
use rate_change_support::{ADC_KINDS, run_until_settled, step};

fn rate_output_issues_a_stage_command(output: rate_change::Output) -> bool {
    match output {
        rate_change::Output::Board(_) | rate_change::Output::SendAnalogWordAtSpeed(_) => true,
        rate_change::Output::HoldVolumeAtFull
        | rate_change::Output::RampVolumeFromFull
        | rate_change::Output::ResetStreams(_)
        | rate_change::Output::SwapCoefficients(_)
        | rate_change::Output::ClearDspState
        | rate_change::Output::ClockValidity(_)
        | rate_change::Output::RaiseFault(_) => false,
    }
}

fn boot_output_issues_a_stage_command(output: boot::Output) -> bool {
    match output {
        boot::Output::Command(_) => true,
        boot::Output::Phantom(_)
        | boot::Output::Supervisor(_)
        | boot::Output::Log(_)
        | boot::Output::StartLcd => false,
    }
}

fn boot_output_drives_the_board(output: boot::Output) -> bool {
    match output {
        boot::Output::Command(_)
        | boot::Output::Phantom(_)
        | boot::Output::Supervisor(_)
        | boot::Output::StartLcd => true,
        boot::Output::Log(_) => false,
    }
}

fn every_rate_output() -> [rate_change::Output; 9] {
    [
        rate_change::Output::Board(Command::StopDma),
        rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Six),
        rate_change::Output::HoldVolumeAtFull,
        rate_change::Output::RampVolumeFromFull,
        rate_change::Output::ResetStreams(SampleRate::Hz44100),
        rate_change::Output::SwapCoefficients(SampleRate::Hz44100),
        rate_change::Output::ClearDspState,
        rate_change::Output::ClockValidity(ClockValidity::Invalid),
        rate_change::Output::RaiseFault(FaultCode::BoardStalled),
    ]
}

fn every_boot_output() -> [boot::Output; 5] {
    [
        boot::Output::Command(Command::StopDma),
        boot::Output::Phantom(phantom::Event::BootReachedRunning),
        boot::Output::Supervisor(supervisor::Event::TransportStarted),
        boot::Output::Log(BootRecord::AdcDetected(AdcKind::Unanswered)),
        boot::Output::StartLcd,
    ]
}

fn run_refusing_call<Output: StageOutput>(
    outputs: &[Output],
    refused_call: Option<usize>,
) -> (Vec<Output>, Refusal) {
    let mut ran = Vec::new();
    let refusal = run_stage_batch(outputs, |output| {
        let call = ran.len();
        ran.push(output);
        if Some(call) == refused_call {
            Refusal::Refused
        } else {
            Refusal::NotRefused
        }
    });
    (ran, refusal)
}

fn expected_after_a_stage_refusal<Output: Copy>(
    outputs: &[Output],
    refused_at: usize,
    issues_a_stage_command: fn(Output) -> bool,
) -> Vec<Output> {
    let mut expected = outputs[..=refused_at].to_vec();
    expected.extend(
        outputs[refused_at + 1..]
            .iter()
            .copied()
            .filter(|output| !issues_a_stage_command(*output)),
    );
    expected
}

fn every_rate_batch_the_machine_emits() -> Vec<Vec<rate_change::Output>> {
    let mut batches = Vec::new();
    for adc in ADC_KINDS {
        for from in SampleRate::ALL {
            for to in SampleRate::ALL {
                let (state, outputs) = step(State::Steady { rate: from, adc }, Event::SetCur(to));
                batches.push(outputs);
                let mut trace = Vec::new();
                run_until_settled(state, &mut trace);
                batches.extend(trace.into_iter().map(|(_, _, outputs)| outputs));
            }
        }
    }
    batches.retain(|batch| !batch.is_empty());
    batches
}

#[test]
fn every_output_variant_names_its_own_batch_role() {
    assert_eq!(
        boot::Output::Command(Command::StopDma).batch_role(),
        BatchRole::StageCommand
    );
    assert_eq!(
        boot::Output::Phantom(phantom::Event::BootReachedRunning).batch_role(),
        BatchRole::StageSafetyEvent
    );
    assert_eq!(
        boot::Output::Supervisor(supervisor::Event::TransportStarted).batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        boot::Output::Log(BootRecord::AdcDetected(AdcKind::Unanswered)).batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        boot::Output::StartLcd.batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::Board(Command::StopDma).batch_role(),
        BatchRole::StageCommand
    );
    assert_eq!(
        rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Six).batch_role(),
        BatchRole::StageCommand
    );
    assert_eq!(
        rate_change::Output::HoldVolumeAtFull.batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::RampVolumeFromFull.batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::ResetStreams(SampleRate::Hz44100).batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::SwapCoefficients(SampleRate::Hz44100).batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::ClearDspState.batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::ClockValidity(ClockValidity::Invalid).batch_role(),
        BatchRole::OutsideTheStage
    );
    assert_eq!(
        rate_change::Output::RaiseFault(FaultCode::BoardStalled).batch_role(),
        BatchRole::OutsideTheStage
    );
}

#[test]
fn the_analog_word_does_not_run_after_the_adc_enable_command_is_refused() {
    let start = State::Steady {
        rate: SampleRate::Hz48000,
        adc: AdcKind::Unanswered,
    };
    let (state, _) = step(start, Event::SetCur(SampleRate::Hz44100));
    let mut trace = Vec::new();
    run_until_settled(state, &mut trace);
    let speed_word_batch = trace
        .into_iter()
        .map(|(_, _, outputs)| outputs)
        .find(|outputs| {
            outputs
                .iter()
                .any(|output| matches!(output, rate_change::Output::SendAnalogWordAtSpeed(_)))
        })
        .expect("a rate change with an unanswered ADC sends the analog word");
    assert_eq!(
        speed_word_batch,
        [
            rate_change::Output::Board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::Low)),
            rate_change::Output::SendAnalogWordAtSpeed(SampleRate::Hz44100.frames_per_tick()),
        ]
    );

    let (ran, refusal) = run_refusing_call(&speed_word_batch, Some(0));

    assert_eq!(
        ran,
        [rate_change::Output::Board(Command::DrivePin(
            OutputPin::AdcEnable,
            PinLevel::Low
        ))]
    );
    assert_eq!(refusal, Refusal::Refused);
}

#[test]
fn after_a_refused_board_command_only_rate_outputs_outside_the_stage_still_run() {
    for later in every_rate_output() {
        let batch = [rate_change::Output::Board(Command::StopTickTimer), later];

        let (ran, refusal) = run_refusing_call(&batch, Some(0));

        assert_eq!(
            ran,
            expected_after_a_stage_refusal(&batch, 0, rate_output_issues_a_stage_command),
            "{later:?} after a refused board command"
        );
        assert_eq!(refusal, Refusal::Refused);
    }
}

#[test]
fn a_refused_analog_word_stops_every_later_stage_command_in_the_rate_batch() {
    let batch = [
        rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Twelve),
        rate_change::Output::Board(Command::DrivePin(OutputPin::AdcEnable, PinLevel::High)),
        rate_change::Output::Board(Command::SwitchPll0Family(
            RateFamily::FortyFourPointOneKilohertz,
        )),
        rate_change::Output::ResetStreams(SampleRate::Hz88200),
        rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Twelve),
        rate_change::Output::RampVolumeFromFull,
    ];

    let (ran, refusal) = run_refusing_call(&batch, Some(0));

    assert_eq!(
        ran,
        [
            rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Twelve),
            rate_change::Output::ResetStreams(SampleRate::Hz88200),
            rate_change::Output::RampVolumeFromFull,
        ]
    );
    assert_eq!(refusal, Refusal::Refused);
}

#[test]
fn in_every_rate_batch_the_machine_emits_no_stage_command_runs_after_a_refusal() {
    let mut refusals_tried = 0;
    let mut stage_commands_stopped = 0;
    for batch in every_rate_batch_the_machine_emits() {
        let (ran, refusal) = run_refusing_call(&batch, None);
        assert_eq!(ran, batch);
        assert_eq!(refusal, Refusal::NotRefused);

        for refused_at in 0..batch.len() {
            if !rate_output_issues_a_stage_command(batch[refused_at]) {
                continue;
            }
            let (ran, refusal) = run_refusing_call(&batch, Some(refused_at));
            let expected = expected_after_a_stage_refusal(
                &batch,
                refused_at,
                rate_output_issues_a_stage_command,
            );
            assert_eq!(ran, expected, "{batch:?} refused at {refused_at}");
            assert_eq!(refusal, Refusal::Refused);
            refusals_tried += 1;
            stage_commands_stopped += batch.len() - expected.len();
        }
    }
    assert!(refusals_tried > 0);
    assert!(stage_commands_stopped > 0);
}

#[test]
fn a_refused_rate_fault_report_is_not_the_stages_refusal_and_stops_nothing() {
    let batch = [
        rate_change::Output::RaiseFault(FaultCode::BoardStalled),
        rate_change::Output::Board(Command::StopDma),
        rate_change::Output::SendAnalogWordAtSpeed(FramesPerTick::Six),
    ];

    let (ran, refusal) = run_refusing_call(&batch, Some(0));

    assert_eq!(ran, batch);
    assert_eq!(refusal, Refusal::NotRefused);
}

#[test]
fn after_a_refused_boot_command_phantom_supervisor_log_and_lcd_still_run() {
    for later in every_boot_output() {
        let batch = [boot::Output::Command(Command::StopTickTimer), later];

        let (ran, refusal) = run_refusing_call(&batch, Some(0));

        assert_eq!(
            ran,
            expected_after_a_stage_refusal(&batch, 0, boot_output_issues_a_stage_command),
            "{later:?} after a refused boot command"
        );
        assert_eq!(refusal, Refusal::Refused);
    }
}

#[test]
fn a_refused_phantom_step_stops_the_later_boot_commands() {
    let batch = [
        boot::Output::Phantom(phantom::Event::Boot),
        boot::Output::Command(Command::StopDma),
        boot::Output::Supervisor(supervisor::Event::TransportStarted),
        boot::Output::Command(Command::StopI2s),
    ];

    let (ran, refusal) = run_refusing_call(&batch, Some(0));

    assert_eq!(
        ran,
        [
            boot::Output::Phantom(phantom::Event::Boot),
            boot::Output::Supervisor(supervisor::Event::TransportStarted),
        ]
    );
    assert_eq!(refusal, Refusal::Refused);
}

#[test]
fn a_refused_supervisor_step_in_boot_is_not_the_stages_refusal_and_stops_nothing() {
    let batch = [
        boot::Output::Supervisor(supervisor::Event::FaultRaised(
            FaultCode::BoardStalled,
            At::Here,
        )),
        boot::Output::Command(Command::StopDma),
        boot::Output::Log(BootRecord::AdcDetected(AdcKind::Answered(AdcAddress::At13))),
    ];

    let (ran, refusal) = run_refusing_call(&batch, Some(0));

    assert_eq!(ran, batch);
    assert_eq!(refusal, Refusal::NotRefused);
}

#[test]
fn the_boot_filler_is_outside_the_stage_and_drives_nothing() {
    let filler = boot::Output::FILLER;

    assert_eq!(filler.batch_role(), BatchRole::OutsideTheStage);
    assert!(!boot_output_issues_a_stage_command(filler), "{filler:?}");
    assert_ne!(filler, boot::Output::StartLcd);
    assert!(!boot_output_drives_the_board(filler), "{filler:?}");
}
