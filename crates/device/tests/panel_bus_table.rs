use device::command::Command;
use device::control_pass::Monitoring;
use device::dac::{Attenuation, DacWrite, VolumeRegister};
use device::expander::ExpanderOutputs;
use device::i2c::Answer;
use device::panel_bus::{
    CONTROL_VOLUME_REGISTERS, Event, Output, PanelTransfer, State, transition,
};
use device::phantom::PhantomGrant;

fn monitored() -> ExpanderOutputs {
    ExpanderOutputs::showing(PhantomGrant::NONE, [Monitoring::On, Monitoring::Off])
}

fn register_label(register: VolumeRegister) -> String {
    let index = CONTROL_VOLUME_REGISTERS
        .iter()
        .position(|r| *r == register)
        .map_or("other".to_string(), |index| index.to_string());
    format!("vol{index}")
}

fn transfer_label(transfer: PanelTransfer) -> String {
    match transfer {
        PanelTransfer::Volume(register, level) => format!(
            "{}={}",
            register_label(register),
            level.half_decibel_steps()
        ),
        PanelTransfer::Leds(outputs) => format!("leds={:02X}", outputs.bits()),
        PanelTransfer::Buttons => "buttons".into(),
    }
}

fn state_label(state: State) -> String {
    let mailbox = state.mailbox();
    let mut pending = Vec::new();
    for register in CONTROL_VOLUME_REGISTERS {
        for steps in [4, 9] {
            let wanted =
                PanelTransfer::Volume(register, Attenuation::from_half_decibel_steps(steps));
            if mailbox.holds(wanted) {
                pending.push(transfer_label(wanted));
            }
        }
    }
    if mailbox.holds(PanelTransfer::Leds(monitored())) {
        pending.push(transfer_label(PanelTransfer::Leds(monitored())));
    }
    if mailbox.holds(PanelTransfer::Buttons) {
        pending.push("buttons".into());
    }
    let shown = mailbox
        .written_leds()
        .map_or("-".to_string(), |outputs| format!("{:02X}", outputs.bits()));
    let head = match state.in_flight() {
        None => "waiting".to_string(),
        Some(transfer) => format!("flying {}", transfer_label(transfer)),
    };
    format!("{head} [{}] shown={shown}", pending.join(","))
}

fn event_label(event: Event) -> String {
    match event {
        Event::Wanted(transfer) => format!("want {}", transfer_label(transfer)),
        Event::BusFree => "free".into(),
        Event::Answered(Answer::Acked) => "acked".into(),
        Event::Answered(Answer::Nacked) => "nacked".into(),
        Event::Answered(Answer::Read(byte)) => format!("read{byte:02X}"),
        Event::Preempted => "preempted".into(),
        Event::ExpanderReady => "expander-ready".into(),
    }
}

fn output_label(output: Output) -> String {
    match output {
        Output::Begin(Command::WriteDac(DacWrite::Volume(register, level))) => format!(
            "begin {}={}",
            register_label(register),
            level.half_decibel_steps()
        ),
        Output::Begin(Command::WriteExpanderOutputs(outputs)) => {
            format!("begin leds={:02X}", outputs.bits())
        }
        Output::Begin(Command::ReadExpanderInputs) => "begin buttons".into(),
        Output::Begin(other) => format!("begin {other:?}"),
        Output::ButtonsRead(inputs) => format!("buttons{:02X}", inputs.0),
        Output::Fault(code) => format!("fault{}", code.number()),
    }
}

fn after(events: &[Event]) -> State {
    events
        .iter()
        .fold(State::START, |state, event| transition(state, *event).0)
}

fn main_a(steps: u8) -> PanelTransfer {
    PanelTransfer::Volume(
        VolumeRegister::MainA,
        Attenuation::from_half_decibel_steps(steps),
    )
}

fn states() -> Vec<State> {
    let leds = PanelTransfer::Leds(monitored());
    vec![
        State::START,
        after(&[Event::Wanted(leds), Event::Wanted(PanelTransfer::Buttons)]),
        after(&[
            Event::Wanted(leds),
            Event::BusFree,
            Event::Answered(Answer::Acked),
        ]),
        after(&[Event::Wanted(main_a(9)), Event::BusFree]),
        after(&[
            Event::Wanted(main_a(9)),
            Event::BusFree,
            Event::Wanted(PanelTransfer::Buttons),
        ]),
        after(&[Event::Wanted(leds), Event::BusFree]),
        after(&[Event::Wanted(PanelTransfer::Buttons), Event::BusFree]),
    ]
}

fn events() -> Vec<Event> {
    vec![
        Event::Wanted(main_a(4)),
        Event::Wanted(PanelTransfer::Leds(monitored())),
        Event::Wanted(PanelTransfer::Buttons),
        Event::BusFree,
        Event::Answered(Answer::Acked),
        Event::Answered(Answer::Nacked),
        Event::Answered(Answer::Read(0x7B)),
        Event::Preempted,
        Event::ExpanderReady,
    ]
}

const TABLE: &str = include_str!("panel_bus_table.txt");

#[test]
fn every_panel_bus_state_and_event_pair_has_its_pinned_result() {
    let mut actual = String::new();
    for state in states() {
        for event in events() {
            let (next, outputs) = transition(state, event);
            let produced: Vec<String> = outputs
                .as_slice()
                .iter()
                .map(|output| output_label(*output))
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
    if std::env::var_os("PANEL_BUS_TABLE_PRINT").is_some() {
        print!("{actual}");
    }
    assert_eq!(normalise(&actual), normalise(TABLE), "actual:\n{actual}");
}
