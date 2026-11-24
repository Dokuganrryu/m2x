use super::bytes::{DescriptorBytes, DescriptorType};
use super::device::ClassTriple;

pub const AUDIO_FUNCTION: ClassTriple = ClassTriple {
    class: 0x01,
    subclass: 0x00,
    protocol: 0x20,
};
pub const AUDIO_CONTROL: ClassTriple = ClassTriple {
    class: 0x01,
    subclass: 0x01,
    protocol: 0x20,
};
pub const AUDIO_STREAMING: ClassTriple = ClassTriple {
    class: 0x01,
    subclass: 0x02,
    protocol: 0x20,
};

const AUDIO_DEVICE_CLASS_2_0: u16 = 0x0200;
const NO_STRING: u8 = 0;
const NO_ASSOCIATED_TERMINAL: u8 = 0;
const NO_CONTROLS: u8 = 0;
const NO_TERMINAL_CONTROLS: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum ControlSubtype {
    Header = 0x01,
    InputTerminal = 0x02,
    OutputTerminal = 0x03,
    ClockSource = 0x0A,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum StreamingSubtype {
    General = 0x01,
    FormatType = 0x02,
}

const ENDPOINT_GENERAL: u8 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityId(u8);

impl EntityId {
    pub const PLAYBACK_STREAM_IN: Self = Self(1);
    pub const LINE_OUT: Self = Self(3);
    pub const MICROPHONE_IN: Self = Self(4);
    pub const CAPTURE_STREAM_OUT: Self = Self(5);
    pub const INTERNAL_CLOCK: Self = Self(10);

    pub const fn value(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum TerminalType {
    UsbStreaming = 0x0101,
    Microphone = 0x0201,
    LineConnector = 0x0603,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Category {
    InputOutputBox = 0x08,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelCluster {
    pub channels: u8,
    pub spatial_locations: u32,
}

impl ChannelCluster {
    const FRONT_LEFT: u32 = 1 << 0;
    const FRONT_RIGHT: u32 = 1 << 1;

    pub const STEREO_FRONT: Self = Self {
        channels: 2,
        spatial_locations: Self::FRONT_LEFT | Self::FRONT_RIGHT,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlAccess {
    ReadOnly = 0b01,
    HostProgrammable = 0b11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClockKind {
    InternalProgrammable = 0b11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSource {
    pub id: EntityId,
    pub kind: ClockKind,
    pub synchronized_to_start_of_frame: bool,
    pub frequency: ControlAccess,
    pub validity: ControlAccess,
}

impl ClockSource {
    pub const LENGTH: usize = 8;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::ClassInterface)
            .byte(ControlSubtype::ClockSource as u8)
            .byte(self.id.value())
            .byte(self.kind as u8 | (self.synchronized_to_start_of_frame as u8) << 2)
            .byte(self.frequency as u8 | (self.validity as u8) << 2)
            .byte(NO_ASSOCIATED_TERMINAL)
            .byte(NO_STRING)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputTerminal {
    pub id: EntityId,
    pub kind: TerminalType,
    pub clock: EntityId,
    pub cluster: ChannelCluster,
}

impl InputTerminal {
    pub const LENGTH: usize = 17;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::ClassInterface)
            .byte(ControlSubtype::InputTerminal as u8)
            .byte(self.id.value())
            .word(self.kind as u16)
            .byte(NO_ASSOCIATED_TERMINAL)
            .byte(self.clock.value())
            .byte(self.cluster.channels)
            .double_word(self.cluster.spatial_locations)
            .byte(NO_STRING)
            .word(NO_TERMINAL_CONTROLS)
            .byte(NO_STRING)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputTerminal {
    pub id: EntityId,
    pub kind: TerminalType,
    pub source: EntityId,
    pub clock: EntityId,
}

impl OutputTerminal {
    pub const LENGTH: usize = 12;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::ClassInterface)
            .byte(ControlSubtype::OutputTerminal as u8)
            .byte(self.id.value())
            .word(self.kind as u16)
            .byte(NO_ASSOCIATED_TERMINAL)
            .byte(self.source.value())
            .byte(self.clock.value())
            .word(NO_TERMINAL_CONTROLS)
            .byte(NO_STRING)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlHeader {
    pub category: Category,
    pub class_block_length: usize,
}

impl ControlHeader {
    pub const LENGTH: usize = 9;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        assert!(
            self.class_block_length <= u16::MAX as usize,
            "the control block fits wTotalLength"
        );
        DescriptorBytes::descriptor(DescriptorType::ClassInterface)
            .byte(ControlSubtype::Header as u8)
            .word(AUDIO_DEVICE_CLASS_2_0)
            .byte(self.category as u8)
            .word(self.class_block_length as u16)
            .byte(NO_CONTROLS)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub subslot_bytes: u8,
    pub resolution_bits: u8,
}

impl PcmFormat {
    pub const TWENTY_FOUR_IN_THIRTY_TWO: Self = Self {
        subslot_bytes: 4,
        resolution_bits: 24,
    };

    const FORMAT_TYPE_I: u8 = 0x01;
    const PCM: u32 = 1 << 0;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamingGeneral {
    pub terminal_link: EntityId,
    pub cluster: ChannelCluster,
}

impl StreamingGeneral {
    pub const LENGTH: usize = 16;

    pub const fn bytes(self) -> [u8; Self::LENGTH] {
        DescriptorBytes::descriptor(DescriptorType::ClassInterface)
            .byte(StreamingSubtype::General as u8)
            .byte(self.terminal_link.value())
            .byte(NO_CONTROLS)
            .byte(PcmFormat::FORMAT_TYPE_I)
            .double_word(PcmFormat::PCM)
            .byte(self.cluster.channels)
            .double_word(self.cluster.spatial_locations)
            .byte(NO_STRING)
            .finish()
    }
}

pub const FORMAT_TYPE_LENGTH: usize = 6;

pub const fn type_one_format(format: PcmFormat) -> [u8; FORMAT_TYPE_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassInterface)
        .byte(StreamingSubtype::FormatType as u8)
        .byte(PcmFormat::FORMAT_TYPE_I)
        .byte(format.subslot_bytes)
        .byte(format.resolution_bits)
        .finish()
}

pub const STREAMING_ENDPOINT_LENGTH: usize = 8;
const NO_LOCK_DELAY_UNITS: u8 = 0;
const NO_LOCK_DELAY: u16 = 0;
const NOT_MAX_PACKETS_ONLY: u8 = 0;

pub const fn asynchronous_streaming_endpoint() -> [u8; STREAMING_ENDPOINT_LENGTH] {
    DescriptorBytes::descriptor(DescriptorType::ClassEndpoint)
        .byte(ENDPOINT_GENERAL)
        .byte(NOT_MAX_PACKETS_ONLY)
        .byte(NO_CONTROLS)
        .byte(NO_LOCK_DELAY_UNITS)
        .word(NO_LOCK_DELAY)
        .finish()
}
