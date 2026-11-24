use super::bytes::{DescriptorBytes, DescriptorType};
use super::device::ClassTriple;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceNumber(u8);

impl InterfaceNumber {
    pub const AUDIO_CONTROL: Self = Self(0);
    pub const PLAYBACK_STREAM: Self = Self(1);
    pub const CAPTURE_STREAM: Self = Self(2);
    pub const FIRST: Self = Self(0);

    pub const fn offset_by(self, count: u8) -> Self {
        Self(self.0 + count)
    }

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlternateSetting {
    Base,
    Streaming,
}

impl AlternateSetting {
    pub const fn value(self) -> u8 {
        match self {
            Self::Base => 0,
            Self::Streaming => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointNumber(u8);

impl EndpointNumber {
    pub const PLAYBACK: Self = Self(1);
    pub const CAPTURE: Self = Self(2);
    pub const SERIAL_NOTIFICATION: Self = Self(3);
    pub const SERIAL_DATA: Self = Self(4);

    const DEVICE_TO_HOST: u8 = 0x80;

    pub const fn host_to_device(self) -> EndpointAddress {
        EndpointAddress(self.0)
    }

    pub const fn device_to_host(self) -> EndpointAddress {
        EndpointAddress(self.0 | Self::DEVICE_TO_HOST)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointAddress(u8);

impl EndpointAddress {
    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollingInterval {
    exponent_plus_one: u8,
}

impl PollingInterval {
    pub const EVERY_MICROFRAME: Self = Self {
        exponent_plus_one: 1,
    };
    pub const EVERY_8_MICROFRAMES: Self = Self {
        exponent_plus_one: 4,
    };

    pub const fn value(self) -> u8 {
        self.exponent_plus_one
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsochronousRole {
    AsynchronousData,
    ExplicitFeedback,
}

impl IsochronousRole {
    const ISOCHRONOUS: u8 = 0b01;
    const NO_SYNCHRONIZATION: u8 = 0b00;
    const ASYNCHRONOUS: u8 = 0b01;
    const DATA: u8 = 0b00;
    const FEEDBACK: u8 = 0b01;

    pub const fn attributes(self) -> u8 {
        let (synchronization, usage) = match self {
            Self::AsynchronousData => (Self::ASYNCHRONOUS, Self::DATA),
            Self::ExplicitFeedback => (Self::NO_SYNCHRONIZATION, Self::FEEDBACK),
        };
        Self::ISOCHRONOUS | synchronization << 2 | usage << 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketBytes(u16);

impl PacketBytes {
    pub const fn new(bytes: u16) -> Self {
        Self(bytes)
    }

    pub const fn value(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsochronousEndpoint {
    pub address: EndpointAddress,
    pub role: IsochronousRole,
    pub max_packet: PacketBytes,
    pub interval: PollingInterval,
}

impl IsochronousEndpoint {
    pub const LENGTH: usize = 7;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::Endpoint)
            .byte(self.address.value())
            .byte(self.role.attributes())
            .word(self.max_packet.value())
            .byte(self.interval.value())
            .finish()
    }
}

const NO_STRING: u8 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    pub number: InterfaceNumber,
    pub alternate: AlternateSetting,
    pub endpoints: u8,
    pub class: ClassTriple,
}

impl Interface {
    pub const LENGTH: usize = 9;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::Interface)
            .byte(self.number.value())
            .byte(self.alternate.value())
            .byte(self.endpoints)
            .byte(self.class.class)
            .byte(self.class.subclass)
            .byte(self.class.protocol)
            .byte(NO_STRING)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterfaceAssociation {
    pub first: InterfaceNumber,
    pub count: u8,
    pub function: ClassTriple,
}

impl InterfaceAssociation {
    pub const LENGTH: usize = 8;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::InterfaceAssociation)
            .byte(self.first.value())
            .byte(self.count)
            .byte(self.function.class)
            .byte(self.function.subclass)
            .byte(self.function.protocol)
            .byte(NO_STRING)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Milliamps(u16);

impl Milliamps {
    pub const BUS_POWERED_HIGH_POWER: Self = Self(500);
    const PER_MAX_POWER_UNIT: u16 = 2;

    const fn max_power_units(self) -> u8 {
        (self.0 / Self::PER_MAX_POWER_UNIT) as u8
    }
}

pub const CONFIGURATION_VALUE: u8 = 1;
const CONFIGURATION_ATTRIBUTES_RESERVED_ONE: u8 = 0x80;

pub const CONFIGURATION_HEADER_LENGTH: usize = 9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationView {
    AtCurrentSpeed,
    AtOtherSpeed,
}

impl ConfigurationView {
    const fn descriptor_type(self) -> DescriptorType {
        match self {
            Self::AtCurrentSpeed => DescriptorType::Configuration,
            Self::AtOtherSpeed => DescriptorType::OtherSpeedConfiguration,
        }
    }
}

pub const fn bus_powered_configuration_header(
    view: ConfigurationView,
    total_length: usize,
    interfaces: u8,
    current: Milliamps,
) -> [u8; CONFIGURATION_HEADER_LENGTH] {
    assert!(
        total_length <= u16::MAX as usize,
        "the configuration fits wTotalLength"
    );
    DescriptorBytes::descriptor(view.descriptor_type())
        .word(total_length as u16)
        .byte(interfaces)
        .byte(CONFIGURATION_VALUE)
        .byte(NO_STRING)
        .byte(CONFIGURATION_ATTRIBUTES_RESERVED_ONE)
        .byte(current.max_power_units())
        .finish()
}
