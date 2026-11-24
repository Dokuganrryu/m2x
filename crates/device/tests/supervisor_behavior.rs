mod boot_board;

use boot_board::{
    Bench, Caller, Responder, Seen, adc_init_write, expander_direction_write,
    first_attenuation_write, notes, run_bench,
};
use device::analog_word::AnalogWord;
use device::boot::State as BootState;
use device::breadcrumb::{At, Place, Weight};
use device::command::Command;
use device::fault::{FaultCode, I2cPeer, fault_routine};
use device::output_pin::OutputPin;
use device::outputs::Filler;
use device::phantom::{Milliseconds, PinLevel};
use device::steps::{
    PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23, PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23,
};
use device::supervisor::{
    Event as SupervisorEvent, Output, Progress, State, Transport, Usb, WATCHDOG_TIMEOUT, transition,
};
use device::usb::function;

const ENOUGH_PASSES: usize = 5000;

fn feeds(state: State, events: &[SupervisorEvent]) -> (State, usize) {
    let mut state = state;
    let mut fed = 0;
    for event in events {
        let (next, outputs) = transition(state, *event);
        fed += outputs
            .as_slice()
            .iter()
            .filter(|o| **o == Output::Command(Command::FeedWatchdog))
            .count();
        state = next;
    }
    (state, fed)
}

#[test]
fn during_boot_a_completed_control_pass_alone_earns_a_feed() {
    let (_, fed) = feeds(
        State::START,
        &[
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(fed, 1);
    let (_, starved) = feeds(State::START, &[SupervisorEvent::FeedWindowEnded]);
    assert_eq!(starved, 0);
}

#[test]
fn once_the_transport_runs_a_feed_also_needs_the_tick_to_have_advanced() {
    let (healthy, _) = feeds(State::START, &[SupervisorEvent::TransportStarted]);
    let (_, without_tick) = feeds(
        healthy,
        &[
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(without_tick, 0);
    let (_, with_tick) = feeds(
        healthy,
        &[
            SupervisorEvent::TickAdvanced,
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(with_tick, 1);
}

#[test]
fn while_the_bus_is_configured_and_awake_a_feed_also_needs_the_usb_interrupt_to_have_run() {
    let (attached, _) = feeds(
        State::START,
        &[
            SupervisorEvent::TransportStarted,
            SupervisorEvent::UsbBus(Usb::Serving),
        ],
    );
    let progress = [
        SupervisorEvent::TickAdvanced,
        SupervisorEvent::ControlPassDone,
    ];
    let (_, without_usb) = feeds(
        attached,
        &[progress[0], progress[1], SupervisorEvent::FeedWindowEnded],
    );
    assert_eq!(without_usb, 0);
    let (_, with_usb) = feeds(
        attached,
        &[
            progress[0],
            progress[1],
            SupervisorEvent::UsbServiced,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(with_usb, 1);
    let (detached, _) = feeds(attached, &[SupervisorEvent::UsbBus(Usb::Silent)]);
    let (_, after_detach) = feeds(
        detached,
        &[progress[0], progress[1], SupervisorEvent::FeedWindowEnded],
    );
    assert_eq!(after_detach, 1);
}

#[test]
fn a_feed_consumes_the_progress_so_the_next_window_needs_fresh_progress() {
    let (_, fed) = feeds(
        State::START,
        &[
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(fed, 1);
}

#[test]
fn a_fault_latches_silence_disconnects_the_outputs_stores_its_code_and_tells_the_other_machines() {
    let code = FaultCode::Unanswered(I2cPeer::Dac);
    let at = At::Step(Place { stage: 9, step: 2 });
    let (state, outputs) = transition(State::START, SupervisorEvent::FaultRaised(code, at));
    assert_eq!(
        outputs.as_slice(),
        &[
            Output::Command(Command::LatchPlaybackSilence),
            Output::Command(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)),
            Output::Command(Command::StopTickTimer),
            Output::Command(Command::RecordFault(code, at, Weight::Raised)),
            Output::FaultRaised(code),
        ]
    );
    let (again, repeated) = transition(
        state,
        SupervisorEvent::FaultRaised(FaultCode::Panic, At::Here),
    );
    assert_eq!(again, state);
    assert!(repeated.as_slice().is_empty());
}

#[test]
fn a_fault_after_a_noted_panel_fault_silences_tells_the_phantom_and_stores_its_own_code() {
    let bench = Bench {
        unanswered_transfer: Some(expander_direction_write()),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert!(
        matches!(caller.boot, BootState::Running(..)),
        "an expander fault must not stop the boot: {:?}",
        caller.boot
    );
    let second = FaultCode::Unanswered(I2cPeer::Dac);
    let (faulted, outputs) = transition(
        caller.supervisor,
        SupervisorEvent::FaultRaised(second, At::Here),
    );
    assert_eq!(
        outputs.as_slice(),
        &[
            Output::Command(Command::LatchPlaybackSilence),
            Output::Command(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)),
            Output::Command(Command::StopTickTimer),
            Output::Command(Command::RecordFault(second, At::Here, Weight::Raised)),
            Output::FaultRaised(second),
        ],
        "a fatal fault after the expander fault must store its own code"
    );
    assert!(matches!(faulted, State::Faulted { code, .. } if code == second));
    assert!(
        caller
            .trace
            .iter()
            .any(|seen| notes(*seen, FaultCode::Unanswered(I2cPeer::Expander))),
        "the boot must note the expander fault"
    );
    assert_eq!(
        caller.reported,
        vec![SupervisorEvent::TransportStarted],
        "the boot must report no fault of the panel to the supervisor"
    );
}

fn stored_by(outputs: &[Output]) -> Vec<FaultCode> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::Command(Command::RecordFault(code, _, Weight::Raised)) => Some(*code),
            Output::Command(_) | Output::FaultRaised(_) | Output::Nothing => None,
        })
        .collect()
}

#[test]
fn the_supervisor_stores_the_fatal_code_from_the_reports_of_a_boot_that_skipped_a_dac_write() {
    let bench = Bench {
        unanswered_transfer: Some(first_attenuation_write()),
        also_unanswered: Some(adc_init_write()),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    let faults: Vec<SupervisorEvent> = caller
        .reported
        .iter()
        .filter(|event| matches!(event, SupervisorEvent::FaultRaised(..)))
        .copied()
        .collect();
    let mut state = State::START;
    let mut stored = Vec::new();
    for event in &caller.reported {
        let (next, outputs) = transition(state, *event);
        stored.extend(stored_by(outputs.as_slice()));
        state = next;
    }
    assert_eq!(
        stored,
        vec![FaultCode::Unanswered(I2cPeer::Adc)],
        "the supervisor stored {stored:?} from the boot's fault reports {faults:?}"
    );
    assert!(
        matches!(state, State::Faulted { code, .. } if code == FaultCode::Unanswered(I2cPeer::Adc))
    );
}

#[test]
fn after_a_boot_that_skipped_a_dac_write_every_run_time_fault_stores_its_code() {
    let bench = Bench {
        unanswered_transfer: Some(first_attenuation_write()),
        ..Bench::ALL_ACK
    };
    let caller = run_bench(bench, ENOUGH_PASSES);
    assert!(
        matches!(caller.boot, BootState::Running(..)),
        "a skipped attenuation write must not fault the boot: {:?}",
        caller.boot
    );
    assert!(
        matches!(caller.supervisor, State::Healthy { .. }),
        "the supervisor must run the transport after the boot: {:?}",
        caller.supervisor
    );
    for code in FaultCode::ALL {
        let (_, outputs) = transition(
            caller.supervisor,
            SupervisorEvent::FaultRaised(code, At::Here),
        );
        assert_eq!(
            stored_by(outputs.as_slice()),
            vec![code],
            "a run-time {code:?} after the boot's note must store its code"
        );
    }
    assert!(
        caller
            .trace
            .iter()
            .any(|seen| notes(*seen, FaultCode::Unanswered(I2cPeer::Dac))),
        "the boot must note the skipped DAC write"
    );
}

#[test]
fn the_control_loop_keeps_feeding_the_watchdog_after_a_fault() {
    let (faulted, _) = feeds(
        State::START,
        &[
            SupervisorEvent::TransportStarted,
            SupervisorEvent::FaultRaised(FaultCode::BoardStalled, At::Here),
        ],
    );
    let (_, fed) = feeds(
        faulted,
        &[
            SupervisorEvent::TickAdvanced,
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(fed, 1);
}

#[test]
fn a_fault_after_the_transport_started_asks_for_the_control_pass_alone() {
    let (faulted, _) = feeds(
        State::START,
        &[
            SupervisorEvent::TransportStarted,
            SupervisorEvent::FaultRaised(FaultCode::BoardStalled, At::Here),
        ],
    );
    assert_eq!(
        faulted.required(),
        Progress::NONE.with(device::supervisor::Fact::ControlPassDone)
    );
    let (_, fed) = feeds(
        faulted,
        &[
            SupervisorEvent::ControlPassDone,
            SupervisorEvent::FeedWindowEnded,
        ],
    );
    assert_eq!(fed, 1);
    let (_, starved) = feeds(faulted, &[SupervisorEvent::FeedWindowEnded]);
    assert_eq!(starved, 0);
}

#[test]
fn a_run_time_fault_stops_the_tick_timer_the_supervisor_no_longer_requires() {
    let (healthy, _) = feeds(State::START, &[SupervisorEvent::TransportStarted]);
    assert!(
        healthy
            .required()
            .contains(device::supervisor::Fact::TickAdvanced)
    );
    let (faulted, outputs) = transition(
        healthy,
        SupervisorEvent::FaultRaised(FaultCode::Unanswered(I2cPeer::Dac), At::Here),
    );
    assert!(
        outputs
            .as_slice()
            .contains(&Output::Command(Command::StopTickTimer))
    );
    assert_eq!(
        faulted.required(),
        Progress::NONE.with(device::supervisor::Fact::ControlPassDone)
    );
}

#[test]
fn the_fault_routine_silences_then_stores_the_code_and_leaves_the_ending_to_the_board() {
    assert_eq!(
        fault_routine(FaultCode::HardFault),
        [
            Command::StopTransmitDma,
            Command::DrivePin(OutputPin::TransportActive, PinLevel::Low),
            Command::ZeroTransmitBuffers,
            Command::DrivePin(OutputPin::PhantomActive, PinLevel::Low),
            Command::SendAnalogWord(AnalogWord::ALL_OFF),
            Command::RecordFault(FaultCode::HardFault, At::Here, Weight::Raised),
        ]
    );
}

#[test]
fn every_fault_code_has_its_own_stored_number() {
    let mut numbers: Vec<u8> = FaultCode::ALL.iter().map(|c| c.number()).collect();
    numbers.sort_unstable();
    numbers.dedup();
    assert_eq!(numbers.len(), FaultCode::ALL.len());
    assert!(!numbers.contains(&0));
    let pinned = [
        (FaultCode::Unanswered(I2cPeer::Dac), 3),
        (FaultCode::Unanswered(I2cPeer::Adc), 4),
        (FaultCode::Unanswered(I2cPeer::Expander), 5),
        (FaultCode::BoardStalled, 6),
        (FaultCode::Panic, 7),
        (FaultCode::HardFault, 8),
        (FaultCode::UnexpectedException, 9),
        (FaultCode::FenceUnavailable, 10),
        (FaultCode::RefusedCommand, 11),
        (FaultCode::CrystalNotReady, 12),
        (FaultCode::CorePllUnlocked, 13),
        (FaultCode::AudioPllUnlocked, 14),
        (FaultCode::UsbPhyPllUnlocked, 16),
        #[cfg(feature = "sound3")]
        (FaultCode::CoreClockHeld, 17),
    ];
    let listed: Vec<(FaultCode, u8)> = FaultCode::ALL.iter().map(|&c| (c, c.number())).collect();
    assert_eq!(listed, pinned);
}

#[test]
fn the_boot_never_blocks_the_main_loop_with_a_delay_so_holds_cannot_starve_the_watchdog() {
    let caller = run_bench(Bench::ALL_ACK, ENOUGH_PASSES);
    assert!(matches!(caller.boot, BootState::Running(..)));
    let longest_hold = caller
        .trace
        .iter()
        .filter_map(|seen| match seen {
            Seen::Waited(Milliseconds(ms)) => Some(*ms),
            _ => None,
        })
        .max()
        .unwrap();
    assert_eq!(longest_hold, WATCHDOG_TIMEOUT.0 + 1);
    let mut last_feed = 0;
    for at in &caller.feeds_at {
        assert!(
            at - last_feed < u64::from(WATCHDOG_TIMEOUT.0),
            "gap ending at {at}"
        );
        last_feed = *at;
    }
    assert!(caller.now - last_feed < u64::from(WATCHDOG_TIMEOUT.0));
}

#[test]
fn every_boot_timeout_is_shorter_than_the_watchdog() {
    for timeout in [
        PLACEHOLDER_I2C_ANSWER_TIMEOUT_UNTIL_T23,
        PLACEHOLDER_STEP_CONFIRMATION_TIMEOUT_UNTIL_T23,
    ] {
        assert!(timeout.0 < WATCHDOG_TIMEOUT.0);
    }
}

#[test]
fn after_a_boot_failure_silence_is_latched_and_feeds_continue() {
    let bench = Bench {
        unanswered_transfer: Some(12),
        ..Bench::ALL_ACK
    };
    let (mut caller, mut emitted) = Caller::new();
    let mut responder = Responder::new(bench);
    let mut passes_after_failure = 0;
    while passes_after_failure < 100 {
        emitted = match responder.answer(&emitted) {
            Some(event) => caller.feed(event),
            None => caller.main_loop_pass(&bench),
        };
        if let BootState::Degraded(_) = caller.boot {
            passes_after_failure += 1;
            emitted.clear();
        }
    }
    assert!(matches!(caller.supervisor, State::Faulted { .. }));
    assert!(caller.supervised.contains(&Command::LatchPlaybackSilence));
    let failed_at = caller.now - 100;
    assert!(caller.feeds_at.iter().filter(|at| **at > failed_at).count() >= 10);
    assert!(!caller.trace.iter().any(|seen| matches!(
        seen,
        Seen::Command(Command::DrivePin(OutputPin::PhantomActive, PinLevel::High))
    )));
}

#[test]
fn the_feed_predicate_reads_as_plain_set_logic() {
    let control = Progress::NONE.with(device::supervisor::Fact::ControlPassDone);
    let all = control
        .with(device::supervisor::Fact::TickAdvanced)
        .with(device::supervisor::Fact::UsbServiced);
    assert!(all.covers(control));
    assert!(!control.covers(all));
    assert!(Progress::NONE.covers(Progress::NONE));
    let faulted = State::Faulted {
        code: FaultCode::Panic,
        transport: Transport::Stopped,
        usb: Usb::Silent,
        seen: Progress::NONE,
    };
    assert_eq!(faulted.required(), control);
}

#[test]
fn with_no_host_or_a_suspended_host_the_feed_needs_no_usb_interrupt_for_five_seconds() {
    let windows = (5_000 / WATCHDOG_TIMEOUT.0) as usize * 2;
    for usb in [
        function::State::INITIAL,
        function::transition(function::State::INITIAL, function::Event::Attached).0,
        function::transition(
            function::transition(function::State::INITIAL, function::Event::Attached).0,
            function::Event::Suspended,
        )
        .0,
    ] {
        assert_eq!(usb.supervised(), Usb::Silent, "{usb:?}");
        let (mut state, _) = feeds(
            State::START,
            &[
                SupervisorEvent::TransportStarted,
                SupervisorEvent::UsbBus(usb.supervised()),
            ],
        );
        let mut fed = 0;
        for _ in 0..windows {
            let (next, once) = feeds(
                state,
                &[
                    SupervisorEvent::TickAdvanced,
                    SupervisorEvent::ControlPassDone,
                    SupervisorEvent::FeedWindowEnded,
                ],
            );
            state = next;
            fed += once;
        }
        assert_eq!(fed, windows, "{usb:?}");
    }
}

#[test]
fn the_supervisor_filler_does_nothing() {
    assert_eq!(Output::FILLER, Output::Nothing);
}
