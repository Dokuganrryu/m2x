mod console_support;

use console_support::typed;
use device::console::{Output, Refusal};

const PROTOCOL_LINES: [&str; 6] = [
    "register0 signed",
    "register0 spare",
    "register0 unplugged",
    "register0 request",
    "register0 confirm 424242",
    "register0 abort",
];

fn outcome(line: &str) -> Output {
    let text = format!("{line}\n");
    match typed(text.as_bytes()).as_slice() {
        [output] => *output,
        other => panic!("{line:?} gave {other:?}"),
    }
}

#[test]
fn register_62_and_unknown_protocol_words_are_refused_in_every_build() {
    for line in [
        "register62",
        "register62 signed",
        "register0 62",
        "register0 write 0",
        "register0",
        "reg0 signed",
    ] {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::NotAllowlisted),
            "{line}"
        );
    }
}

#[cfg(not(feature = "register-zero-protocol"))]
#[test]
fn the_normal_console_refuses_every_register_zero_protocol_command_by_name() {
    for line in PROTOCOL_LINES {
        assert_eq!(
            outcome(line),
            Output::Reject(Refusal::NotAllowlisted),
            "{line}"
        );
    }
}

#[cfg(feature = "register-zero-protocol")]
mod protocol_build {
    use super::{PROTOCOL_LINES, outcome};
    use device::console::{ConsoleCommand, Output, Refusal};
    use device::phantom::PhantomGrant;
    use device::register_zero::{
        BoardRecord, ConfirmationNonce, Event, OperatorStep, Output as SessionOutput, State,
        TransmitBuffers, VolumeEvidence, transition,
    };

    fn step_of(line: &str) -> OperatorStep {
        match outcome(line) {
            Output::Execute(ConsoleCommand::RegisterZero(step)) => step,
            other => panic!("{line:?} gave {other:?}"),
        }
    }

    #[test]
    fn the_protocol_console_maps_each_command_to_its_operator_step() {
        let nonce = ConfirmationNonce::new(424_242).expect("six digits");
        let steps: Vec<OperatorStep> = PROTOCOL_LINES.iter().map(|line| step_of(line)).collect();
        assert_eq!(
            steps,
            [
                OperatorStep::ProtocolSigned,
                OperatorStep::SpareUnit,
                OperatorStep::OutputsUnplugged,
                OperatorStep::RequestReplay,
                OperatorStep::Confirm(nonce),
                OperatorStep::Abort,
            ]
        );
    }

    #[test]
    fn a_nonce_typed_with_its_leading_zeros_is_the_printed_nonce() {
        let printed = ConfirmationNonce::from_boot_entropy(4_213);
        let line = format!(
            "register0 confirm {}",
            String::from_utf8_lossy(&printed.digits())
        );
        assert_eq!(step_of(&line), OperatorStep::Confirm(printed));
    }

    #[test]
    fn a_malformed_or_out_of_range_confirmation_is_refused() {
        for (line, refusal) in [
            ("register0 confirm", Refusal::MalformedArguments),
            ("register0 confirm 1000000", Refusal::SettingOutOfRange),
            ("register0 confirm -1", Refusal::MalformedArguments),
            ("register0 confirm 12.5", Refusal::MalformedArguments),
            (
                "register0 confirm 424242 424242",
                Refusal::MalformedArguments,
            ),
            ("register0 signed now", Refusal::MalformedArguments),
        ] {
            assert_eq!(outcome(line), Output::Reject(refusal), "{line}");
        }
    }

    #[test]
    fn typed_console_lines_walk_the_session_to_the_single_register_zero_write() {
        let record = BoardRecord {
            volume: VolumeEvidence::FullyAttenuatedAndAckedThisBoot,
            transmit: TransmitBuffers::Zeroed,
            phantom: PhantomGrant::NONE,
        };
        let mut state = State::locked(ConfirmationNonce::from_boot_entropy(424_242));
        let mut emitted = Vec::new();
        for line in &PROTOCOL_LINES[..5] {
            let (next, outputs) = transition(state, Event::from_operator(step_of(line), record));
            emitted.extend_from_slice(outputs.as_slice());
            state = next;
        }
        let writes = emitted
            .iter()
            .filter(|output| matches!(output, SessionOutput::WriteRegisterZero(_)))
            .count();
        assert_eq!(writes, 1);
    }
}
