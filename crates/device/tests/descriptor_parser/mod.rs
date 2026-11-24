#![allow(dead_code)]

pub const FEEDBACK_NOTE: &str = include_str!("../fixtures/uac2-descriptors.md");
pub const CDC_NOTE: &str = include_str!("../fixtures/cdc-acm-descriptors.md");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Device,
    DeviceQualifier,
    Configuration,
    OtherSpeedConfiguration,
    InterfaceAssociation,
    Interface,
    Endpoint,
    ControlHeader,
    ClockSource,
    InputTerminal,
    OutputTerminal,
    StreamingGeneral,
    FormatType,
    StreamingEndpoint,
    CdcHeader,
    AbstractControlManagement,
    Union,
    CallManagement,
    Unrecognised { descriptor_type: u8, subtype: u8 },
}

#[derive(Clone, Debug)]
pub struct Descriptor {
    pub kind: Kind,
    pub offset: usize,
    pub interface: Option<(u8, u8)>,
    pub fields: Vec<(&'static str, u32)>,
}

impl Descriptor {
    pub fn field(&self, name: &str) -> u32 {
        self.fields
            .iter()
            .find(|(field, _)| *field == name)
            .map(|&(_, value)| value)
            .unwrap_or_else(|| panic!("{:?} at {} has no field {name}", self.kind, self.offset))
    }
}

const DEVICE: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bcdUSB", 2),
    ("bDeviceClass", 1),
    ("bDeviceSubClass", 1),
    ("bDeviceProtocol", 1),
    ("bMaxPacketSize0", 1),
    ("idVendor", 2),
    ("idProduct", 2),
    ("bcdDevice", 2),
    ("iManufacturer", 1),
    ("iProduct", 1),
    ("iSerialNumber", 1),
    ("bNumConfigurations", 1),
];

const DEVICE_QUALIFIER: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bcdUSB", 2),
    ("bDeviceClass", 1),
    ("bDeviceSubClass", 1),
    ("bDeviceProtocol", 1),
    ("bMaxPacketSize0", 1),
    ("bNumConfigurations", 1),
    ("bReserved", 1),
];

const CONFIGURATION: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("wTotalLength", 2),
    ("bNumInterfaces", 1),
    ("bConfigurationValue", 1),
    ("iConfiguration", 1),
    ("bmAttributes", 1),
    ("bMaxPower", 1),
];

const INTERFACE_ASSOCIATION: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bFirstInterface", 1),
    ("bInterfaceCount", 1),
    ("bFunctionClass", 1),
    ("bFunctionSubClass", 1),
    ("bFunctionProtocol", 1),
    ("iFunction", 1),
];

const INTERFACE: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bInterfaceNumber", 1),
    ("bAlternateSetting", 1),
    ("bNumEndpoints", 1),
    ("bInterfaceClass", 1),
    ("bInterfaceSubClass", 1),
    ("bInterfaceProtocol", 1),
    ("iInterface", 1),
];

const ENDPOINT: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bEndpointAddress", 1),
    ("bmAttributes", 1),
    ("wMaxPacketSize", 2),
    ("bInterval", 1),
];

const CONTROL_HEADER: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bcdADC", 2),
    ("bCategory", 1),
    ("wTotalLength", 2),
    ("bmControls", 1),
];

const CLOCK_SOURCE: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bClockID", 1),
    ("bmAttributes", 1),
    ("bmControls", 1),
    ("bAssocTerminal", 1),
    ("iClockSource", 1),
];

const INPUT_TERMINAL: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bTerminalID", 1),
    ("wTerminalType", 2),
    ("bAssocTerminal", 1),
    ("bCSourceID", 1),
    ("bNrChannels", 1),
    ("bmChannelConfig", 4),
    ("iChannelNames", 1),
    ("bmControls", 2),
    ("iTerminal", 1),
];

const OUTPUT_TERMINAL: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bTerminalID", 1),
    ("wTerminalType", 2),
    ("bAssocTerminal", 1),
    ("bSourceID", 1),
    ("bCSourceID", 1),
    ("bmControls", 2),
    ("iTerminal", 1),
];

const STREAMING_GENERAL: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bTerminalLink", 1),
    ("bmControls", 1),
    ("bFormatType", 1),
    ("bmFormats", 4),
    ("bNrChannels", 1),
    ("bmChannelConfig", 4),
    ("iChannelNames", 1),
];

const FORMAT_TYPE: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bFormatType", 1),
    ("bSubslotSize", 1),
    ("bBitResolution", 1),
];

const STREAMING_ENDPOINT: &[(&str, usize)] = &[
    ("bLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bmAttributes", 1),
    ("bmControls", 1),
    ("bLockDelayUnits", 1),
    ("wLockDelay", 2),
];

const CDC_HEADER: &[(&str, usize)] = &[
    ("bFunctionLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bcdCDC", 2),
];

const ABSTRACT_CONTROL_MANAGEMENT: &[(&str, usize)] = &[
    ("bFunctionLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bmCapabilities", 1),
];

const UNION_WITH_ONE_SUBORDINATE: &[(&str, usize)] = &[
    ("bFunctionLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bControlInterface", 1),
    ("bSubordinateInterface0", 1),
];

const CALL_MANAGEMENT: &[(&str, usize)] = &[
    ("bFunctionLength", 1),
    ("bDescriptorType", 1),
    ("bDescriptorSubtype", 1),
    ("bmCapabilities", 1),
    ("bDataInterface", 1),
];

const AUDIO_CONTROL: (u8, u8) = (0x01, 0x01);
const AUDIO_STREAMING: (u8, u8) = (0x01, 0x02);
const ABSTRACT_CONTROL_MODEL: (u8, u8) = (0x02, 0x02);

fn layout(kind: Kind) -> &'static [(&'static str, usize)] {
    match kind {
        Kind::Device => DEVICE,
        Kind::DeviceQualifier => DEVICE_QUALIFIER,
        Kind::Configuration | Kind::OtherSpeedConfiguration => CONFIGURATION,
        Kind::InterfaceAssociation => INTERFACE_ASSOCIATION,
        Kind::Interface => INTERFACE,
        Kind::Endpoint => ENDPOINT,
        Kind::ControlHeader => CONTROL_HEADER,
        Kind::ClockSource => CLOCK_SOURCE,
        Kind::InputTerminal => INPUT_TERMINAL,
        Kind::OutputTerminal => OUTPUT_TERMINAL,
        Kind::StreamingGeneral => STREAMING_GENERAL,
        Kind::FormatType => FORMAT_TYPE,
        Kind::StreamingEndpoint => STREAMING_ENDPOINT,
        Kind::CdcHeader => CDC_HEADER,
        Kind::AbstractControlManagement => ABSTRACT_CONTROL_MANAGEMENT,
        Kind::Union => UNION_WITH_ONE_SUBORDINATE,
        Kind::CallManagement => CALL_MANAGEMENT,
        Kind::Unrecognised { .. } => &[("bLength", 1), ("bDescriptorType", 1)],
    }
}

fn classify(descriptor_type: u8, subtype: u8, interface_class: Option<(u8, u8)>) -> Kind {
    match (descriptor_type, interface_class, subtype) {
        (0x01, _, _) => Kind::Device,
        (0x06, _, _) => Kind::DeviceQualifier,
        (0x02, _, _) => Kind::Configuration,
        (0x07, _, _) => Kind::OtherSpeedConfiguration,
        (0x0B, _, _) => Kind::InterfaceAssociation,
        (0x04, _, _) => Kind::Interface,
        (0x05, _, _) => Kind::Endpoint,
        (0x24, Some(AUDIO_CONTROL), 0x01) => Kind::ControlHeader,
        (0x24, Some(AUDIO_CONTROL), 0x02) => Kind::InputTerminal,
        (0x24, Some(AUDIO_CONTROL), 0x03) => Kind::OutputTerminal,
        (0x24, Some(AUDIO_CONTROL), 0x0A) => Kind::ClockSource,
        (0x24, Some(AUDIO_STREAMING), 0x01) => Kind::StreamingGeneral,
        (0x24, Some(AUDIO_STREAMING), 0x02) => Kind::FormatType,
        (0x25, Some(AUDIO_STREAMING), 0x01) => Kind::StreamingEndpoint,
        (0x24, Some(ABSTRACT_CONTROL_MODEL), 0x00) => Kind::CdcHeader,
        (0x24, Some(ABSTRACT_CONTROL_MODEL), 0x01) => Kind::CallManagement,
        (0x24, Some(ABSTRACT_CONTROL_MODEL), 0x02) => Kind::AbstractControlManagement,
        (0x24, Some(ABSTRACT_CONTROL_MODEL), 0x06) => Kind::Union,
        _ => Kind::Unrecognised {
            descriptor_type,
            subtype,
        },
    }
}

fn little_endian(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .rev()
        .fold(0, |value, &byte| (value << 8) | u32::from(byte))
}

pub fn parse(bytes: &[u8]) -> Vec<Descriptor> {
    let mut descriptors = Vec::new();
    let mut offset = 0;
    let mut interface = None;
    let mut interface_class = None;
    while offset < bytes.len() {
        let length = usize::from(bytes[offset]);
        assert!(length >= 2, "descriptor at {offset} has bLength {length}");
        let body = bytes
            .get(offset..offset + length)
            .unwrap_or_else(|| panic!("descriptor at {offset} runs past the end"));
        let subtype = body.get(2).copied().unwrap_or(0);
        let kind = classify(body[1], subtype, interface_class);
        if kind == Kind::Interface {
            interface = Some((body[2], body[3]));
            interface_class = Some((body[5], body[6]));
        }
        let fields_layout = layout(kind);
        if !matches!(kind, Kind::Unrecognised { .. }) {
            let width: usize = fields_layout.iter().map(|&(_, width)| width).sum();
            assert_eq!(width, length, "{kind:?} at {offset} has bLength {length}");
        }
        let mut at = 0;
        let fields = fields_layout
            .iter()
            .map(|&(name, width)| {
                let value = little_endian(&body[at..at + width]);
                at += width;
                (name, value)
            })
            .collect();
        descriptors.push(Descriptor {
            kind,
            offset,
            interface,
            fields,
        });
        offset += length;
    }
    descriptors
}

pub fn table_after(heading: &str) -> Vec<Vec<&'static str>> {
    table_in(FEEDBACK_NOTE, heading)
}

pub fn cdc_table_after(heading: &str) -> Vec<Vec<&'static str>> {
    table_in(CDC_NOTE, heading)
}

fn table_in(note: &'static str, heading: &str) -> Vec<Vec<&'static str>> {
    let start = note
        .find(heading)
        .unwrap_or_else(|| panic!("the note has the text {heading}"));
    note[start..]
        .lines()
        .skip_while(|line| !line.starts_with('|'))
        .take_while(|line| line.starts_with('|'))
        .skip(2)
        .map(|line| {
            line.trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn number(token: &str) -> Option<u32> {
    match token.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => token.parse().ok(),
    }
}

pub fn single_values(cell: &str) -> Option<Vec<u32>> {
    let lower = cell.to_ascii_lowercase();
    if [" or ", " to ", "index", "sum of", "not "]
        .iter()
        .any(|word| lower.contains(word))
    {
        return None;
    }
    let quoted: Vec<u32> = cell
        .split('`')
        .skip(1)
        .step_by(2)
        .filter_map(number)
        .collect();
    if !quoted.is_empty() {
        return Some(quoted);
    }
    cell.split_whitespace()
        .next()
        .and_then(number)
        .map(|n| vec![n])
}

pub fn field_names(cell: &str) -> Vec<String> {
    let mut parts = cell.split(',').map(str::trim);
    let first = parts.next().expect("a field name").to_string();
    let stem = first.strip_suffix("Class").unwrap_or(&first).to_string();
    core::iter::once(first.clone())
        .chain(parts.map(|part| format!("{stem}{part}")))
        .collect()
}

pub struct Expectation {
    pub field: String,
    pub value: u32,
}

pub fn expectations(rows: &[Vec<&str>], value_column: usize) -> Vec<Expectation> {
    rows.iter()
        .filter_map(|row| {
            let values = single_values(row[value_column])?;
            let names = field_names(row[0]);
            assert_eq!(
                names.len(),
                values.len(),
                "row {row:?} names as many fields as values"
            );
            Some(
                names
                    .into_iter()
                    .zip(values)
                    .map(|(field, value)| Expectation { field, value })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

pub fn assert_matches_note(descriptor: &Descriptor, rows: &[Vec<&str>], value_column: usize) {
    let checked = expectations(rows, value_column);
    assert!(!checked.is_empty(), "the note table yields expectations");
    for Expectation { field, value } in checked {
        assert_eq!(
            descriptor.field(&field),
            value,
            "{:?} at {} field {field} against the note",
            descriptor.kind,
            descriptor.offset
        );
    }
}
