use device::factory::{FACTORY_MAGIC, FactoryRead};
use device::usb::{FACTORY_SERIAL_TEMPLATE, SerialNumber};

#[test]
fn the_factory_magic_is_the_word_0x5554_4f4d() {
    assert_eq!(FACTORY_MAGIC, 0x5554_4F4D);
}

#[test]
fn a_page_with_the_magic_and_a_valid_serial_gives_that_serial() {
    let read = FactoryRead::Loaded {
        magic: 0x5554_4F4D,
        serial: *b"M2A1234567",
    };
    assert_eq!(read.serial_number().as_str(), "M2A1234567");
}

#[test]
fn an_unreadable_page_gives_the_factory_template_serial() {
    assert_eq!(
        FactoryRead::Unreadable.serial_number(),
        FACTORY_SERIAL_TEMPLATE
    );
}

#[test]
fn an_erased_page_that_reads_as_all_ones_gives_the_factory_template_serial() {
    let read = FactoryRead::Loaded {
        magic: 0xFFFF_FFFF,
        serial: [0xFF; 10],
    };
    assert_eq!(read.serial_number(), FACTORY_SERIAL_TEMPLATE);
}

#[test]
fn a_valid_serial_behind_a_wrong_magic_gives_the_factory_template_serial() {
    for magic in [0, 0x4D4F_5455, 0x5554_4F4C, 0xFFFF_FFFF] {
        let read = FactoryRead::Loaded {
            magic,
            serial: *b"M2A1234567",
        };
        assert_eq!(
            read.serial_number(),
            FACTORY_SERIAL_TEMPLATE,
            "{magic:#010x}"
        );
    }
}

#[test]
fn a_right_magic_with_one_bad_serial_byte_gives_the_factory_template_serial() {
    let mut serial = *b"M2A1234567";
    serial[9] = b',';
    let read = FactoryRead::Loaded {
        magic: 0x5554_4F4D,
        serial,
    };
    assert_eq!(read.serial_number(), FACTORY_SERIAL_TEMPLATE);
    assert_eq!(
        read.serial_number(),
        SerialNumber::from_factory_copy(serial)
    );
}
