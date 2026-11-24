#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools
)]

use core::sync::atomic::{AtomicU32, Ordering};

use stream::{Frames, TxDmaPosition};

use super::audio_function::{HIGH_SPEED_FEEDBACK_BYTES, STREAM_PACKET};
use super::configuration::BusSpeed;
use super::device::HIGH_SPEED_CONTROL_PACKET_BYTES;
use super::request::{Address, HaltableEndpoint, SETUP_PACKET_LENGTH};
use super::standard::EndpointNumber;
use crate::outputs::{Filler, OutputList};

pub const USB_RAM_BYTES: usize = 16 * 1024;
pub const BUFFER_ALIGNMENT_BYTES: u16 = 64;
pub const OUT_WRITE_GRANULE_BYTES: u16 = 8;
pub const LARGEST_ISOCHRONOUS_IN_BYTES: u16 = 1023;
pub const ENDPOINT_LIST_BYTES: u16 = 0x60;
pub const MOST_COMMANDS_PER_EVENT: usize = 16;

const BYTES_PER_WORD: usize = 4;
const USB_RAM_WORDS: usize = USB_RAM_BYTES / BYTES_PER_WORD;
const LOG2_BYTES_PER_WORD: u32 = BYTES_PER_WORD.trailing_zeros();
const BYTE_LANE_MASK: usize = BYTES_PER_WORD - 1;
const BITS_PER_BYTE: u32 = 8;
const LIST_BYTES_PER_ENDPOINT: u16 = 0x10;
const LIST_IN_AFTER_OUT_BYTES: u16 = 8;
const LIST_SETUP_AFTER_CONTROL_OUT_BYTES: u16 = 4;
const LIST_SECOND_BUFFER_AFTER_FIRST_BYTES: u16 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BufferSlot {
    First,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutEntry {
    Control,
    Playback(BufferSlot),
    Serial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InEntry {
    Control,
    Feedback,
    Capture(BufferSlot),
    Serial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListEntry {
    Setup,
    Out(OutEntry),
    In(InEntry),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferKind {
    Control,
    Bulk,
    Isochronous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteCount(u16);

impl ByteCount {
    pub const ZERO: Self = Self(0);

    pub const fn new(bytes: u16) -> Self {
        Self(bytes)
    }

    pub const fn value(self) -> u16 {
        self.0
    }

    const fn at_most(self, limit: Self) -> Self {
        if self.0 < limit.0 { self } else { limit }
    }

    const fn whole_out_granules(self) -> Self {
        Self(self.0 & !(OUT_WRITE_GRANULE_BYTES - 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub offset: u16,
    pub capacity: ByteCount,
}

impl Region {
    pub const fn end(self) -> u16 {
        self.offset + self.capacity.0
    }
}

const fn slot_index(slot: BufferSlot) -> u16 {
    match slot {
        BufferSlot::First => 0,
        BufferSlot::Second => 1,
    }
}

const fn endpoint_list_base(number: EndpointNumber) -> u16 {
    number.host_to_device().value() as u16 * LIST_BYTES_PER_ENDPOINT
}

impl ListEntry {
    pub const ALL: [Self; 10] = [
        Self::Setup,
        Self::Out(OutEntry::Control),
        Self::In(InEntry::Control),
        Self::Out(OutEntry::Playback(BufferSlot::First)),
        Self::Out(OutEntry::Playback(BufferSlot::Second)),
        Self::In(InEntry::Feedback),
        Self::In(InEntry::Capture(BufferSlot::First)),
        Self::In(InEntry::Capture(BufferSlot::Second)),
        Self::Out(OutEntry::Serial),
        Self::In(InEntry::Serial),
    ];

    const fn index(self) -> usize {
        match self {
            Self::Setup => 0,
            Self::Out(OutEntry::Control) => 1,
            Self::In(InEntry::Control) => 2,
            Self::Out(OutEntry::Playback(BufferSlot::First)) => 3,
            Self::Out(OutEntry::Playback(BufferSlot::Second)) => 4,
            Self::In(InEntry::Feedback) => 5,
            Self::In(InEntry::Capture(BufferSlot::First)) => 6,
            Self::In(InEntry::Capture(BufferSlot::Second)) => 7,
            Self::Out(OutEntry::Serial) => 8,
            Self::In(InEntry::Serial) => 9,
        }
    }

    const fn capacity(self) -> ByteCount {
        ByteCount(match self {
            Self::Setup => SETUP_PACKET_LENGTH as u16,
            Self::Out(OutEntry::Control) | Self::In(InEntry::Control) => {
                HIGH_SPEED_CONTROL_PACKET_BYTES as u16
            }
            Self::Out(OutEntry::Playback(_)) | Self::In(InEntry::Capture(_)) => {
                STREAM_PACKET.value()
            }
            Self::In(InEntry::Feedback) => HIGH_SPEED_FEEDBACK_BYTES,
            Self::Out(OutEntry::Serial) | Self::In(InEntry::Serial) => {
                BusSpeed::High.bulk_packet().value()
            }
        })
    }

    pub const fn kind(self) -> TransferKind {
        match self {
            Self::Setup | Self::Out(OutEntry::Control) | Self::In(InEntry::Control) => {
                TransferKind::Control
            }
            Self::Out(OutEntry::Playback(_))
            | Self::In(InEntry::Feedback | InEntry::Capture(_)) => TransferKind::Isochronous,
            Self::Out(OutEntry::Serial) | Self::In(InEntry::Serial) => TransferKind::Bulk,
        }
    }

    pub const fn list_offset(self) -> u16 {
        match self {
            Self::Out(OutEntry::Control) => 0,
            Self::Setup => LIST_SETUP_AFTER_CONTROL_OUT_BYTES,
            Self::In(InEntry::Control) => LIST_IN_AFTER_OUT_BYTES,
            Self::Out(OutEntry::Playback(slot)) => {
                endpoint_list_base(EndpointNumber::PLAYBACK)
                    + slot_index(slot) * LIST_SECOND_BUFFER_AFTER_FIRST_BYTES
            }
            Self::In(InEntry::Feedback) => {
                endpoint_list_base(EndpointNumber::PLAYBACK) + LIST_IN_AFTER_OUT_BYTES
            }
            Self::In(InEntry::Capture(slot)) => {
                endpoint_list_base(EndpointNumber::CAPTURE)
                    + LIST_IN_AFTER_OUT_BYTES
                    + slot_index(slot) * LIST_SECOND_BUFFER_AFTER_FIRST_BYTES
            }
            Self::Out(OutEntry::Serial) => endpoint_list_base(EndpointNumber::SERIAL_DATA),
            Self::In(InEntry::Serial) => {
                endpoint_list_base(EndpointNumber::SERIAL_DATA) + LIST_IN_AFTER_OUT_BYTES
            }
        }
    }

    pub const fn region(self) -> Region {
        LAYOUT[self.index()]
    }
}

const fn aligned(offset: u16) -> u16 {
    (offset + BUFFER_ALIGNMENT_BYTES - 1) & !(BUFFER_ALIGNMENT_BYTES - 1)
}

const fn layout() -> [Region; ListEntry::ALL.len()] {
    let mut regions = [Region {
        offset: 0,
        capacity: ByteCount::ZERO,
    }; ListEntry::ALL.len()];
    let mut next = aligned(ENDPOINT_LIST_BYTES);
    let mut at = 0;
    while at < ListEntry::ALL.len() {
        let entry = ListEntry::ALL[at];
        assert!(entry.index() == at, "ALL lists the entries in index order");
        let capacity = entry.capacity();
        regions[at] = Region {
            offset: next,
            capacity,
        };
        next = aligned(next + capacity.0);
        at += 1;
    }
    assert!(
        next as usize <= USB_RAM_BYTES,
        "every buffer fits the USB RAM"
    );
    regions
}

const LAYOUT: [Region; ListEntry::ALL.len()] = layout();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaPositions {
    pub playback: TxDmaPosition,
    pub capture: Frames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackArrival {
    pub slot: BufferSlot,
    pub untransferred: ByteCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamCompletions {
    pub playback: [Option<PlaybackArrival>; 2],
    pub feedback_sent: bool,
    pub capture_sent: [Option<BufferSlot>; 2],
}

impl StreamCompletions {
    pub const NONE: Self = Self {
        playback: [None; 2],
        feedback_sent: false,
        capture_sent: [None; 2],
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameStart {
    pub positions: DmaPositions,
    pub completed: StreamCompletions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Attached,
    Detached,
    BusReset(BusSpeed),
    Suspended,
    Resumed,
    StartOfFrame(FrameStart),
    Setup([u8; SETUP_PACKET_LENGTH]),
    OutCompleted {
        entry: OutEntry,
        untransferred: ByteCount,
    },
    InCompleted(InEntry),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    ArmOut { entry: OutEntry, room: ByteCount },
    ArmIn { entry: InEntry, length: ByteCount },
    DisarmOut(OutEntry),
    DisarmIn(InEntry),
    StallControl,
    ReleaseControlStall,
    Halt(HaltableEndpoint),
    ReleaseHalt(HaltableEndpoint),
    ApplyAddress(Address),
    ResetEndpoints,
}

impl Filler for Command {
    const FILLER: Self = Command::ResetEndpoints;
}

pub type Commands = OutputList<Command, MOST_COMMANDS_PER_EVENT>;

pub struct UsbRam {
    words: [AtomicU32; USB_RAM_WORDS],
}

impl Default for UsbRam {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbRam {
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU32::new(0) }; USB_RAM_WORDS],
        }
    }

    pub fn split(&mut self) -> (Buffers<'_>, ControllerView<'_>) {
        let ram: &UsbRam = self;
        let out = |entry| OutBuffer {
            words: ram.region_words(ListEntry::Out(entry)),
            entry,
        };
        let held_in = |entry| InBuffer {
            words: ram.region_words(ListEntry::In(entry)),
            entry,
        };
        (
            Buffers {
                control_out: out(OutEntry::Control),
                control_in: held_in(InEntry::Control),
                playback: [
                    out(OutEntry::Playback(BufferSlot::First)),
                    out(OutEntry::Playback(BufferSlot::Second)),
                ],
                feedback: held_in(InEntry::Feedback),
                capture: [
                    held_in(InEntry::Capture(BufferSlot::First)),
                    held_in(InEntry::Capture(BufferSlot::Second)),
                ],
                serial_out: out(OutEntry::Serial),
                serial_in: held_in(InEntry::Serial),
            },
            ControllerView { ram },
        )
    }

    fn region_words(&self, entry: ListEntry) -> &[AtomicU32] {
        self.words(entry, entry.capacity())
    }

    fn words(&self, entry: ListEntry, length: ByteCount) -> &[AtomicU32] {
        let region = entry.region();
        let first = (region.offset as usize) >> LOG2_BYTES_PER_WORD;
        let count =
            (length.at_most(region.capacity).0 as usize + BYTE_LANE_MASK) >> LOG2_BYTES_PER_WORD;
        self.words.get(first..first + count).unwrap_or(&[])
    }
}

fn byte_at(words: &[AtomicU32], at: usize) -> u8 {
    let lane_shift = (at & BYTE_LANE_MASK) as u32 * BITS_PER_BYTE;
    words
        .get(at >> LOG2_BYTES_PER_WORD)
        .map_or(0, |word| (word.load(Ordering::Relaxed) >> lane_shift) as u8)
}

fn store_bytes(words: &[AtomicU32], bytes: impl IntoIterator<Item = u8>) -> ByteCount {
    let mut bytes = bytes.into_iter();
    let mut written = 0;
    for word in words {
        let mut packed = 0;
        let mut lanes = 0;
        while lanes < BYTES_PER_WORD {
            let Some(byte) = bytes.next() else { break };
            packed |= u32::from(byte) << (lanes as u32 * BITS_PER_BYTE);
            lanes += 1;
        }
        if lanes == 0 {
            break;
        }
        word.store(packed, Ordering::Relaxed);
        written += lanes as u16;
        if lanes < BYTES_PER_WORD {
            break;
        }
    }
    ByteCount(written)
}

fn store_words(words: &[AtomicU32], values: impl IntoIterator<Item = u32>) -> ByteCount {
    let mut written = 0;
    for (word, value) in words.iter().zip(values) {
        word.store(value, Ordering::Relaxed);
        written += BYTES_PER_WORD as u16;
    }
    ByteCount(written)
}

pub struct ControllerView<'a> {
    ram: &'a UsbRam,
}

impl ControllerView<'_> {
    pub fn read(&self, entry: ListEntry, length: ByteCount) -> impl Iterator<Item = u8> + '_ {
        let words = self.ram.words(entry, length);
        let length = length.at_most(entry.region().capacity).0 as usize;
        (0..length).map(move |at| byte_at(words, at))
    }

    pub fn write(&self, entry: ListEntry, bytes: &[u8]) -> ByteCount {
        let length = ByteCount(bytes.len().min(u16::MAX as usize) as u16);
        store_bytes(self.ram.words(entry, length), bytes.iter().copied())
    }
}

pub struct Buffers<'a> {
    pub control_out: OutBuffer<'a>,
    pub control_in: InBuffer<'a>,
    pub playback: [OutBuffer<'a>; 2],
    pub feedback: InBuffer<'a>,
    pub capture: [InBuffer<'a>; 2],
    pub serial_out: OutBuffer<'a>,
    pub serial_in: InBuffer<'a>,
}

pub struct OutBuffer<'a> {
    words: &'a [AtomicU32],
    entry: OutEntry,
}

pub struct ArmedOut<'a> {
    words: &'a [AtomicU32],
    entry: OutEntry,
    room: ByteCount,
}

pub struct Received<'a> {
    words: &'a [AtomicU32],
    entry: OutEntry,
    length: ByteCount,
}

pub struct InBuffer<'a> {
    words: &'a [AtomicU32],
    entry: InEntry,
}

pub struct ArmedIn<'a> {
    words: &'a [AtomicU32],
    entry: InEntry,
}

impl<'a> OutBuffer<'a> {
    pub const fn entry(&self) -> OutEntry {
        self.entry
    }

    pub fn receive(self, room: ByteCount) -> (ArmedOut<'a>, Command) {
        self.lend(room)
    }

    fn lend(&self, room: ByteCount) -> (ArmedOut<'a>, Command) {
        let room = room
            .at_most(ListEntry::Out(self.entry).capacity())
            .whole_out_granules();
        (
            ArmedOut {
                words: self.words,
                entry: self.entry,
                room,
            },
            Command::ArmOut {
                entry: self.entry,
                room,
            },
        )
    }
}

impl<'a> ArmedOut<'a> {
    pub fn landed(self, untransferred: ByteCount) -> Received<'a> {
        self.landing(untransferred)
    }

    fn landing(&self, untransferred: ByteCount) -> Received<'a> {
        Received {
            words: self.words,
            entry: self.entry,
            length: ByteCount(self.room.0.saturating_sub(untransferred.0)),
        }
    }

    pub fn retire(self) -> (OutBuffer<'a>, Command) {
        let command = Command::DisarmOut(self.entry);
        (self.reclaim(), command)
    }

    pub fn reclaim(self) -> OutBuffer<'a> {
        self.handed_back()
    }

    fn handed_back(&self) -> OutBuffer<'a> {
        OutBuffer {
            words: self.words,
            entry: self.entry,
        }
    }
}

impl<'a> Received<'a> {
    pub const fn length(&self) -> ByteCount {
        self.length
    }

    pub fn words(&self) -> &'a [AtomicU32] {
        let count = (self.length.0 as usize + BYTE_LANE_MASK) >> LOG2_BYTES_PER_WORD;
        self.words.get(..count).unwrap_or(self.words)
    }

    pub fn bytes(&self) -> impl Iterator<Item = u8> + '_ {
        let words = self.words();
        (0..self.length.0 as usize).map(move |at| byte_at(words, at))
    }

    pub fn whole_words(&self) -> &'a [AtomicU32] {
        let whole = (self.length.0 as usize) >> LOG2_BYTES_PER_WORD;
        self.words().get(..whole).unwrap_or(&[])
    }

    pub fn release(self) -> OutBuffer<'a> {
        OutBuffer {
            words: self.words,
            entry: self.entry,
        }
    }
}

impl<'a> InBuffer<'a> {
    pub const fn entry(&self) -> InEntry {
        self.entry
    }

    fn all_words(&self) -> &'a [AtomicU32] {
        self.words
    }

    pub fn fill_bytes(&mut self, bytes: impl IntoIterator<Item = u8>) -> ByteCount {
        store_bytes(self.all_words(), bytes)
    }

    pub fn fill_words(&mut self, words: impl IntoIterator<Item = u32>) -> ByteCount {
        store_words(self.all_words(), words)
    }

    pub fn fill_bytes_in_place(&mut self, fill: impl FnOnce(&[AtomicU32]) -> usize) -> ByteCount {
        let words = self.all_words();
        ByteCount(fill(words).min(words.len() * BYTES_PER_WORD) as u16)
    }

    pub fn fill_words_in_place(&mut self, fill: impl FnOnce(&[AtomicU32]) -> usize) -> ByteCount {
        let words = self.all_words();
        ByteCount(fill(words).min(words.len()) as u16 * BYTES_PER_WORD as u16)
    }

    pub fn send(self, length: ByteCount) -> (ArmedIn<'a>, Command) {
        self.lend(length)
    }

    fn lend(&self, length: ByteCount) -> (ArmedIn<'a>, Command) {
        let length = length.at_most(ListEntry::In(self.entry).capacity());
        (
            ArmedIn {
                words: self.words,
                entry: self.entry,
            },
            Command::ArmIn {
                entry: self.entry,
                length,
            },
        )
    }
}

impl<'a> ArmedIn<'a> {
    pub fn sent(self) -> InBuffer<'a> {
        self.reclaim()
    }

    pub fn retire(self) -> (InBuffer<'a>, Command) {
        let command = Command::DisarmIn(self.entry);
        (self.reclaim(), command)
    }

    pub fn reclaim(self) -> InBuffer<'a> {
        self.handed_back()
    }

    fn handed_back(&self) -> InBuffer<'a> {
        InBuffer {
            words: self.words,
            entry: self.entry,
        }
    }
}

pub(crate) enum OutSlot<'a> {
    Held(OutBuffer<'a>),
    Lent(ArmedOut<'a>),
}

pub(crate) enum InSlot<'a> {
    Held(InBuffer<'a>),
    Lent(ArmedIn<'a>),
}

impl<'a> OutSlot<'a> {
    pub(crate) fn arm(&mut self, room: ByteCount, commands: &mut Commands) {
        if let Self::Held(buffer) = self {
            let (armed, command) = buffer.lend(room);
            commands.push(command);
            *self = Self::Lent(armed);
        }
    }

    pub(crate) fn complete(
        &mut self,
        untransferred: ByteCount,
        read: impl FnOnce(&Received<'a>),
    ) -> bool {
        let Self::Lent(armed) = self else {
            return false;
        };
        let received = armed.landing(untransferred);
        read(&received);
        *self = Self::Held(received.release());
        true
    }

    pub(crate) fn retire(&mut self, commands: &mut Commands) {
        if let Self::Lent(armed) = self {
            commands.push(Command::DisarmOut(armed.entry));
            *self = Self::Held(armed.handed_back());
        }
    }

    pub(crate) fn reclaim(&mut self) {
        if let Self::Lent(armed) = self {
            *self = Self::Held(armed.handed_back());
        }
    }
}

impl<'a> InSlot<'a> {
    pub(crate) fn send_if_held(
        &mut self,
        fill: impl FnOnce(&mut InBuffer<'a>) -> Option<ByteCount>,
        commands: &mut Commands,
    ) {
        if let Self::Held(buffer) = self {
            if let Some(length) = fill(buffer) {
                let (armed, command) = buffer.lend(length);
                commands.push(command);
                *self = Self::Lent(armed);
            }
        }
    }

    pub(crate) fn complete(&mut self) -> bool {
        let Self::Lent(armed) = self else {
            return false;
        };
        *self = Self::Held(armed.handed_back());
        true
    }

    pub(crate) fn retire(&mut self, commands: &mut Commands) {
        if let Self::Lent(armed) = self {
            commands.push(Command::DisarmIn(armed.entry));
            *self = Self::Held(armed.handed_back());
        }
    }

    pub(crate) fn reclaim(&mut self) {
        if let Self::Lent(armed) = self {
            *self = Self::Held(armed.handed_back());
        }
    }
}

#[cfg(doctest)]
#[doc = include_str!("../../compile_fail/usb_buffer_ownership.md")]
pub struct ArmedBuffersHaveNoByteAccess;
