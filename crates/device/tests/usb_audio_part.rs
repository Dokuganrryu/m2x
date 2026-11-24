mod descriptor_parser;

use descriptor_parser::{Descriptor, Kind, assert_matches_note, parse, table_after};
use device::usb::{AUDIO_PART, AUDIO_PART_LENGTH};
use std::collections::BTreeSet;

fn audio_part() -> Vec<Descriptor> {
    parse(&AUDIO_PART)
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

fn interface(descriptors: &[Descriptor], number: u8, alternate: u8) -> Descriptor {
    only(descriptors, |d| {
        d.kind == Kind::Interface && d.interface == Some((number, alternate))
    })
}

fn endpoint(descriptors: &[Descriptor], address: u32) -> Descriptor {
    only(descriptors, |d| {
        d.kind == Kind::Endpoint && d.field("bEndpointAddress") == address
    })
}

fn in_interface(descriptors: &[Descriptor], kind: Kind, number: u8) -> Descriptor {
    only(descriptors, |d| {
        d.kind == kind && d.interface.map(|(n, _)| n) == Some(number)
    })
}

fn terminal(descriptors: &[Descriptor], kind: Kind, id: u32) -> Descriptor {
    only(descriptors, |d| {
        d.kind == kind && d.field("bTerminalID") == id
    })
}

const FEEDBACK_ATTRIBUTES: u32 = 0x11;

#[test]
fn the_audio_part_is_218_bytes() {
    assert_eq!(AUDIO_PART_LENGTH, 218);
    assert_eq!(AUDIO_PART.len(), 218);
}

#[test]
fn the_audio_part_has_no_feature_unit_and_no_descriptor_the_parser_does_not_know() {
    for descriptor in audio_part() {
        assert!(
            !matches!(descriptor.kind, Kind::Unrecognised { .. }),
            "{:?} at {}",
            descriptor.kind,
            descriptor.offset
        );
    }
}

#[test]
fn the_audio_part_follows_the_order_of_the_feedback_note_section_3() {
    let kinds: Vec<Kind> = audio_part().iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::Configuration,
            Kind::InterfaceAssociation,
            Kind::Interface,
            Kind::ControlHeader,
            Kind::ClockSource,
            Kind::InputTerminal,
            Kind::OutputTerminal,
            Kind::InputTerminal,
            Kind::OutputTerminal,
            Kind::Interface,
            Kind::Interface,
            Kind::StreamingGeneral,
            Kind::FormatType,
            Kind::Endpoint,
            Kind::StreamingEndpoint,
            Kind::Endpoint,
            Kind::Interface,
            Kind::Interface,
            Kind::StreamingGeneral,
            Kind::FormatType,
            Kind::Endpoint,
            Kind::StreamingEndpoint,
        ]
    );
}

#[test]
fn the_configuration_header_matches_the_note_table_and_counts_the_audio_part() {
    let parsed = audio_part();
    let configuration = only(&parsed, |d| d.kind == Kind::Configuration);
    assert_matches_note(
        &configuration,
        &table_after("### Configuration descriptor"),
        1,
    );
    assert_eq!(configuration.field("wTotalLength"), 218);
}

#[test]
fn the_configuration_is_bus_powered_at_500_milliamps() {
    let configuration = only(&audio_part(), |d| d.kind == Kind::Configuration);
    assert_eq!(configuration.field("bmAttributes"), 0x80);
    assert_eq!(configuration.field("bMaxPower") * 2, 500);
}

#[test]
fn the_interface_association_matches_the_note_table() {
    let association = only(&audio_part(), |d| d.kind == Kind::InterfaceAssociation);
    assert_matches_note(
        &association,
        &table_after("### Interface Association Descriptor"),
        1,
    );
}

#[test]
fn the_audio_control_interface_matches_the_note_table_with_no_interrupt_endpoint() {
    let control = interface(&audio_part(), 0, 0);
    assert_matches_note(&control, &table_after("### Standard AC interface"), 1);
    assert_eq!(control.field("bNumEndpoints"), 0);
}

#[test]
fn the_control_header_matches_the_note_table_with_the_io_box_category() {
    let header = only(&audio_part(), |d| d.kind == Kind::ControlHeader);
    assert_matches_note(&header, &table_after("### Class-specific AC header"), 1);
    assert_eq!(header.field("bCategory"), 0x08);
}

#[test]
fn the_clock_source_matches_the_note_table() {
    let clock = only(&audio_part(), |d| d.kind == Kind::ClockSource);
    assert_matches_note(&clock, &table_after("### Clock Source"), 1);
}

#[test]
fn the_playback_stream_input_terminal_matches_the_note_table() {
    let terminal_one = terminal(&audio_part(), Kind::InputTerminal, 1);
    assert_matches_note(
        &terminal_one,
        &table_after("### Input Terminal 1, USB streaming"),
        1,
    );
}

#[test]
fn the_line_out_terminal_matches_the_note_table_as_a_line_connector() {
    let terminal_three = terminal(&audio_part(), Kind::OutputTerminal, 3);
    assert_matches_note(
        &terminal_three,
        &table_after("### Output Terminal 3, line out"),
        1,
    );
    assert_eq!(terminal_three.field("wTerminalType"), 0x0603);
    assert_eq!(terminal_three.field("bSourceID"), 1);
}

#[test]
fn the_analog_input_terminal_shares_the_stream_terminal_layout_as_a_microphone() {
    let parsed = audio_part();
    let terminal_four = terminal(&parsed, Kind::InputTerminal, 4);
    assert_matches_note(
        &terminal_four,
        &table_after("### Input Terminal 1, USB streaming")
            .into_iter()
            .filter(|row| !["bTerminalID", "wTerminalType"].contains(&row[0]))
            .collect::<Vec<_>>(),
        1,
    );
    assert_eq!(terminal_four.field("wTerminalType"), 0x0201);
}

#[test]
fn the_capture_stream_output_terminal_takes_the_analog_input() {
    let terminal_five = terminal(&audio_part(), Kind::OutputTerminal, 5);
    assert_matches_note(
        &terminal_five,
        &table_after("### Output Terminal 3, line out")
            .into_iter()
            .filter(|row| row[0] != "bTerminalID")
            .collect::<Vec<_>>(),
        1,
    );
    assert_eq!(terminal_five.field("wTerminalType"), 0x0101);
    assert_eq!(terminal_five.field("bSourceID"), 4);
}

#[test]
fn the_playback_interface_settings_match_the_note_table() {
    let parsed = audio_part();
    let rows = table_after("### AS interface 1, playback");
    assert_matches_note(&interface(&parsed, 1, 0), &rows, 1);
    assert_matches_note(&interface(&parsed, 1, 1), &rows, 2);
}

#[test]
fn the_capture_interface_settings_match_the_playback_table_with_one_endpoint() {
    let parsed = audio_part();
    let rows: Vec<_> = table_after("### AS interface 1, playback")
        .into_iter()
        .filter(|row| !["bInterfaceNumber", "bNumEndpoints"].contains(&row[0]))
        .collect();
    let idle = interface(&parsed, 2, 0);
    let streaming = interface(&parsed, 2, 1);
    assert_matches_note(&idle, &rows, 1);
    assert_matches_note(&streaming, &rows, 2);
    assert_eq!(idle.field("bNumEndpoints"), 0);
    assert_eq!(streaming.field("bNumEndpoints"), 1);
}

#[test]
fn both_streaming_general_descriptors_match_the_note_table_and_link_terminals_one_and_five() {
    let parsed = audio_part();
    let rows = table_after("Class-specific AS general:");
    let playback = in_interface(&parsed, Kind::StreamingGeneral, 1);
    assert_matches_note(&playback, &rows, 1);
    let capture_rows: Vec<_> = rows
        .into_iter()
        .filter(|row| row[0] != "bTerminalLink")
        .collect();
    let capture = in_interface(&parsed, Kind::StreamingGeneral, 2);
    assert_matches_note(&capture, &capture_rows, 1);
    assert_eq!(capture.field("bTerminalLink"), 5);
}

#[test]
fn both_format_descriptors_are_type_one_pcm_24_bits_in_4_byte_subslots() {
    let parsed = audio_part();
    let rows = table_after("Type I format type:");
    for number in [1, 2] {
        assert_matches_note(&in_interface(&parsed, Kind::FormatType, number), &rows, 1);
    }
}

#[test]
fn the_playback_data_endpoint_matches_the_note_table() {
    let out = endpoint(&audio_part(), 0x01);
    assert_matches_note(
        &out,
        &table_after("Standard isochronous data endpoint, OUT:"),
        1,
    );
}

#[test]
fn the_capture_data_endpoint_is_0x82_asynchronous_200_bytes_every_microframe() {
    let input = endpoint(&audio_part(), 0x82);
    assert_eq!(input.field("bmAttributes"), 0x05);
    assert_eq!(input.field("wMaxPacketSize"), 200);
    assert_eq!(input.field("bInterval"), 1);
}

#[test]
fn both_class_specific_endpoints_match_the_note_table() {
    let parsed = audio_part();
    let rows = table_after("Class-specific isochronous data endpoint:");
    for number in [1, 2] {
        assert_matches_note(
            &in_interface(&parsed, Kind::StreamingEndpoint, number),
            &rows,
            1,
        );
    }
}

#[test]
fn the_feedback_endpoint_matches_the_note_table_at_0x81_polled_every_8_microframes() {
    let feedback = endpoint(&audio_part(), 0x81);
    assert_matches_note(&feedback, &table_after("### Descriptor fields"), 1);
    assert_eq!(feedback.field("bmAttributes"), FEEDBACK_ATTRIBUTES);
    assert_eq!(feedback.field("wMaxPacketSize"), 4);
    assert_eq!(feedback.field("bInterval"), 4);
}

#[test]
fn every_length_field_equals_the_bytes_it_counts() {
    let parsed = audio_part();
    let configuration = only(&parsed, |d| d.kind == Kind::Configuration);
    assert_eq!(
        configuration.field("wTotalLength") as usize,
        AUDIO_PART.len()
    );
    let header = only(&parsed, |d| d.kind == Kind::ControlHeader);
    let class_block: u32 = parsed
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                Kind::ControlHeader
                    | Kind::ClockSource
                    | Kind::InputTerminal
                    | Kind::OutputTerminal
            )
        })
        .map(|d| d.field("bLength"))
        .sum();
    assert_eq!(header.field("wTotalLength"), class_block);
    assert_eq!(header.field("wTotalLength"), 75);
    let header_end = header.offset + header.field("wTotalLength") as usize;
    let next_interface = parsed
        .iter()
        .find(|d| d.kind == Kind::Interface && d.offset > header.offset)
        .expect("a streaming interface follows the control block");
    assert_eq!(header_end, next_interface.offset);
}

#[test]
fn interface_numbers_are_contiguous_and_the_association_and_configuration_count_them() {
    let parsed = audio_part();
    let numbers: BTreeSet<u32> = of_kind(&parsed, Kind::Interface)
        .iter()
        .map(|d| d.field("bInterfaceNumber"))
        .collect();
    assert_eq!(numbers, BTreeSet::from([0, 1, 2]));
    let association = only(&parsed, |d| d.kind == Kind::InterfaceAssociation);
    assert_eq!(association.field("bFirstInterface"), 0);
    assert_eq!(association.field("bInterfaceCount") as usize, numbers.len());
    let configuration = only(&parsed, |d| d.kind == Kind::Configuration);
    assert_eq!(
        configuration.field("bNumInterfaces") as usize,
        numbers.len()
    );
}

#[test]
fn alternate_settings_ascend_from_an_empty_setting_zero() {
    let parsed = audio_part();
    for number in [0, 1, 2] {
        let alternates: Vec<u32> = of_kind(&parsed, Kind::Interface)
            .iter()
            .filter(|d| d.field("bInterfaceNumber") == number)
            .map(|d| d.field("bAlternateSetting"))
            .collect();
        let expected: Vec<u32> = (0..alternates.len() as u32).collect();
        assert_eq!(alternates, expected);
        assert_eq!(
            interface(&parsed, number as u8, 0).field("bNumEndpoints"),
            0
        );
    }
}

#[test]
fn every_interface_declares_exactly_the_endpoints_that_follow_it() {
    let parsed = audio_part();
    for setting in of_kind(&parsed, Kind::Interface) {
        let following = parsed
            .iter()
            .filter(|d| d.offset > setting.offset)
            .take_while(|d| d.kind != Kind::Interface)
            .filter(|d| d.kind == Kind::Endpoint)
            .count();
        assert_eq!(
            setting.field("bNumEndpoints") as usize,
            following,
            "interface {:?}",
            setting.interface
        );
    }
}

#[test]
fn entity_ids_are_unique_and_nonzero_and_every_reference_names_a_declared_entity() {
    let parsed = audio_part();
    let clocks: Vec<u32> = of_kind(&parsed, Kind::ClockSource)
        .iter()
        .map(|d| d.field("bClockID"))
        .collect();
    let terminals: Vec<u32> = parsed
        .iter()
        .filter(|d| matches!(d.kind, Kind::InputTerminal | Kind::OutputTerminal))
        .map(|d| d.field("bTerminalID"))
        .collect();
    let all: Vec<u32> = clocks.iter().chain(&terminals).copied().collect();
    let unique: BTreeSet<u32> = all.iter().copied().collect();
    assert_eq!(unique.len(), all.len());
    assert!(!unique.contains(&0));
    assert_eq!(clocks, [10]);
    for entity in parsed
        .iter()
        .filter(|d| matches!(d.kind, Kind::InputTerminal | Kind::OutputTerminal))
    {
        assert!(clocks.contains(&entity.field("bCSourceID")));
    }
    for output in of_kind(&parsed, Kind::OutputTerminal) {
        let source = output.field("bSourceID");
        assert!(terminals.contains(&source));
        assert_ne!(source, output.field("bTerminalID"));
    }
}

#[test]
fn each_streaming_interface_links_a_usb_streaming_terminal_facing_its_data_direction() {
    let parsed = audio_part();
    for number in [1, 2] {
        let link = in_interface(&parsed, Kind::StreamingGeneral, number).field("bTerminalLink");
        let data = only(&parsed, |d| {
            d.kind == Kind::Endpoint
                && d.interface.map(|(n, _)| n) == Some(number)
                && d.field("bmAttributes") != FEEDBACK_ATTRIBUTES
        });
        let toward_host = data.field("bEndpointAddress") & 0x80 != 0;
        let linked_kind = if toward_host {
            Kind::OutputTerminal
        } else {
            Kind::InputTerminal
        };
        let linked = terminal(&parsed, linked_kind, link);
        assert_eq!(linked.field("wTerminalType"), 0x0101);
    }
}

#[test]
fn the_format_type_of_each_streaming_interface_equals_its_general_descriptor() {
    let parsed = audio_part();
    for number in [1, 2] {
        assert_eq!(
            in_interface(&parsed, Kind::StreamingGeneral, number).field("bFormatType"),
            in_interface(&parsed, Kind::FormatType, number).field("bFormatType")
        );
    }
}

#[test]
fn the_feedback_endpoint_shares_the_playback_number_in_the_opposite_direction() {
    let parsed = audio_part();
    let feedback = only(&parsed, |d| {
        d.kind == Kind::Endpoint && d.field("bmAttributes") == FEEDBACK_ATTRIBUTES
    });
    let playback = only(&parsed, |d| {
        d.kind == Kind::Endpoint
            && d.field("bEndpointAddress") & 0x80 == 0
            && d.field("bmAttributes") != FEEDBACK_ATTRIBUTES
    });
    assert_eq!(feedback.interface, playback.interface);
    assert_eq!(
        feedback.field("bEndpointAddress"),
        playback.field("bEndpointAddress") | 0x80
    );
}

#[test]
fn endpoint_addresses_are_unique() {
    let addresses: Vec<u32> = of_kind(&audio_part(), Kind::Endpoint)
        .iter()
        .map(|d| d.field("bEndpointAddress"))
        .collect();
    let unique: BTreeSet<u32> = addresses.iter().copied().collect();
    assert_eq!(unique.len(), addresses.len());
    assert_eq!(unique, BTreeSet::from([0x01, 0x81, 0x82]));
}

#[test]
fn one_200_byte_setting_carries_the_largest_packet_at_every_rate_in_both_directions() {
    let largest_packets = [48, 56, 96, 104, 184, 200];
    let parsed = audio_part();
    for address in [0x01, 0x82] {
        let reserved = endpoint(&parsed, address).field("wMaxPacketSize");
        assert_eq!(reserved, *largest_packets.iter().max().unwrap());
    }
}
