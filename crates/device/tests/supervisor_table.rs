use device::breadcrumb::{At, Weight};
use device::command::Command;
use device::fault::{FaultCode, I2cPeer};
use device::output_pin::OutputPin;
use device::phantom::PinLevel;
use device::supervisor::{Event, Fact, Output, Progress, State, Transport, Usb, transition};

fn progress_label(progress: Progress) -> String {
    let mut label = String::new();
    for (fact, letter) in [
        (Fact::TickAdvanced, 't'),
        (Fact::ControlPassDone, 'c'),
        (Fact::UsbServiced, 'u'),
    ] {
        if progress.contains(fact) {
            label.push(letter);
        }
    }
    if label.is_empty() { "-".into() } else { label }
}

fn usb_label(usb: Usb) -> &'static str {
    match usb {
        Usb::Silent => "silent",
        Usb::Serving => "serving",
    }
}

fn state_label(state: State) -> String {
    match state {
        State::Booting { usb, seen, .. } => {
            format!("booting {} {}", usb_label(usb), progress_label(seen))
        }
        State::Healthy { usb, seen, .. } => {
            format!("healthy {} {}", usb_label(usb), progress_label(seen))
        }
        State::Faulted {
            code,
            transport,
            usb,
            seen,
            ..
        } => format!(
            "faulted{} {} {} {}",
            code.number(),
            match transport {
                Transport::Stopped => "stopped",
                Transport::Running => "running",
            },
            usb_label(usb),
            progress_label(seen)
        ),
    }
}

fn event_label(event: Event) -> &'static str {
    match event {
        Event::TickAdvanced => "tick",
        Event::ControlPassDone => "pass",
        Event::UsbServiced => "usb",
        Event::UsbBus(Usb::Serving) => "bus-serving",
        Event::UsbBus(Usb::Silent) => "bus-silent",
        Event::TransportStarted => "transport",
        Event::FaultRaised(..) => "fault",
        Event::FeedWindowEnded => "window",
    }
}

fn output_label(output: Output) -> String {
    match output {
        Output::Command(Command::FeedWatchdog) => "feed".into(),
        Output::Command(Command::LatchPlaybackSilence) => "silence".into(),
        Output::Command(Command::DrivePin(OutputPin::TransportActive, PinLevel::Low)) => {
            "disconnect".into()
        }
        Output::Command(Command::StopTickTimer) => "stop-tick".into(),
        Output::Command(Command::RecordFault(code, At::Here, Weight::Raised)) => {
            format!("store{}", code.number())
        }
        Output::Command(other) => format!("{other:?}"),
        Output::FaultRaised(code) => format!("raise{}", code.number()),
        Output::Nothing => "nothing".into(),
    }
}

const EVENTS: [Event; 8] = [
    Event::TickAdvanced,
    Event::ControlPassDone,
    Event::UsbServiced,
    Event::UsbBus(Usb::Serving),
    Event::UsbBus(Usb::Silent),
    Event::TransportStarted,
    Event::FaultRaised(FaultCode::Unanswered(I2cPeer::Adc), At::Here),
    Event::FeedWindowEnded,
];

fn states() -> [State; 8] {
    let c = Progress::NONE.with(Fact::ControlPassDone);
    let tc = c.with(Fact::TickAdvanced);
    let tcu = tc.with(Fact::UsbServiced);
    [
        State::START,
        State::Booting {
            usb: Usb::Serving,
            seen: c,
        },
        State::Healthy {
            usb: Usb::Silent,
            seen: c,
        },
        State::Healthy {
            usb: Usb::Silent,
            seen: tc,
        },
        State::Healthy {
            usb: Usb::Serving,
            seen: tc,
        },
        State::Healthy {
            usb: Usb::Serving,
            seen: tcu,
        },
        State::Faulted {
            code: FaultCode::BoardStalled,
            transport: Transport::Stopped,
            usb: Usb::Silent,
            seen: c,
        },
        State::Faulted {
            code: FaultCode::BoardStalled,
            transport: Transport::Running,
            usb: Usb::Serving,
            seen: tc,
        },
    ]
}

const TABLE: &str = "\
booting silent - | tick => booting silent t | 
booting silent - | pass => booting silent c | 
booting silent - | usb => booting silent u | 
booting silent - | bus-serving => booting serving - | 
booting silent - | bus-silent => booting silent - | 
booting silent - | transport => healthy silent - | 
booting silent - | fault => faulted4 stopped silent - | silence disconnect stop-tick store4 raise4
booting silent - | window => booting silent - | 
booting serving c | tick => booting serving tc | 
booting serving c | pass => booting serving c | 
booting serving c | usb => booting serving cu | 
booting serving c | bus-serving => booting serving c | 
booting serving c | bus-silent => booting silent c | 
booting serving c | transport => healthy serving c | 
booting serving c | fault => faulted4 stopped serving c | silence disconnect stop-tick store4 raise4
booting serving c | window => booting serving c | 
healthy silent c | tick => healthy silent tc | 
healthy silent c | pass => healthy silent c | 
healthy silent c | usb => healthy silent cu | 
healthy silent c | bus-serving => healthy serving c | 
healthy silent c | bus-silent => healthy silent c | 
healthy silent c | transport => healthy silent c | 
healthy silent c | fault => faulted4 stopped silent c | silence disconnect stop-tick store4 raise4
healthy silent c | window => healthy silent c | 
healthy silent tc | tick => healthy silent tc | 
healthy silent tc | pass => healthy silent tc | 
healthy silent tc | usb => healthy silent tcu | 
healthy silent tc | bus-serving => healthy serving tc | 
healthy silent tc | bus-silent => healthy silent tc | 
healthy silent tc | transport => healthy silent tc | 
healthy silent tc | fault => faulted4 stopped silent tc | silence disconnect stop-tick store4 raise4
healthy silent tc | window => healthy silent - | feed
healthy serving tc | tick => healthy serving tc | 
healthy serving tc | pass => healthy serving tc | 
healthy serving tc | usb => healthy serving tcu | 
healthy serving tc | bus-serving => healthy serving tc | 
healthy serving tc | bus-silent => healthy silent tc | 
healthy serving tc | transport => healthy serving tc | 
healthy serving tc | fault => faulted4 stopped serving tc | silence disconnect stop-tick store4 raise4
healthy serving tc | window => healthy serving tc | 
healthy serving tcu | tick => healthy serving tcu | 
healthy serving tcu | pass => healthy serving tcu | 
healthy serving tcu | usb => healthy serving tcu | 
healthy serving tcu | bus-serving => healthy serving tcu | 
healthy serving tcu | bus-silent => healthy silent tcu | 
healthy serving tcu | transport => healthy serving tcu | 
healthy serving tcu | fault => faulted4 stopped serving tcu | silence disconnect stop-tick store4 raise4
healthy serving tcu | window => healthy serving - | feed
faulted6 stopped silent c | tick => faulted6 stopped silent tc | 
faulted6 stopped silent c | pass => faulted6 stopped silent c | 
faulted6 stopped silent c | usb => faulted6 stopped silent cu | 
faulted6 stopped silent c | bus-serving => faulted6 stopped serving c | 
faulted6 stopped silent c | bus-silent => faulted6 stopped silent c | 
faulted6 stopped silent c | transport => faulted6 running silent c | 
faulted6 stopped silent c | fault => faulted6 stopped silent c | 
faulted6 stopped silent c | window => faulted6 stopped silent - | feed
faulted6 running serving tc | tick => faulted6 running serving tc | 
faulted6 running serving tc | pass => faulted6 running serving tc | 
faulted6 running serving tc | usb => faulted6 running serving tcu | 
faulted6 running serving tc | bus-serving => faulted6 running serving tc | 
faulted6 running serving tc | bus-silent => faulted6 running silent tc | 
faulted6 running serving tc | transport => faulted6 running serving tc | 
faulted6 running serving tc | fault => faulted6 running serving tc | 
faulted6 running serving tc | window => faulted6 running serving tc | 
";

#[test]
fn every_supervisor_state_and_event_pair_has_its_pinned_result() {
    let mut actual = String::new();
    for state in states() {
        for event in EVENTS {
            let (next, outputs) = transition(state, event);
            let produced: Vec<String> = outputs
                .as_slice()
                .iter()
                .map(|o| output_label(*o))
                .collect();
            actual.push_str(&format!(
                "{} | {} => {} | {}\n",
                state_label(state),
                event_label(event),
                state_label(next),
                produced.join(" ")
            ));
        }
    }
    let normalise = |text: &str| -> Vec<String> {
        text.lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    };
    assert_eq!(normalise(&actual), normalise(TABLE), "actual:\n{actual}");
}
