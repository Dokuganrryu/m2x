use super::bytes::{DescriptorBytes, DescriptorType};
use super::identity::{
    DeviceRelease, PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED, UsbIds,
};

pub const USB_2_0: u16 = 0x0200;
pub const HIGH_SPEED_CONTROL_PACKET_BYTES: u8 = 64;
pub const CONFIGURATION_COUNT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StringIndex {
    Manufacturer = 1,
    Product = 2,
    SerialNumber = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassTriple {
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
}

pub const MISCELLANEOUS_INTERFACE_ASSOCIATION: ClassTriple = ClassTriple {
    class: 0xEF,
    subclass: 0x02,
    protocol: 0x01,
};

pub const DEVICE_DESCRIPTOR_LENGTH: usize = 18;

pub const fn device_descriptor(
    ids: UsbIds,
    release: DeviceRelease,
) -> [u8; DEVICE_DESCRIPTOR_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::Device)
        .word(USB_2_0)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.class)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.subclass)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.protocol)
        .byte(HIGH_SPEED_CONTROL_PACKET_BYTES)
        .word(ids.vendor().value())
        .word(ids.product().value())
        .word(release.binary_coded_decimal())
        .byte(StringIndex::Manufacturer as u8)
        .byte(StringIndex::Product as u8)
        .byte(StringIndex::SerialNumber as u8)
        .byte(CONFIGURATION_COUNT)
        .finish()
}

pub static BENCH_DEVICE_DESCRIPTOR: [u8; DEVICE_DESCRIPTOR_LENGTH] = device_descriptor(
    PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED,
    DeviceRelease::FIRST,
);
