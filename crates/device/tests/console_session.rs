mod console_support;

use console_support::{feed, opened, typed};
use device::console::{
    BandNumber, ConsoleCommand, Event, LINE_CAPACITY, Output, Refusal, Report, State, transition,
};
use dsp::{Band, DspMode, LimiterSettings};

const STATUS: Output = Output::Execute(ConsoleCommand::Report(Report::Status));
const FAULT: Output = Output::Execute(ConsoleCommand::Report(Report::Fault));

fn padded_status(total: usize) -> Vec<u8> {
    let mut line = vec![b' '; total - b"status".len()];
    line.extend_from_slice(b"status");
    line
}

#[test]
fn line_feed_carriage_return_and_both_together_each_end_one_line() {
    assert_eq!(typed(b"status\n"), [STATUS]);
    assert_eq!(typed(b"status\r"), [STATUS]);
    assert_eq!(typed(b"status\r\n"), [STATUS]);
    assert_eq!(typed(b"status\r\nfault\r\n"), [STATUS, FAULT]);
    assert_eq!(typed(b"status\n\rfault\n\r"), [STATUS, FAULT]);
}

#[test]
fn a_blank_or_all_space_line_emits_nothing() {
    assert_eq!(typed(b"\n\r\n\r\r\n"), []);
    assert_eq!(typed(b"     \n"), []);
}

#[test]
fn a_partial_line_emits_nothing_until_its_line_end_arrives() {
    let (state, outputs) = feed(opened(), b"sta");
    assert_eq!(outputs, []);
    let (state, outputs) = feed(state, b"tu");
    assert_eq!(outputs, []);
    let (_, outputs) = feed(state, b"s\n");
    assert_eq!(outputs, [STATUS]);
}

#[test]
fn a_line_of_exactly_the_capacity_is_parsed() {
    let mut text = padded_status(LINE_CAPACITY);
    text.push(b'\n');
    assert_eq!(typed(&text), [STATUS]);
}

#[test]
fn a_line_one_byte_over_the_capacity_is_refused_once_at_its_end_and_the_next_line_parses_clean() {
    let mut text = padded_status(LINE_CAPACITY + 1);
    let (state, outputs) = feed(opened(), &text);
    assert_eq!(outputs, []);
    assert_eq!(
        feed(state, b"\nstatus\n").1,
        [Output::Reject(Refusal::LineTooLong), STATUS]
    );
    text.extend_from_slice(b"\r\nstatus\r\n");
    assert_eq!(typed(&text), [Output::Reject(Refusal::LineTooLong), STATUS]);
}

#[test]
fn an_overlong_line_that_ends_in_a_valid_command_never_executes() {
    let mut text = vec![b'x'; 10 * LINE_CAPACITY];
    text.extend_from_slice(b" status\n");
    assert_eq!(typed(&text), [Output::Reject(Refusal::LineTooLong)]);
}

#[test]
fn a_byte_outside_printable_ascii_refuses_its_whole_line_and_the_next_line_parses_clean() {
    for poison in [0x00u8, 0x07, b'\t', 0x08, 0x1B, 0x7F, 0x80, 0xC3, 0xFF] {
        let text = [b"sta".as_slice(), &[poison], b"tus\nstatus\n"].concat();
        assert_eq!(
            typed(&text),
            [Output::Reject(Refusal::NotPrintableAscii), STATUS],
            "{poison:#04x}"
        );
    }
}

#[test]
fn the_first_cause_of_a_discarded_line_is_the_one_reported() {
    let mut overlong_then_poison = vec![b'x'; LINE_CAPACITY + 1];
    overlong_then_poison.extend_from_slice(&[0xFF, b'\n']);
    assert_eq!(
        typed(&overlong_then_poison),
        [Output::Reject(Refusal::LineTooLong)]
    );
    let mut poison_then_overlong = vec![0xFF];
    poison_then_overlong.extend_from_slice(&[b'x'; 2 * LINE_CAPACITY]);
    poison_then_overlong.push(b'\n');
    assert_eq!(
        typed(&poison_then_overlong),
        [Output::Reject(Refusal::NotPrintableAscii)]
    );
}

#[test]
fn bytes_before_the_port_opens_are_ignored() {
    let (state, outputs) = feed(State::Closed, b"status\nfault\n");
    assert_eq!((state, outputs), (State::Closed, vec![]));
}

#[test]
fn closing_the_port_drops_a_partial_line() {
    let (state, _) = feed(opened(), b"peq off");
    let (state, outputs) = transition(state, Event::PortClosed);
    assert_eq!((state, outputs.as_slice()), (State::Closed, [].as_slice()));
    let (state, _) = transition(state, Event::PortOpened);
    assert_eq!(
        feed(state, b" 1\nstatus\n").1,
        [Output::Reject(Refusal::NotAllowlisted), STATUS]
    );
}

#[test]
fn reopening_the_port_drops_a_partial_or_discarded_line() {
    for partial in [b"peq off".as_slice(), &[0xFF], &[b'x'; LINE_CAPACITY + 1]] {
        let (state, _) = feed(opened(), partial);
        let (state, outputs) = transition(state, Event::PortOpened);
        assert_eq!(outputs.as_slice(), []);
        assert_eq!(feed(state, b"status\n").1, [STATUS]);
    }
}

#[test]
fn every_console_command_variant_carries_only_dsp_settings_or_a_report_name() {
    fn payload_types(command: ConsoleCommand) {
        match command {
            ConsoleCommand::Report(report) => {
                let _: Report = report;
            }
            ConsoleCommand::SetDspMode(mode) => {
                let _: DspMode = mode;
            }
            ConsoleCommand::SetBand { band, settings } => {
                let _: (BandNumber, Band) = (band, settings);
            }
            ConsoleCommand::DisableBand(band) => {
                let _: BandNumber = band;
            }
            ConsoleCommand::SetLimiter(settings) => {
                let _: LimiterSettings = settings;
            }
            ConsoleCommand::ResetMeters => {}
            #[cfg(feature = "register-zero-protocol")]
            ConsoleCommand::RegisterZero(step) => {
                let _: device::register_zero::OperatorStep = step;
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::Preset(preset) => {
                let _: Option<device::preset::Preset> = preset;
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::SetTrim(trim) => {
                let _: dsp::PresetTrim = trim;
            }
            #[cfg(feature = "sound2")]
            ConsoleCommand::BeginBands | ConsoleCommand::CommitBands => {}
            #[cfg(feature = "screen2")]
            ConsoleCommand::LcdMotion(choice) => {
                let _: Option<bool> = choice;
            }
            #[cfg(feature = "sound3")]
            ConsoleCommand::MeterProfile(choice) => {
                let _: Option<device::lcd::screen::MeterProfileName> = choice;
            }
        }
    }
    payload_types(ConsoleCommand::ResetMeters);
    assert_eq!(size_of::<BandNumber>(), 1);
}
