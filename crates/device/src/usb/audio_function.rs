use dsp::SampleRate;

use super::audio_class::{
    AUDIO_CONTROL, AUDIO_FUNCTION, AUDIO_STREAMING, Category, ChannelCluster, ClockKind,
    ClockSource, ControlAccess, ControlHeader, EntityId, FORMAT_TYPE_LENGTH, InputTerminal,
    OutputTerminal, PcmFormat, STREAMING_ENDPOINT_LENGTH, StreamingGeneral, TerminalType,
    asynchronous_streaming_endpoint, type_one_format,
};
use super::bytes::DescriptorBytes;
use super::standard::{
    AlternateSetting, CONFIGURATION_HEADER_LENGTH, ConfigurationView, EndpointAddress,
    EndpointNumber, Interface, InterfaceAssociation, InterfaceNumber, IsochronousEndpoint,
    IsochronousRole, Milliamps, PacketBytes, PollingInterval, bus_powered_configuration_header,
};

const HIGH_SPEED_MICROFRAMES_PER_SECOND: u32 = 8000;
pub(super) const HIGH_SPEED_FEEDBACK_BYTES: u16 = 4;
const STREAM_CLUSTER: ChannelCluster = ChannelCluster::STEREO_FRONT;
const STREAM_FORMAT: PcmFormat = PcmFormat::TWENTY_FOUR_IN_THIRTY_TWO;
pub(super) const AUDIO_SLOT_BYTES: u32 =
    STREAM_CLUSTER.channels as u32 * STREAM_FORMAT.subslot_bytes as u32;

const fn largest_packet_slots(rate: SampleRate) -> u32 {
    rate.hz() / HIGH_SPEED_MICROFRAMES_PER_SECOND + 1
}

const fn largest_stream_packet() -> PacketBytes {
    let mut largest = 0;
    let mut at = 0;
    while at < SampleRate::ALL.len() {
        let bytes = largest_packet_slots(SampleRate::ALL[at]) * AUDIO_SLOT_BYTES;
        if bytes > largest {
            largest = bytes;
        }
        at += 1;
    }
    assert!(largest <= u16::MAX as u32, "a packet fits wMaxPacketSize");
    PacketBytes::new(largest as u16)
}

pub const STREAM_PACKET: PacketBytes = largest_stream_packet();

const CLOCK: ClockSource = ClockSource {
    id: EntityId::INTERNAL_CLOCK,
    kind: ClockKind::InternalProgrammable,
    synchronized_to_start_of_frame: false,
    frequency: ControlAccess::HostProgrammable,
    validity: ControlAccess::ReadOnly,
};

const PLAYBACK_STREAM_IN: InputTerminal = InputTerminal {
    id: EntityId::PLAYBACK_STREAM_IN,
    kind: TerminalType::UsbStreaming,
    clock: EntityId::INTERNAL_CLOCK,
    cluster: STREAM_CLUSTER,
};

const LINE_OUT: OutputTerminal = OutputTerminal {
    id: EntityId::LINE_OUT,
    kind: TerminalType::LineConnector,
    source: EntityId::PLAYBACK_STREAM_IN,
    clock: EntityId::INTERNAL_CLOCK,
};

const MICROPHONE_IN: InputTerminal = InputTerminal {
    id: EntityId::MICROPHONE_IN,
    kind: TerminalType::Microphone,
    clock: EntityId::INTERNAL_CLOCK,
    cluster: STREAM_CLUSTER,
};

const CAPTURE_STREAM_OUT: OutputTerminal = OutputTerminal {
    id: EntityId::CAPTURE_STREAM_OUT,
    kind: TerminalType::UsbStreaming,
    source: EntityId::MICROPHONE_IN,
    clock: EntityId::INTERNAL_CLOCK,
};

const CONTROL_CLASS_BLOCK_LENGTH: usize = ControlHeader::LENGTH
    + ClockSource::LENGTH
    + 2 * InputTerminal::LENGTH
    + 2 * OutputTerminal::LENGTH;

const CONTROL_CLASS_BLOCK: [u8; CONTROL_CLASS_BLOCK_LENGTH] = DescriptorBytes::empty()
    .append(
        &ControlHeader {
            category: Category::InputOutputBox,
            class_block_length: CONTROL_CLASS_BLOCK_LENGTH,
        }
        .bytes(),
    )
    .append(&CLOCK.bytes())
    .append(&PLAYBACK_STREAM_IN.bytes())
    .append(&LINE_OUT.bytes())
    .append(&MICROPHONE_IN.bytes())
    .append(&CAPTURE_STREAM_OUT.bytes())
    .finish();

const fn idle_setting(number: InterfaceNumber) -> [u8; Interface::LENGTH] {
    Interface {
        number,
        alternate: AlternateSetting::Base,
        endpoints: 0,
        class: AUDIO_STREAMING,
    }
    .bytes()
}

const fn streaming_setting(number: InterfaceNumber, endpoints: u8) -> [u8; Interface::LENGTH] {
    Interface {
        number,
        alternate: AlternateSetting::Streaming,
        endpoints,
        class: AUDIO_STREAMING,
    }
    .bytes()
}

const fn stream_data_endpoint(address: EndpointAddress) -> [u8; IsochronousEndpoint::LENGTH] {
    IsochronousEndpoint {
        address,
        role: IsochronousRole::AsynchronousData,
        max_packet: STREAM_PACKET,
        interval: PollingInterval::EVERY_MICROFRAME,
    }
    .bytes()
}

const PLAYBACK_FEEDBACK: IsochronousEndpoint = IsochronousEndpoint {
    address: EndpointNumber::PLAYBACK.device_to_host(),
    role: IsochronousRole::ExplicitFeedback,
    max_packet: PacketBytes::new(HIGH_SPEED_FEEDBACK_BYTES),
    interval: PollingInterval::EVERY_8_MICROFRAMES,
};

const STREAMING_SETTING_HEADER_LENGTH: usize =
    2 * Interface::LENGTH + StreamingGeneral::LENGTH + FORMAT_TYPE_LENGTH;

const PLAYBACK_INTERFACE_LENGTH: usize = STREAMING_SETTING_HEADER_LENGTH
    + IsochronousEndpoint::LENGTH
    + STREAMING_ENDPOINT_LENGTH
    + IsochronousEndpoint::LENGTH;

const PLAYBACK_INTERFACE: [u8; PLAYBACK_INTERFACE_LENGTH] = DescriptorBytes::empty()
    .append(&idle_setting(InterfaceNumber::PLAYBACK_STREAM))
    .append(&streaming_setting(InterfaceNumber::PLAYBACK_STREAM, 2))
    .append(
        &StreamingGeneral {
            terminal_link: EntityId::PLAYBACK_STREAM_IN,
            cluster: STREAM_CLUSTER,
        }
        .bytes(),
    )
    .append(&type_one_format(STREAM_FORMAT))
    .append(&stream_data_endpoint(
        EndpointNumber::PLAYBACK.host_to_device(),
    ))
    .append(&asynchronous_streaming_endpoint())
    .append(&PLAYBACK_FEEDBACK.bytes())
    .finish();

const CAPTURE_INTERFACE_LENGTH: usize =
    STREAMING_SETTING_HEADER_LENGTH + IsochronousEndpoint::LENGTH + STREAMING_ENDPOINT_LENGTH;

const CAPTURE_INTERFACE: [u8; CAPTURE_INTERFACE_LENGTH] = DescriptorBytes::empty()
    .append(&idle_setting(InterfaceNumber::CAPTURE_STREAM))
    .append(&streaming_setting(InterfaceNumber::CAPTURE_STREAM, 1))
    .append(
        &StreamingGeneral {
            terminal_link: EntityId::CAPTURE_STREAM_OUT,
            cluster: STREAM_CLUSTER,
        }
        .bytes(),
    )
    .append(&type_one_format(STREAM_FORMAT))
    .append(&stream_data_endpoint(
        EndpointNumber::CAPTURE.device_to_host(),
    ))
    .append(&asynchronous_streaming_endpoint())
    .finish();

pub const AUDIO_INTERFACE_COUNT: u8 = 3;

pub const AUDIO_FUNCTION_LENGTH: usize = InterfaceAssociation::LENGTH
    + Interface::LENGTH
    + CONTROL_CLASS_BLOCK_LENGTH
    + PLAYBACK_INTERFACE_LENGTH
    + CAPTURE_INTERFACE_LENGTH;

pub const AUDIO_FUNCTION_DESCRIPTORS: [u8; AUDIO_FUNCTION_LENGTH] = DescriptorBytes::empty()
    .append(
        &InterfaceAssociation {
            first: InterfaceNumber::AUDIO_CONTROL,
            count: AUDIO_INTERFACE_COUNT,
            function: AUDIO_FUNCTION,
        }
        .bytes(),
    )
    .append(
        &Interface {
            number: InterfaceNumber::AUDIO_CONTROL,
            alternate: AlternateSetting::Base,
            endpoints: 0,
            class: AUDIO_CONTROL,
        }
        .bytes(),
    )
    .append(&CONTROL_CLASS_BLOCK)
    .append(&PLAYBACK_INTERFACE)
    .append(&CAPTURE_INTERFACE)
    .finish();

pub const AUDIO_PART_LENGTH: usize = CONFIGURATION_HEADER_LENGTH + AUDIO_FUNCTION_LENGTH;

pub static AUDIO_PART: [u8; AUDIO_PART_LENGTH] = DescriptorBytes::empty()
    .append(&bus_powered_configuration_header(
        ConfigurationView::AtCurrentSpeed,
        AUDIO_PART_LENGTH,
        AUDIO_INTERFACE_COUNT,
        Milliamps::BUS_POWERED_HIGH_POWER,
    ))
    .append(&AUDIO_FUNCTION_DESCRIPTORS)
    .finish();
