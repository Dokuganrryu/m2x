mod console_support;

use console_support::feed;
use device::boot::{self, Outcome};
use device::console::{ConsoleCommand, Event, Output, Refusal, Report, State, transition};
use device::rate_change;
use device::usb::handover::{NoticesSeen, UsbNotices};
use device::usb::request::DataTerminal;
use device::usb::service::Notice;
use dsp::SampleRate;

const HARDWARE_NOTICES: [Notice; 5] = [
    Notice::RateChange(rate_change::Event::SetCur(SampleRate::Hz96000)),
    Notice::FadeOutputsToSilence,
    Notice::ReleaseOutputSilence,
    Notice::SuspendPhantom,
    Notice::ResumePhantom,
];

const READ_ONLY_LINES: [(&[u8], Report); 12] = [
    (b"status\n", Report::Status),
    (b"counters\n", Report::Counters),
    (b"dma\n", Report::Dma),
    (b"knob\n", Report::Knob),
    (b"serial\n", Report::Serial),
    (b"version\n", Report::Version),
    (b"cycles\n", Report::Cycles),
    (b"fault\n", Report::Fault),
    (b"panel\n", Report::Panel),
    (b"clocks\n", Report::Clocks),
    (b"i2c\n", Report::Bus),
    (b"power\n", Report::Power),
];

const HARDWARE_LINES: [&[u8]; 6] = [
    b"dsp bypass\n",
    b"dsp on\n",
    b"peq 1 peak 1000 3 1\n",
    b"peq off 1\n",
    b"lim -1.0 100\n",
    b"meter reset\n",
];

fn states_before_running() -> [(&'static str, boot::State); 2] {
    [
        ("booting", boot::begin().0),
        ("degraded", boot::State::Degraded(Outcome::START)),
    ]
}

fn the_thread_sees_running(state: &boot::State) -> bool {
    matches!(state, boot::State::Running(..))
}

fn console_event(notice: Notice) -> Option<Event> {
    match notice {
        Notice::DataTerminal(DataTerminal::Present) => Some(Event::PortOpened),
        Notice::DataTerminal(DataTerminal::Absent) => Some(Event::PortClosed),
        Notice::RateChange(_)
        | Notice::FadeOutputsToSilence
        | Notice::ReleaseOutputSilence
        | Notice::SuspendPhantom
        | Notice::ResumePhantom => None,
    }
}

fn answered(running: bool, published: &[Notice], typed: &[u8]) -> (Vec<Notice>, Vec<Output>) {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    for notice in published {
        notices.publish(*notice);
    }
    let delivered = seen.drain(&notices, running).as_slice().to_vec();
    let session = delivered
        .iter()
        .filter_map(|notice| console_event(*notice))
        .fold(State::Closed, |state, event| transition(state, event).0);
    let outputs = feed(session, typed)
        .1
        .into_iter()
        .map(|output| output.admitted(running))
        .collect();
    (delivered, outputs)
}

#[test]
fn before_running_the_terminal_opens_and_the_fault_report_answers() {
    for (stage, state) in states_before_running() {
        let (delivered, outputs) = answered(
            the_thread_sees_running(&state),
            &[Notice::DataTerminal(DataTerminal::Present)],
            b"fault\n",
        );
        assert_eq!(
            delivered,
            [Notice::DataTerminal(DataTerminal::Present)],
            "{stage}"
        );
        assert_eq!(
            outputs,
            [Output::Execute(ConsoleCommand::Report(Report::Fault))],
            "{stage}"
        );
    }
}

#[test]
fn before_running_no_notice_but_the_consoles_own_passes() {
    for (stage, state) in states_before_running() {
        let running = the_thread_sees_running(&state);
        for notice in HARDWARE_NOTICES {
            let mut published = vec![notice, Notice::DataTerminal(DataTerminal::Present)];
            assert_eq!(
                answered(running, &published, b"").0,
                [Notice::DataTerminal(DataTerminal::Present)],
                "{stage}: {notice:?}"
            );
            published.push(Notice::DataTerminal(DataTerminal::Absent));
            assert_eq!(
                answered(running, &published, b"").0,
                [
                    Notice::DataTerminal(DataTerminal::Present),
                    Notice::DataTerminal(DataTerminal::Absent),
                ],
                "{stage}: {notice:?}"
            );
        }
    }
}

#[test]
fn a_rate_request_dropped_before_running_is_not_replayed_when_the_boot_reaches_running() {
    let notices = UsbNotices::new();
    let mut seen = NoticesSeen::START;
    notices.publish(Notice::RateChange(rate_change::Event::SetCur(
        SampleRate::Hz96000,
    )));
    notices.publish(Notice::DataTerminal(DataTerminal::Present));
    assert_eq!(
        seen.drain(&notices, false).as_slice(),
        [Notice::DataTerminal(DataTerminal::Present)]
    );
    assert!(seen.drain(&notices, true).as_slice().is_empty());
}

#[test]
fn before_running_every_report_answers_and_every_command_that_acts_is_refused() {
    let opened = [Notice::DataTerminal(DataTerminal::Present)];
    for (stage, state) in states_before_running() {
        let running = the_thread_sees_running(&state);
        for (line, report) in READ_ONLY_LINES {
            assert_eq!(
                answered(running, &opened, line).1,
                [Output::Execute(ConsoleCommand::Report(report))],
                "{stage}: {report:?}"
            );
        }
        for line in HARDWARE_LINES {
            assert_eq!(
                answered(running, &opened, line).1,
                [Output::Reject(Refusal::NotRunning)],
                "{stage}: {}",
                String::from_utf8_lossy(line)
            );
        }
    }
}

#[test]
fn once_running_every_command_is_executed_as_before() {
    for line in HARDWARE_LINES {
        let mut session = transition(State::Closed, Event::PortOpened).0;
        let mut expected = Vec::new();
        for &byte in line {
            let (next, outputs) = transition(session, Event::Byte(byte));
            session = next;
            expected.extend_from_slice(outputs.as_slice());
        }
        assert!(matches!(expected.as_slice(), [Output::Execute(_)]));
        assert_eq!(
            answered(true, &[Notice::DataTerminal(DataTerminal::Present)], line).1,
            expected
        );
    }
}

#[test]
fn a_refusal_before_running_says_so() {
    assert_eq!(Refusal::NotRunning.reply(), b"refused: not running\r\n");
}
