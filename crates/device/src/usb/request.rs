#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use dsp::SampleRate;
use stream::AlternateSetting;

use super::audio_class::EntityId;
use super::audio_function::AUDIO_INTERFACE_COUNT;
use super::bytes::DescriptorType;
use super::configuration::BusSpeed;
use super::device::StringIndex;
use super::function::Halt;
use super::standard::{CONFIGURATION_VALUE, EndpointNumber, InterfaceNumber};

pub const SETUP_PACKET_LENGTH: usize = 8;
pub const ENGLISH_UNITED_STATES: u16 = 0x0409;
pub const LINE_CODING_LENGTH: usize = 7;
pub const SAMPLING_FREQUENCY_LENGTH: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stall;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplyLimit(u16);

impl ReplyLimit {
    pub const fn bytes(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceAddress(u8);

impl DeviceAddress {
    const HIGHEST: u16 = 127;

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Default,
    Assigned(DeviceAddress),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interface {
    AudioControl,
    PlaybackStream,
    CaptureStream,
    SerialControl,
    SerialData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterfaceSelection {
    AudioControl,
    Playback(AlternateSetting),
    Capture(AlternateSetting),
    SerialControl,
    SerialData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HaltableEndpoint {
    SerialNotification,
    SerialReceive,
    SerialTransmit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Control,
    PlaybackData,
    PlaybackFeedback,
    CaptureData,
    Serial(HaltableEndpoint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitsPerSecond(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopBits {
    One,
    OneAndAHalf,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
    Mark,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataBits {
    Five,
    Six,
    Seven,
    Eight,
    Sixteen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCoding {
    pub rate: BitsPerSecond,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub data_bits: DataBits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataTerminal {
    Present,
    Absent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readable {
    DeviceDescriptor,
    ConfigurationDescriptor,
    DeviceQualifier,
    OtherSpeedConfigurationDescriptor,
    LanguageIds,
    Text(StringIndex),
    DeviceStatus,
    InterfaceStatus(Interface),
    EndpointStatus(Endpoint),
    ConfigurationValue,
    AlternateSetting(Interface),
    SamplingFrequency,
    SamplingFrequencyRange,
    ClockValidity,
    LineCoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writable {
    Address(Address),
    Configuration(ConfigurationChoice),
    Interface(InterfaceSelection),
    EndpointHalt(HaltableEndpoint, Halt),
    SamplingFrequency(SampleRate),
    LineCoding(LineCoding),
    DataTerminal(DataTerminal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationChoice {
    Unconfigured,
    TheOnlyConfiguration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlRequest {
    Read(Readable, ReplyLimit),
    Write(Writable),
}

const DEVICE_TO_HOST: u8 = 0x80;
const TYPE_SHIFT: u32 = 5;
const TYPE_MASK: u8 = 0b11;

#[derive(Clone, Copy)]
enum RequestKind {
    Standard,
    Class,
    Vendor,
    Reserved,
}

const STANDARD_DEVICE_OUT: u8 = 0x00;
const STANDARD_INTERFACE_OUT: u8 = 0x01;
const STANDARD_ENDPOINT_OUT: u8 = 0x02;
const STANDARD_DEVICE_IN: u8 = 0x80;
const STANDARD_INTERFACE_IN: u8 = 0x81;
const STANDARD_ENDPOINT_IN: u8 = 0x82;
const CLASS_INTERFACE_OUT: u8 = 0x21;
const CLASS_INTERFACE_IN: u8 = 0xA1;

const GET_STATUS: u8 = 0;
const CLEAR_FEATURE: u8 = 1;
const SET_FEATURE: u8 = 3;
const SET_ADDRESS: u8 = 5;
const GET_DESCRIPTOR: u8 = 6;
const GET_CONFIGURATION: u8 = 8;
const SET_CONFIGURATION: u8 = 9;
const GET_INTERFACE: u8 = 10;
const SET_INTERFACE: u8 = 11;

const ENDPOINT_HALT_FEATURE: u16 = 0;
const STATUS_LENGTH: u16 = 2;
const ONE_BYTE: u16 = 1;
const NO_DATA: u16 = 0;
const UNCONFIGURED_VALUE: u16 = 0;
const FIRST_DESCRIPTOR: u8 = 0;
const LANGUAGE_TABLE: u8 = 0;
const NO_LANGUAGE: u16 = 0;

const AUDIO_CURRENT: u8 = 0x01;
const AUDIO_RANGE: u8 = 0x02;
const SAMPLING_FREQUENCY_CONTROL: u8 = 0x01;
const CLOCK_VALIDITY_CONTROL: u8 = 0x02;
const MASTER_CHANNEL: u8 = 0;
const THE_INTERFACE_ITSELF: u8 = 0;

const SET_LINE_CODING: u8 = 0x20;
const GET_LINE_CODING: u8 = 0x21;
const SET_CONTROL_LINE_STATE: u8 = 0x22;
const DATA_TERMINAL_PRESENT: u16 = 1 << 0;
const CARRIER_ACTIVATED: u16 = 1 << 1;

#[derive(Clone, Copy)]
struct Setup {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
}

impl Setup {
    const fn parse(packet: [u8; SETUP_PACKET_LENGTH]) -> Self {
        let [
            request_type,
            request,
            value_low,
            value_high,
            index_low,
            index_high,
            length_low,
            length_high,
        ] = packet;
        Self {
            request_type,
            request,
            value: u16::from_le_bytes([value_low, value_high]),
            index: u16::from_le_bytes([index_low, index_high]),
            length: u16::from_le_bytes([length_low, length_high]),
        }
    }

    const fn kind(self) -> RequestKind {
        match (self.request_type >> TYPE_SHIFT) & TYPE_MASK {
            0 => RequestKind::Standard,
            1 => RequestKind::Class,
            2 => RequestKind::Vendor,
            _ => RequestKind::Reserved,
        }
    }

    const fn reads(self) -> bool {
        self.request_type & DEVICE_TO_HOST != 0
    }

    const fn limit(self) -> ReplyLimit {
        ReplyLimit(self.length)
    }

    const fn read(self, item: Readable) -> Result<ControlRequest, Stall> {
        Ok(ControlRequest::Read(item, self.limit()))
    }

    const fn data_stage_matches(self, data: &[u8]) -> bool {
        if self.reads() {
            data.is_empty()
        } else {
            data.len() == self.length as usize
        }
    }
}

const fn write(item: Writable) -> Result<ControlRequest, Stall> {
    Ok(ControlRequest::Write(item))
}

const fn found<T: Copy>(value: Option<T>) -> Result<T, Stall> {
    match value {
        Some(value) => Ok(value),
        None => Err(Stall),
    }
}

impl Interface {
    const fn serial_control_number(speed: BusSpeed) -> InterfaceNumber {
        match speed {
            BusSpeed::High => InterfaceNumber::FIRST.offset_by(AUDIO_INTERFACE_COUNT),
            BusSpeed::Full => InterfaceNumber::FIRST,
        }
    }

    pub const fn number(self, speed: BusSpeed) -> Option<u8> {
        let serial_control = Self::serial_control_number(speed);
        match (self, speed) {
            (Self::AudioControl, BusSpeed::High) => Some(InterfaceNumber::AUDIO_CONTROL.value()),
            (Self::PlaybackStream, BusSpeed::High) => {
                Some(InterfaceNumber::PLAYBACK_STREAM.value())
            }
            (Self::CaptureStream, BusSpeed::High) => Some(InterfaceNumber::CAPTURE_STREAM.value()),
            (Self::AudioControl | Self::PlaybackStream | Self::CaptureStream, BusSpeed::Full) => {
                None
            }
            (Self::SerialControl, BusSpeed::High | BusSpeed::Full) => Some(serial_control.value()),
            (Self::SerialData, BusSpeed::High | BusSpeed::Full) => {
                Some(serial_control.offset_by(1).value())
            }
        }
    }

    const ALL: [Self; 5] = [
        Self::AudioControl,
        Self::PlaybackStream,
        Self::CaptureStream,
        Self::SerialControl,
        Self::SerialData,
    ];

    fn at(index: u16, speed: BusSpeed) -> Option<Self> {
        let [number, reserved] = index.to_le_bytes();
        if reserved != 0 {
            return None;
        }
        Self::numbered(number, speed)
    }

    fn numbered(number: u8, speed: BusSpeed) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|interface| interface.number(speed) == Some(number))
    }
}

impl InterfaceSelection {
    fn new(interface: Interface, setting: u16) -> Option<Self> {
        let alternate = match setting {
            0 => AlternateSetting::Zero,
            1 => AlternateSetting::One,
            _ => return None,
        };
        match (interface, alternate) {
            (Interface::PlaybackStream, _) => Some(Self::Playback(alternate)),
            (Interface::CaptureStream, _) => Some(Self::Capture(alternate)),
            (Interface::AudioControl, AlternateSetting::Zero) => Some(Self::AudioControl),
            (Interface::SerialControl, AlternateSetting::Zero) => Some(Self::SerialControl),
            (Interface::SerialData, AlternateSetting::Zero) => Some(Self::SerialData),
            (
                Interface::AudioControl | Interface::SerialControl | Interface::SerialData,
                AlternateSetting::One,
            ) => None,
        }
    }
}

impl HaltableEndpoint {
    pub const fn address(self) -> u8 {
        match self {
            Self::SerialNotification => {
                EndpointNumber::SERIAL_NOTIFICATION.device_to_host().value()
            }
            Self::SerialReceive => EndpointNumber::SERIAL_DATA.host_to_device().value(),
            Self::SerialTransmit => EndpointNumber::SERIAL_DATA.device_to_host().value(),
        }
    }
}

impl Endpoint {
    const CONTROL_OUT: u8 = 0x00;
    const CONTROL_IN: u8 = 0x80;

    const ALL: [Self; 7] = [
        Self::PlaybackData,
        Self::PlaybackFeedback,
        Self::CaptureData,
        Self::Serial(HaltableEndpoint::SerialNotification),
        Self::Serial(HaltableEndpoint::SerialReceive),
        Self::Serial(HaltableEndpoint::SerialTransmit),
        Self::Control,
    ];

    pub const fn address(self, speed: BusSpeed) -> Option<u8> {
        match (self, speed) {
            (Self::Control, BusSpeed::High | BusSpeed::Full) => Some(Self::CONTROL_OUT),
            (Self::PlaybackData, BusSpeed::High) => {
                Some(EndpointNumber::PLAYBACK.host_to_device().value())
            }
            (Self::PlaybackFeedback, BusSpeed::High) => {
                Some(EndpointNumber::PLAYBACK.device_to_host().value())
            }
            (Self::CaptureData, BusSpeed::High) => {
                Some(EndpointNumber::CAPTURE.device_to_host().value())
            }
            (Self::PlaybackData | Self::PlaybackFeedback | Self::CaptureData, BusSpeed::Full) => {
                None
            }
            (Self::Serial(endpoint), BusSpeed::High | BusSpeed::Full) => Some(endpoint.address()),
        }
    }

    fn at(index: u16, speed: BusSpeed) -> Option<Self> {
        let [address, reserved] = index.to_le_bytes();
        if reserved != 0 {
            return None;
        }
        if address == Self::CONTROL_IN {
            return Some(Self::Control);
        }
        Self::ALL
            .into_iter()
            .find(|endpoint| endpoint.address(speed) == Some(address))
    }
}

impl LineCoding {
    pub const CONSOLE_DEFAULT: Self = Self {
        rate: BitsPerSecond(115_200),
        stop_bits: StopBits::One,
        parity: Parity::None,
        data_bits: DataBits::Eight,
    };

    fn parse(data: &[u8]) -> Option<Self> {
        let [r0, r1, r2, r3, stop_bits, parity, data_bits] =
            <[u8; LINE_CODING_LENGTH]>::try_from(data).ok()?;
        Some(Self {
            rate: BitsPerSecond(u32::from_le_bytes([r0, r1, r2, r3])),
            stop_bits: match stop_bits {
                0 => StopBits::One,
                1 => StopBits::OneAndAHalf,
                2 => StopBits::Two,
                _ => return None,
            },
            parity: match parity {
                0 => Parity::None,
                1 => Parity::Odd,
                2 => Parity::Even,
                3 => Parity::Mark,
                4 => Parity::Space,
                _ => return None,
            },
            data_bits: match data_bits {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                8 => DataBits::Eight,
                16 => DataBits::Sixteen,
                _ => return None,
            },
        })
    }

    pub const fn bytes(self) -> [u8; LINE_CODING_LENGTH] {
        let [r0, r1, r2, r3] = self.rate.0.to_le_bytes();
        let stop_bits = match self.stop_bits {
            StopBits::One => 0,
            StopBits::OneAndAHalf => 1,
            StopBits::Two => 2,
        };
        let parity = match self.parity {
            Parity::None => 0,
            Parity::Odd => 1,
            Parity::Even => 2,
            Parity::Mark => 3,
            Parity::Space => 4,
        };
        let data_bits = match self.data_bits {
            DataBits::Five => 5,
            DataBits::Six => 6,
            DataBits::Seven => 7,
            DataBits::Eight => 8,
            DataBits::Sixteen => 16,
        };
        [r0, r1, r2, r3, stop_bits, parity, data_bits]
    }
}

impl DataTerminal {
    const fn from_control_signals(bitmap: u16) -> Option<Self> {
        if bitmap & !(DATA_TERMINAL_PRESENT | CARRIER_ACTIVATED) != 0 {
            None
        } else if bitmap & DATA_TERMINAL_PRESENT != 0 {
            Some(Self::Present)
        } else {
            Some(Self::Absent)
        }
    }
}

pub fn closest_supported_rate(hertz: u32) -> SampleRate {
    let mut closest = SampleRate::ALL[0];
    for rate in SampleRate::ALL {
        if rate.hz().abs_diff(hertz) < closest.hz().abs_diff(hertz) {
            closest = rate;
        }
    }
    closest
}

impl ControlRequest {
    pub fn decode(
        packet: [u8; SETUP_PACKET_LENGTH],
        data: &[u8],
        speed: BusSpeed,
    ) -> Result<Self, Stall> {
        let setup = Setup::parse(packet);
        if !setup.data_stage_matches(data) {
            return Err(Stall);
        }
        match setup.kind() {
            RequestKind::Standard => standard(setup, speed),
            RequestKind::Class => class(setup, data, speed),
            RequestKind::Vendor | RequestKind::Reserved => Err(Stall),
        }
    }
}

fn standard(setup: Setup, speed: BusSpeed) -> Result<ControlRequest, Stall> {
    let Setup {
        request_type,
        request,
        value,
        index,
        length,
    } = setup;
    match (request_type, request) {
        (STANDARD_DEVICE_IN, GET_STATUS) if value == 0 && index == 0 && length == STATUS_LENGTH => {
            setup.read(Readable::DeviceStatus)
        }
        (STANDARD_INTERFACE_IN, GET_STATUS) if value == 0 && length == STATUS_LENGTH => setup.read(
            Readable::InterfaceStatus(found(Interface::at(index, speed))?),
        ),
        (STANDARD_ENDPOINT_IN, GET_STATUS) if value == 0 && length == STATUS_LENGTH => {
            setup.read(Readable::EndpointStatus(found(Endpoint::at(index, speed))?))
        }
        (STANDARD_ENDPOINT_OUT, CLEAR_FEATURE | SET_FEATURE)
            if value == ENDPOINT_HALT_FEATURE && length == NO_DATA =>
        {
            let Endpoint::Serial(endpoint) = found(Endpoint::at(index, speed))? else {
                return Err(Stall);
            };
            let halt = if request == SET_FEATURE {
                Halt::Set
            } else {
                Halt::Clear
            };
            write(Writable::EndpointHalt(endpoint, halt))
        }
        (STANDARD_DEVICE_OUT, SET_ADDRESS) if index == 0 && length == NO_DATA => {
            match u8::try_from(value) {
                Ok(0) => write(Writable::Address(Address::Default)),
                Ok(address) if u16::from(address) <= DeviceAddress::HIGHEST => {
                    write(Writable::Address(Address::Assigned(DeviceAddress(address))))
                }
                Ok(_) | Err(_) => Err(Stall),
            }
        }
        (STANDARD_DEVICE_IN, GET_DESCRIPTOR) => setup.read(found(descriptor(value, index))?),
        (STANDARD_DEVICE_IN, GET_CONFIGURATION)
            if value == 0 && index == 0 && length == ONE_BYTE =>
        {
            setup.read(Readable::ConfigurationValue)
        }
        (STANDARD_DEVICE_OUT, SET_CONFIGURATION) if index == 0 && length == NO_DATA => {
            match value {
                UNCONFIGURED_VALUE => {
                    write(Writable::Configuration(ConfigurationChoice::Unconfigured))
                }
                chosen if chosen == u16::from(CONFIGURATION_VALUE) => write(
                    Writable::Configuration(ConfigurationChoice::TheOnlyConfiguration),
                ),
                _ => Err(Stall),
            }
        }
        (STANDARD_INTERFACE_IN, GET_INTERFACE) if value == 0 && length == ONE_BYTE => setup.read(
            Readable::AlternateSetting(found(Interface::at(index, speed))?),
        ),
        (STANDARD_INTERFACE_OUT, SET_INTERFACE) if length == NO_DATA => {
            let interface = found(Interface::at(index, speed))?;
            write(Writable::Interface(found(InterfaceSelection::new(
                interface, value,
            ))?))
        }
        _ => Err(Stall),
    }
}

fn descriptor(value: u16, language: u16) -> Option<Readable> {
    let [descriptor_index, descriptor_type] = value.to_le_bytes();
    let fixed = |readable| {
        (descriptor_index == FIRST_DESCRIPTOR && language == NO_LANGUAGE).then_some(readable)
    };
    match descriptor_type {
        t if t == DescriptorType::Device as u8 => fixed(Readable::DeviceDescriptor),
        t if t == DescriptorType::Configuration as u8 => fixed(Readable::ConfigurationDescriptor),
        t if t == DescriptorType::DeviceQualifier as u8 => fixed(Readable::DeviceQualifier),
        t if t == DescriptorType::OtherSpeedConfiguration as u8 => {
            fixed(Readable::OtherSpeedConfigurationDescriptor)
        }
        t if t == DescriptorType::String as u8 => text(descriptor_index, language),
        _ => None,
    }
}

fn text(index: u8, language: u16) -> Option<Readable> {
    if index == LANGUAGE_TABLE {
        return (language == NO_LANGUAGE).then_some(Readable::LanguageIds);
    }
    if language != ENGLISH_UNITED_STATES {
        return None;
    }
    [
        StringIndex::Manufacturer,
        StringIndex::Product,
        StringIndex::SerialNumber,
    ]
    .into_iter()
    .find(|known| *known as u8 == index)
    .map(Readable::Text)
}

fn class(setup: Setup, data: &[u8], speed: BusSpeed) -> Result<ControlRequest, Stall> {
    if setup.request_type != CLASS_INTERFACE_IN && setup.request_type != CLASS_INTERFACE_OUT {
        return Err(Stall);
    }
    let [number, entity] = setup.index.to_le_bytes();
    match found(Interface::numbered(number, speed))? {
        Interface::AudioControl if entity == EntityId::INTERNAL_CLOCK.value() => {
            clock_source(setup, data)
        }
        Interface::SerialControl if entity == THE_INTERFACE_ITSELF => serial(setup, data),
        Interface::AudioControl
        | Interface::SerialControl
        | Interface::PlaybackStream
        | Interface::CaptureStream
        | Interface::SerialData => Err(Stall),
    }
}

fn clock_source(setup: Setup, data: &[u8]) -> Result<ControlRequest, Stall> {
    let [channel, selector] = setup.value.to_le_bytes();
    if channel != MASTER_CHANNEL {
        return Err(Stall);
    }
    match (setup.request_type, setup.request, selector) {
        (CLASS_INTERFACE_IN, AUDIO_CURRENT, SAMPLING_FREQUENCY_CONTROL) => {
            setup.read(Readable::SamplingFrequency)
        }
        (CLASS_INTERFACE_IN, AUDIO_RANGE, SAMPLING_FREQUENCY_CONTROL) => {
            setup.read(Readable::SamplingFrequencyRange)
        }
        (CLASS_INTERFACE_IN, AUDIO_CURRENT, CLOCK_VALIDITY_CONTROL) => {
            setup.read(Readable::ClockValidity)
        }
        (CLASS_INTERFACE_OUT, AUDIO_CURRENT, SAMPLING_FREQUENCY_CONTROL) => {
            let hertz = <[u8; SAMPLING_FREQUENCY_LENGTH]>::try_from(data).map_err(|_| Stall)?;
            write(Writable::SamplingFrequency(closest_supported_rate(
                u32::from_le_bytes(hertz),
            )))
        }
        _ => Err(Stall),
    }
}

fn serial(setup: Setup, data: &[u8]) -> Result<ControlRequest, Stall> {
    match (setup.request_type, setup.request) {
        (CLASS_INTERFACE_OUT, SET_LINE_CODING) if setup.value == 0 => {
            write(Writable::LineCoding(found(LineCoding::parse(data))?))
        }
        (CLASS_INTERFACE_IN, GET_LINE_CODING) if setup.value == 0 => {
            setup.read(Readable::LineCoding)
        }
        (CLASS_INTERFACE_OUT, SET_CONTROL_LINE_STATE) if setup.length == NO_DATA => write(
            Writable::DataTerminal(found(DataTerminal::from_control_signals(setup.value))?),
        ),
        _ => Err(Stall),
    }
}
