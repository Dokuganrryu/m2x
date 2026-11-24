pub mod answer;
mod audio_class;
mod audio_function;
mod bytes;
mod configuration;
pub mod control;
mod device;
pub mod dispatch;
pub mod endpoint;
pub mod function;
pub mod handover;
mod identity;
pub mod request;
mod serial_function;
pub mod service;
mod standard;
mod text;

pub use audio_function::{AUDIO_PART, AUDIO_PART_LENGTH};
pub use configuration::{
    BusSpeed, DEVICE_QUALIFIER, DEVICE_QUALIFIER_LENGTH, configuration, other_speed_configuration,
};
pub use device::{
    BENCH_DEVICE_DESCRIPTOR, CONFIGURATION_COUNT, ClassTriple, DEVICE_DESCRIPTOR_LENGTH,
    HIGH_SPEED_CONTROL_PACKET_BYTES, MISCELLANEOUS_INTERFACE_ASSOCIATION, StringIndex, USB_2_0,
    device_descriptor,
};
pub use identity::{
    DeviceRelease, IdentityRefusal, OPEN_VENDOR_ID,
    PID_CODES_SHARED_TEST_PAIR_FOR_PRIVATE_BENCH_ONLY_NEVER_SHIPPED, ProductId, UsbIds, VendorId,
};
pub use text::{
    BENCH_STRINGS, DescriptorText, DeviceStrings, FACTORY_SERIAL_TEMPLATE, LONGEST_DESCRIPTOR_TEXT,
    SERIAL_NUMBER_DESCRIPTOR_LENGTH, SERIAL_NUMBER_LENGTH, SerialNumber, TextRefusal,
};
