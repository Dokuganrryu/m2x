use super::audio_function::{
    AUDIO_FUNCTION_DESCRIPTORS, AUDIO_FUNCTION_LENGTH, AUDIO_INTERFACE_COUNT,
};
use super::bytes::{DescriptorBytes, DescriptorType};
use super::device::{
    CONFIGURATION_COUNT, HIGH_SPEED_CONTROL_PACKET_BYTES, MISCELLANEOUS_INTERFACE_ASSOCIATION,
    USB_2_0,
};
use super::serial_function::{SERIAL_FUNCTION_LENGTH, SERIAL_INTERFACE_COUNT, serial_function};
use super::standard::{
    CONFIGURATION_HEADER_LENGTH, ConfigurationView, InterfaceNumber, Milliamps,
    bus_powered_configuration_header,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusSpeed {
    Full,
    High,
}

const HIGH_SPEED_CONFIGURATION_LENGTH: usize =
    CONFIGURATION_HEADER_LENGTH + AUDIO_FUNCTION_LENGTH + SERIAL_FUNCTION_LENGTH;

const fn high_speed_configuration(
    view: ConfigurationView,
) -> [u8; HIGH_SPEED_CONFIGURATION_LENGTH] {
    DescriptorBytes::empty()
        .append(&bus_powered_configuration_header(
            view,
            HIGH_SPEED_CONFIGURATION_LENGTH,
            AUDIO_INTERFACE_COUNT + SERIAL_INTERFACE_COUNT,
            Milliamps::BUS_POWERED_HIGH_POWER,
        ))
        .append(&AUDIO_FUNCTION_DESCRIPTORS)
        .append(&serial_function(
            InterfaceNumber::FIRST.offset_by(AUDIO_INTERFACE_COUNT),
            BusSpeed::High,
        ))
        .finish()
}

const FULL_SPEED_CONFIGURATION_LENGTH: usize = CONFIGURATION_HEADER_LENGTH + SERIAL_FUNCTION_LENGTH;

const fn full_speed_configuration(
    view: ConfigurationView,
) -> [u8; FULL_SPEED_CONFIGURATION_LENGTH] {
    DescriptorBytes::empty()
        .append(&bus_powered_configuration_header(
            view,
            FULL_SPEED_CONFIGURATION_LENGTH,
            SERIAL_INTERFACE_COUNT,
            Milliamps::BUS_POWERED_HIGH_POWER,
        ))
        .append(&serial_function(InterfaceNumber::FIRST, BusSpeed::Full))
        .finish()
}

static HIGH_SPEED_AT_HIGH_SPEED: [u8; HIGH_SPEED_CONFIGURATION_LENGTH] =
    high_speed_configuration(ConfigurationView::AtCurrentSpeed);
static HIGH_SPEED_SEEN_FROM_FULL_SPEED: [u8; HIGH_SPEED_CONFIGURATION_LENGTH] =
    high_speed_configuration(ConfigurationView::AtOtherSpeed);
static FULL_SPEED_AT_FULL_SPEED: [u8; FULL_SPEED_CONFIGURATION_LENGTH] =
    full_speed_configuration(ConfigurationView::AtCurrentSpeed);
static FULL_SPEED_SEEN_FROM_HIGH_SPEED: [u8; FULL_SPEED_CONFIGURATION_LENGTH] =
    full_speed_configuration(ConfigurationView::AtOtherSpeed);

pub fn configuration(current: BusSpeed) -> &'static [u8] {
    match current {
        BusSpeed::High => &HIGH_SPEED_AT_HIGH_SPEED,
        BusSpeed::Full => &FULL_SPEED_AT_FULL_SPEED,
    }
}

pub fn other_speed_configuration(current: BusSpeed) -> &'static [u8] {
    match current {
        BusSpeed::High => &FULL_SPEED_SEEN_FROM_HIGH_SPEED,
        BusSpeed::Full => &HIGH_SPEED_SEEN_FROM_FULL_SPEED,
    }
}

const CONTROL_PACKET_BYTES_AT_EITHER_SPEED: u8 = HIGH_SPEED_CONTROL_PACKET_BYTES;
const RESERVED_ZERO: u8 = 0;

pub const DEVICE_QUALIFIER_LENGTH: usize = 10;

pub static DEVICE_QUALIFIER: [u8; DEVICE_QUALIFIER_LENGTH] =
    DescriptorBytes::descriptor(DescriptorType::DeviceQualifier)
        .word(USB_2_0)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.class)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.subclass)
        .byte(MISCELLANEOUS_INTERFACE_ASSOCIATION.protocol)
        .byte(CONTROL_PACKET_BYTES_AT_EITHER_SPEED)
        .byte(CONFIGURATION_COUNT)
        .byte(RESERVED_ZERO)
        .finish();
