use core::sync::atomic::AtomicU32;

use super::ring::{ByteSink, ByteSource, PacketReader, PacketRing, PacketWriter, copy_packet};
use crate::usb::BusSpeed;
use crate::usb::dispatch::SerialPipe;

const INBOUND_SLOTS: usize = 2;
const INBOUND_WORDS: usize = 128;
const OUTBOUND_SLOTS: usize = 4;
const OUTBOUND_WORDS: usize = 16;

pub const OUTBOUND_PACKET_BYTES: usize = OUTBOUND_WORDS * 4 - 1;

const _: () = assert!(
    PacketRing::<INBOUND_SLOTS, INBOUND_WORDS>::PACKET_BYTES
        >= BusSpeed::High.bulk_packet().value() as usize
);
const _: () = assert!(OUTBOUND_PACKET_BYTES < BusSpeed::Full.bulk_packet().value() as usize);

pub type ConsoleOut<'a> = ByteSink<'a, OUTBOUND_SLOTS, OUTBOUND_WORDS>;

type Inbound = PacketRing<INBOUND_SLOTS, INBOUND_WORDS>;
type Outbound = PacketRing<OUTBOUND_SLOTS, OUTBOUND_WORDS>;

pub struct ConsoleRings {
    from_host: Inbound,
    to_host: Outbound,
}

pub struct ConsoleUsb<'a> {
    from_host: PacketWriter<'a, INBOUND_SLOTS, INBOUND_WORDS>,
    to_host: PacketReader<'a, OUTBOUND_SLOTS, OUTBOUND_WORDS>,
}

pub struct ConsoleThread<'a> {
    pub from_host: ByteSource<'a, INBOUND_SLOTS, INBOUND_WORDS>,
    pub to_host: ConsoleOut<'a>,
}

impl ConsoleRings {
    pub const fn new() -> Self {
        Self {
            from_host: Inbound::new(),
            to_host: Outbound::new(),
        }
    }

    pub fn split(&mut self) -> (ConsoleUsb<'_>, ConsoleThread<'_>) {
        let (from_host_writer, from_host_reader) = self.from_host.split();
        let (to_host_writer, to_host_reader) = self.to_host.split();
        (
            ConsoleUsb {
                from_host: from_host_writer,
                to_host: to_host_reader,
            },
            ConsoleThread {
                from_host: ByteSource::new(from_host_reader),
                to_host: ByteSink::new(to_host_writer, OUTBOUND_PACKET_BYTES),
            },
        )
    }
}

impl Default for ConsoleRings {
    fn default() -> Self {
        Self::new()
    }
}

impl SerialPipe for ConsoleUsb<'_> {
    #[inline(always)]
    fn has_room(&self) -> bool {
        self.from_host.has_room()
    }

    #[inline(always)]
    fn has_data(&self) -> bool {
        self.to_host.has_data()
    }

    #[inline(always)]
    fn received(&mut self, words: &[AtomicU32], length: usize) {
        self.from_host
            .publish(|slot| copy_packet(slot, words, length));
    }

    #[inline(always)]
    fn send_into(&mut self, words: &[AtomicU32]) -> usize {
        self.to_host
            .take(|from, length| copy_packet(words, from, length))
            .unwrap_or(0)
    }
}
