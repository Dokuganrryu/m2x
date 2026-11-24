use super::bytes::{DescriptorBytes, DescriptorType};
use super::configuration::BusSpeed;
use super::device::ClassTriple;
use super::standard::{
    AlternateSetting, EndpointAddress, EndpointNumber, Interface, InterfaceAssociation,
    InterfaceNumber, PacketBytes,
};

const COMMUNICATIONS_ABSTRACT_CONTROL: ClassTriple = ClassTriple {
    class: 0x02,
    subclass: 0x02,
    protocol: 0x00,
};

const DATA_INTERFACE: ClassTriple = ClassTriple {
    class: 0x0A,
    subclass: 0x00,
    protocol: 0x00,
};

const CDC_RELEASE_1_2: u16 = 0x0120;

#[derive(Clone, Copy)]
#[repr(u8)]
enum FunctionalSubtype {
    Header = 0x00,
    CallManagement = 0x01,
    AbstractControlManagement = 0x02,
    Union = 0x06,
}

const LINE_CODING_CONTROL_LINE_STATE_AND_SERIAL_STATE: u8 = 0b0000_0010;
const DEVICE_HANDLES_NO_CALL_MANAGEMENT: u8 = 0b0000_0000;

const HEADER_LENGTH: usize = 5;
const ABSTRACT_CONTROL_MANAGEMENT_LENGTH: usize = 4;
const UNION_LENGTH: usize = 5;
const CALL_MANAGEMENT_LENGTH: usize = 5;

const fn header() -> [u8; HEADER_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassInterface)
        .byte(FunctionalSubtype::Header as u8)
        .word(CDC_RELEASE_1_2)
        .finish()
}

const fn abstract_control_management() -> [u8; ABSTRACT_CONTROL_MANAGEMENT_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassInterface)
        .byte(FunctionalSubtype::AbstractControlManagement as u8)
        .byte(LINE_CODING_CONTROL_LINE_STATE_AND_SERIAL_STATE)
        .finish()
}

const fn union(control: InterfaceNumber, data: InterfaceNumber) -> [u8; UNION_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassInterface)
        .byte(FunctionalSubtype::Union as u8)
        .byte(control.value())
        .byte(data.value())
        .finish()
}

const fn call_management(data: InterfaceNumber) -> [u8; CALL_MANAGEMENT_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassInterface)
        .byte(FunctionalSubtype::CallManagement as u8)
        .byte(DEVICE_HANDLES_NO_CALL_MANAGEMENT)
        .byte(data.value())
        .finish()
}

#[derive(Clone, Copy)]
#[repr(u8)]
enum TransferType {
    Bulk = 0b10,
    Interrupt = 0b11,
}

const MICROFRAMES_PER_MILLISECOND: u32 = 8;
const NOTIFICATION_PERIOD_MILLISECONDS: u32 = 16;
const NOTIFICATION_PACKET: PacketBytes = PacketBytes::new(64);
const BULK_INTERVAL_IGNORED_BY_THE_HOST: u8 = 0;

impl BusSpeed {
    pub(crate) const fn bulk_packet(self) -> PacketBytes {
        match self {
            Self::High => PacketBytes::new(512),
            Self::Full => PacketBytes::new(64),
        }
    }

    const fn notification_interval(self) -> u8 {
        match self {
            Self::High => {
                let microframes = NOTIFICATION_PERIOD_MILLISECONDS * MICROFRAMES_PER_MILLISECOND;
                assert!(
                    microframes.is_power_of_two(),
                    "a high-speed interrupt period is a power of two"
                );
                (microframes.trailing_zeros() + 1) as u8
            }
            Self::Full => NOTIFICATION_PERIOD_MILLISECONDS as u8,
        }
    }
}

const ENDPOINT_LENGTH: usize = 7;

const fn endpoint(
    address: EndpointAddress,
    transfer: TransferType,
    max_packet: PacketBytes,
    encoded_interval: u8,
) -> [u8; ENDPOINT_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::Endpoint)
        .byte(address.value())
        .byte(transfer as u8)
        .word(max_packet.value())
        .byte(encoded_interval)
        .finish()
}

const fn bulk(address: EndpointAddress, speed: BusSpeed) -> [u8; ENDPOINT_LENGTH] {
    endpoint(
        address,
        TransferType::Bulk,
        speed.bulk_packet(),
        BULK_INTERVAL_IGNORED_BY_THE_HOST,
    )
}

pub const SERIAL_INTERFACE_COUNT: u8 = 2;

pub const SERIAL_FUNCTION_LENGTH: usize = InterfaceAssociation::LENGTH
    + 2 * Interface::LENGTH
    + HEADER_LENGTH
    + ABSTRACT_CONTROL_MANAGEMENT_LENGTH
    + UNION_LENGTH
    + CALL_MANAGEMENT_LENGTH
    + 3 * ENDPOINT_LENGTH;

pub const fn serial_function(
    control: InterfaceNumber,
    speed: BusSpeed,
) -> [u8; SERIAL_FUNCTION_LENGTH] {
    let data = control.offset_by(1);
    DescriptorBytes::empty()
        .append(
            &InterfaceAssociation {
                first: control,
                count: SERIAL_INTERFACE_COUNT,
                function: COMMUNICATIONS_ABSTRACT_CONTROL,
            }
            .bytes(),
        )
        .append(
            &Interface {
                number: control,
                alternate: AlternateSetting::Base,
                endpoints: 1,
                class: COMMUNICATIONS_ABSTRACT_CONTROL,
            }
            .bytes(),
        )
        .append(&header())
        .append(&abstract_control_management())
        .append(&union(control, data))
        .append(&call_management(data))
        .append(&endpoint(
            EndpointNumber::SERIAL_NOTIFICATION.device_to_host(),
            TransferType::Interrupt,
            NOTIFICATION_PACKET,
            speed.notification_interval(),
        ))
        .append(
            &Interface {
                number: data,
                alternate: AlternateSetting::Base,
                endpoints: 2,
                class: DATA_INTERFACE,
            }
            .bytes(),
        )
        .append(&bulk(EndpointNumber::SERIAL_DATA.host_to_device(), speed))
        .append(&bulk(EndpointNumber::SERIAL_DATA.device_to_host(), speed))
        .finish()
}
