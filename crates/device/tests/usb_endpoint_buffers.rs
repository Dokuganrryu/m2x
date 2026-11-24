use device::usb::endpoint::{
    BUFFER_ALIGNMENT_BYTES, BufferSlot, ByteCount, Command, ENDPOINT_LIST_BYTES, InEntry,
    LARGEST_ISOCHRONOUS_IN_BYTES, ListEntry, OUT_WRITE_GRANULE_BYTES, OutEntry, TransferKind,
    USB_RAM_BYTES, UsbRam,
};
use device::usb::{BusSpeed, configuration};

const ENDPOINT_DESCRIPTOR: u8 = 0x05;
const DEVICE_TO_HOST: u8 = 0x80;
const ISOCHRONOUS: u8 = 0b01;
const TRANSFER_TYPE_MASK: u8 = 0b11;

struct DeclaredEndpoint {
    address: u8,
    attributes: u8,
    max_packet: u16,
}

fn declared_endpoints(speed: BusSpeed) -> Vec<DeclaredEndpoint> {
    let bytes = configuration(speed);
    let mut endpoints = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let length = bytes[at] as usize;
        if bytes[at + 1] == ENDPOINT_DESCRIPTOR {
            endpoints.push(DeclaredEndpoint {
                address: bytes[at + 2],
                attributes: bytes[at + 3],
                max_packet: u16::from_le_bytes([bytes[at + 4], bytes[at + 5]]),
            });
        }
        at += length;
    }
    endpoints
}

fn entries_serving(address: u8) -> Vec<ListEntry> {
    let slots = [BufferSlot::First, BufferSlot::Second];
    match address {
        0x01 => slots
            .map(|slot| ListEntry::Out(OutEntry::Playback(slot)))
            .to_vec(),
        0x81 => vec![ListEntry::In(InEntry::Feedback)],
        0x82 => slots
            .map(|slot| ListEntry::In(InEntry::Capture(slot)))
            .to_vec(),
        0x04 => vec![ListEntry::Out(OutEntry::Serial)],
        0x84 => vec![ListEntry::In(InEntry::Serial)],
        0x83 => vec![],
        other => panic!("the configuration declares an unexpected endpoint {other:#04X}"),
    }
}

#[test]
fn every_buffer_starts_on_a_64_byte_boundary_after_the_endpoint_list() {
    for entry in ListEntry::ALL {
        let region = entry.region();
        assert_eq!(region.offset % BUFFER_ALIGNMENT_BYTES, 0, "{entry:?}");
        assert!(region.offset >= ENDPOINT_LIST_BYTES, "{entry:?}");
    }
}

#[test]
fn no_two_buffers_overlap_and_all_fit_the_16_kilobyte_usb_ram() {
    assert_eq!(USB_RAM_BYTES, 16 * 1024);
    let mut regions: Vec<_> = ListEntry::ALL.iter().map(|e| (e.region(), *e)).collect();
    regions.sort_by_key(|(region, _)| region.offset);
    for pair in regions.windows(2) {
        assert!(pair[0].0.end() <= pair[1].0.offset, "{pair:?}");
    }
    let last = regions.last().unwrap().0;
    assert!(last.end() as usize <= USB_RAM_BYTES);
}

#[test]
fn every_out_buffer_holds_whole_eight_byte_groups_for_errata_usb_4() {
    assert_eq!(OUT_WRITE_GRANULE_BYTES, 8);
    for entry in ListEntry::ALL {
        if let ListEntry::Out(_) = entry {
            assert_eq!(entry.region().capacity.value() % 8, 0, "{entry:?}");
        }
    }
}

#[test]
fn every_isochronous_in_buffer_stays_at_or_under_1023_bytes_for_errata_usb_5() {
    assert_eq!(LARGEST_ISOCHRONOUS_IN_BYTES, 1023);
    for entry in ListEntry::ALL {
        if let (ListEntry::In(_), TransferKind::Isochronous) = (entry, entry.kind()) {
            assert!(entry.region().capacity.value() <= 1023, "{entry:?}");
        }
    }
}

#[test]
fn every_buffer_holds_the_largest_packet_its_endpoint_declares_at_either_speed() {
    for speed in [BusSpeed::High, BusSpeed::Full] {
        for endpoint in declared_endpoints(speed) {
            for entry in entries_serving(endpoint.address) {
                assert!(
                    entry.region().capacity.value() >= endpoint.max_packet,
                    "{entry:?} is smaller than {:#04X} at {speed:?}",
                    endpoint.address
                );
            }
        }
    }
    assert_eq!(
        ListEntry::Setup.region().capacity,
        ByteCount::new(8),
        "a setup packet is 8 bytes"
    );
    assert_eq!(
        ListEntry::Out(OutEntry::Control).region().capacity,
        ByteCount::new(64)
    );
    assert_eq!(
        ListEntry::In(InEntry::Control).region().capacity,
        ByteCount::new(64)
    );
}

#[test]
fn transfer_kinds_follow_the_declared_endpoint_attributes() {
    for endpoint in declared_endpoints(BusSpeed::High) {
        let isochronous = endpoint.attributes & TRANSFER_TYPE_MASK == ISOCHRONOUS;
        for entry in entries_serving(endpoint.address) {
            assert_eq!(
                entry.kind() == TransferKind::Isochronous,
                isochronous,
                "{entry:?}"
            );
            assert_eq!(
                matches!(entry, ListEntry::In(_)),
                endpoint.address & DEVICE_TO_HOST != 0,
                "{entry:?}"
            );
        }
    }
}

#[test]
fn list_offsets_follow_the_fixed_order_of_um_figure_163() {
    let expected = [
        (ListEntry::Out(OutEntry::Control), 0x00),
        (ListEntry::Setup, 0x04),
        (ListEntry::In(InEntry::Control), 0x08),
        (ListEntry::Out(OutEntry::Playback(BufferSlot::First)), 0x10),
        (ListEntry::Out(OutEntry::Playback(BufferSlot::Second)), 0x14),
        (ListEntry::In(InEntry::Feedback), 0x18),
        (ListEntry::In(InEntry::Capture(BufferSlot::First)), 0x28),
        (ListEntry::In(InEntry::Capture(BufferSlot::Second)), 0x2C),
        (ListEntry::Out(OutEntry::Serial), 0x40),
        (ListEntry::In(InEntry::Serial), 0x48),
    ];
    assert_eq!(expected.len(), ListEntry::ALL.len());
    for (entry, offset) in expected {
        assert_eq!(entry.list_offset(), offset, "{entry:?}");
    }
}

#[test]
fn an_out_buffer_reads_back_exactly_the_bytes_the_controller_landed() {
    let mut ram = UsbRam::new();
    let (buffers, controller) = ram.split();
    let (armed, command) = buffers.serial_out.receive(ByteCount::new(512));
    assert_eq!(
        command,
        Command::ArmOut {
            entry: OutEntry::Serial,
            room: ByteCount::new(512)
        }
    );
    let message = b"counters\n";
    controller.write(ListEntry::Out(OutEntry::Serial), message);
    let received = armed.landed(ByteCount::new(512 - message.len() as u16));
    assert_eq!(received.length(), ByteCount::new(message.len() as u16));
    assert_eq!(received.bytes().collect::<Vec<_>>(), message);
}

#[test]
fn an_out_room_is_rounded_down_to_whole_eight_byte_groups_and_capped_at_capacity() {
    let mut ram = UsbRam::new();
    let (buffers, _) = ram.split();
    let (_, command) = buffers.control_out.receive(ByteCount::new(63));
    assert_eq!(
        command,
        Command::ArmOut {
            entry: OutEntry::Control,
            room: ByteCount::new(56)
        }
    );
    let (_, command) = buffers.serial_out.receive(ByteCount::new(4096));
    assert_eq!(
        command,
        Command::ArmOut {
            entry: OutEntry::Serial,
            room: ByteCount::new(512)
        }
    );
}

#[test]
fn an_in_buffer_sends_the_bytes_it_was_filled_with_and_never_more_than_capacity() {
    let mut ram = UsbRam::new();
    let (mut buffers, controller) = ram.split();
    let length = buffers.control_in.fill_bytes(0u8..=200);
    assert_eq!(length, ByteCount::new(64));
    let (_, command) = buffers.control_in.send(ByteCount::new(300));
    assert_eq!(
        command,
        Command::ArmIn {
            entry: InEntry::Control,
            length: ByteCount::new(64)
        }
    );
    let sent: Vec<u8> = controller
        .read(ListEntry::In(InEntry::Control), ByteCount::new(64))
        .collect();
    assert_eq!(sent, (0u8..64).collect::<Vec<_>>());
}

#[test]
fn words_are_stored_least_significant_byte_first() {
    let mut ram = UsbRam::new();
    let (mut buffers, controller) = ram.split();
    let length = buffers.feedback.fill_words([0x0006_0000]);
    assert_eq!(length, ByteCount::new(4));
    let sent: Vec<u8> = controller
        .read(ListEntry::In(InEntry::Feedback), length)
        .collect();
    assert_eq!(sent, [0x00, 0x00, 0x06, 0x00]);
}

#[test]
fn retiring_an_armed_buffer_disarms_its_entry_and_gives_the_buffer_back() {
    let mut ram = UsbRam::new();
    let (buffers, _) = ram.split();
    let [first, _] = buffers.capture;
    let (armed, _) = first.send(ByteCount::new(48));
    let (buffer, command) = armed.retire();
    assert_eq!(
        command,
        Command::DisarmIn(InEntry::Capture(BufferSlot::First))
    );
    assert_eq!(buffer.entry(), InEntry::Capture(BufferSlot::First));
}

#[test]
fn the_whole_layout_ends_inside_the_factory_four_kilobyte_window() {
    let end = ListEntry::ALL
        .iter()
        .map(|entry| entry.region().end())
        .max()
        .unwrap();
    assert_eq!(end, 0x780 + 512, "the serial IN buffer is the last region");
    assert!(end <= 4096);
}
