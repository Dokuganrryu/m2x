use crate::usb::{FACTORY_SERIAL_TEMPLATE, SERIAL_NUMBER_LENGTH, SerialNumber};

pub const FACTORY_MAGIC: u32 = 0x5554_4F4D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactoryRead {
    Unreadable,
    Loaded {
        magic: u32,
        serial: [u8; SERIAL_NUMBER_LENGTH],
    },
}

impl FactoryRead {
    pub const fn serial_number(self) -> SerialNumber {
        match self {
            FactoryRead::Loaded {
                magic: FACTORY_MAGIC,
                serial,
            } => SerialNumber::from_factory_copy(serial),
            FactoryRead::Loaded { .. } | FactoryRead::Unreadable => FACTORY_SERIAL_TEMPLATE,
        }
    }
}
