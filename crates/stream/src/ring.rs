use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use dsp::{Frame, StreamSample};

use crate::Frames;

const CAPACITY_FRAMES: usize = 512;
const SLOT_MASK: u32 = CAPACITY_FRAMES as u32 - 1;
const _: () = assert!(CAPACITY_FRAMES.is_power_of_two());

pub const RING_CAPACITY: Frames = Frames::new(CAPACITY_FRAMES as u32);

type FrameCell = [AtomicI32; 2];

const WORDS_PER_FRAME: usize = 2;

pub struct FrameRing {
    cells: [FrameCell; CAPACITY_FRAMES],
    frames_written: AtomicU32,
    frames_read: AtomicU32,
}

pub struct RingWriter<'a> {
    ring: &'a FrameRing,
}

pub struct RingReader<'a> {
    ring: &'a FrameRing,
}

impl FrameRing {
    pub const fn new() -> Self {
        Self {
            cells: [const { [AtomicI32::new(0), AtomicI32::new(0)] }; CAPACITY_FRAMES],
            frames_written: AtomicU32::new(0),
            frames_read: AtomicU32::new(0),
        }
    }

    pub fn split(&mut self) -> (RingWriter<'_>, RingReader<'_>) {
        let ring = &*self;
        (RingWriter { ring }, RingReader { ring })
    }

    fn level_between(written: u32, read: u32) -> Frames {
        Frames::new(written.wrapping_sub(read))
    }

    fn runs_from(&self, index: u32, count: usize) -> [&[FrameCell]; 2] {
        let (before, from_index) = self.cells.split_at((index & SLOT_MASK) as usize);
        let first = count.min(from_index.len());
        let wrapped = (count - first).min(before.len());
        [&from_index[..first], &before[..wrapped]]
    }
}

fn store_run(cells: &[FrameCell], frames: &[Frame<StreamSample>]) {
    for (cell, frame) in cells.iter().zip(frames) {
        cell[0].store(frame[0].to_bits(), Ordering::Relaxed);
        cell[1].store(frame[1].to_bits(), Ordering::Relaxed);
    }
}

fn load_wire_run(cells: &[FrameCell], words: &[AtomicU32]) {
    for (cell, pair) in cells.iter().zip(words.chunks_exact(WORDS_PER_FRAME)) {
        if let [left, right] = pair {
            cell[0].store(left.load(Ordering::Relaxed) as i32, Ordering::Relaxed);
            cell[1].store(right.load(Ordering::Relaxed) as i32, Ordering::Relaxed);
        }
    }
}

fn load_run(cells: &[FrameCell], frames: &mut [Frame<StreamSample>]) {
    for (cell, frame) in cells.iter().zip(frames) {
        *frame = [
            StreamSample::from_bits(cell[0].load(Ordering::Relaxed)),
            StreamSample::from_bits(cell[1].load(Ordering::Relaxed)),
        ];
    }
}

pub(crate) struct RingFrames<'a> {
    runs: [&'a [FrameCell]; 2],
}

fn store_wire_run(cells: &[FrameCell], words: &[AtomicU32]) {
    for (pair, cell) in words.chunks_exact(WORDS_PER_FRAME).zip(cells) {
        if let [left, right] = pair {
            left.store(cell[0].load(Ordering::Relaxed) as u32, Ordering::Relaxed);
            right.store(cell[1].load(Ordering::Relaxed) as u32, Ordering::Relaxed);
        }
    }
}

impl RingFrames<'_> {
    pub(crate) fn store_as_wire_words(&self, words: &[AtomicU32]) -> usize {
        let [first, wrapped] = self.runs;
        let room = words.len() / WORDS_PER_FRAME;
        let in_first = first.len().min(room);
        let (into_first, into_wrapped) =
            words.split_at((in_first * WORDS_PER_FRAME).min(words.len()));
        store_wire_run(first, into_first);
        store_wire_run(wrapped, into_wrapped);
        in_first + wrapped.len().min(room - in_first)
    }

    pub(crate) fn copy_into(&self, frames: &mut [Frame<StreamSample>]) -> usize {
        let [first, wrapped] = self.runs;
        let (into_first, into_wrapped) = frames.split_at_mut(first.len().min(frames.len()));
        load_run(first, into_first);
        load_run(wrapped, into_wrapped);
        into_first.len() + wrapped.len().min(into_wrapped.len())
    }
}

impl Default for FrameRing {
    fn default() -> Self {
        Self::new()
    }
}

impl RingWriter<'_> {
    pub fn level(&self) -> Frames {
        let read = self.ring.frames_read.load(Ordering::Acquire);
        FrameRing::level_between(self.ring.frames_written.load(Ordering::Relaxed), read)
    }

    pub fn push(&mut self, packet: &[Frame<StreamSample>]) -> Frames {
        let written = self.ring.frames_written.load(Ordering::Relaxed);
        let read = self.ring.frames_read.load(Ordering::Acquire);
        let free = RING_CAPACITY.count() - FrameRing::level_between(written, read).count();
        let stored = packet.len().min(free as usize);
        let [first, wrapped] = self.ring.runs_from(written, stored);
        let (into_first, into_wrapped) = packet.split_at(first.len().min(packet.len()));
        store_run(first, into_first);
        store_run(wrapped, into_wrapped);
        self.ring
            .frames_written
            .store(written.wrapping_add(stored as u32), Ordering::Release);
        Frames::new(stored as u32)
    }

    pub fn push_wire_words(&mut self, words: &[AtomicU32]) -> Frames {
        let written = self.ring.frames_written.load(Ordering::Relaxed);
        let read = self.ring.frames_read.load(Ordering::Acquire);
        let free = RING_CAPACITY.count() - FrameRing::level_between(written, read).count();
        let stored = (words.len() / WORDS_PER_FRAME).min(free as usize);
        let [first, wrapped] = self.ring.runs_from(written, stored);
        let (from_first, from_wrapped) =
            words.split_at((first.len() * WORDS_PER_FRAME).min(words.len()));
        load_wire_run(first, from_first);
        load_wire_run(wrapped, from_wrapped);
        self.ring
            .frames_written
            .store(written.wrapping_add(stored as u32), Ordering::Release);
        Frames::new(stored as u32)
    }
}

impl RingReader<'_> {
    pub fn level(&self) -> Frames {
        let written = self.ring.frames_written.load(Ordering::Acquire);
        FrameRing::level_between(written, self.ring.frames_read.load(Ordering::Relaxed))
    }

    pub fn pop_block<const N: usize>(&mut self, block: &mut [Frame<StreamSample>; N]) -> Frames {
        self.pop_into(block)
    }

    pub fn pop_into(&mut self, frames: &mut [Frame<StreamSample>]) -> Frames {
        let read = self.ring.frames_read.load(Ordering::Relaxed);
        let written = self.ring.frames_written.load(Ordering::Acquire);
        let taken = (FrameRing::level_between(written, read).count() as usize).min(frames.len());
        let [first, wrapped] = self.ring.runs_from(read, taken);
        let (from_first, from_wrapped) = frames.split_at_mut(first.len().min(frames.len()));
        load_run(first, from_first);
        load_run(wrapped, from_wrapped);
        self.ring
            .frames_read
            .store(read.wrapping_add(taken as u32), Ordering::Release);
        Frames::new(taken as u32)
    }

    pub(crate) fn pop_through<R>(
        &mut self,
        most: usize,
        take: impl FnOnce(RingFrames<'_>) -> R,
    ) -> R {
        let read = self.ring.frames_read.load(Ordering::Relaxed);
        let written = self.ring.frames_written.load(Ordering::Acquire);
        let taken = (FrameRing::level_between(written, read).count() as usize).min(most);
        let [first, wrapped] = self.ring.runs_from(read, taken);
        let result = take(RingFrames {
            runs: [first, wrapped],
        });
        self.ring
            .frames_read
            .store(read.wrapping_add(taken as u32), Ordering::Release);
        result
    }

    pub fn discard_oldest(&mut self, frames: Frames) {
        let read = self.ring.frames_read.load(Ordering::Relaxed);
        let available = self.level().count();
        let dropped = frames.count().min(available);
        self.ring
            .frames_read
            .store(read.wrapping_add(dropped), Ordering::Release);
    }

    pub fn discard_all(&mut self) {
        let written = self.ring.frames_written.load(Ordering::Acquire);
        self.ring.frames_read.store(written, Ordering::Release);
    }
}
