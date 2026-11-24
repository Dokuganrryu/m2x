mod descriptor_parser;

use descriptor_parser::{
    Descriptor, Kind, assert_matches_note, cdc_table_after, parse, table_after,
};
use device::usb::{
    AUDIO_PART, BENCH_DEVICE_DESCRIPTOR, BusSpeed, DEVICE_QUALIFIER, configuration,
    other_speed_configuration,
};
use std::collections::BTreeSet;

const BOTH_SPEEDS: [BusSpeed; 2] = [BusSpeed::High, BusSpeed::Full];
const COMMUNICATIONS_CLASS: u32 = 0x02;
const DATA_CLASS: u32 = 0x0A;
const AUDIO_CLASS: u32 = 0x01;
const OTHER_SPEED_CONFIGURATION_TYPE: u8 = 0x07;

fn every_configuration() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("high-speed configuration", configuration(BusSpeed::High)),
        ("full-speed configuration", configuration(BusSpeed::Full)),
        (
            "other-speed seen at high speed",
            other_speed_configuration(BusSpeed::High),
        ),
        (
            "other-speed seen at full speed",
            other_speed_configuration(BusSpeed::Full),
        ),
    ]
}

fn only(descriptors: &[Descriptor], wanted: impl Fn(&Descriptor) -> bool) -> Descriptor {
    let found: Vec<&Descriptor> = descriptors.iter().filter(|d| wanted(d)).collect();
    assert_eq!(found.len(), 1, "exactly one descriptor matches");
    found[0].clone()
}

fn of_kind(descriptors: &[Descriptor], kind: Kind) -> Vec<Descriptor> {
    descriptors
        .iter()
        .filter(|d| d.kind == kind)
        .cloned()
        .collect()
}

fn interface_of_class(descriptors: &[Descriptor], class: u32) -> Descriptor {
    only(descriptors, |d| {
        d.kind == Kind::Interface && d.field("bInterfaceClass") == class
    })
}

fn serial_association(descriptors: &[Descriptor]) -> Descriptor {
    only(descriptors, |d| {
        d.kind == Kind::InterfaceAssociation && d.field("bFunctionClass") == COMMUNICATIONS_CLASS
    })
}

fn endpoint(descriptors: &[Descriptor], address: u32) -> Descriptor {
    only(descriptors, |d| {
        d.kind == Kind::Endpoint && d.field("bEndpointAddress") == address
    })
}

fn header(descriptors: &[Descriptor]) -> Descriptor {
    only(descriptors, |d| {
        matches!(d.kind, Kind::Configuration | Kind::OtherSpeedConfiguration)
    })
}

fn polling_period_microframes(speed: BusSpeed, interval: u32) -> u32 {
    match speed {
        BusSpeed::High => 1 << (interval - 1),
        BusSpeed::Full => interval * 8,
    }
}

#[test]
fn the_high_speed_configuration_is_the_218_byte_audio_part_plus_the_66_byte_serial_function() {
    let bytes = configuration(BusSpeed::High);
    assert_eq!(bytes.len(), 284);
    assert_eq!(bytes[9..218], AUDIO_PART[9..]);
    let parsed = parse(bytes);
    assert_eq!(header(&parsed).field("wTotalLength"), 284);
    assert_eq!(header(&parsed).field("bNumInterfaces"), 5);
}

#[test]
fn the_full_speed_configuration_carries_the_serial_function_only() {
    let bytes = configuration(BusSpeed::Full);
    assert_eq!(bytes.len(), 75);
    let parsed = parse(bytes);
    assert_eq!(header(&parsed).field("wTotalLength"), 75);
    assert_eq!(header(&parsed).field("bNumInterfaces"), 2);
    assert!(
        parsed
            .iter()
            .all(|d| d.kind != Kind::Interface || d.field("bInterfaceClass") != AUDIO_CLASS)
    );
}

#[test]
fn the_serial_function_follows_the_order_of_the_cdc_note() {
    for speed in BOTH_SPEEDS {
        let parsed = parse(configuration(speed));
        let association = serial_association(&parsed);
        let kinds: Vec<Kind> = parsed
            .iter()
            .filter(|d| d.offset >= association.offset)
            .map(|d| d.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                Kind::InterfaceAssociation,
                Kind::Interface,
                Kind::CdcHeader,
                Kind::AbstractControlManagement,
                Kind::Union,
                Kind::CallManagement,
                Kind::Endpoint,
                Kind::Interface,
                Kind::Endpoint,
                Kind::Endpoint,
            ]
        );
    }
}

#[test]
fn the_serial_function_is_66_bytes_at_both_speeds() {
    for speed in BOTH_SPEEDS {
        let bytes = configuration(speed);
        let association = serial_association(&parse(bytes));
        assert_eq!(bytes.len() - association.offset, 66);
    }
}

#[test]
fn the_serial_descriptors_carry_every_value_of_the_cdc_note_tables_at_both_speeds() {
    for speed in BOTH_SPEEDS {
        let parsed = parse(configuration(speed));
        assert_matches_note(
            &serial_association(&parsed),
            &cdc_table_after("### Serial Interface Association Descriptor"),
            1,
        );
        assert_matches_note(
            &interface_of_class(&parsed, COMMUNICATIONS_CLASS),
            &cdc_table_after("### Communications Class interface"),
            1,
        );
        assert_matches_note(
            &only(&parsed, |d| d.kind == Kind::CdcHeader),
            &cdc_table_after("### Header functional descriptor"),
            1,
        );
        assert_matches_note(
            &only(&parsed, |d| d.kind == Kind::AbstractControlManagement),
            &cdc_table_after("### Abstract Control Management functional descriptor"),
            1,
        );
        assert_matches_note(
            &only(&parsed, |d| d.kind == Kind::Union),
            &cdc_table_after("### Union functional descriptor"),
            1,
        );
        assert_matches_note(
            &only(&parsed, |d| d.kind == Kind::CallManagement),
            &cdc_table_after("### Call Management functional descriptor"),
            1,
        );
        assert_matches_note(
            &endpoint(&parsed, 0x83),
            &cdc_table_after("### Notification endpoint"),
            1,
        );
        assert_matches_note(
            &interface_of_class(&parsed, DATA_CLASS),
            &cdc_table_after("### Data Class interface"),
            1,
        );
        for address in [0x04, 0x84] {
            assert_matches_note(
                &endpoint(&parsed, address),
                &cdc_table_after("### Bulk endpoints"),
                1,
            );
        }
    }
}

#[test]
fn serial_interfaces_are_three_and_four_at_high_speed_and_zero_and_one_at_full_speed() {
    for (speed, expected) in [(BusSpeed::High, [3, 4]), (BusSpeed::Full, [0, 1])] {
        let parsed = parse(configuration(speed));
        assert_eq!(
            [
                interface_of_class(&parsed, COMMUNICATIONS_CLASS).field("bInterfaceNumber"),
                interface_of_class(&parsed, DATA_CLASS).field("bInterfaceNumber"),
            ],
            expected
        );
    }
}

#[test]
fn the_association_union_and_call_management_name_the_serial_interfaces_of_their_configuration() {
    for speed in BOTH_SPEEDS {
        let parsed = parse(configuration(speed));
        let control = interface_of_class(&parsed, COMMUNICATIONS_CLASS).field("bInterfaceNumber");
        let data = interface_of_class(&parsed, DATA_CLASS).field("bInterfaceNumber");
        assert_eq!(
            serial_association(&parsed).field("bFirstInterface"),
            control
        );
        let union = only(&parsed, |d| d.kind == Kind::Union);
        assert_eq!(union.field("bControlInterface"), control);
        assert_eq!(union.field("bSubordinateInterface0"), data);
        let call = only(&parsed, |d| d.kind == Kind::CallManagement);
        assert_eq!(call.field("bDataInterface"), data);
    }
}

#[test]
fn bulk_endpoints_are_512_bytes_at_high_speed_and_64_bytes_at_full_speed() {
    for (speed, size) in [(BusSpeed::High, 512), (BusSpeed::Full, 64)] {
        let parsed = parse(configuration(speed));
        for address in [0x04, 0x84] {
            assert_eq!(endpoint(&parsed, address).field("wMaxPacketSize"), size);
        }
    }
}

#[test]
fn the_notification_endpoint_polls_every_16_milliseconds_at_both_speeds() {
    for (speed, encoded) in [(BusSpeed::High, 8), (BusSpeed::Full, 16)] {
        let interval = endpoint(&parse(configuration(speed)), 0x83).field("bInterval");
        assert_eq!(interval, encoded);
        assert_eq!(polling_period_microframes(speed, interval), 16 * 8);
    }
}

#[test]
fn each_other_speed_configuration_is_the_configuration_of_the_other_speed_typed_0x07() {
    for (current, other) in [
        (BusSpeed::High, BusSpeed::Full),
        (BusSpeed::Full, BusSpeed::High),
    ] {
        let mut expected = configuration(other).to_vec();
        expected[1] = OTHER_SPEED_CONFIGURATION_TYPE;
        assert_eq!(other_speed_configuration(current), expected.as_slice());
        let parsed = parse(other_speed_configuration(current));
        assert_eq!(parsed[0].kind, Kind::OtherSpeedConfiguration);
    }
}

#[test]
fn the_other_speed_header_matches_the_cdc_note_table() {
    for speed in BOTH_SPEEDS {
        let parsed = parse(other_speed_configuration(speed));
        assert_matches_note(
            &header(&parsed),
            &cdc_table_after("### Other-speed configuration descriptor"),
            1,
        );
    }
}

#[test]
fn both_configurations_share_value_power_and_attributes() {
    let headers: Vec<Descriptor> = BOTH_SPEEDS
        .iter()
        .map(|&speed| header(&parse(configuration(speed))))
        .collect();
    for configuration_header in &headers {
        assert_eq!(configuration_header.field("bConfigurationValue"), 1);
        assert_eq!(configuration_header.field("iConfiguration"), 0);
        assert_eq!(configuration_header.field("bmAttributes"), 0x80);
        assert_eq!(configuration_header.field("bMaxPower"), 250);
    }
}

#[test]
fn the_device_qualifier_matches_the_feedback_note_table() {
    let parsed = parse(&DEVICE_QUALIFIER);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].kind, Kind::DeviceQualifier);
    assert_matches_note(&parsed[0], &table_after("### Device qualifier"), 1);
}

#[test]
fn the_device_qualifier_repeats_the_device_class_and_counts_one_other_speed_configuration() {
    let qualifier = parse(&DEVICE_QUALIFIER).remove(0);
    let device = parse(&BENCH_DEVICE_DESCRIPTOR).remove(0);
    for field in [
        "bcdUSB",
        "bDeviceClass",
        "bDeviceSubClass",
        "bDeviceProtocol",
        "bMaxPacketSize0",
    ] {
        assert_eq!(qualifier.field(field), device.field(field), "{field}");
    }
    assert_eq!(qualifier.field("bNumConfigurations"), 1);
}

#[test]
fn every_configuration_holds_only_descriptors_the_parser_knows() {
    for (name, bytes) in every_configuration() {
        for descriptor in parse(bytes) {
            assert!(
                !matches!(descriptor.kind, Kind::Unrecognised { .. }),
                "{name}: {:?} at {}",
                descriptor.kind,
                descriptor.offset
            );
        }
    }
}

#[test]
fn every_configuration_counts_its_own_bytes_and_interfaces() {
    for (name, bytes) in every_configuration() {
        let parsed = parse(bytes);
        let configuration_header = header(&parsed);
        assert_eq!(configuration_header.offset, 0, "{name}");
        assert_eq!(
            configuration_header.field("wTotalLength") as usize,
            bytes.len(),
            "{name}"
        );
        let numbers: BTreeSet<u32> = of_kind(&parsed, Kind::Interface)
            .iter()
            .map(|d| d.field("bInterfaceNumber"))
            .collect();
        let expected: BTreeSet<u32> = (0..numbers.len() as u32).collect();
        assert_eq!(numbers, expected, "{name} numbers interfaces from zero");
        assert_eq!(
            configuration_header.field("bNumInterfaces") as usize,
            numbers.len(),
            "{name}"
        );
    }
}

#[test]
fn every_interface_belongs_to_the_association_before_it_and_associations_cover_every_interface() {
    for (name, bytes) in every_configuration() {
        let parsed = parse(bytes);
        let associations = of_kind(&parsed, Kind::InterfaceAssociation);
        let mut covered = BTreeSet::new();
        for association in &associations {
            let first = association.field("bFirstInterface");
            let count = association.field("bInterfaceCount");
            let following: BTreeSet<u32> = parsed
                .iter()
                .filter(|d| d.offset > association.offset)
                .take_while(|d| d.kind != Kind::InterfaceAssociation)
                .filter(|d| d.kind == Kind::Interface)
                .map(|d| d.field("bInterfaceNumber"))
                .collect();
            assert_eq!(following, (first..first + count).collect(), "{name}");
            covered.extend(following);
        }
        let all: BTreeSet<u32> = of_kind(&parsed, Kind::Interface)
            .iter()
            .map(|d| d.field("bInterfaceNumber"))
            .collect();
        assert_eq!(covered, all, "{name}");
        assert_eq!(parsed[1].kind, Kind::InterfaceAssociation, "{name}");
    }
}

#[test]
fn every_interface_declares_exactly_the_endpoints_that_follow_it_in_every_configuration() {
    for (name, bytes) in every_configuration() {
        let parsed = parse(bytes);
        for setting in of_kind(&parsed, Kind::Interface) {
            let following = parsed
                .iter()
                .filter(|d| d.offset > setting.offset)
                .take_while(|d| !matches!(d.kind, Kind::Interface | Kind::InterfaceAssociation))
                .filter(|d| d.kind == Kind::Endpoint)
                .count();
            assert_eq!(
                setting.field("bNumEndpoints") as usize,
                following,
                "{name} interface {:?}",
                setting.interface
            );
        }
    }
}

#[test]
fn endpoint_addresses_are_unique_in_every_configuration() {
    let expected_high = BTreeSet::from([0x01, 0x81, 0x82, 0x83, 0x04, 0x84]);
    let expected_full = BTreeSet::from([0x83, 0x04, 0x84]);
    for (bytes, expected) in [
        (configuration(BusSpeed::High), &expected_high),
        (configuration(BusSpeed::Full), &expected_full),
    ] {
        let addresses: Vec<u32> = of_kind(&parse(bytes), Kind::Endpoint)
            .iter()
            .map(|d| d.field("bEndpointAddress"))
            .collect();
        let unique: BTreeSet<u32> = addresses.iter().copied().collect();
        assert_eq!(unique.len(), addresses.len());
        assert_eq!(&unique, expected);
    }
}

#[test]
fn the_data_interface_holds_one_bulk_in_and_one_bulk_out_and_the_control_interface_one_interrupt_in()
 {
    for speed in BOTH_SPEEDS {
        let parsed = parse(configuration(speed));
        let data = interface_of_class(&parsed, DATA_CLASS).interface;
        let control = interface_of_class(&parsed, COMMUNICATIONS_CLASS).interface;
        let endpoints = of_kind(&parsed, Kind::Endpoint);
        let in_data: Vec<&Descriptor> = endpoints.iter().filter(|d| d.interface == data).collect();
        assert_eq!(in_data.len(), 2);
        assert!(in_data.iter().all(|d| d.field("bmAttributes") == 0x02));
        let directions: BTreeSet<u32> = in_data
            .iter()
            .map(|d| d.field("bEndpointAddress") & 0x80)
            .collect();
        assert_eq!(directions, BTreeSet::from([0x00, 0x80]));
        let notify = only(&endpoints, |d| d.interface == control);
        assert_eq!(notify.field("bmAttributes"), 0x03);
        assert_ne!(notify.field("bEndpointAddress") & 0x80, 0);
    }
}
