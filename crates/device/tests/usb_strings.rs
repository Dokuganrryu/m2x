use device::usb::{
    BENCH_STRINGS, DescriptorText, FACTORY_SERIAL_TEMPLATE, SerialNumber, TextRefusal,
};

fn utf16_string_descriptor(text: &str) -> Vec<u8> {
    let units: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    [vec![u8::try_from(units.len() + 2).unwrap(), 0x03], units].concat()
}

#[test]
fn a_manufacturer_string_renders_as_a_utf16_little_endian_string_descriptor() {
    let text = DescriptorText::new("m2x bench").expect("plain text");
    assert_eq!(text.as_str(), "m2x bench");
    assert_eq!(
        text.descriptor(),
        utf16_string_descriptor("m2x bench").as_slice()
    );
}

#[test]
fn an_empty_string_is_refused() {
    assert_eq!(DescriptorText::new(""), Err(TextRefusal::Empty));
}

#[test]
fn a_string_outside_printable_ascii_is_refused() {
    for text in ["tab\there", "caf\u{e9}", "line\n"] {
        assert_eq!(
            DescriptorText::new(text),
            Err(TextRefusal::NotPrintableAscii),
            "{text:?}"
        );
    }
}

#[test]
fn the_longest_string_that_fits_one_descriptor_is_126_characters() {
    let longest = "a".repeat(126);
    let text = DescriptorText::new(longest.leak()).expect("fits bLength 254");
    assert_eq!(text.descriptor().len(), 254);
    assert_eq!(text.descriptor()[0], 254);
    assert_eq!(
        DescriptorText::new("a".repeat(127).leak()),
        Err(TextRefusal::LongerThanOneStringDescriptor)
    );
}

#[test]
fn the_bench_strings_render_as_descriptors() {
    for text in [BENCH_STRINGS.manufacturer, BENCH_STRINGS.product] {
        assert_eq!(
            text.descriptor(),
            utf16_string_descriptor(text.as_str()).as_slice()
        );
    }
}

#[test]
fn a_valid_factory_serial_is_carried_unchanged_into_string_three() {
    let serial = SerialNumber::from_factory_copy(*b"M2A1234567");
    assert_eq!(serial.as_str(), "M2A1234567");
    assert_eq!(
        serial.descriptor().as_slice(),
        utf16_string_descriptor("M2A1234567")
    );
}

#[test]
fn every_visible_ascii_byte_other_than_comma_is_accepted_in_a_serial() {
    for byte in (0x21..=0x7E_u8).filter(|&byte| byte != b',') {
        let copy = [byte; 10];
        assert_eq!(
            SerialNumber::from_factory_copy(copy).ascii(),
            copy,
            "{byte:#04x}"
        );
    }
}

#[test]
fn a_serial_with_any_byte_outside_0x21_to_0x7e_or_a_comma_becomes_the_factory_template() {
    for bad in [0x00, 0x20, b',', 0x7F, 0x80, 0xFF] {
        for position in 0..10 {
            let mut copy = *b"ABCDEFGHIJ";
            copy[position] = bad;
            assert_eq!(
                SerialNumber::from_factory_copy(copy),
                FACTORY_SERIAL_TEMPLATE,
                "{bad:#04x} at {position}"
            );
        }
    }
}

#[test]
fn the_factory_serial_template_is_ten_ascii_zeros() {
    assert_eq!(FACTORY_SERIAL_TEMPLATE.as_str(), "0000000000");
    assert_eq!(
        SerialNumber::from_factory_copy([0xFF; 10]).as_str(),
        "0000000000"
    );
}
