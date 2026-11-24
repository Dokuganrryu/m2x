mod boot_board;

use boot_board::{Bench, Seen, is_transfer, notes, raises, run_bench};
use device::adc::{AdcAddress, AdcKind, AdcWrite};
use device::analog_word::AnalogWord;
use device::boot::{
    BootRecord, ClockInput, Course, Faults, Outcome, Output, State, Subsystem, Subsystems, begin,
};
use device::breadcrumb::{At, Place, Weight};
use device::command::Command;
use device::dac::{
    Attenuation, ConfigRegister, DacWrite, MasterTrim, StatusRegister, VolumeRegister,
};
use device::fault::{FaultCode, I2cPeer};
use device::output_pin::OutputPin;
use device::phantom::{self, Milliseconds, PinLevel};
use dsp::{FramesPerTick, RateFamily};

const ENOUGH_PASSES: usize = 5000;

fn running(adc: AdcKind) -> State {
    State::Running(
        adc,
        Outcome {
            faults: Faults::NONE,
            running: Subsystems::NONE
                .with(Subsystem::CorePll)
                .with(Subsystem::Usb)
                .with(Subsystem::AnalogChain)
                .with(Subsystem::Rails)
                .with(Subsystem::Panel)
                .with(Subsystem::Audio),
            course: Course::Whole,
        },
    )
}

fn c(command: Command) -> Seen {
    Seen::Command(command)
}

fn pin(pin: OutputPin, level: PinLevel) -> Seen {
    c(Command::DrivePin(pin, level))
}

fn waited(ms: u32) -> Seen {
    Seen::Waited(Milliseconds(ms))
}

fn held(ms: u32) -> Seen {
    Seen::Waited(Milliseconds(ms + 1))
}

fn volume(register: VolumeRegister, steps: u8) -> Seen {
    c(Command::WriteDac(DacWrite::Volume(
        register,
        Attenuation::from_half_decibel_steps(steps),
    )))
}

fn config(register: ConfigRegister) -> Seen {
    c(Command::WriteDac(DacWrite::Config(register)))
}

fn status_read_logged(byte: u8) -> [Seen; 2] {
    [
        c(Command::ReadDac(StatusRegister::Register113)),
        Seen::Log(BootRecord::DacStatus(byte)),
    ]
}

fn boot_table_through_the_panel(adc_init: &[Seen], detected: AdcKind) -> Vec<Seen> {
    let mut expected = vec![
        pin(OutputPin::TransportActive, PinLevel::Low),
        pin(OutputPin::SecondBringUp, PinLevel::Low),
        pin(OutputPin::FirstBringUp, PinLevel::Low),
        pin(OutputPin::ThirdBringUp, PinLevel::Low),
        c(Command::DisableInterrupts),
        c(Command::StopDma),
        c(Command::StopTickTimer),
        c(Command::StopI2s),
        c(Command::DetachUsb),
        c(Command::FeedWatchdog),
        c(Command::FenceFlashController),
        c(Command::ArmWatchdog),
        c(Command::StartClockInput),
        c(Command::ReadClockInput),
        c(Command::StartCorePll),
        held(1),
        c(Command::SelectCorePllAsMainClock),
        c(Command::AttachUsb),
        c(Command::ConfigureAnalogWordBus),
        c(Command::SendAnalogWord(AnalogWord::ALL_OFF)),
        pin(OutputPin::PhantomActive, PinLevel::Low),
        pin(OutputPin::ConverterEnable, PinLevel::Low),
        pin(OutputPin::AdcEnable, PinLevel::Low),
        pin(OutputPin::FirstBringUp, PinLevel::High),
        held(5),
        pin(OutputPin::SecondBringUp, PinLevel::High),
        held(5),
        pin(OutputPin::ThirdBringUp, PinLevel::High),
        held(100),
        c(Command::SendAnalogWord(AnalogWord::ALL_OFF)),
        pin(OutputPin::PhantomActive, PinLevel::Low),
        c(Command::ConfigureConverterBus),
        Seen::StartLcd,
        c(Command::SwitchPll0Family(RateFamily::FortyEightKilohertz)),
        held(3),
        c(Command::EnableMasterClockOutput),
    ];
    expected.extend(VolumeRegister::ALL.map(|r| volume(r, 0xFF)));
    expected.extend([
        pin(OutputPin::AdcEnable, PinLevel::Low),
        held(5),
        pin(OutputPin::ConverterEnable, PinLevel::High),
        pin(OutputPin::AdcEnable, PinLevel::High),
        held(100),
        c(Command::ProbeAdc(AdcAddress::At13)),
        held(1),
        c(Command::ProbeAdc(AdcAddress::At4F)),
        Seen::Log(BootRecord::AdcDetected(detected)),
        c(Command::ZeroTransmitBuffers),
        c(Command::StartDma),
        c(Command::StartI2s(FramesPerTick::Six)),
        c(Command::StartTickTimer(FramesPerTick::Six)),
        config(ConfigRegister::Register2),
        config(ConfigRegister::Register1),
        config(ConfigRegister::Register7),
        config(ConfigRegister::Register10),
        config(ConfigRegister::Register12),
        config(ConfigRegister::Register13),
        config(ConfigRegister::Register38),
        config(ConfigRegister::Register39),
        config(ConfigRegister::Register40),
        config(ConfigRegister::Register41),
        config(ConfigRegister::Register8),
        config(ConfigRegister::Register46),
        c(Command::WriteDac(DacWrite::MasterTrim(
            MasterTrim::MINUS_ONE_DB,
        ))),
        config(ConfigRegister::Register47),
        config(ConfigRegister::Register48),
        config(ConfigRegister::Register49),
        config(ConfigRegister::Register50),
        config(ConfigRegister::Register51),
        config(ConfigRegister::Register52),
        config(ConfigRegister::Register63),
    ]);
    for _ in 0..3 {
        expected.extend(status_read_logged(0x5A));
    }
    expected.push(held(100));
    expected.extend(VolumeRegister::ALL.map(|r| volume(r, 0xFF)));
    expected.push(pin(OutputPin::TransportActive, PinLevel::High));
    expected.extend_from_slice(adc_init);
    expected.extend([
        c(Command::WriteExpanderDirection),
        c(Command::StartKnobScan),
    ]);
    expected
}

fn place(trace: &[Seen], wanted: Seen) -> usize {
    trace
        .iter()
        .position(|seen| *seen == wanted)
        .unwrap_or_else(|| panic!("{wanted:?} never happened"))
}

fn through_the_panel(trace: &[Seen]) -> &[Seen] {
    &trace[..=place(trace, c(Command::StartKnobScan))]
}

fn after_the_panel(trace: &[Seen]) -> &[Seen] {
    &trace[through_the_panel(trace).len()..]
}

#[test]
fn boot_begins_by_quiescing_what_the_factory_image_left_running_and_arming_the_watchdog() {
    let (_, outputs) = begin();
    assert_eq!(
        outputs.as_slice(),
        &[
            Output::Command(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::SecondBringUp, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::FirstBringUp, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::ThirdBringUp, PinLevel::Low)),
            Output::Command(Command::DisableInterrupts),
            Output::Command(Command::StopDma),
            Output::Command(Command::StopTickTimer),
            Output::Command(Command::StopI2s),
            Output::Command(Command::DetachUsb),
            Output::Command(Command::FeedWatchdog),
            Output::Command(Command::FenceFlashController),
            Output::Command(Command::ArmWatchdog),
        ]
    );
}

#[test]
fn quiesce_drives_the_three_rail_pins_low_right_after_p0_28() {
    let (_, outputs) = begin();
    assert_eq!(
        outputs.as_slice()[..4],
        [
            Output::Command(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::SecondBringUp, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::FirstBringUp, PinLevel::Low)),
            Output::Command(Command::DrivePin(OutputPin::ThirdBringUp, PinLevel::Low)),
        ]
    );
}

#[test]
fn with_every_answer_the_command_order_and_delays_equal_the_d9_table_for_an_adc_at_13() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let adc = [
        c(Command::WriteAdc(AdcWrite::At13Register0)),
        c(Command::WriteAdc(AdcWrite::At13Register1)),
        c(Command::WriteAdc(AdcWrite::At13Register2)),
        c(Command::WriteAdc(AdcWrite::At13Register4)),
    ];
    assert_eq!(
        through_the_panel(&caller.trace),
        boot_table_through_the_panel(&adc, AdcKind::Answered(AdcAddress::At13)).as_slice()
    );
    assert_eq!(caller.boot, running(AdcKind::Answered(AdcAddress::At13)));
}

#[test]
fn an_adc_answering_only_at_4f_gets_the_type_2_write() {
    let bench = Bench {
        acks_at_13: false,
        acks_at_4f: true,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let adc = [c(Command::WriteAdc(AdcWrite::At4FRegister1))];
    assert_eq!(
        through_the_panel(&caller.trace),
        boot_table_through_the_panel(&adc, AdcKind::Answered(AdcAddress::At4F)).as_slice()
    );
}

#[test]
fn an_adc_answering_at_both_addresses_is_the_part_at_13() {
    let bench = Bench {
        acks_at_13: true,
        acks_at_4f: true,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_eq!(caller.boot, running(AdcKind::Answered(AdcAddress::At13)));
}

#[test]
fn with_no_adc_answering_the_type_0_init_pulses_the_adc_enable_pin() {
    let bench = Bench {
        acks_at_13: false,
        acks_at_4f: false,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let adc = [
        pin(OutputPin::AdcEnable, PinLevel::Low),
        held(5),
        pin(OutputPin::AdcEnable, PinLevel::High),
    ];
    assert_eq!(
        through_the_panel(&caller.trace),
        boot_table_through_the_panel(&adc, AdcKind::Unanswered).as_slice()
    );
    assert_eq!(caller.boot, running(AdcKind::Unanswered));
}

#[test]
fn the_all_off_word_precedes_every_command_that_reaches_the_analog_chain() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let first_word = caller
        .trace
        .iter()
        .position(|seen| matches!(seen, Seen::Command(Command::SendAnalogWord(_))))
        .unwrap();
    assert_eq!(
        caller.trace[first_word],
        c(Command::SendAnalogWord(AnalogWord::ALL_OFF))
    );
    assert_eq!(
        caller.trace[first_word - 1],
        c(Command::ConfigureAnalogWordBus)
    );
    assert!(caller.trace[..first_word - 1].iter().all(|seen| matches!(
        seen,
        Seen::Waited(_)
            | Seen::Command(
                Command::DrivePin(
                    OutputPin::TransportActive
                        | OutputPin::SecondBringUp
                        | OutputPin::FirstBringUp
                        | OutputPin::ThirdBringUp,
                    PinLevel::Low
                ) | Command::DisableInterrupts
                    | Command::StopDma
                    | Command::StopTickTimer
                    | Command::StopI2s
                    | Command::DetachUsb
                    | Command::FeedWatchdog
                    | Command::FenceFlashController
                    | Command::ArmWatchdog
                    | Command::StartClockInput
                    | Command::ReadClockInput
                    | Command::StartCorePll
                    | Command::SelectCorePllAsMainClock
                    | Command::AttachUsb
            )
    )));
}

#[test]
fn the_dac_init_writes_the_minus_one_decibel_trim_once_and_logs_three_status_bytes() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let registers: Vec<u8> = caller
        .trace
        .iter()
        .filter_map(|seen| match seen {
            Seen::Command(Command::WriteDac(write)) => Some(write.transactions()),
            _ => None,
        })
        .flat_map(|t| t.as_slice().to_vec())
        .map(|t| t.register.number())
        .collect();
    assert!(!registers.contains(&0));
    assert!(!registers.contains(&62));
    let trims: Vec<_> = caller
        .trace
        .iter()
        .filter(|seen| {
            matches!(
                seen,
                Seen::Command(Command::WriteDac(DacWrite::MasterTrim(_)))
            )
        })
        .collect();
    assert_eq!(
        trims,
        vec![&c(Command::WriteDac(DacWrite::MasterTrim(
            MasterTrim::new(0x7214_82BF).unwrap()
        )))]
    );
    let logged: Vec<_> = caller
        .trace
        .iter()
        .filter(|seen| matches!(seen, Seen::Log(BootRecord::DacStatus(_))))
        .collect();
    assert_eq!(logged, vec![&Seen::Log(BootRecord::DacStatus(0x5A)); 3]);
}

#[test]
fn the_volume_ramp_moves_each_register_one_half_decibel_per_control_pass_toward_its_target() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let ramp = after_the_panel(&caller.trace);
    let first_pass = [
        volume(VolumeRegister::FactoryZeroA, 0xFE),
        volume(VolumeRegister::FactoryZeroB, 0xFE),
        volume(VolumeRegister::MainA, 0xFE),
        volume(VolumeRegister::MainB, 0xFE),
        volume(VolumeRegister::HeadphoneA, 0xFE),
        volume(VolumeRegister::HeadphoneB, 0xFE),
    ];
    let writes: Vec<Seen> = ramp
        .iter()
        .filter(|seen| matches!(seen, Seen::Command(_)))
        .copied()
        .collect();
    assert_eq!(&writes[..6], &first_pass);
    let gaps: Vec<_> = ramp
        .iter()
        .filter(|seen| matches!(seen, Seen::Waited(_)))
        .collect();
    assert!(gaps.iter().skip(1).all(|gap| **gap == waited(5)));
    let last_writes = |register| {
        writes
            .iter()
            .rev()
            .find_map(|seen| match seen {
                Seen::Command(Command::WriteDac(DacWrite::Volume(r, level))) if *r == register => {
                    Some(level.half_decibel_steps())
                }
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(last_writes(VolumeRegister::FactoryZeroA), 0x00);
    assert_eq!(last_writes(VolumeRegister::FactoryZeroB), 0x00);
    assert_eq!(last_writes(VolumeRegister::MainA), 0xF0);
    assert_eq!(last_writes(VolumeRegister::MainB), 0xF0);
    assert_eq!(last_writes(VolumeRegister::HeadphoneA), 0xF8);
    assert_eq!(last_writes(VolumeRegister::HeadphoneB), 0xF8);
    assert_eq!(writes.len(), 2 * 0xFF + 2 * 0x0F + 2 * 0x07);
    assert!(writes.iter().all(|seen| !matches!(
        seen,
        Seen::Command(Command::WriteDac(DacWrite::Volume(
            VolumeRegister::FactoryHeldA | VolumeRegister::FactoryHeldB,
            _
        )))
    )));
}

#[test]
fn reaching_running_hands_the_phantom_machine_its_running_event() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    assert!(matches!(caller.phantom, phantom::State::Running(..)));
}

#[test]
fn one_nack_is_retried_with_the_same_write_and_the_boot_carries_on() {
    let mut nack_once = boot_board::Responder::new(Bench::ALL_ACK);
    let (mut caller, mut emitted) = boot_board::Caller::new();
    let mut refused = false;
    while !matches!(caller.boot, State::Running(..) | State::Degraded(_)) {
        let first_config = emitted
            == [Command::WriteDac(DacWrite::Config(
                ConfigRegister::Register2,
            ))];
        emitted = if first_config && !refused {
            refused = true;
            caller.feed(device::boot::Event::I2cNacked)
        } else {
            match nack_once.answer(&emitted) {
                Some(event) => caller.feed(event),
                None => caller.main_loop_pass(&Bench::ALL_ACK),
            }
        };
    }
    let resent = caller
        .trace
        .iter()
        .filter(|seen| **seen == config(ConfigRegister::Register2))
        .count();
    assert_eq!(resent, 2);
    assert!(matches!(caller.boot, State::Running(..)));
}

fn recorded(caller: &boot_board::Caller, code: FaultCode) -> bool {
    caller.log.iter().any(|seen| raises(*seen, code))
}

fn noted(caller: &boot_board::Caller, code: FaultCode) -> bool {
    caller.trace.iter().any(|seen| notes(*seen, code))
}

fn stored_any(caller: &boot_board::Caller) -> bool {
    caller.log.iter().any(|seen| {
        matches!(
            seen,
            Seen::Command(Command::RecordFault(_, _, Weight::Raised))
        )
    })
}

fn transfers_with_every_answer() -> Vec<Command> {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    through_the_panel(&caller.trace)
        .iter()
        .filter_map(|seen| match seen {
            Seen::Command(command) if is_transfer(*command) => Some(*command),
            _ => None,
        })
        .collect()
}

#[test]
fn a_persistent_nack_at_each_i2c_step_records_the_code_and_keeps_the_boot_going() {
    let transfers = transfers_with_every_answer();
    let first_probe = transfers
        .iter()
        .position(|t| matches!(t, Command::ProbeAdc(_)))
        .unwrap();
    for (index, transfer) in transfers.iter().enumerate() {
        let caller = run_bench(
            Bench {
                unanswered_transfer: Some(index + 1),
                ..Bench::ALL_ACK
            },
            ENOUGH_PASSES,
        );
        let in_attenuate = index < first_probe;
        let is_probe = matches!(transfer, Command::ProbeAdc(_));
        let attempts = caller
            .trace
            .iter()
            .filter(|seen| **seen == c(*transfer))
            .count();
        if is_probe {
            assert!(
                matches!(caller.boot, State::Running(_, _)),
                "transfer {index} {transfer:?}"
            );
            assert!(
                !caller
                    .supervised
                    .iter()
                    .any(|command| matches!(command, Command::RecordFault(_, _, Weight::Raised))),
                "transfer {index} {transfer:?}"
            );
            continue;
        }
        let peer = match transfer {
            Command::WriteDac(_) | Command::ReadDac(_) => I2cPeer::Dac,
            Command::WriteAdc(_) => I2cPeer::Adc,
            _ => I2cPeer::Expander,
        };
        if in_attenuate {
            assert!(
                noted(&caller, FaultCode::Unanswered(peer)) && !stored_any(&caller),
                "transfer {index} {transfer:?}: a skipped attenuation write is noted, not stored"
            );
        } else if peer == I2cPeer::Expander {
            assert!(
                noted(&caller, FaultCode::Unanswered(peer)) && !stored_any(&caller),
                "transfer {index} {transfer:?}: an expander fault is noted, not stored"
            );
        } else {
            assert!(
                recorded(&caller, FaultCode::Unanswered(peer)),
                "transfer {index} {transfer:?}"
            );
        }
        let sent_earlier = transfers[..index].iter().filter(|t| *t == transfer).count();
        let sent_later = transfers[index + 1..]
            .iter()
            .filter(|t| *t == transfer)
            .count();
        let expected = if in_attenuate {
            4 + sent_earlier + sent_later
        } else {
            4 + sent_earlier
        };
        assert_eq!(attempts, expected, "transfer {index} {transfer:?}");
        assert!(
            caller.trace.contains(&Seen::StartLcd),
            "transfer {index} {transfer:?}"
        );
        assert!(
            caller.trace.contains(&c(Command::AttachUsb)),
            "transfer {index} {transfer:?}"
        );
        if peer == I2cPeer::Expander || in_attenuate {
            assert!(
                matches!(caller.boot, State::Running(_, _)),
                "transfer {index} {transfer:?}"
            );
        } else {
            assert!(
                matches!(caller.boot, State::Degraded(_)),
                "transfer {index} {transfer:?}"
            );
        }
    }
}

#[test]
fn a_crystal_that_never_starts_stores_the_clock_input_code_before_the_core_pll_starts() {
    let dead_crystal = Bench {
        clock_input: ClockInput::NotReady,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(dead_crystal, 1_000);
    let State::Degraded(outcome) = caller.boot else {
        panic!("a dead crystal must end degraded, not {:?}", caller.boot);
    };
    assert!(outcome.faults.contains(FaultCode::CrystalNotReady));
    assert_eq!(outcome.faults.len(), 1);
    assert!(!outcome.runs(Subsystem::Audio));
    assert!(!outcome.runs(Subsystem::Usb));
    assert!(outcome.runs(Subsystem::Panel));
    assert!(caller.supervised.contains(&Command::RecordFault(
        FaultCode::CrystalNotReady,
        At::Step(Place { stage: 2, step: 1 }),
        Weight::Raised
    )));
    assert!(caller.supervised.contains(&Command::DrivePin(
        OutputPin::TransportActive,
        PinLevel::Low
    )));
    assert!(
        caller
            .trace
            .contains(&Seen::Command(Command::StartClockInput))
    );
    assert!(!caller.trace.contains(&Seen::Command(Command::StartCorePll)));
    assert_eq!(FaultCode::CrystalNotReady.number(), 12);
}

#[test]
fn usb_attaches_right_after_the_clock_stage() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let trace = &caller.trace;
    let attach = place(trace, c(Command::AttachUsb));
    assert_eq!(trace[attach - 1], c(Command::SelectCorePllAsMainClock));
    assert_eq!(trace[attach + 1], c(Command::ConfigureAnalogWordBus));
    assert_eq!(
        &trace[attach - 5..attach],
        &[
            c(Command::StartClockInput),
            c(Command::ReadClockInput),
            c(Command::StartCorePll),
            held(1),
            c(Command::SelectCorePllAsMainClock),
        ]
    );
    assert_eq!(trace[attach - 6], c(Command::ArmWatchdog));
    assert_eq!(
        trace
            .iter()
            .filter(|seen| **seen == c(Command::AttachUsb))
            .count(),
        1
    );
}

#[test]
fn the_screen_starts_right_after_the_rails() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let trace = &caller.trace;
    let screen = place(trace, Seen::StartLcd);
    assert_eq!(
        &trace[screen - 5..screen],
        &[
            pin(OutputPin::ThirdBringUp, PinLevel::High),
            held(100),
            c(Command::SendAnalogWord(AnalogWord::ALL_OFF)),
            pin(OutputPin::PhantomActive, PinLevel::Low),
            c(Command::ConfigureConverterBus),
        ]
    );
    assert_eq!(
        trace[screen + 1],
        c(Command::SwitchPll0Family(RateFamily::FortyEightKilohertz))
    );
    assert_eq!(
        trace.iter().filter(|seen| **seen == Seen::StartLcd).count(),
        1
    );
    assert_eq!(
        &trace[place(trace, c(Command::WriteExpanderDirection))..][..2],
        &[
            c(Command::WriteExpanderDirection),
            c(Command::StartKnobScan)
        ]
    );
}

#[test]
fn the_master_clock_starts_only_after_the_rails() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    let trace = &caller.trace;
    let rails_done = place(trace, c(Command::ConfigureConverterBus));
    let audio_pll = place(
        trace,
        c(Command::SwitchPll0Family(RateFamily::FortyEightKilohertz)),
    );
    let core_clock = place(trace, c(Command::SelectCorePllAsMainClock));
    let first_rail = place(trace, pin(OutputPin::FirstBringUp, PinLevel::High));
    assert!(
        core_clock < first_rail,
        "the core clock at {core_clock} moved ahead of the rails at {first_rail}, and only it"
    );
    assert!(
        place(trace, pin(OutputPin::ThirdBringUp, PinLevel::High)) < rails_done,
        "the last rail rises inside the rails stage"
    );
    assert!(
        rails_done < audio_pll,
        "the audio PLL started at {audio_pll}, before the rails completed at {rails_done}"
    );
    assert_eq!(
        &trace[audio_pll..audio_pll + 4],
        &[
            c(Command::SwitchPll0Family(RateFamily::FortyEightKilohertz)),
            held(3),
            c(Command::EnableMasterClockOutput),
            volume(VolumeRegister::FactoryZeroA, 0xFF),
        ]
    );
    assert_eq!(
        trace
            .iter()
            .filter(|seen| **seen == c(Command::EnableMasterClockOutput))
            .count(),
        1
    );
}

fn expander_direction_waits(bench: Bench) -> Vec<(bool, Vec<Command>)> {
    let (mut caller, mut emitted) = boot_board::Caller::new();
    let mut responder = boot_board::Responder::new(bench);
    let mut steps = Vec::new();
    for _ in 0..ENOUGH_PASSES {
        if !matches!(caller.boot, State::Scripted(..)) {
            break;
        }
        emitted = match responder.answer(&emitted) {
            Some(event) => caller.feed(event),
            None => caller.main_loop_pass(&bench),
        };
        steps.push((caller.boot.awaits_expander_direction(), emitted.to_vec()));
    }
    steps
}

#[test]
fn the_boot_awaits_the_expander_direction_from_its_write_until_it_is_acked_or_skipped() {
    let expander = boot_board::expander_direction_write();
    for (bench, skipped) in [
        (Bench::ALL_ACK, false),
        (
            Bench {
                unanswered_transfer: Some(expander),
                ..Bench::ALL_ACK
            },
            true,
        ),
    ] {
        let steps = expander_direction_waits(bench);
        let first = steps.iter().position(|(awaits, _)| *awaits).unwrap();
        let last = steps.iter().rposition(|(awaits, _)| *awaits).unwrap();
        assert!(steps[first..=last].iter().all(|(awaits, _)| *awaits));
        assert!(steps[first].1.contains(&Command::WriteExpanderDirection));
        assert!(steps[first + 1..=last].iter().all(|(_, emitted)| {
            emitted
                .iter()
                .all(|command| *command == Command::WriteExpanderDirection)
        }));
        let settled = &steps[last + 1].1;
        assert!(settled.contains(&Command::StartKnobScan));
        assert_eq!(
            settled.contains(&Command::RecordFault(
                FaultCode::Unanswered(I2cPeer::Expander),
                At::Step(Place { stage: 14, step: 0 }),
                Weight::Noted
            )),
            skipped
        );
    }
}
