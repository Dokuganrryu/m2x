#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use dsp::SampleRate;
use stream::AlternateSetting;

use super::bytes::{DescriptorBytes, DescriptorType};
use super::configuration::{
    BusSpeed, DEVICE_QUALIFIER, configuration as configuration_descriptor,
    other_speed_configuration,
};
use super::device::{DEVICE_DESCRIPTOR_LENGTH, StringIndex};
use super::function::{self, Bus, Configuration, Halt};
use super::request::{
    ConfigurationChoice, ControlRequest, DataTerminal, ENGLISH_UNITED_STATES, Endpoint, Interface,
    LineCoding, Readable, ReplyLimit, SETUP_PACKET_LENGTH, Stall, Writable,
};
use super::standard::CONFIGURATION_VALUE;
use super::text::{DeviceStrings, SERIAL_NUMBER_DESCRIPTOR_LENGTH, SerialNumber};
use crate::rate_change::{self, ClockValidity};

const SUBRANGE_COUNT_BYTES: usize = 2;
const SUBRANGE_BYTES: usize = 12;
const NO_RESOLUTION_BETWEEN_DISCRETE_RATES: u32 = 0;
pub const SAMPLING_FREQUENCY_RANGE_LENGTH: usize =
    SUBRANGE_COUNT_BYTES + SampleRate::ALL.len() * SUBRANGE_BYTES;

const fn sampling_frequency_range() -> [u8; SAMPLING_FREQUENCY_RANGE_LENGTH] {
    let mut bytes = DescriptorBytes::empty().word(SampleRate::ALL.len() as u16);
    let mut at = 0;
    while at < SampleRate::ALL.len() {
        let hertz = SampleRate::ALL[at].hz();
        bytes = bytes
            .double_word(hertz)
            .double_word(hertz)
            .double_word(NO_RESOLUTION_BETWEEN_DISCRETE_RATES);
        at += 1;
    }
    bytes.finish()
}

static SAMPLING_FREQUENCY_RANGE: [u8; SAMPLING_FREQUENCY_RANGE_LENGTH] = sampling_frequency_range();

const LANGUAGE_TABLE_LENGTH: usize = 4;
static LANGUAGE_TABLE: [u8; LANGUAGE_TABLE_LENGTH] =
    DescriptorBytes::descriptor(DescriptorType::String)
        .word(ENGLISH_UNITED_STATES)
        .finish();

const BUS_POWERED_WITHOUT_REMOTE_WAKEUP: [u8; 2] = [0, 0];
const INTERFACE_STATUS_RESERVED: [u8; 2] = [0, 0];
const ENDPOINT_RUNNING: [u8; 2] = [0, 0];
const ENDPOINT_HALTED: [u8; 2] = [1, 0];
const NOT_CONFIGURED: u8 = 0;
const CLOCK_VALID: u8 = 1;
const CLOCK_INVALID: u8 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptors<'a> {
    pub device: &'a [u8; DEVICE_DESCRIPTOR_LENGTH],
    pub strings: &'a DeviceStrings,
    pub serial: SerialNumber,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlContext<'a> {
    pub function: function::State,
    pub rate: rate_change::ClockReport,
    pub line_coding: LineCoding,
    pub descriptors: Descriptors<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Function(function::Event),
    RateChange(rate_change::Event),
    LineCoding(LineCoding),
    DataTerminal(DataTerminal),
}

const LONGEST_OWNED_REPLY: usize = SERIAL_NUMBER_DESCRIPTOR_LENGTH;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnedReply {
    bytes: [u8; LONGEST_OWNED_REPLY],
    length: usize,
}

impl OwnedReply {
    const fn of<const N: usize>(reply: [u8; N]) -> Self {
        const { assert!(N <= LONGEST_OWNED_REPLY) };
        let mut bytes = [0; LONGEST_OWNED_REPLY];
        let mut at = 0;
        while at < N {
            bytes[at] = reply[at];
            at += 1;
        }
        Self { bytes, length: N }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReplyBytes<'a> {
    Borrowed(&'a [u8]),
    Owned(OwnedReply),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply<'a> {
    bytes: ReplyBytes<'a>,
    limit: ReplyLimit,
}

impl Reply<'_> {
    pub fn bytes(&self) -> &[u8] {
        let whole = match &self.bytes {
            ReplyBytes::Borrowed(bytes) => bytes,
            ReplyBytes::Owned(owned) => owned.bytes.get(..owned.length).unwrap_or(&[]),
        };
        whole.get(..self.limit.bytes()).unwrap_or(whole)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer<'a> {
    Stall,
    Reply(Reply<'a>),
    Accept(Effect),
}

#[derive(Clone, Copy)]
enum Stage {
    Default(BusSpeed),
    Addressed(BusSpeed),
    Configured(Configuration),
}

impl Stage {
    const fn of(bus: Bus) -> Option<Self> {
        match bus {
            Bus::Powered => None,
            Bus::Default(speed) => Some(Self::Default(speed)),
            Bus::Addressed(speed) => Some(Self::Addressed(speed)),
            Bus::Configured(configuration) => Some(Self::Configured(configuration)),
        }
    }

    const fn speed(self) -> BusSpeed {
        match self {
            Self::Default(speed) | Self::Addressed(speed) => speed,
            Self::Configured(configuration) => configuration.speed(),
        }
    }

    const fn addressed(self) -> bool {
        match self {
            Self::Default(_) => false,
            Self::Addressed(_) | Self::Configured(_) => true,
        }
    }

    const fn configuration(self) -> Option<Configuration> {
        match self {
            Self::Default(_) | Self::Addressed(_) => None,
            Self::Configured(configuration) => Some(configuration),
        }
    }
}

pub fn answer<'a>(
    packet: [u8; SETUP_PACKET_LENGTH],
    data: &[u8],
    context: &ControlContext<'a>,
) -> Answer<'a> {
    let Some(stage) = context.function.active_bus().and_then(Stage::of) else {
        return Answer::Stall;
    };
    match ControlRequest::decode(packet, data, stage.speed()) {
        Ok(ControlRequest::Read(item, limit)) => match read(item, stage, context) {
            Ok(bytes) => Answer::Reply(Reply { bytes, limit }),
            Err(Stall) => Answer::Stall,
        },
        Ok(ControlRequest::Write(item)) => match accept(item, stage) {
            Ok(effect) => Answer::Accept(effect),
            Err(Stall) => Answer::Stall,
        },
        Err(Stall) => Answer::Stall,
    }
}

const fn borrowed(bytes: &[u8]) -> Result<ReplyBytes<'_>, Stall> {
    Ok(ReplyBytes::Borrowed(bytes))
}

const fn owned<const N: usize>(bytes: [u8; N]) -> Result<ReplyBytes<'static>, Stall> {
    Ok(ReplyBytes::Owned(OwnedReply::of(bytes)))
}

const fn configured(stage: Stage) -> Result<Configuration, Stall> {
    match stage.configuration() {
        Some(configuration) => Ok(configuration),
        None => Err(Stall),
    }
}

const fn audio(stage: Stage) -> Result<function::AudioAndSerial, Stall> {
    match stage.configuration() {
        Some(Configuration::HighSpeed(audio)) => Ok(audio),
        Some(Configuration::FullSpeed(_)) | None => Err(Stall),
    }
}

const fn setting_byte(setting: AlternateSetting) -> [u8; 1] {
    match setting {
        AlternateSetting::Zero => [0],
        AlternateSetting::One => [1],
    }
}

fn read<'a>(
    item: Readable,
    stage: Stage,
    context: &ControlContext<'a>,
) -> Result<ReplyBytes<'a>, Stall> {
    let descriptors = &context.descriptors;
    match item {
        Readable::DeviceDescriptor => borrowed(descriptors.device),
        Readable::ConfigurationDescriptor => borrowed(configuration_descriptor(stage.speed())),
        Readable::DeviceQualifier => borrowed(&DEVICE_QUALIFIER),
        Readable::OtherSpeedConfigurationDescriptor => {
            borrowed(other_speed_configuration(stage.speed()))
        }
        Readable::LanguageIds => borrowed(&LANGUAGE_TABLE),
        Readable::Text(StringIndex::Manufacturer) => {
            borrowed(descriptors.strings.manufacturer.descriptor())
        }
        Readable::Text(StringIndex::Product) => borrowed(descriptors.strings.product.descriptor()),
        Readable::Text(StringIndex::SerialNumber) => owned(descriptors.serial.descriptor()),
        Readable::DeviceStatus if stage.addressed() => owned(BUS_POWERED_WITHOUT_REMOTE_WAKEUP),
        Readable::ConfigurationValue if stage.addressed() => match stage.configuration() {
            Some(_) => owned([CONFIGURATION_VALUE]),
            None => owned([NOT_CONFIGURED]),
        },
        Readable::EndpointStatus(Endpoint::Control) if stage.addressed() => owned(ENDPOINT_RUNNING),
        Readable::DeviceStatus
        | Readable::ConfigurationValue
        | Readable::EndpointStatus(Endpoint::Control) => Err(Stall),
        Readable::InterfaceStatus(_) => {
            configured(stage)?;
            owned(INTERFACE_STATUS_RESERVED)
        }
        Readable::EndpointStatus(Endpoint::Serial(endpoint)) => {
            match configured(stage)?.serial().halt(endpoint) {
                Halt::Set => owned(ENDPOINT_HALTED),
                Halt::Clear => owned(ENDPOINT_RUNNING),
            }
        }
        Readable::EndpointStatus(Endpoint::PlaybackData | Endpoint::PlaybackFeedback) => {
            match audio(stage)?.playback() {
                AlternateSetting::One => owned(ENDPOINT_RUNNING),
                AlternateSetting::Zero => Err(Stall),
            }
        }
        Readable::EndpointStatus(Endpoint::CaptureData) => match audio(stage)?.capture() {
            AlternateSetting::One => owned(ENDPOINT_RUNNING),
            AlternateSetting::Zero => Err(Stall),
        },
        Readable::AlternateSetting(interface) => {
            let configuration = configured(stage)?;
            match (interface, configuration) {
                (Interface::PlaybackStream, Configuration::HighSpeed(audio)) => {
                    owned(setting_byte(audio.playback()))
                }
                (Interface::CaptureStream, Configuration::HighSpeed(audio)) => {
                    owned(setting_byte(audio.capture()))
                }
                (
                    Interface::PlaybackStream | Interface::CaptureStream,
                    Configuration::FullSpeed(_),
                ) => Err(Stall),
                (
                    Interface::AudioControl | Interface::SerialControl | Interface::SerialData,
                    Configuration::HighSpeed(_) | Configuration::FullSpeed(_),
                ) => owned(setting_byte(AlternateSetting::Zero)),
            }
        }
        Readable::SamplingFrequency => {
            audio(stage)?;
            owned(context.rate.rate.hz().to_le_bytes())
        }
        Readable::SamplingFrequencyRange => {
            audio(stage)?;
            borrowed(&SAMPLING_FREQUENCY_RANGE)
        }
        Readable::ClockValidity => {
            audio(stage)?;
            match context.rate.validity {
                ClockValidity::Valid => owned([CLOCK_VALID]),
                ClockValidity::Invalid => owned([CLOCK_INVALID]),
            }
        }
        Readable::LineCoding => {
            configured(stage)?;
            owned(context.line_coding.bytes())
        }
    }
}

fn accept(item: Writable, stage: Stage) -> Result<Effect, Stall> {
    let function = |event| Ok(Effect::Function(event));
    match item {
        Writable::Address(address) => match stage {
            Stage::Default(_) | Stage::Addressed(_) => {
                function(function::Event::AddressSet(address))
            }
            Stage::Configured(_) => Err(Stall),
        },
        Writable::Configuration(choice) if stage.addressed() => function(match choice {
            ConfigurationChoice::Unconfigured => function::Event::ConfigurationCleared,
            ConfigurationChoice::TheOnlyConfiguration => function::Event::ConfigurationSet,
        }),
        Writable::Configuration(_) => Err(Stall),
        Writable::Interface(selection) => {
            configured(stage)?;
            function(function::Event::InterfaceSelected(selection))
        }
        Writable::EndpointHalt(endpoint, halt) => {
            configured(stage)?;
            function(match halt {
                Halt::Set => function::Event::EndpointHalted(endpoint),
                Halt::Clear => function::Event::EndpointHaltCleared(endpoint),
            })
        }
        Writable::SamplingFrequency(rate) => {
            audio(stage)?;
            Ok(Effect::RateChange(rate_change::Event::SetCur(rate)))
        }
        Writable::LineCoding(coding) => {
            configured(stage)?;
            Ok(Effect::LineCoding(coding))
        }
        Writable::DataTerminal(terminal) => {
            configured(stage)?;
            Ok(Effect::DataTerminal(terminal))
        }
    }
}
