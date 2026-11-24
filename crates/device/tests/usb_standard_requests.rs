mod usb_support;

use device::usb::answer::Effect;
use device::usb::function::{Event, State};
use device::usb::request::{HaltableEndpoint, InterfaceSelection};
use device::usb::{
    BENCH_DEVICE_DESCRIPTOR, BENCH_STRINGS, BusSpeed, DEVICE_QUALIFIER, FACTORY_SERIAL_TEMPLATE,
    configuration, other_speed_configuration,
};
use stream::AlternateSetting;
use usb_support::{assigned, context, effect, enumerated_at, reply, run, setup, stalls, step};

const GET_DESCRIPTOR: u8 = 6;
const ENGLISH_UNITED_STATES: u16 = 0x0409;
const MICROSOFT_OS_STRING_INDEX: u16 = 0xEE;

fn string(index: u16, language: u16, length: u16) -> [u8; 8] {
    setup(0x80, GET_DESCRIPTOR, 0x0300 | index, language, length)
}

fn default_at(speed: BusSpeed) -> State {
    run(&[Event::Attached, Event::BusReset(speed)])
}

fn addressed_at(speed: BusSpeed) -> State {
    run(&[
        Event::Attached,
        Event::BusReset(speed),
        Event::AddressSet(assigned(5)),
    ])
}

#[test]
fn string_zero_lists_english_united_states_as_the_only_language() {
    assert_eq!(
        reply(string(0, 0, 255), &context(default_at(BusSpeed::High))),
        [0x04, 0x03, 0x09, 0x04]
    );
}

#[test]
fn the_three_strings_answer_in_english_united_states() {
    let configured = context(enumerated_at(BusSpeed::High));
    assert_eq!(
        reply(string(1, ENGLISH_UNITED_STATES, 255), &configured),
        BENCH_STRINGS.manufacturer.descriptor()
    );
    assert_eq!(
        reply(string(2, ENGLISH_UNITED_STATES, 255), &configured),
        BENCH_STRINGS.product.descriptor()
    );
    assert_eq!(
        reply(string(3, ENGLISH_UNITED_STATES, 255), &configured),
        FACTORY_SERIAL_TEMPLATE.descriptor()
    );
    assert_eq!(
        reply(string(3, ENGLISH_UNITED_STATES, 4), &configured),
        [22, 3, b'0', 0]
    );
}

#[test]
fn string_0xee_the_microsoft_os_string_stalls() {
    for state in [default_at(BusSpeed::High), enumerated_at(BusSpeed::High)] {
        for language in [0, ENGLISH_UNITED_STATES] {
            assert!(stalls(
                string(MICROSOFT_OS_STRING_INDEX, language, 0x12),
                &[],
                &context(state)
            ));
        }
    }
}

#[test]
fn unknown_strings_and_languages_stall() {
    let configured = context(enumerated_at(BusSpeed::High));
    for packet in [
        string(4, ENGLISH_UNITED_STATES, 255),
        string(0xFF, ENGLISH_UNITED_STATES, 255),
        string(1, 0x0407, 255),
        string(1, 0, 255),
        string(0, ENGLISH_UNITED_STATES, 255),
    ] {
        assert!(stalls(packet, &[], &configured), "{packet:02X?}");
    }
}

#[test]
fn descriptors_answer_from_the_default_state_and_truncate_to_wlength() {
    for speed in [BusSpeed::High, BusSpeed::Full] {
        let fresh = context(default_at(speed));
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0100, 0, 64), &fresh),
            BENCH_DEVICE_DESCRIPTOR
        );
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0100, 0, 8), &fresh),
            BENCH_DEVICE_DESCRIPTOR[..8]
        );
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0200, 0, 0xFFFF), &fresh),
            configuration(speed)
        );
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0200, 0, 9), &fresh),
            configuration(speed)[..9]
        );
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0600, 0, 10), &fresh),
            DEVICE_QUALIFIER
        );
        assert_eq!(
            reply(setup(0x80, GET_DESCRIPTOR, 0x0700, 0, 0xFFFF), &fresh),
            other_speed_configuration(speed)
        );
    }
    assert_eq!(configuration(BusSpeed::High).len(), 284);
    assert_eq!(configuration(BusSpeed::Full).len(), 75);
}

#[test]
fn descriptors_that_do_not_exist_stall() {
    let configured = context(enumerated_at(BusSpeed::High));
    for packet in [
        setup(0x80, GET_DESCRIPTOR, 0x0101, 0, 18),
        setup(0x80, GET_DESCRIPTOR, 0x0201, 0, 9),
        setup(0x80, GET_DESCRIPTOR, 0x0100, 0x0409, 18),
        setup(0x80, GET_DESCRIPTOR, 0x0400, 0, 9),
        setup(0x80, GET_DESCRIPTOR, 0x0500, 0, 7),
        setup(0x80, GET_DESCRIPTOR, 0x0F00, 0, 5),
        setup(0x80, GET_DESCRIPTOR, 0x2100, 0, 9),
        setup(0x81, GET_DESCRIPTOR, 0x2200, 0, 64),
        setup(0x00, 7, 0x0100, 0, 0),
    ] {
        assert!(stalls(packet, &[], &configured), "{packet:02X?}");
    }
}

#[test]
fn set_address_assigns_from_default_or_addressed_and_stalls_once_configured() {
    let fresh = context(default_at(BusSpeed::High));
    assert_eq!(
        effect(setup(0x00, 5, 42, 0, 0), &[], &fresh),
        Effect::Function(Event::AddressSet(assigned(42)))
    );
    assert_eq!(
        effect(
            setup(0x00, 5, 127, 0, 0),
            &[],
            &context(addressed_at(BusSpeed::Full))
        ),
        Effect::Function(Event::AddressSet(assigned(127)))
    );
    assert!(stalls(setup(0x00, 5, 128, 0, 0), &[], &fresh));
    assert!(stalls(setup(0x00, 5, 0x0105, 0, 0), &[], &fresh));
    assert!(stalls(setup(0x00, 5, 5, 1, 0), &[], &fresh));
    assert!(stalls(
        setup(0x00, 5, 9, 0, 0),
        &[],
        &context(enumerated_at(BusSpeed::High))
    ));
}

#[test]
fn get_configuration_reads_zero_when_addressed_and_one_when_configured() {
    let get = setup(0x80, 8, 0, 0, 1);
    assert_eq!(reply(get, &context(addressed_at(BusSpeed::High))), [0]);
    assert_eq!(reply(get, &context(enumerated_at(BusSpeed::Full))), [1]);
    assert!(stalls(get, &[], &context(default_at(BusSpeed::High))));
}

#[test]
fn set_configuration_accepts_zero_and_one_only_after_an_address() {
    let addressed = context(addressed_at(BusSpeed::High));
    assert_eq!(
        effect(setup(0x00, 9, 1, 0, 0), &[], &addressed),
        Effect::Function(Event::ConfigurationSet)
    );
    assert_eq!(
        effect(setup(0x00, 9, 0, 0, 0), &[], &addressed),
        Effect::Function(Event::ConfigurationCleared)
    );
    assert!(stalls(setup(0x00, 9, 2, 0, 0), &[], &addressed));
    assert!(stalls(setup(0x00, 9, 0x0101, 0, 0), &[], &addressed));
    assert!(stalls(
        setup(0x00, 9, 1, 0, 0),
        &[],
        &context(default_at(BusSpeed::High))
    ));
}

#[test]
fn set_interface_selects_stream_settings_only_when_configured() {
    let configured = context(enumerated_at(BusSpeed::High));
    for (interface, setting, selection) in [
        (1, 1, InterfaceSelection::Playback(AlternateSetting::One)),
        (1, 0, InterfaceSelection::Playback(AlternateSetting::Zero)),
        (2, 1, InterfaceSelection::Capture(AlternateSetting::One)),
        (2, 0, InterfaceSelection::Capture(AlternateSetting::Zero)),
        (0, 0, InterfaceSelection::AudioControl),
        (3, 0, InterfaceSelection::SerialControl),
        (4, 0, InterfaceSelection::SerialData),
    ] {
        assert_eq!(
            effect(setup(0x01, 11, setting, interface, 0), &[], &configured),
            Effect::Function(Event::InterfaceSelected(selection)),
            "interface {interface} setting {setting}"
        );
    }
    for (interface, setting) in [(1, 2), (2, 2), (0, 1), (3, 1), (4, 1), (5, 0), (0x0101, 1)] {
        assert!(
            stalls(setup(0x01, 11, setting, interface, 0), &[], &configured),
            "interface {interface} setting {setting}"
        );
    }
    assert!(stalls(
        setup(0x01, 11, 1, 1, 0),
        &[],
        &context(addressed_at(BusSpeed::High))
    ));
}

#[test]
fn at_full_speed_interfaces_zero_and_one_are_the_serial_function() {
    let configured = context(enumerated_at(BusSpeed::Full));
    assert_eq!(
        effect(setup(0x01, 11, 0, 0, 0), &[], &configured),
        Effect::Function(Event::InterfaceSelected(InterfaceSelection::SerialControl))
    );
    assert_eq!(
        effect(setup(0x01, 11, 0, 1, 0), &[], &configured),
        Effect::Function(Event::InterfaceSelected(InterfaceSelection::SerialData))
    );
    assert!(stalls(setup(0x01, 11, 1, 1, 0), &[], &configured));
    assert!(stalls(setup(0x01, 11, 0, 2, 0), &[], &configured));
}

#[test]
fn get_interface_reads_the_selected_setting() {
    let (playing, _) = step(
        enumerated_at(BusSpeed::High),
        Event::InterfaceSelected(InterfaceSelection::Playback(AlternateSetting::One)),
    );
    let get = |interface| reply(setup(0x81, 10, 0, interface, 1), &context(playing));
    assert_eq!(get(1), [1]);
    assert_eq!(get(2), [0]);
    assert_eq!(get(0), [0]);
    assert_eq!(get(4), [0]);
    assert!(stalls(setup(0x81, 10, 0, 5, 1), &[], &context(playing)));
}

#[test]
fn get_status_reports_a_bus_powered_device_and_each_halt() {
    let configured = enumerated_at(BusSpeed::High);
    assert_eq!(reply(setup(0x80, 0, 0, 0, 2), &context(configured)), [0, 0]);
    assert_eq!(reply(setup(0x81, 0, 0, 2, 2), &context(configured)), [0, 0]);
    assert_eq!(
        reply(setup(0x82, 0, 0, 0x84, 2), &context(configured)),
        [0, 0]
    );
    assert_eq!(
        effect(setup(0x02, 3, 0, 0x84, 0), &[], &context(configured)),
        Effect::Function(Event::EndpointHalted(HaltableEndpoint::SerialTransmit))
    );
    let (halted, _) = step(
        configured,
        Event::EndpointHalted(HaltableEndpoint::SerialTransmit),
    );
    assert_eq!(reply(setup(0x82, 0, 0, 0x84, 2), &context(halted)), [1, 0]);
    assert_eq!(reply(setup(0x82, 0, 0, 0x04, 2), &context(halted)), [0, 0]);
    assert_eq!(
        effect(setup(0x02, 1, 0, 0x84, 0), &[], &context(halted)),
        Effect::Function(Event::EndpointHaltCleared(HaltableEndpoint::SerialTransmit))
    );
    assert_eq!(reply(setup(0x82, 0, 0, 0x80, 2), &context(halted)), [0, 0]);
}

#[test]
fn a_streaming_endpoint_exists_only_while_its_setting_is_one() {
    let configured = enumerated_at(BusSpeed::High);
    let status = |endpoint| setup(0x82, 0, 0, endpoint, 2);
    assert!(stalls(status(0x01), &[], &context(configured)));
    assert!(stalls(status(0x82), &[], &context(configured)));
    let (playing, _) = step(
        configured,
        Event::InterfaceSelected(InterfaceSelection::Playback(AlternateSetting::One)),
    );
    assert_eq!(reply(status(0x01), &context(playing)), [0, 0]);
    assert_eq!(reply(status(0x81), &context(playing)), [0, 0]);
    assert!(stalls(status(0x82), &[], &context(playing)));
    assert!(stalls(status(0x02), &[], &context(playing)));
}

#[test]
fn features_this_device_lacks_stall() {
    let configured = context(enumerated_at(BusSpeed::High));
    for packet in [
        setup(0x00, 3, 1, 0, 0),
        setup(0x00, 1, 1, 0, 0),
        setup(0x00, 3, 2, 0x0100, 0),
        setup(0x01, 3, 0, 1, 0),
        setup(0x02, 3, 0, 0x01, 0),
        setup(0x02, 3, 0, 0x81, 0),
        setup(0x02, 3, 0, 0x82, 0),
        setup(0x02, 3, 0, 0x00, 0),
        setup(0x02, 3, 1, 0x84, 0),
        setup(0x82, 12, 0, 0x82, 2),
    ] {
        assert!(stalls(packet, &[], &configured), "{packet:02X?}");
    }
}

#[test]
fn no_request_is_answered_while_detached_powered_or_suspended() {
    let configured = enumerated_at(BusSpeed::High);
    let (suspended, _) = step(configured, Event::Suspended);
    let device_descriptor = setup(0x80, GET_DESCRIPTOR, 0x0100, 0, 18);
    for state in [State::INITIAL, run(&[Event::Attached]), suspended] {
        assert!(stalls(device_descriptor, &[], &context(state)), "{state:?}");
    }
}
