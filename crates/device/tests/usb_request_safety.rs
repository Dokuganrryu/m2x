mod usb_support;

use device::usb::BusSpeed;
use device::usb::answer::{Answer, answer};
use device::usb::function::{Event, State};
use device::usb::request::{ControlRequest, Readable, Stall};
use usb_support::{XorShift, context, enumerated_at, run, setup, step};

const FACTORY_VENDOR_INDEX_THAT_ERASES_THE_FACTORY_PAGE: u16 = 7;
const FACTORY_ERASE_REQUESTS: [u8; 2] = *b"qr";
const SEQUENCES: u64 = 200_000;

fn every_reachable_state() -> Vec<State> {
    let high = enumerated_at(BusSpeed::High);
    let full = enumerated_at(BusSpeed::Full);
    vec![
        State::INITIAL,
        run(&[Event::Attached]),
        run(&[Event::Attached, Event::BusReset(BusSpeed::High)]),
        run(&[Event::Attached, Event::BusReset(BusSpeed::Full)]),
        high,
        full,
        step(high, Event::Suspended).0,
    ]
}

#[test]
fn vendor_out_q_and_r_that_erase_the_factory_factory_page_are_refused_with_no_effect() {
    for state in every_reachable_state() {
        let context = context(state);
        for request_type in [0x40, 0x41, 0x42, 0x43] {
            for request in FACTORY_ERASE_REQUESTS {
                for value in [0, 1, 0x00FF, 0xFFFF] {
                    for (length, data) in [(0, &[][..]), (4, &[0xDE, 0xAD, 0xBE, 0xEF][..])] {
                        let packet = setup(
                            request_type,
                            request,
                            value,
                            FACTORY_VENDOR_INDEX_THAT_ERASES_THE_FACTORY_PAGE,
                            length,
                        );
                        assert_eq!(
                            answer(packet, data, &context),
                            Answer::Stall,
                            "{packet:02X?} in {state:?}"
                        );
                        assert_eq!(
                            ControlRequest::decode(packet, data, BusSpeed::High),
                            Err(Stall)
                        );
                    }
                }
            }
        }
    }
}

const VENDOR_TYPE: u8 = 0b10 << 5;
const RESERVED_TYPE: u8 = 0b11 << 5;
const TYPE_BITS: u8 = 0b11 << 5;

#[test]
fn every_vendor_and_reserved_type_request_is_refused_with_no_effect() {
    for state in every_reachable_state() {
        let context = context(state);
        for template in VALID_SETUPS {
            let length = usize::from(u16::from_le_bytes([template[6], template[7]]));
            let data = if template[0] & 0x80 == 0 {
                vec![0x5A; length]
            } else {
                vec![]
            };
            for kind in [VENDOR_TYPE, RESERVED_TYPE] {
                for request in 0..=u8::MAX {
                    let mut packet = template;
                    packet[0] = (packet[0] & !TYPE_BITS) | kind;
                    packet[1] = request;
                    assert_eq!(
                        answer(packet, &data, &context),
                        Answer::Stall,
                        "{packet:02X?} in {state:?}"
                    );
                }
            }
        }
    }
}

const VALID_SETUPS: [[u8; 8]; 24] = [
    setup(0x80, 6, 0x0100, 0, 18),
    setup(0x80, 6, 0x0200, 0, 284),
    setup(0x80, 6, 0x0300, 0, 255),
    setup(0x80, 6, 0x0302, 0x0409, 255),
    setup(0x80, 6, 0x0600, 0, 10),
    setup(0x80, 6, 0x0700, 0, 75),
    setup(0x00, 5, 9, 0, 0),
    setup(0x00, 9, 1, 0, 0),
    setup(0x80, 8, 0, 0, 1),
    setup(0x80, 0, 0, 0, 2),
    setup(0x81, 0, 0, 1, 2),
    setup(0x82, 0, 0, 0x84, 2),
    setup(0x02, 1, 0, 0x04, 0),
    setup(0x02, 3, 0, 0x83, 0),
    setup(0x01, 11, 1, 1, 0),
    setup(0x81, 10, 0, 2, 1),
    setup(0xA1, 1, 0x0100, 0x0A00, 4),
    setup(0x21, 1, 0x0100, 0x0A00, 4),
    setup(0xA1, 2, 0x0100, 0x0A00, 74),
    setup(0xA1, 1, 0x0200, 0x0A00, 1),
    setup(0x21, 0x20, 0, 3, 7),
    setup(0xA1, 0x21, 0, 3, 7),
    setup(0x21, 0x22, 3, 3, 0),
    setup(0x21, 0x22, 1, 0, 0),
];

fn random_exchange(random: &mut XorShift) -> ([u8; 8], Vec<u8>) {
    let mut packet = VALID_SETUPS[random.below(VALID_SETUPS.len() as u64) as usize];
    match random.below(4) {
        0 => {}
        1 => packet[random.below(8) as usize] = random.byte(),
        2 => packet[random.below(8) as usize] ^= 1 << random.below(8),
        _ => packet.iter_mut().for_each(|byte| *byte = random.byte()),
    }
    let declared = usize::from(u16::from_le_bytes([packet[6], packet[7]]));
    let data_length = match random.below(4) {
        0 | 1 => declared.min(300),
        2 => random.below(80) as usize,
        _ => 0,
    };
    let data = (0..data_length).map(|_| random.byte()).collect();
    (packet, data)
}

#[test]
fn random_setup_packets_and_data_stages_decode_to_a_request_or_a_stall_without_panicking() {
    let mut random = XorShift(0x0BAD_5EED_F00D);
    let mut decoded = 0_u64;
    for _ in 0..SEQUENCES {
        let (packet, data) = random_exchange(&mut random);
        for speed in [BusSpeed::High, BusSpeed::Full] {
            let outcome = ControlRequest::decode(packet, &data, speed);
            if (packet[0] >> 5) & 0b11 >= 2 {
                assert_eq!(outcome, Err(Stall), "{packet:02X?}");
            }
            match outcome {
                Ok(ControlRequest::Read(item, limit)) => {
                    decoded += 1;
                    assert!(packet[0] & 0x80 != 0, "{packet:02X?} reads {item:?}");
                    assert!(data.is_empty(), "{packet:02X?} reads with a data stage");
                    assert_eq!(
                        limit.bytes(),
                        usize::from(u16::from_le_bytes([packet[6], packet[7]]))
                    );
                    assert!(!matches!(item, Readable::Text(_)) || packet[5] == 0x04);
                }
                Ok(ControlRequest::Write(item)) => {
                    decoded += 1;
                    assert!(packet[0] & 0x80 == 0, "{packet:02X?} writes {item:?}");
                    assert_eq!(
                        data.len(),
                        usize::from(u16::from_le_bytes([packet[6], packet[7]])),
                        "{packet:02X?} writes {item:?} with a short data stage"
                    );
                }
                Err(Stall) => {}
            }
        }
    }
    assert!(
        decoded > SEQUENCES / 100,
        "the fuzz reaches decoded requests: {decoded}"
    );
}

#[test]
fn no_answer_to_any_random_request_returns_more_bytes_than_wlength() {
    let mut random = XorShift(0xFEED_FACE_CAFE);
    let states = every_reachable_state();
    for _ in 0..SEQUENCES {
        let (packet, data) = random_exchange(&mut random);
        let context = context(states[random.below(states.len() as u64) as usize]);
        if let Answer::Reply(reply) = answer(packet, &data, &context) {
            let length = usize::from(u16::from_le_bytes([packet[6], packet[7]]));
            assert!(reply.bytes().len() <= length, "{packet:02X?}");
            assert!(packet[0] & 0x80 != 0, "{packet:02X?}");
        }
    }
}
