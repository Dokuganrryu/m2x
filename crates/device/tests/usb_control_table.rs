mod usb_control_support;
mod usb_support;

use device::usb::BusSpeed;
use device::usb::answer::ControlContext;
use device::usb::control::{Event, State};
use usb_control_support::*;
use usb_support::{context, enumerated_at};

fn representative_states<'a>(context: &ControlContext<'a>) -> Vec<(&'static str, State<'a>)> {
    let scripts: [(&str, Vec<Event<'static>>); 7] = [
        ("Idle", vec![]),
        (
            "SendingReply",
            vec![Event::Setup(GET_CONFIGURATION_DESCRIPTOR_255)],
        ),
        (
            "AwaitingHostStatus",
            vec![Event::Setup(GET_DEVICE_DESCRIPTOR_18), Event::InSent],
        ),
        ("ReceivingData", vec![Event::Setup(SET_LINE_CODING)]),
        ("ConfirmingWrite", vec![Event::Setup(SET_CONFIGURATION_1)]),
        (
            "ConfirmingEmptyRead",
            vec![Event::Setup(GET_DEVICE_DESCRIPTOR_NO_DATA)],
        ),
        ("Stalled", vec![Event::Setup(VENDOR_OUT_Q)]),
    ];
    scripts
        .into_iter()
        .map(|(name, events)| {
            let state = drive(&events, context).0;
            assert_eq!(state_name(&state), name, "the script for {name}");
            (name, state)
        })
        .collect()
}

fn events() -> Vec<(&'static str, Event<'static>)> {
    vec![
        ("Setup read", Event::Setup(GET_DEVICE_DESCRIPTOR_64)),
        ("Setup refused", Event::Setup(VENDOR_OUT_Q)),
        ("Setup write", Event::Setup(SET_PLAYBACK_ALTERNATE_1)),
        (
            "Setup write with data",
            Event::Setup(SET_CUR_SAMPLING_FREQUENCY),
        ),
        ("InSent", Event::InSent),
        ("OutReceived empty", Event::OutReceived(&[])),
        (
            "OutReceived line coding",
            Event::OutReceived(&LINE_CODING_115200_8N1),
        ),
        (
            "OutReceived full packet",
            Event::OutReceived(&FULL_JUNK_PACKET),
        ),
        ("BusReset", Event::BusReset),
    ]
}

fn every_event_variant_is_tabled(event: Event<'_>) {
    match event {
        Event::Setup(_) | Event::InSent | Event::OutReceived(_) | Event::BusReset => {}
    }
}

const PLAYBACK_ON: &str = "Effect(Function(InterfaceSelected(Playback(One))))";
const CONFIGURED: &str = "Effect(Function(ConfigurationSet))";

fn expected(state: &str, event: &str) -> (&'static str, Vec<&'static str>) {
    match (state, event) {
        (_, "Setup read") => ("SendingReply", vec!["ReleaseStall", "Send(18)", "Receive"]),
        (_, "Setup refused") => ("Stalled", vec!["ReleaseStall", "Stall"]),
        (_, "Setup write") => ("ConfirmingWrite", vec!["ReleaseStall", "Send(0)"]),
        (_, "Setup write with data") => ("ReceivingData", vec!["ReleaseStall", "Receive"]),
        (_, "BusReset") => ("Idle", vec![]),

        ("SendingReply", "InSent") => ("SendingReply", vec!["Send(64)"]),
        ("ConfirmingWrite", "InSent") => ("Idle", vec![CONFIGURED]),
        ("ConfirmingEmptyRead", "InSent") => ("Idle", vec![]),

        ("SendingReply" | "AwaitingHostStatus", "OutReceived empty") => ("Idle", vec![]),
        (
            "SendingReply" | "AwaitingHostStatus",
            "OutReceived line coding" | "OutReceived full packet",
        ) => ("Stalled", vec!["Stall"]),

        ("ReceivingData", "OutReceived empty") => ("Stalled", vec!["Stall"]),
        ("ReceivingData", "OutReceived line coding") => ("ConfirmingWrite", vec!["Send(0)"]),
        ("ReceivingData", "OutReceived full packet") => ("Stalled", vec!["Stall"]),

        (unchanged, _) => (
            match unchanged {
                "Idle" => "Idle",
                "AwaitingHostStatus" => "AwaitingHostStatus",
                "ReceivingData" => "ReceivingData",
                "ConfirmingWrite" => "ConfirmingWrite",
                "ConfirmingEmptyRead" => "ConfirmingEmptyRead",
                "Stalled" => "Stalled",
                other => panic!("no row for {other} on {event}"),
            },
            vec![],
        ),
    }
}

#[test]
fn every_state_and_event_pair_gives_the_tabled_next_state_and_outputs() {
    let context = context(enumerated_at(BusSpeed::High));
    let mut pairs = 0;
    for (state_label, state) in representative_states(&context) {
        for (event_label, event) in events() {
            every_event_variant_is_tabled(event);
            let (next, outputs) = step(state, event, &context);
            let described: Vec<String> = outputs.iter().map(describe).collect();
            let (expected_state, expected_outputs) = expected(state_label, event_label);
            assert_eq!(
                (state_name(&next), described),
                (
                    expected_state,
                    expected_outputs.iter().map(|s| s.to_string()).collect()
                ),
                "{state_label} on {event_label}"
            );
            if expected_state == "ConfirmingWrite" && event_label == "Setup write" {
                let (_, confirmed) = step(next, Event::InSent, &context);
                assert_eq!(
                    confirmed.iter().map(describe).collect::<Vec<_>>(),
                    [PLAYBACK_ON]
                );
            }
            pairs += 1;
        }
    }
    assert_eq!(pairs, 7 * 9);
}

#[test]
fn a_line_coding_written_in_the_data_stage_reaches_the_effect_only_after_the_status_stage() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, outputs) = drive(
        &[
            Event::Setup(SET_LINE_CODING),
            Event::OutReceived(&LINE_CODING_115200_8N1),
        ],
        &context,
    );
    assert!(outputs.iter().all(|o| !describe(o).starts_with("Effect")));
    let (_, outputs) = step(state, Event::InSent, &context);
    assert_eq!(
        outputs.iter().map(describe).collect::<Vec<_>>(),
        [
            "Effect(LineCoding(LineCoding { rate: BitsPerSecond(115200), stop_bits: One, parity: None, data_bits: Eight }))"
        ]
    );
}
