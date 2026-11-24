mod boot_board;

use boot_board::{Bench, Caller, Responder, is_transfer, run_bench};
use device::adc::{AdcAddress, AdcKind};
use device::boot::{
    ClockInput, Event, Outcome, Output, Pll, PllLock, Stage, State, Subsystem, transition,
};
use device::command::Command;
use device::fault::FaultCode;
use device::phantom::Milliseconds;

const ENOUGH_PASSES: usize = 5_000;

fn kind_label(kind: AdcKind) -> &'static str {
    match kind {
        AdcKind::Unanswered => "t0",
        AdcKind::Answered(AdcAddress::At13) => "t1",
        AdcKind::Answered(AdcAddress::At4F) => "t2",
    }
}

fn faults_label(outcome: Outcome) -> String {
    if outcome.faults.is_empty() {
        return "-".into();
    }
    let mut numbers: Vec<String> = Vec::new();
    for code in FaultCode::ALL {
        if outcome.faults.contains(code) {
            numbers.push(code.number().to_string());
        }
    }
    numbers.join("+")
}

fn running_label(outcome: Outcome) -> String {
    let mut label = String::new();
    for (subsystem, letter) in [
        (Subsystem::Audio, 'a'),
        (Subsystem::Panel, 'p'),
        (Subsystem::Usb, 'u'),
    ] {
        if outcome.runs(subsystem) {
            label.push(letter);
        }
    }
    if label.is_empty() { "-".into() } else { label }
}

fn state_label(state: State) -> String {
    match state {
        State::Scripted(stage, _, outcome) => {
            format!("{} {}", stage_label(stage), faults_label(outcome))
        }
        State::VolumeRamp(kind, _, outcome) => {
            format!("volume_ramp_{} {}", kind_label(kind), faults_label(outcome))
        }
        State::Running(kind, outcome) => {
            format!("running_{} {}", kind_label(kind), faults_label(outcome))
        }
        State::Degraded(outcome) => {
            format!(
                "degraded_{} {}",
                running_label(outcome),
                faults_label(outcome)
            )
        }
    }
}

fn stage_label(stage: Stage) -> String {
    match stage {
        Stage::Quiesce => "quiesce".into(),
        Stage::Clocks => "clocks".into(),
        Stage::UsbAttach => "usb_attach".into(),
        Stage::AnalogOff => "analog_off".into(),
        Stage::Rails => "rails".into(),
        Stage::StartLcd => "start_lcd".into(),
        Stage::AudioClocks => "audio_clocks".into(),
        Stage::DacAttenuate => "dac_attenuate".into(),
        Stage::AdcProbe(kind) => format!("adc_probe_{}", kind_label(kind)),
        Stage::TransportStart(kind) => format!("transport_start_{}", kind_label(kind)),
        Stage::DacInit(kind) => format!("dac_init_{}", kind_label(kind)),
        Stage::OutputConnect(kind) => format!("output_connect_{}", kind_label(kind)),
        Stage::AdcInit(kind) => format!("adc_init_{}", kind_label(kind)),
        Stage::Panel(kind) => format!("panel_{}", kind_label(kind)),
    }
}

fn events() -> [(&'static str, Event); 14] {
    let all = [
        Event::StepDone,
        Event::I2cAcked,
        Event::I2cNacked,
        Event::StatusRead(0x11),
        Event::ClockInputRead(ClockInput::Ready),
        Event::ClockInputRead(ClockInput::NotReady),
        Event::PllLockRead(Pll::Core, PllLock::Locked),
        Event::PllLockRead(Pll::Core, PllLock::TimedOut),
        Event::PllLockRead(Pll::Audio, PllLock::Locked),
        Event::PllLockRead(Pll::Audio, PllLock::TimedOut),
        Event::PllLockRead(Pll::UsbPhy, PllLock::Locked),
        Event::PllLockRead(Pll::UsbPhy, PllLock::TimedOut),
        Event::CommandRefused,
        Event::Elapsed(Milliseconds(1)),
    ];
    all.map(|event| {
        let label = match event {
            Event::StepDone => "done",
            Event::I2cAcked => "ack",
            Event::I2cNacked => "nack",
            Event::StatusRead(_) => "read",
            Event::ClockInputRead(ClockInput::Ready) => "xo",
            Event::ClockInputRead(ClockInput::NotReady) => "no-xo",
            Event::PllLockRead(Pll::Core, PllLock::Locked) => "core-lock",
            Event::PllLockRead(Pll::Core, PllLock::TimedOut) => "core-no-lock",
            Event::PllLockRead(Pll::Audio, PllLock::Locked) => "audio-lock",
            Event::PllLockRead(Pll::Audio, PllLock::TimedOut) => "audio-no-lock",
            Event::PllLockRead(Pll::UsbPhy, PllLock::Locked) => "phy-lock",
            Event::PllLockRead(Pll::UsbPhy, PllLock::TimedOut) => "phy-no-lock",
            Event::CommandRefused => "refused",
            Event::Elapsed(_) => "1ms",
            Event::ControlPassElapsed(_) => "pass",
        };
        (label, event)
    })
}

fn waiting_on(emitted: &[Output]) -> &'static str {
    let awaited = emitted
        .iter()
        .rev()
        .find(|output| !matches!(output, Output::Log(_)));
    match awaited {
        None => "idle",
        Some(Output::Command(Command::ReadClockInput)) => "clock-input",
        Some(Output::Command(Command::SelectCorePllAsMainClock)) => "core-pll",
        Some(Output::Command(Command::SwitchPll0Family(_))) => "audio-pll",
        Some(Output::Command(Command::AttachUsb)) => "usb-phy-pll",
        Some(Output::Command(command)) if is_transfer(*command) => match command {
            Command::ReadDac(_) => "read",
            Command::ProbeAdc(_) => "probe",
            _ => "write",
        },
        Some(_) => "batch",
    }
}

fn summary(outputs: &[Output]) -> String {
    match outputs {
        [] => "-".into(),
        [Output::Command(command)] => format!("{command:?}"),
        [Output::Supervisor(event)] => format!("{event:?}"),
        [Output::Phantom(event)] => format!("{event:?}"),
        [Output::Log(record), rest @ ..] => format!("{record:?}+{}", rest.len()),
        [first, rest @ ..] => format!("{first:?}+{}", rest.len()),
    }
}

fn transfers_of(bench: Bench) -> Vec<Command> {
    run_bench(bench, ENOUGH_PASSES)
        .trace
        .iter()
        .filter_map(|seen| match seen {
            boot_board::Seen::Command(command) if is_transfer(*command) => Some(*command),
            _ => None,
        })
        .collect()
}

fn representative_states() -> Vec<(String, State)> {
    let mut found: Vec<(String, State)> = Vec::new();
    let first_dac_init_write = 12;
    let panel_write = transfers_of(Bench::ALL_ACK).len();
    let benches = [
        Bench::ALL_ACK,
        Bench {
            acks_at_13: false,
            acks_at_4f: true,
            ..Bench::ALL_ACK
        },
        Bench {
            acks_at_13: false,
            acks_at_4f: false,
            ..Bench::ALL_ACK
        },
        Bench {
            unanswered_transfer: Some(first_dac_init_write),
            ..Bench::ALL_ACK
        },
        Bench {
            unanswered_transfer: Some(panel_write),
            ..Bench::ALL_ACK
        },
        Bench {
            clock_input: ClockInput::NotReady,
            ..Bench::ALL_ACK
        },
        Bench {
            core_pll_lock: PllLock::TimedOut,
            ..Bench::ALL_ACK
        },
        Bench {
            audio_pll_lock: PllLock::TimedOut,
            ..Bench::ALL_ACK
        },
    ];
    for bench in benches {
        let (mut caller, first) = Caller::new();
        let mut emitted = first;
        let mut raw = device::boot::begin().1.as_slice().to_vec();
        let mut responder = Responder::new(bench);
        let mut idle_steps = 0u32;
        while !matches!(caller.boot, State::Running(..) | State::Degraded(_)) {
            let key = format!("{} {}", state_label(caller.boot), waiting_on(&raw));
            if !found.iter().any(|(k, _)| *k == key) {
                found.push((key, caller.boot));
            }
            let event = match responder.answer(&emitted) {
                Some(event) => event,
                None => {
                    idle_steps += 1;
                    if idle_steps % 5 == 0 {
                        Event::ControlPassElapsed(bench.targets)
                    } else {
                        Event::Elapsed(Milliseconds(1))
                    }
                }
            };
            let before = caller.boot;
            raw = transition(before, event).1.as_slice().to_vec();
            emitted = caller.feed(event);
        }
        let key = format!("{} idle", state_label(caller.boot));
        if !found.iter().any(|(k, _)| *k == key) {
            found.push((key, caller.boot));
        }
    }
    found
}

const TABLE: &str = include_str!("boot_table.txt");

#[test]
fn every_state_and_event_pair_has_its_pinned_result() {
    let mut actual = String::new();
    for (key, state) in representative_states() {
        for (label, event) in events() {
            let (next, outputs) = transition(state, event);
            let next_label = if next == state {
                "same".to_string()
            } else if outputs.as_slice().is_empty() && state_label(next) == state_label(state) {
                "waiting".to_string()
            } else {
                format!("{} {}", state_label(next), waiting_on(outputs.as_slice()))
            };
            actual.push_str(&format!(
                "{key} | {label} => {next_label} | {}\n",
                summary(outputs.as_slice())
            ));
        }
    }
    if actual.trim() != TABLE.trim() {
        std::fs::write(
            concat!(env!("CARGO_TARGET_TMPDIR"), "/boot-table-actual.txt"),
            &actual,
        )
        .unwrap();
        panic!("table differs; actual written to the target tmpdir");
    }
}

#[test]
fn the_table_covers_every_stage_and_terminal_state() {
    let keys: Vec<String> = representative_states()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    for wanted in [
        "quiesce",
        "clocks",
        "usb_attach",
        "analog_off",
        "rails",
        "start_lcd",
        "audio_clocks",
        "dac_attenuate",
        "adc_probe_t0",
        "adc_probe_t1",
        "transport_start_t0",
        "transport_start_t1",
        "transport_start_t2",
        "dac_init_t1",
        "output_connect_t1",
        "adc_init_t0",
        "adc_init_t1",
        "adc_init_t2",
        "panel_t1",
        "volume_ramp_t1",
        "running_t1",
        "degraded_pu",
        "degraded_p",
    ] {
        assert!(
            keys.iter().any(|k| k.split(' ').next() == Some(wanted)),
            "{wanted} missing from {keys:?}"
        );
    }
}
