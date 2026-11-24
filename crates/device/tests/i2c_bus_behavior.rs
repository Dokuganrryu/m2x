use device::adc::{AdcAddress, AdcWrite};
use device::command::Command;
use device::dac::{Attenuation, DacWrite, MasterTrim, StatusRegister, VolumeRegister};
use device::i2c::{Answer, BusAction, Event, MasterStatus, Output, State, Transfer, transition};

fn run(state: State, events: &[Event]) -> (State, Vec<Output>) {
    let mut state = state;
    let mut seen = Vec::new();
    for &event in events {
        let (next, outputs) = transition(state, event);
        state = next;
        seen.extend_from_slice(outputs.as_slice());
    }
    (state, seen)
}

fn begin(command: Command) -> Event {
    Event::Begin(Transfer::of(command).expect("the command is a bus transfer"))
}

fn act(action: BusAction) -> Output {
    Output::Act(action)
}

const VOLUME_WRITE: Command =
    Command::WriteDac(DacWrite::Volume(VolumeRegister::MainA, Attenuation::FULL));

#[test]
fn a_dac_volume_write_sends_address_register_value_and_stop_then_answers_acked() {
    let (state, seen) = run(
        State::Idle,
        &[
            begin(VOLUME_WRITE),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        seen,
        [
            act(BusAction::Transmit(0x92)),
            act(BusAction::Start),
            act(BusAction::Transmit(18)),
            act(BusAction::Continue),
            act(BusAction::Transmit(0xFF)),
            act(BusAction::Continue),
            act(BusAction::Stop),
            Output::Answered(Answer::Acked),
        ]
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn a_busy_master_changes_nothing_until_it_is_pending() {
    let (_, started) = run(State::Idle, &[begin(VOLUME_WRITE)]);
    let (state, _) = run(State::Idle, &[begin(VOLUME_WRITE)]);
    let (after, seen) = run(state, &[Event::Polled(MasterStatus::InProgress)]);
    assert_eq!(after, state);
    assert!(seen.is_empty());
    assert_eq!(started.len(), 2);
}

#[test]
fn an_address_nack_stops_the_bus_and_answers_nacked_once_it_is_idle() {
    let (state, seen) = run(
        State::Idle,
        &[
            begin(VOLUME_WRITE),
            Event::Polled(MasterStatus::NackAddress),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        &seen[2..],
        [act(BusAction::Stop), Output::Answered(Answer::Nacked)]
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn a_data_nack_on_the_value_stops_the_bus_and_answers_nacked() {
    let (_, seen) = run(
        State::Idle,
        &[
            begin(VOLUME_WRITE),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::NackData),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        &seen[6..],
        [act(BusAction::Stop), Output::Answered(Answer::Nacked)]
    );
}

#[test]
fn a_master_trim_write_is_four_transactions_low_byte_first_each_with_its_own_start_and_stop() {
    let mut events = vec![Event::Begin(
        Transfer::of(Command::WriteDac(DacWrite::MasterTrim(
            MasterTrim::MINUS_ONE_DB,
        )))
        .expect("a trim write is a transfer"),
    )];
    for _ in 0..4 {
        events.extend([
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::Idle),
        ]);
    }
    let (state, seen) = run(State::Idle, &events);
    let transmitted: Vec<u8> = seen
        .iter()
        .filter_map(|output| match output {
            Output::Act(BusAction::Transmit(byte)) => Some(*byte),
            _ => None,
        })
        .collect();
    assert_eq!(
        transmitted,
        [
            0x92, 24, 0xBF, 0x92, 25, 0x82, 0x92, 26, 0x14, 0x92, 27, 0x72
        ]
    );
    let starts = seen
        .iter()
        .filter(|output| **output == act(BusAction::Start))
        .count();
    let stops = seen
        .iter()
        .filter(|output| **output == act(BusAction::Stop))
        .count();
    assert_eq!((starts, stops), (4, 4));
    assert_eq!(seen.last(), Some(&Output::Answered(Answer::Acked)));
    assert_eq!(state, State::Idle);
}

#[test]
fn a_nack_in_the_second_trim_transaction_ends_the_transfer_without_the_last_two() {
    let (state, seen) = run(
        State::Idle,
        &[
            Event::Begin(
                Transfer::of(Command::WriteDac(DacWrite::MasterTrim(
                    MasterTrim::MINUS_ONE_DB,
                )))
                .expect("a trim write is a transfer"),
            ),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::Idle),
            Event::Polled(MasterStatus::NackAddress),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(seen.last(), Some(&Output::Answered(Answer::Nacked)));
    assert_eq!(
        seen.iter()
            .filter(|output| **output == act(BusAction::Start))
            .count(),
        2
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn a_status_read_writes_the_register_then_restarts_for_reading_and_answers_the_byte() {
    let (state, seen) = run(
        State::Idle,
        &[
            begin(Command::ReadDac(StatusRegister::Register113)),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::ReceiveReady(0x5A)),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        seen,
        [
            act(BusAction::Transmit(0x92)),
            act(BusAction::Start),
            act(BusAction::Transmit(113)),
            act(BusAction::Continue),
            act(BusAction::Transmit(0x93)),
            act(BusAction::Start),
            act(BusAction::Stop),
            Output::Answered(Answer::Read(0x5A)),
        ]
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn a_dac_read_stops_after_its_one_byte_and_never_acknowledges_a_second() {
    let (_, seen) = run(
        State::Idle,
        &[
            begin(Command::ReadDac(StatusRegister::Register113)),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::ReceiveReady(0x5A)),
            Event::Polled(MasterStatus::ReceiveReady(0x11)),
            Event::Polled(MasterStatus::ReceiveReady(0x22)),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    let read_start = seen
        .iter()
        .position(|output| *output == act(BusAction::Transmit(0x93)))
        .expect("the read address is sent");
    let after_read_address = &seen[read_start + 2..];
    assert_eq!(after_read_address.first(), Some(&act(BusAction::Stop)));
    assert!(!after_read_address.contains(&act(BusAction::Continue)));
    assert!(!seen.iter().any(|output| matches!(
        output,
        Output::Answered(Answer::Read(byte)) if *byte != 0x5A
    )));
}

#[test]
fn an_adc_probe_sends_only_the_address_and_answers_with_the_acknowledge() {
    let (_, acked) = run(
        State::Idle,
        &[
            begin(Command::ProbeAdc(AdcAddress::At13)),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        acked,
        [
            act(BusAction::Transmit(0x26)),
            act(BusAction::Start),
            act(BusAction::Stop),
            Output::Answered(Answer::Acked),
        ]
    );
    let (_, refused) = run(
        State::Idle,
        &[
            begin(Command::ProbeAdc(AdcAddress::At4F)),
            Event::Polled(MasterStatus::NackAddress),
            Event::Polled(MasterStatus::Idle),
        ],
    );
    assert_eq!(
        refused,
        [
            act(BusAction::Transmit(0x9E)),
            act(BusAction::Start),
            act(BusAction::Stop),
            Output::Answered(Answer::Nacked),
        ]
    );
}

#[test]
fn the_adc_and_expander_writes_address_their_own_parts() {
    let first_byte = |command| {
        let (_, seen) = run(State::Idle, &[begin(command)]);
        seen[0]
    };
    assert_eq!(
        first_byte(Command::WriteAdc(AdcWrite::At4FRegister1)),
        act(BusAction::Transmit(0x9E))
    );
    assert_eq!(
        first_byte(Command::WriteExpanderDirection),
        act(BusAction::Transmit(0x72))
    );
}

#[test]
fn the_expander_direction_write_sends_register_three_with_the_four_button_inputs() {
    let (_, seen) = run(
        State::Idle,
        &[
            begin(Command::WriteExpanderDirection),
            Event::Polled(MasterStatus::TransmitReady),
            Event::Polled(MasterStatus::TransmitReady),
        ],
    );
    assert_eq!(
        &seen[2..],
        [
            act(BusAction::Transmit(3)),
            act(BusAction::Continue),
            act(BusAction::Transmit(0x8D)),
            act(BusAction::Continue),
        ]
    );
}

#[test]
fn a_new_transfer_while_one_is_in_flight_resets_the_master_before_it_starts() {
    let (state, _) = run(
        State::Idle,
        &[
            begin(VOLUME_WRITE),
            Event::Polled(MasterStatus::TransmitReady),
        ],
    );
    let (_, seen) = run(state, &[begin(Command::ProbeAdc(AdcAddress::At13))]);
    assert_eq!(
        seen,
        [
            act(BusAction::ResetMaster),
            act(BusAction::Transmit(0x26)),
            act(BusAction::Start),
        ]
    );
}

#[test]
fn a_disrupted_bus_resets_the_master_and_answers_nacked() {
    let (state, seen) = run(
        State::Idle,
        &[begin(VOLUME_WRITE), Event::Polled(MasterStatus::Disrupted)],
    );
    assert_eq!(
        &seen[2..],
        [
            act(BusAction::ResetMaster),
            Output::Answered(Answer::Nacked)
        ]
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn a_status_that_does_not_fit_the_phase_resets_the_master_and_answers_nacked() {
    let (state, seen) = run(
        State::Idle,
        &[
            begin(VOLUME_WRITE),
            Event::Polled(MasterStatus::ReceiveReady(1)),
        ],
    );
    assert_eq!(
        &seen[2..],
        [
            act(BusAction::ResetMaster),
            Output::Answered(Answer::Nacked)
        ]
    );
    assert_eq!(state, State::Idle);
}

#[test]
fn an_idle_bus_ignores_every_poll() {
    for status in [
        MasterStatus::InProgress,
        MasterStatus::Idle,
        MasterStatus::ReceiveReady(7),
        MasterStatus::TransmitReady,
        MasterStatus::NackAddress,
        MasterStatus::NackData,
        MasterStatus::Disrupted,
    ] {
        let (state, outputs) = transition(State::Idle, Event::Polled(status));
        assert_eq!(state, State::Idle);
        assert!(outputs.as_slice().is_empty());
    }
}

#[test]
fn only_bus_commands_are_transfers() {
    assert!(Transfer::of(Command::StopI2s).is_none());
    assert!(Transfer::of(VOLUME_WRITE).is_some());
    assert!(Transfer::of(Command::WriteExpanderDirection).is_some());
}
