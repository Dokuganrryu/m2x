use core::sync::atomic::{AtomicU32, Ordering};

const LOG2_BYTES_PER_WORD: usize = 2;
const BYTES_PER_WORD: usize = 1 << LOG2_BYTES_PER_WORD;
const LANE_MASK: usize = BYTES_PER_WORD - 1;
const BITS_PER_BYTE: usize = 8;
const BYTE_MASK: u32 = 0xFF;

struct Slot<const WORDS: usize> {
    words: [AtomicU32; WORDS],
    length: AtomicU32,
}

pub struct PacketRing<const SLOTS: usize, const WORDS: usize> {
    slots: [Slot<WORDS>; SLOTS],
    published: AtomicU32,
    taken: AtomicU32,
}

pub struct PacketWriter<'a, const SLOTS: usize, const WORDS: usize> {
    ring: &'a PacketRing<SLOTS, WORDS>,
}

pub struct PacketReader<'a, const SLOTS: usize, const WORDS: usize> {
    ring: &'a PacketRing<SLOTS, WORDS>,
}

const fn words_holding(bytes: usize) -> usize {
    (bytes + LANE_MASK) >> LOG2_BYTES_PER_WORD
}

#[inline(always)]
pub fn copy_packet(to: &[AtomicU32], from: &[AtomicU32], bytes: usize) -> usize {
    let words = words_holding(bytes).min(to.len()).min(from.len());
    for (into, word) in to.iter().zip(from).take(words) {
        into.store(word.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    bytes.min(words << LOG2_BYTES_PER_WORD)
}

fn byte_at(words: &[AtomicU32], at: usize) -> u8 {
    let shift = (at & LANE_MASK) * BITS_PER_BYTE;
    words
        .get(at >> LOG2_BYTES_PER_WORD)
        .map_or(0, |word| (word.load(Ordering::Relaxed) >> shift) as u8)
}

fn store_byte(words: &[AtomicU32], at: usize, byte: u8) {
    let shift = (at & LANE_MASK) * BITS_PER_BYTE;
    if let Some(word) = words.get(at >> LOG2_BYTES_PER_WORD) {
        let kept = word.load(Ordering::Relaxed) & !(BYTE_MASK << shift);
        word.store(kept | (u32::from(byte) << shift), Ordering::Relaxed);
    }
}

impl<const SLOTS: usize, const WORDS: usize> PacketRing<SLOTS, WORDS> {
    const SLOT_MASK: u32 = {
        assert!(SLOTS.is_power_of_two());
        SLOTS as u32 - 1
    };
    pub const PACKET_BYTES: usize = WORDS * BYTES_PER_WORD;

    pub const fn new() -> Self {
        Self {
            slots: [const {
                Slot {
                    words: [const { AtomicU32::new(0) }; WORDS],
                    length: AtomicU32::new(0),
                }
            }; SLOTS],
            published: AtomicU32::new(0),
            taken: AtomicU32::new(0),
        }
    }

    pub fn split(
        &mut self,
    ) -> (
        PacketWriter<'_, SLOTS, WORDS>,
        PacketReader<'_, SLOTS, WORDS>,
    ) {
        let ring = &*self;
        (PacketWriter { ring }, PacketReader { ring })
    }

    fn slot(&self, count: u32) -> Option<&Slot<WORDS>> {
        self.slots.get((count & Self::SLOT_MASK) as usize)
    }
}

impl<const SLOTS: usize, const WORDS: usize> Default for PacketRing<SLOTS, WORDS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize, const WORDS: usize> PacketWriter<'_, SLOTS, WORDS> {
    #[inline(always)]
    pub fn free_slots(&self) -> usize {
        let published = self.ring.published.load(Ordering::Relaxed);
        let taken = self.ring.taken.load(Ordering::Acquire);
        SLOTS.saturating_sub(published.wrapping_sub(taken) as usize)
    }

    #[inline(always)]
    pub fn has_room(&self) -> bool {
        self.free_slots() > 0
    }

    fn pending(&self) -> Option<&Slot<WORDS>> {
        self.has_room()
            .then(|| self.ring.slot(self.ring.published.load(Ordering::Relaxed)))
            .flatten()
    }

    #[inline(always)]
    pub fn publish(&mut self, fill: impl FnOnce(&[AtomicU32]) -> usize) -> bool {
        let Some(slot) = self.pending() else {
            return false;
        };
        let length = fill(&slot.words).min(PacketRing::<SLOTS, WORDS>::PACKET_BYTES);
        slot.length.store(length as u32, Ordering::Relaxed);
        let published = self.ring.published.load(Ordering::Relaxed);
        self.ring
            .published
            .store(published.wrapping_add(1), Ordering::Release);
        true
    }
}

impl<'a, const SLOTS: usize, const WORDS: usize> PacketReader<'a, SLOTS, WORDS> {
    fn front(&self) -> Option<(&'a [AtomicU32], usize)> {
        let taken = self.ring.taken.load(Ordering::Relaxed);
        if self.ring.published.load(Ordering::Acquire) == taken {
            return None;
        }
        let slot = self.ring.slot(taken)?;
        Some((&slot.words, slot.length.load(Ordering::Relaxed) as usize))
    }

    #[inline(always)]
    pub fn has_data(&self) -> bool {
        self.ring.published.load(Ordering::Relaxed) != self.ring.taken.load(Ordering::Relaxed)
    }

    fn release(&mut self) {
        let taken = self.ring.taken.load(Ordering::Relaxed);
        self.ring
            .taken
            .store(taken.wrapping_add(1), Ordering::Release);
    }

    #[inline(always)]
    pub fn take<R>(&mut self, read: impl FnOnce(&[AtomicU32], usize) -> R) -> Option<R> {
        let (words, length) = self.front()?;
        let read = read(words, length);
        self.release();
        Some(read)
    }
}

pub struct ByteSource<'a, const SLOTS: usize, const WORDS: usize> {
    reader: PacketReader<'a, SLOTS, WORDS>,
    offset: usize,
}

impl<'a, const SLOTS: usize, const WORDS: usize> ByteSource<'a, SLOTS, WORDS> {
    pub const fn new(reader: PacketReader<'a, SLOTS, WORDS>) -> Self {
        Self { reader, offset: 0 }
    }

    pub fn next_byte(&mut self) -> Option<u8> {
        loop {
            let (words, length) = self.reader.front()?;
            if self.offset < length {
                let byte = byte_at(words, self.offset);
                self.offset += 1;
                if self.offset == length {
                    self.offset = 0;
                    self.reader.release();
                }
                return Some(byte);
            }
            self.offset = 0;
            self.reader.release();
        }
    }
}

pub struct ByteSink<'a, const SLOTS: usize, const WORDS: usize> {
    writer: PacketWriter<'a, SLOTS, WORDS>,
    packet_bytes: usize,
    pending: usize,
}

impl<'a, const SLOTS: usize, const WORDS: usize> ByteSink<'a, SLOTS, WORDS> {
    pub fn new(writer: PacketWriter<'a, SLOTS, WORDS>, packet_bytes: usize) -> Self {
        Self {
            writer,
            packet_bytes: packet_bytes.clamp(1, PacketRing::<SLOTS, WORDS>::PACKET_BYTES),
            pending: 0,
        }
    }

    #[inline(never)]
    pub fn push(&mut self, byte: u8) -> bool {
        let Some(slot) = self.writer.pending() else {
            return false;
        };
        store_byte(&slot.words, self.pending, byte);
        self.pending += 1;
        if self.pending == self.packet_bytes {
            self.flush();
        }
        true
    }

    pub fn flush(&mut self) {
        if self.pending > 0 {
            let pending = self.pending;
            self.writer.publish(|_| pending);
            self.pending = 0;
        }
    }

    pub fn free_bytes(&self) -> usize {
        (self.writer.free_slots() * self.packet_bytes).saturating_sub(self.pending)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    fn words_of(bytes: &[u8]) -> Vec<AtomicU32> {
        bytes
            .chunks(BYTES_PER_WORD)
            .map(|chunk| {
                let mut word = [0u8; BYTES_PER_WORD];
                word[..chunk.len()].copy_from_slice(chunk);
                AtomicU32::new(u32::from_le_bytes(word))
            })
            .collect()
    }

    fn drain<const S: usize, const W: usize>(source: &mut ByteSource<'_, S, W>) -> Vec<u8> {
        core::iter::from_fn(|| source.next_byte()).collect()
    }

    #[test]
    fn a_packet_written_as_words_reads_back_as_the_same_bytes_in_order() {
        let mut ring = PacketRing::<2, 4>::new();
        let (mut writer, reader) = ring.split();
        let mut source = ByteSource::new(reader);
        let sent = *b"status\r\n12345";
        let from = words_of(&sent[..13]);
        assert!(writer.publish(|slot| copy_packet(slot, &from, 13)));
        assert_eq!(drain(&mut source), b"status\r\n12345".to_vec());
        assert!(writer.free_slots() == 2);
    }

    #[test]
    fn a_full_ring_refuses_the_next_packet_until_the_reader_frees_a_slot() {
        let mut ring = PacketRing::<2, 2>::new();
        let (mut writer, reader) = ring.split();
        let mut source = ByteSource::new(reader);
        let from = words_of(b"abcdefgh");
        assert!(writer.publish(|slot| copy_packet(slot, &from, 8)));
        assert!(writer.publish(|slot| copy_packet(slot, &from, 3)));
        assert!(!writer.has_room());
        assert!(!writer.publish(|slot| copy_packet(slot, &from, 8)));
        for _ in 0..8 {
            source.next_byte();
        }
        assert!(writer.has_room());
        assert_eq!(drain(&mut source), b"abc".to_vec());
    }

    #[test]
    fn an_empty_packet_is_skipped_by_the_byte_reader() {
        let mut ring = PacketRing::<2, 2>::new();
        let (mut writer, reader) = ring.split();
        let mut source = ByteSource::new(reader);
        let from = words_of(b"xy");
        assert!(writer.publish(|_| 0));
        assert!(writer.publish(|slot| copy_packet(slot, &from, 2)));
        assert_eq!(drain(&mut source), b"xy".to_vec());
    }

    #[test]
    fn a_copy_never_runs_past_either_buffer() {
        let to = words_of(&[0; 4]);
        let from = words_of(b"abcdefgh");
        assert_eq!(copy_packet(&to, &from, 8), 4);
        assert_eq!(to[0].load(Ordering::Relaxed), u32::from_le_bytes(*b"abcd"));
    }

    #[test]
    fn the_sink_publishes_a_packet_at_its_packet_size_and_on_flush() {
        let mut ring = PacketRing::<4, 2>::new();
        let (writer, mut reader) = ring.split();
        let mut sink = ByteSink::new(writer, 7);
        for byte in *b"0123456789" {
            assert!(sink.push(byte));
        }
        let mut packets = Vec::new();
        let take = |reader: &mut PacketReader<'_, 4, 2>| {
            reader
                .take(|words, length| (0..length).map(|at| byte_at(words, at)).collect::<Vec<_>>())
        };
        packets.extend(take(&mut reader));
        assert_eq!(take(&mut reader), None);
        sink.flush();
        packets.extend(take(&mut reader));
        assert_eq!(packets, [b"0123456".to_vec(), b"789".to_vec()]);
    }

    #[test]
    fn a_sink_with_no_free_slot_refuses_bytes_and_says_how_much_room_is_left() {
        let mut ring = PacketRing::<2, 1>::new();
        let (writer, _reader) = ring.split();
        let mut sink = ByteSink::new(writer, 4);
        assert_eq!(sink.free_bytes(), 8);
        for byte in *b"abcdefgh" {
            assert!(sink.push(byte));
        }
        assert_eq!(sink.free_bytes(), 0);
        assert!(!sink.push(b'i'));
    }
}
