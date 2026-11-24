use device::adc::AdcAddress;
use device::command::Command;
use device::dac::{Attenuation, DacWrite, MasterTrim, StatusRegister, VolumeRegister};
use device::i2c::{
    Answer, BusAction, Ending, Event, MasterStatus, Output, ProbePhase, ReadEnding, ReadPhase,
    State, Transfer, WritePhase, transition,
};

fn ending(ending: Ending) -> &'static str {
    match ending {
        Ending::Acked => "acked",
        Ending::Nacked => "nacked",
    }
}

fn state_label(state: State) -> String {
    match state {
        State::Idle => "idle".into(),
        State::Writing(writing) => {
            let phase = match writing.phase() {
                WritePhase::Address => "address".to_string(),
                WritePhase::Register => "register".to_string(),
                WritePhase::Value => "value".to_string(),
                WritePhase::Stopping(end) => format!("stopping_{}", ending(end)),
            };
            format!("write{}_{phase}", writing.transaction())
        }
        State::Reading(reading) => match reading.phase() {
            ReadPhase::Address => "read_address".into(),
            ReadPhase::Register => "read_register".into(),
            ReadPhase::ReadAddress => "read_turnaround".into(),
            ReadPhase::Stopping(ReadEnding::Byte(byte)) => format!("read_stopping_byte_{byte:02X}"),
            ReadPhase::Stopping(ReadEnding::Refused) => "read_stopping_refused".into(),
        },
        State::Probing(probing) => match probing.phase() {
            ProbePhase::Address => "probe_address".into(),
            ProbePhase::Stopping(end) => format!("probe_stopping_{}", ending(end)),
        },
    }
}

fn output_label(output: Output) -> String {
    match output {
        Output::Act(BusAction::Transmit(byte)) => format!("TX{byte:02X}"),
        Output::Act(BusAction::Start) => "START".into(),
        Output::Act(BusAction::Continue) => "CONT".into(),
        Output::Act(BusAction::Stop) => "STOP".into(),
        Output::Act(BusAction::ResetMaster) => "RESET".into(),
        Output::Answered(Answer::Acked) => "=ACK".into(),
        Output::Answered(Answer::Nacked) => "=NACK".into(),
        Output::Answered(Answer::Read(byte)) => format!("=READ{byte:02X}"),
    }
}

fn transfer(command: Command) -> Event {
    Event::Begin(Transfer::of(command).expect("bus command"))
}

fn events() -> [(&'static str, Event); 8] {
    [
        ("begin_probe", transfer(Command::ProbeAdc(AdcAddress::At13))),
        ("in_progress", Event::Polled(MasterStatus::InProgress)),
        ("idle", Event::Polled(MasterStatus::Idle)),
        (
            "receive_A5",
            Event::Polled(MasterStatus::ReceiveReady(0xA5)),
        ),
        ("transmit", Event::Polled(MasterStatus::TransmitReady)),
        ("nack_address", Event::Polled(MasterStatus::NackAddress)),
        ("nack_data", Event::Polled(MasterStatus::NackData)),
        ("disrupted", Event::Polled(MasterStatus::Disrupted)),
    ]
}

fn walk(command: Command, statuses: &[MasterStatus]) -> Vec<State> {
    let mut state = transition(State::Idle, transfer(command)).0;
    let mut seen = vec![state];
    for &status in statuses {
        state = transition(state, Event::Polled(status)).0;
        seen.push(state);
    }
    seen
}

fn representative_states() -> Vec<State> {
    use MasterStatus::{Idle, NackAddress, NackData, ReceiveReady, TransmitReady};
    let trim = Command::WriteDac(DacWrite::MasterTrim(MasterTrim::MINUS_ONE_DB));
    let volume = Command::WriteDac(DacWrite::Volume(VolumeRegister::MainA, Attenuation::FULL));
    let read = Command::ReadDac(StatusRegister::Register113);
    let probe = Command::ProbeAdc(AdcAddress::At13);
    let mut states = vec![State::Idle];
    states.extend(walk(
        trim,
        &[TransmitReady, TransmitReady, TransmitReady, Idle],
    ));
    states.extend(walk(trim, &[TransmitReady, NackData]));
    states.extend(walk(volume, &[TransmitReady, TransmitReady, TransmitReady]));
    states.extend(walk(
        read,
        &[TransmitReady, TransmitReady, ReceiveReady(0x3C)],
    ));
    states.extend(walk(read, &[NackAddress]));
    states.extend(walk(probe, &[TransmitReady]));
    states.extend(walk(probe, &[NackAddress]));
    let mut unique: Vec<State> = Vec::new();
    for state in states {
        if !unique.contains(&state) {
            unique.push(state);
        }
    }
    unique
}

const TABLE: &str = include_str!("i2c_bus_table.txt");

fn actual_table() -> String {
    let mut table = String::new();
    for state in representative_states() {
        for (event_label, event) in events() {
            let (next, outputs) = transition(state, event);
            let outputs: Vec<String> = outputs
                .as_slice()
                .iter()
                .map(|o| output_label(*o))
                .collect();
            table.push_str(&format!(
                "{} | {} | {} | {}\n",
                state_label(state),
                event_label,
                if outputs.is_empty() {
                    "-".to_string()
                } else {
                    outputs.join(" ")
                },
                state_label(next)
            ));
        }
    }
    table
}

#[test]
fn every_state_and_event_pair_has_its_pinned_result() {
    let actual = actual_table();
    if actual != TABLE.replace("\r\n", "\n") {
        std::fs::write(
            concat!(env!("CARGO_TARGET_TMPDIR"), "/i2c-bus-table-actual.txt"),
            &actual,
        )
        .expect("the actual table is written for review");
        panic!("the bus transition table changed; compare with i2c-bus-table-actual.txt");
    }
}

#[test]
fn the_table_reaches_every_phase_of_every_transfer_kind() {
    let labels: Vec<String> = representative_states()
        .into_iter()
        .map(state_label)
        .collect();
    for wanted in [
        "idle",
        "write0_address",
        "write0_register",
        "write0_value",
        "write0_stopping_acked",
        "write0_stopping_nacked",
        "write1_address",
        "read_address",
        "read_register",
        "read_turnaround",
        "read_stopping_byte_3C",
        "read_stopping_refused",
        "probe_address",
        "probe_stopping_acked",
        "probe_stopping_nacked",
    ] {
        assert!(
            labels.iter().any(|label| label == wanted),
            "{wanted} is walked"
        );
    }
}
