mod usb_control_support;
mod usb_support;

use device::usb::answer::{ControlContext, Descriptors, Effect};
use device::usb::control::{Event, Output, State};
use device::usb::function;
use device::usb::{
    BENCH_DEVICE_DESCRIPTOR, BusSpeed, DescriptorText, DeviceStrings, FACTORY_SERIAL_TEMPLATE,
    configuration,
};
use usb_control_support::*;
use usb_support::{context, enumerated_at, run};

fn with_product(
    function: function::State,
    strings: &'static DeviceStrings,
) -> ControlContext<'static> {
    ControlContext {
        descriptors: Descriptors {
            device: &BENCH_DEVICE_DESCRIPTOR,
            strings,
            serial: FACTORY_SERIAL_TEMPLATE,
        },
        ..context(function)
    }
}

fn strings_with_product(product: &'static str) -> &'static DeviceStrings {
    Box::leak(Box::new(DeviceStrings {
        manufacturer: DescriptorText::new("m2x").unwrap(),
        product: DescriptorText::new(product).unwrap(),
    }))
}

const THIRTY_ONE_CHARACTERS: &str = "abcdefghijklmnopqrstuvwxyz01234";

#[test]
fn a_284_byte_configuration_goes_out_as_four_full_packets_and_one_short_packet() {
    let context = context(enumerated_at(BusSpeed::High));
    let transfer = read_until_short_packet(GET_CONFIGURATION_DESCRIPTOR_ALL, &context);
    let sizes: Vec<usize> = transfer.packets.iter().map(Vec::len).collect();
    assert_eq!(sizes, [64, 64, 64, 64, 28]);
    assert_eq!(transfer.packets.concat(), configuration(BusSpeed::High));
    assert!(
        transfer.status_armed,
        "the status OUT is armed with the data"
    );
    assert_eq!(transfer.final_state, "AwaitingHostStatus");
}

#[test]
fn a_reply_longer_than_w_length_is_cut_at_w_length_with_no_zero_length_packet() {
    let context = context(enumerated_at(BusSpeed::High));
    let nine = read_until_short_packet(GET_CONFIGURATION_DESCRIPTOR_9, &context);
    assert_eq!(nine.packets, [configuration(BusSpeed::High)[..9].to_vec()]);
    let first_255 = read_until_short_packet(GET_CONFIGURATION_DESCRIPTOR_255, &context);
    let sizes: Vec<usize> = first_255.packets.iter().map(Vec::len).collect();
    assert_eq!(sizes, [64, 64, 64, 63]);
}

#[test]
fn a_reply_that_fills_whole_packets_and_is_shorter_than_w_length_ends_with_a_zero_length_packet() {
    let strings = strings_with_product(THIRTY_ONE_CHARACTERS);
    let context = with_product(enumerated_at(BusSpeed::High), strings);
    let transfer = read_until_short_packet(GET_PRODUCT_STRING_255, &context);
    let sizes: Vec<usize> = transfer.packets.iter().map(Vec::len).collect();
    assert_eq!(sizes, [64, 0]);
    assert_eq!(transfer.packets[0], strings.product.descriptor());
}

#[test]
fn a_reply_that_fills_whole_packets_and_equals_w_length_sends_no_zero_length_packet() {
    let strings = strings_with_product(THIRTY_ONE_CHARACTERS);
    let context = with_product(enumerated_at(BusSpeed::High), strings);
    let transfer = read_until_short_packet(GET_PRODUCT_STRING_64, &context);
    let sizes: Vec<usize> = transfer.packets.iter().map(Vec::len).collect();
    assert_eq!(sizes, [64]);
}

#[test]
fn a_read_with_no_data_stage_answers_with_a_zero_length_status_packet() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, outputs) = drive(&[Event::Setup(GET_DEVICE_DESCRIPTOR_NO_DATA)], &context);
    assert_eq!(sent_packets(&outputs), [Vec::<u8>::new()]);
    assert!(!outputs.contains(&Output::Receive));
    assert_eq!(step(state, Event::InSent, &context).0, State::Idle);
}

#[test]
fn the_host_may_end_an_in_data_stage_early_with_its_status_packet() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, _) = drive(&[Event::Setup(GET_CONFIGURATION_DESCRIPTOR_ALL)], &context);
    let (state, outputs) = step(state, Event::OutReceived(&[]), &context);
    assert_eq!(state, State::Idle);
    assert!(outputs.is_empty());
}

#[test]
fn set_address_reaches_the_function_only_after_its_status_stage_completes() {
    let default_state = run(&[
        function::Event::Attached,
        function::Event::BusReset(BusSpeed::High),
    ]);
    let context = context(default_state);
    let (state, outputs) = drive(&[Event::Setup(SET_ADDRESS_9)], &context);
    assert_eq!(sent_packets(&outputs), [Vec::<u8>::new()]);
    assert!(
        !outputs.iter().any(|o| matches!(o, Output::Effect(_))),
        "no address before the status stage"
    );
    let (state, outputs) = step(state, Event::InSent, &context);
    assert_eq!(state, State::Idle);
    assert_eq!(
        outputs,
        [Output::Effect(Effect::Function(
            function::Event::AddressSet(usb_support::assigned(9))
        ))]
    );
}

#[test]
fn a_new_setup_abandons_a_write_whose_data_stage_was_half_done() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, _) = drive(&[Event::Setup(SET_LINE_CODING)], &context);
    let (state, outputs) = step(state, Event::Setup(GET_DEVICE_DESCRIPTOR_18), &context);
    assert_eq!(state_name(&state), "SendingReply");
    assert!(!outputs.iter().any(|o| matches!(o, Output::Effect(_))));
    let (_, outputs) = step(state, Event::OutReceived(&LINE_CODING_115200_8N1), &context);
    assert_eq!(outputs, [Output::Stall], "late data is a protocol error");
}

#[test]
fn a_write_longer_than_any_accepted_request_stalls_at_its_setup() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, outputs) = drive(&[Event::Setup(SET_LINE_CODING_TOO_LONG)], &context);
    assert_eq!(state, State::Stalled);
    assert_eq!(outputs, [Output::ReleaseStall, Output::Stall]);
}

#[test]
fn a_set_cur_carried_in_the_data_stage_becomes_one_rate_change_effect() {
    let context = context(enumerated_at(BusSpeed::High));
    let (_, outputs) = drive(
        &[
            Event::Setup(SET_CUR_SAMPLING_FREQUENCY),
            Event::OutReceived(&NINETY_SIX_KILOHERTZ),
            Event::InSent,
        ],
        &context,
    );
    let effects: Vec<_> = outputs
        .iter()
        .filter(|o| matches!(o, Output::Effect(_)))
        .collect();
    assert_eq!(
        effects,
        [&Output::Effect(Effect::RateChange(
            device::rate_change::Event::SetCur(dsp::SampleRate::Hz96000)
        ))]
    );
}

#[test]
fn a_short_first_data_packet_ends_the_data_stage_and_a_mismatched_length_stalls() {
    let context = context(enumerated_at(BusSpeed::High));
    let (state, outputs) = drive(
        &[
            Event::Setup(SET_CUR_SAMPLING_FREQUENCY),
            Event::OutReceived(&NINETY_SIX_KILOHERTZ[..2]),
        ],
        &context,
    );
    assert_eq!(state, State::Stalled);
    assert_eq!(outputs.last(), Some(&Output::Stall));
}
