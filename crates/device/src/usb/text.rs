use super::bytes::{DescriptorBytes, DescriptorType};

const STRING_HEADER_BYTES: usize = 2;
const UTF16_UNIT_BYTES: usize = 2;
const LONGEST_STRING_DESCRIPTOR: usize =
    (u8::MAX as usize - STRING_HEADER_BYTES) / UTF16_UNIT_BYTES * UTF16_UNIT_BYTES
        + STRING_HEADER_BYTES;
pub const LONGEST_DESCRIPTOR_TEXT: usize =
    (LONGEST_STRING_DESCRIPTOR - STRING_HEADER_BYTES) / UTF16_UNIT_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRefusal {
    Empty,
    LongerThanOneStringDescriptor,
    NotPrintableAscii,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorText {
    text: &'static str,
    descriptor: [u8; LONGEST_STRING_DESCRIPTOR],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceStrings {
    pub manufacturer: DescriptorText,
    pub product: DescriptorText,
}

const fn is_printable_ascii(byte: u8) -> bool {
    matches!(byte, b' '..=b'~')
}

const fn ascii_string_descriptor<const LENGTH: usize>(text: &[u8]) -> [u8; LENGTH] {
    let mut bytes = DescriptorBytes::<LENGTH>::empty()
        .byte((STRING_HEADER_BYTES + text.len() * UTF16_UNIT_BYTES) as u8)
        .byte(DescriptorType::String as u8);
    let mut at = 0;
    while at < text.len() {
        bytes = bytes.word(text[at] as u16);
        at += 1;
    }
    bytes.pad_to_end()
}

impl DescriptorText {
    pub const fn new(text: &'static str) -> Result<Self, TextRefusal> {
        let ascii = text.as_bytes();
        if ascii.is_empty() {
            return Err(TextRefusal::Empty);
        }
        if ascii.len() > LONGEST_DESCRIPTOR_TEXT {
            return Err(TextRefusal::LongerThanOneStringDescriptor);
        }
        let mut at = 0;
        while at < ascii.len() {
            if !is_printable_ascii(ascii[at]) {
                return Err(TextRefusal::NotPrintableAscii);
            }
            at += 1;
        }
        Ok(Self {
            text,
            descriptor: ascii_string_descriptor(ascii),
        })
    }

    pub const fn as_str(&self) -> &'static str {
        self.text
    }

    pub fn descriptor(&self) -> &[u8] {
        let length = usize::from(self.descriptor[0]);
        self.descriptor.get(..length).unwrap_or(&self.descriptor)
    }
}

pub const SERIAL_NUMBER_LENGTH: usize = 10;
pub const SERIAL_NUMBER_DESCRIPTOR_LENGTH: usize =
    STRING_HEADER_BYTES + SERIAL_NUMBER_LENGTH * UTF16_UNIT_BYTES;
const FACTORY_SERIAL_SEPARATOR: u8 = b',';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialNumber([u8; SERIAL_NUMBER_LENGTH]);

pub const FACTORY_SERIAL_TEMPLATE: SerialNumber = SerialNumber([b'0'; SERIAL_NUMBER_LENGTH]);

const fn is_factory_serial_byte(byte: u8) -> bool {
    matches!(byte, 0x21..=0x7E) && byte != FACTORY_SERIAL_SEPARATOR
}

impl SerialNumber {
    pub const fn from_factory_copy(copy: [u8; SERIAL_NUMBER_LENGTH]) -> Self {
        let mut at = 0;
        while at < SERIAL_NUMBER_LENGTH {
            if !is_factory_serial_byte(copy[at]) {
                return FACTORY_SERIAL_TEMPLATE;
            }
            at += 1;
        }
        Self(copy)
    }

    pub const fn ascii(&self) -> [u8; SERIAL_NUMBER_LENGTH] {
        self.0
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.0).unwrap_or_default()
    }

    pub const fn descriptor(&self) -> [u8; SERIAL_NUMBER_DESCRIPTOR_LENGTH] {
        ascii_string_descriptor(&self.0)
    }
}

const fn accepted_at_compile_time(text: &'static str) -> DescriptorText {
    match DescriptorText::new(text) {
        Ok(accepted) => accepted,
        Err(_) => panic!("the text is refused"),
    }
}

pub const BENCH_STRINGS: DeviceStrings = DeviceStrings {
    manufacturer: accepted_at_compile_time("m2x"),
    product: accepted_at_compile_time("m2x bench"),
};
