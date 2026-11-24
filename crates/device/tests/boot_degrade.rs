mod boot_board;

use boot_board::{
    Bench, Caller, Emitted, Responder, Seen, clean_boot_transfer_number as transfer_number, notes,
    raises, run_bench,
};
use device::adc::AdcKind;
use device::analog_word::AnalogWord;
use device::boot::{ClockInput, Event, Outcome, PllLock, Stage, StageClass, State, Subsystem};
use device::breadcrumb::{At, Place, Weight};
use device::command::Command;
use device::dac::DacWrite;
use device::fault::{FaultCode, I2cPeer};
use device::output_pin::OutputPin;
use device::phantom::{Milliseconds, PinLevel};

const ENOUGH_PASSES: usize = 5_000;

struct Expected {
    code: FaultCode,
    panel: bool,
    usb: bool,
    audio: bool,
}

fn position(caller: &Caller, wanted: impl Fn(Seen) -> bool) -> Option<usize> {
    caller.log.iter().position(|seen| wanted(*seen))
}

fn recorded(caller: &Caller, record: Command) -> bool {
    position(caller, |seen| seen == Seen::Command(record)).is_some()
}

fn belongs_to_an_audio_stage(command: Command) -> bool {
    matches!(
        command,
        Command::SwitchPll0Family(_)
            | Command::EnableMasterClockOutput
            | Command::WriteDac(_)
            | Command::ReadDac(_)
            | Command::WriteAdc(_)
            | Command::ProbeAdc(_)
            | Command::ZeroTransmitBuffers
            | Command::StartDma
            | Command::StartI2s(_)
            | Command::StartTickTimer(_)
            | Command::DrivePin(OutputPin::ConverterEnable, PinLevel::High)
            | Command::DrivePin(OutputPin::AdcEnable, PinLevel::High)
            | Command::DrivePin(OutputPin::TransportActive, PinLevel::High)
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

fn refused_once_when(mut now: impl FnMut(State, &Emitted) -> bool) -> Caller {
    let bench = Bench::ALL_ACK;
    let (mut caller, mut emitted) = Caller::new();
    let mut responder = Responder::new(bench);
    let mut refused = false;
    while !matches!(caller.boot, State::Running(_, _) | State::Degraded(_)) {
        if !refused && now(caller.boot, &emitted) {
            refused = true;
            emitted = caller.feed(Event::CommandRefused);
            continue;
        }
        emitted = match responder.answer(&emitted) {
            Some(event) => caller.feed(event),
            None => caller.main_loop_pass(&bench),
        };
    }
    assert!(refused, "the boot never reached the refusal");
    caller
}

type InStage = fn(Stage) -> bool;

fn refused_on_entering(stage: impl Fn(Stage) -> bool) -> Caller {
    refused_once_when(|state, _| matches!(state, State::Scripted(now, _, _) if stage(now)))
}

fn after_the_fault(caller: &Caller) -> Vec<Command> {
    let stored = position(caller, |seen| {
        matches!(
            seen,
            Seen::Command(Command::RecordFault(_, _, Weight::Raised))
        )
    })
    .expect("the fault was stored");
    caller.log[stored..]
        .iter()
        .filter_map(|seen| match seen {
            Seen::Command(command) => Some(*command),
            Seen::Waited(_) | Seen::Log(_) | Seen::StartLcd => None,
        })
        .collect()
}

fn c(command: Command) -> Seen {
    Seen::Command(command)
}

fn pin(pin: OutputPin, level: PinLevel) -> Seen {
    c(Command::DrivePin(pin, level))
}

fn held(milliseconds: u32) -> Seen {
    Seen::Waited(Milliseconds(milliseconds + 1))
}

const AUDIO_STOPS: [Command; 3] = [Command::StopTickTimer, Command::StopDma, Command::StopI2s];

fn the_ladder_from_the_analog_word() -> Vec<Seen> {
    let mut ladder: Vec<Seen> = AUDIO_STOPS.map(c).to_vec();
    ladder.extend([
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
        c(Command::WriteExpanderDirection),
        c(Command::StartKnobScan),
    ]);
    ladder
}

fn the_ladder_from_the_panel() -> Vec<Seen> {
    let mut ladder: Vec<Seen> = AUDIO_STOPS.map(c).to_vec();
    ladder.extend([
        c(Command::WriteExpanderDirection),
        c(Command::StartKnobScan),
    ]);
    ladder
}

fn from_the_audio_stops(caller: &Caller) -> &[Seen] {
    let stopped = caller
        .trace
        .iter()
        .rposition(|seen| *seen == c(Command::StopTickTimer))
        .expect("the fault stopped the tick timer");
    &caller.trace[stopped..]
}

fn stored_any(caller: &Caller) -> bool {
    caller.log.iter().any(|seen| {
        matches!(
            seen,
            Seen::Command(Command::RecordFault(_, _, Weight::Raised))
        )
    })
}

fn outcomes(caller: &Caller) -> Outcome {
    match caller.boot {
        State::Scripted(_, _, outcome)
        | State::VolumeRamp(_, _, outcome)
        | State::Running(_, outcome)
        | State::Degraded(outcome) => outcome,
    }
}

fn assert_the_fault_was_recorded(caller: &Caller, code: FaultCode, what: &str) {
    let outcome = outcomes(caller);
    assert!(
        outcome.faults.contains(code),
        "{what}: the terminal state does not carry the fault: {outcome:?}"
    );
    assert_eq!(
        outcome.faults.len(),
        1,
        "{what}: a second fault was recorded"
    );
    assert!(
        !position(caller, |seen| seen
            == Seen::Command(Command::DrivePin(
                OutputPin::PhantomActive,
                PinLevel::High
            )))
        .is_some(),
        "{what}: phantom turned on"
    );
}

fn assert_the_boot_degraded(caller: &Caller, expected: &Expected, what: &str) {
    let lit = position(caller, |seen| seen == Seen::StartLcd).is_some();
    assert_eq!(lit, expected.panel, "{what}: panel start");
    if expected.audio {
        assert!(
            caller.log.iter().any(|seen| notes(*seen, expected.code)),
            "{what}: the fault code was not noted"
        );
        assert!(
            !stored_any(caller),
            "{what}: a fault the audio survives claimed the stored code"
        );
    } else {
        assert!(
            caller.log.iter().any(|seen| raises(*seen, expected.code)),
            "{what}: the fault code was not stored"
        );
        let later = after_the_fault(caller);
        assert!(
            !later
                .iter()
                .any(|command| belongs_to_an_audio_stage(*command)),
            "{what}: an audio stage ran after the fault: {later:?}"
        );
    }
    assert_eq!(
        position(caller, |seen| seen == Seen::Command(Command::AttachUsb)).is_some(),
        expected.usb,
        "{what}: USB attach"
    );
    assert_eq!(
        outcomes(caller).runs(Subsystem::Usb),
        expected.usb,
        "{what}: the terminal state and USB"
    );
    assert_eq!(
        outcomes(caller).runs(Subsystem::Panel),
        expected.panel,
        "{what}: the terminal state and the screen"
    );
    assert_the_fault_was_recorded(caller, expected.code, what);
    let connected = position(caller, |seen| {
        seen == Seen::Command(Command::DrivePin(
            OutputPin::TransportActive,
            PinLevel::High,
        ))
    });
    if expected.audio {
        assert!(connected.is_some(), "{what}: the outputs never connected");
        assert!(
            matches!(caller.boot, State::Running(_, _)),
            "{what}: audio runs but the boot is {:?}",
            caller.boot
        );
        assert!(outcomes(caller).runs(Subsystem::Audio), "{what}");
    } else {
        let fault = position(caller, |seen| {
            matches!(
                seen,
                Seen::Command(Command::RecordFault(_, _, Weight::Raised))
            )
        })
        .expect("the fault was stored");
        assert!(
            connected.is_none_or(|connected| connected < fault),
            "{what}: the outputs connected after the fault"
        );
        assert!(
            !outcomes(caller).runs(Subsystem::Audio),
            "{what}: the terminal state says audio runs"
        );
    }
}

#[test]
fn a_dac_that_stops_answering_leaves_the_outputs_off_and_still_panels_and_attaches() {
    let number =
        transfer_number(|command| matches!(command, Command::WriteDac(DacWrite::Config(_))));
    let bench = Bench {
        unanswered_transfer: Some(number),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::Unanswered(I2cPeer::Dac),
            panel: true,
            usb: true,
            audio: false,
        },
        "dac unanswered",
    );
}

#[test]
fn a_dac_that_never_answers_the_attenuation_writes_is_skipped_and_the_boot_plays() {
    let number =
        transfer_number(|command| matches!(command, Command::WriteDac(DacWrite::Volume(..))));
    let bench = Bench {
        unanswered_transfer: Some(number),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert!(
        matches!(caller.boot, State::Running(_, _)),
        "a skipped attenuation stage must not fault the boot"
    );
    assert!(
        outcomes(&caller)
            .faults
            .contains(FaultCode::Unanswered(I2cPeer::Dac)),
        "the skip must record the unanswered DAC write: {:?}",
        outcomes(&caller)
    );
    assert_eq!(outcomes(&caller).faults.len(), 1);
    assert!(outcomes(&caller).runs(Subsystem::Audio));
    assert!(
        caller
            .trace
            .iter()
            .any(|seen| notes(*seen, FaultCode::Unanswered(I2cPeer::Dac))),
        "the skip must note the unanswered DAC code"
    );
    assert!(
        !caller.log.iter().any(|seen| matches!(
            seen,
            Seen::Command(Command::RecordFault(_, _, Weight::Raised))
        )),
        "the skip must leave the stored code free for a fatal fault"
    );
}

#[test]
fn a_dac_that_answers_only_full_attenuation_degrades_when_the_init_writes_start() {
    let number = transfer_number(|command| matches!(command, Command::ReadDac(_)));
    let bench = Bench {
        unanswered_transfer: Some(number),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::Unanswered(I2cPeer::Dac),
            panel: true,
            usb: true,
            audio: false,
        },
        "dac status read unanswered",
    );
}

#[test]
fn an_adc_that_stops_answering_after_its_probe_leaves_the_outputs_off() {
    let number = transfer_number(|command| matches!(command, Command::WriteAdc(_)));
    let bench = Bench {
        unanswered_transfer: Some(number),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::Unanswered(I2cPeer::Adc),
            panel: true,
            usb: true,
            audio: false,
        },
        "adc unanswered",
    );
}

#[test]
fn an_expander_that_never_answers_still_starts_the_screen_and_keeps_the_audio() {
    let number = transfer_number(|command| command == Command::WriteExpanderDirection);
    let bench = Bench {
        unanswered_transfer: Some(number),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::Unanswered(I2cPeer::Expander),
            panel: true,
            usb: true,
            audio: true,
        },
        "expander unanswered",
    );
}

#[test]
fn a_crystal_that_never_reports_ready_skips_usb_and_lights_the_screen() {
    let bench = Bench {
        clock_input: ClockInput::NotReady,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::CrystalNotReady,
            panel: true,
            usb: false,
            audio: false,
        },
        "crystal not ready",
    );
    assert!(matches!(caller.boot, State::Degraded(_)));
    assert!(!recorded(&caller, Command::StartCorePll));
    assert!(!recorded(&caller, Command::SelectCorePllAsMainClock));
    assert_eq!(
        from_the_audio_stops(&caller),
        the_ladder_from_the_analog_word().as_slice()
    );
}

#[test]
fn a_core_pll_that_never_locks_skips_usb_and_lights_the_screen() {
    let bench = Bench {
        core_pll_lock: PllLock::TimedOut,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::CorePllUnlocked,
            panel: true,
            usb: false,
            audio: false,
        },
        "core PLL never locks",
    );
    assert!(matches!(caller.boot, State::Degraded(_)));
    assert_eq!(
        from_the_audio_stops(&caller),
        the_ladder_from_the_analog_word().as_slice()
    );
}

#[test]
fn a_refused_command_before_the_core_pll_locks_skips_usb() {
    let cases: [(&str, Caller); 4] = [
        (
            "refused while quiescing",
            refused_on_entering(|stage| stage == Stage::Quiesce),
        ),
        (
            "refused on entering the clock stage",
            refused_on_entering(|stage| stage == Stage::Clocks),
        ),
        (
            "refused while the core PLL starts",
            refused_once_when(|_, emitted| emitted.contains(&Command::StartCorePll)),
        ),
        (
            "refused while the lock read is awaited",
            refused_once_when(|_, emitted| emitted.contains(&Command::SelectCorePllAsMainClock)),
        ),
    ];
    for (what, caller) in cases {
        assert_the_boot_degraded(
            &caller,
            &Expected {
                code: FaultCode::RefusedCommand,
                panel: true,
                usb: false,
                audio: false,
            },
            what,
        );
        assert_eq!(
            from_the_audio_stops(&caller),
            the_ladder_from_the_analog_word().as_slice(),
            "{what}"
        );
    }
}

#[test]
fn an_audio_pll_that_never_locks_keeps_usb_and_lights_the_screen() {
    let bench = Bench {
        audio_pll_lock: PllLock::TimedOut,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::AudioPllUnlocked,
            panel: true,
            usb: true,
            audio: false,
        },
        "audio PLL never locks",
    );
    assert!(
        !recorded(&caller, Command::EnableMasterClockOutput),
        "the master clock was enabled on an unlocked audio PLL"
    );
    let attached = position(&caller, |seen| seen == c(Command::AttachUsb)).unwrap();
    let lit = position(&caller, |seen| seen == Seen::StartLcd).unwrap();
    let stored = position(&caller, |seen| raises(seen, FaultCode::AudioPllUnlocked)).unwrap();
    assert!(
        attached < lit && lit < stored,
        "USB at {attached}, the screen at {lit}, the fault at {stored}"
    );
    assert_eq!(
        from_the_audio_stops(&caller),
        the_ladder_from_the_panel().as_slice()
    );
}

#[test]
fn a_usb_phy_pll_that_never_locks_records_16_at_usb_attach_and_the_boot_goes_on() {
    let bench = Bench {
        usb_phy_pll_lock: PllLock::TimedOut,
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let noted = c(Command::RecordFault(
        FaultCode::UsbPhyPllUnlocked,
        At::Step(Place { stage: 3, step: 0 }),
        Weight::Noted,
    ));
    assert_eq!(FaultCode::UsbPhyPllUnlocked.number(), 16);
    let attached = position(&caller, |seen| seen == c(Command::AttachUsb)).unwrap();
    assert_eq!(
        caller.log[attached + 1..attached + 3],
        [noted, c(Command::ConfigureAnalogWordBus)],
        "code 16 is noted while USB attaches and AnalogOff follows"
    );
    let State::Running(_, outcome) = caller.boot else {
        panic!("the boot ended in {:?}", caller.boot);
    };
    assert!(outcome.faults.contains(FaultCode::UsbPhyPllUnlocked));
    assert_eq!(outcome.faults.len(), 1);
    assert!(outcome.runs(Subsystem::Usb) && outcome.runs(Subsystem::Audio));
    assert!(
        !caller.log.iter().any(|seen| matches!(
            seen,
            Seen::Command(Command::RecordFault(_, _, Weight::Raised))
        )),
        "a noted code is not a raised fault"
    );
    let with_the_note_removed: Vec<Seen> = caller
        .log
        .iter()
        .copied()
        .filter(|seen| *seen != noted)
        .collect();
    assert_eq!(
        with_the_note_removed,
        run_bench(Bench::ALL_ACK, ENOUGH_PASSES).log,
        "every stage after UsbAttach runs as it does without the fault"
    );
}

#[test]
fn a_refused_analog_word_leaves_usb_attached_and_the_screen_dark() {
    let caller = refused_once_when(|_, emitted| {
        emitted.contains(&Command::SendAnalogWord(AnalogWord::ALL_OFF))
    });
    assert_the_boot_degraded(
        &caller,
        &Expected {
            code: FaultCode::RefusedCommand,
            panel: false,
            usb: true,
            audio: false,
        },
        "refused analog word",
    );
    assert!(
        !caller.log.iter().any(|seen| matches!(
            seen,
            Seen::Command(Command::DrivePin(
                OutputPin::FirstBringUp | OutputPin::SecondBringUp | OutputPin::ThirdBringUp,
                PinLevel::High
            ))
        )),
        "a rail rose after the analog chain was refused"
    );
    assert!(!recorded(&caller, Command::ConfigureConverterBus));
    assert!(!recorded(&caller, Command::WriteExpanderDirection));
    assert_eq!(from_the_audio_stops(&caller), AUDIO_STOPS.map(c).as_slice());
}

#[test]
fn a_rails_fault_leaves_usb_attached_and_the_screen_dark() {
    let mut words = 0;
    let cases: [(&str, Caller); 2] = [
        (
            "refused on entering the rails stage",
            refused_on_entering(|stage| stage == Stage::Rails),
        ),
        (
            "the all-off word after the rails is refused",
            refused_once_when(|_, emitted| {
                if emitted.contains(&Command::SendAnalogWord(AnalogWord::ALL_OFF)) {
                    words += 1;
                }
                words == 2
            }),
        ),
    ];
    for (what, caller) in cases {
        assert_the_boot_degraded(
            &caller,
            &Expected {
                code: FaultCode::RefusedCommand,
                panel: false,
                usb: true,
                audio: false,
            },
            what,
        );
        assert!(
            !recorded(&caller, Command::WriteExpanderDirection),
            "{what}"
        );
        assert!(!recorded(&caller, Command::StartKnobScan), "{what}");
        assert_eq!(
            from_the_audio_stops(&caller),
            AUDIO_STOPS.map(c).as_slice(),
            "{what}"
        );
    }
}

#[test]
fn an_audio_stage_fault_keeps_usb_and_the_screen_and_runs_the_panel_stage() {
    let stages: [(&str, InStage); 7] = [
        ("audio clocks", |stage| stage == Stage::AudioClocks),
        ("dac attenuate", |stage| stage == Stage::DacAttenuate),
        ("adc probe", |stage| matches!(stage, Stage::AdcProbe(_))),
        ("transport start", |stage| {
            matches!(stage, Stage::TransportStart(_))
        }),
        ("dac init", |stage| matches!(stage, Stage::DacInit(_))),
        ("output connect", |stage| {
            matches!(stage, Stage::OutputConnect(_))
        }),
        ("adc init", |stage| matches!(stage, Stage::AdcInit(_))),
    ];
    for (what, stage) in stages {
        let caller = refused_on_entering(stage);
        assert_the_boot_degraded(
            &caller,
            &Expected {
                code: FaultCode::RefusedCommand,
                panel: true,
                usb: true,
                audio: false,
            },
            what,
        );
        let attached = position(&caller, |seen| seen == c(Command::AttachUsb)).unwrap();
        let lit = position(&caller, |seen| seen == Seen::StartLcd).unwrap();
        let stored = position(&caller, |seen| raises(seen, FaultCode::RefusedCommand)).unwrap();
        assert!(
            attached < lit && lit < stored,
            "{what}: USB at {attached}, the screen at {lit}, the fault at {stored}"
        );
        assert_eq!(
            from_the_audio_stops(&caller),
            the_ladder_from_the_panel().as_slice(),
            "{what}"
        );
    }
}

#[test]
fn a_refused_command_while_usb_attaches_or_the_screen_starts_stops_the_audio_before_it_starts() {
    let cases: [(&str, Caller, Vec<Seen>); 2] = [
        (
            "refused while USB attaches",
            refused_once_when(|_, emitted| emitted.contains(&Command::AttachUsb)),
            the_ladder_from_the_analog_word(),
        ),
        (
            "refused while the screen starts",
            refused_once_when(|_, emitted| emitted.started_the_lcd()),
            the_ladder_from_the_panel(),
        ),
    ];
    for (what, caller, ladder) in cases {
        let later = after_the_fault(&caller);
        assert!(
            !later.iter().any(|command| is_a_start(*command)),
            "{what}: the audio started after a refusal: {later:?}"
        );
        assert!(
            !later
                .iter()
                .any(|command| belongs_to_an_audio_stage(*command)),
            "{what}: an audio stage ran after the refusal: {later:?}"
        );
        assert!(
            caller
                .log
                .iter()
                .any(|seen| raises(*seen, FaultCode::RefusedCommand)),
            "{what}: the refusal was not stored"
        );
        assert!(matches!(caller.boot, State::Degraded(_)), "{what}");
        assert_eq!(outcomes(&caller).faults.len(), 1, "{what}");
        assert_eq!(from_the_audio_stops(&caller), ladder.as_slice(), "{what}");
        assert!(
            recorded(&caller, Command::AttachUsb),
            "{what}: USB never attached"
        );
        assert!(
            position(&caller, |seen| seen == Seen::StartLcd).is_some(),
            "{what}: the screen never started"
        );
        assert!(
            !recorded(&caller, Command::DetachUsb) || {
                let detached = caller
                    .log
                    .iter()
                    .rposition(|seen| *seen == c(Command::DetachUsb));
                let attached = position(&caller, |seen| seen == c(Command::AttachUsb));
                detached < attached
            },
            "{what}: USB was detached after it attached"
        );
    }
}

#[test]
fn a_dac_that_stops_answering_during_the_ramp_silences_and_ends_degraded() {
    let bench = Bench {
        dac_stops_at_ramp_write: Some(1),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let State::Degraded(outcome) = caller.boot else {
        panic!(
            "a DAC that dies in the ramp must end degraded, not {:?}",
            caller.boot
        );
    };
    assert!(outcome.faults.contains(FaultCode::Unanswered(I2cPeer::Dac)));
    assert_eq!(outcome.faults.len(), 1);
    assert!(!outcome.runs(Subsystem::Audio));
    assert!(outcome.runs(Subsystem::Panel));
    assert!(outcome.runs(Subsystem::Usb));
    let stored = position(&caller, |seen| {
        raises(seen, FaultCode::Unanswered(I2cPeer::Dac))
    })
    .expect("the ramp fault was stored");
    let silenced: Vec<usize> = [Command::StopTickTimer, Command::StopDma, Command::StopI2s]
        .into_iter()
        .map(|command| {
            position(&caller, |seen| seen == Seen::Command(command))
                .unwrap_or_else(|| panic!("the ramp fault did not issue {command:?}"))
        })
        .collect();
    assert!(
        silenced.iter().all(|at| *at < stored),
        "the audio stops came after the stored code: {silenced:?} against {stored}"
    );
    assert!(
        caller.supervised.contains(&Command::LatchPlaybackSilence),
        "the ramp fault did not latch silence"
    );
    assert!(
        !caller.trace.iter().any(|seen| matches!(
            seen,
            Seen::Command(Command::DrivePin(OutputPin::PhantomActive, PinLevel::High))
        )),
        "the phantom turned on"
    );
    assert!(
        position(&caller, |seen| seen
            == Seen::Command(Command::DrivePin(
                OutputPin::TransportActive,
                PinLevel::High
            )))
        .is_some(),
        "the outputs never connected before the ramp"
    );
}

#[test]
fn a_noted_expander_fault_leaves_the_stored_code_to_the_later_ramp_fault() {
    let expander = transfer_number(|command| command == Command::WriteExpanderDirection);
    let bench = Bench {
        unanswered_transfer: Some(expander),
        dac_stops_at_ramp_write: Some(1),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let State::Degraded(outcome) = caller.boot else {
        panic!("the ramp fault must end degraded, not {:?}", caller.boot);
    };
    assert!(
        outcome
            .faults
            .contains(FaultCode::Unanswered(I2cPeer::Expander))
    );
    assert!(outcome.faults.contains(FaultCode::Unanswered(I2cPeer::Dac)));
    let stored: Vec<FaultCode> = caller
        .supervised
        .iter()
        .filter_map(|command| match command {
            Command::RecordFault(code, _, Weight::Raised) => Some(*code),
            _ => None,
        })
        .collect();
    assert_eq!(
        stored,
        vec![FaultCode::Unanswered(I2cPeer::Dac)],
        "the fatal ramp fault must take the stored code"
    );
    assert!(
        caller
            .trace
            .iter()
            .any(|seen| notes(*seen, FaultCode::Unanswered(I2cPeer::Expander))),
        "the expander fault must be noted"
    );
    assert!(
        caller.supervised.contains(&Command::LatchPlaybackSilence),
        "the ramp fault must still silence the audio"
    );
    assert!(
        position(&caller, |seen| seen
            == Seen::Command(Command::DrivePin(
                OutputPin::TransportActive,
                PinLevel::High
            )))
        .is_some(),
        "the expander fault must not stop the audio before the ramp"
    );
}

#[test]
fn every_boot_stage_has_its_pinned_class() {
    let cases = [
        (Stage::Quiesce, StageClass::AudioPath),
        (Stage::Clocks, StageClass::Clock),
        (Stage::UsbAttach, StageClass::Usb),
        (Stage::AnalogOff, StageClass::AudioPath),
        (Stage::Rails, StageClass::AudioPath),
        (Stage::StartLcd, StageClass::Panel),
        (Stage::AudioClocks, StageClass::Clock),
        (Stage::DacAttenuate, StageClass::AudioPath),
        (Stage::AdcProbe(AdcKind::Unanswered), StageClass::AudioPath),
        (
            Stage::TransportStart(AdcKind::Unanswered),
            StageClass::AudioPath,
        ),
        (Stage::DacInit(AdcKind::Unanswered), StageClass::AudioPath),
        (
            Stage::OutputConnect(AdcKind::Unanswered),
            StageClass::AudioPath,
        ),
        (Stage::AdcInit(AdcKind::Unanswered), StageClass::AudioPath),
        (Stage::Panel(AdcKind::Unanswered), StageClass::Panel),
    ];
    for (stage, class) in cases {
        assert_eq!(stage.class(), class, "{stage:?}");
    }
}
