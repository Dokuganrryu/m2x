use core::sync::atomic::{AtomicU32, Ordering};

use dsp::{Frame, FramesPerTick, I2sWord, MAX_FRAMES};

use crate::Frames;

pub const DMA_BUFFER_FRAMES: usize = 96;
pub const TRANSMIT_LEAD_BLOCKS: u32 = 2;

const WORDS_PER_FRAME: usize = 2;
const DMA_BUFFER_WORDS: usize = DMA_BUFFER_FRAMES * WORDS_PER_FRAME;
const TICK_FRAMES_AFTER_THE_BLOCK_BOUNDARY: u32 = 1;
const LAST_TRANSFER: u32 = DMA_BUFFER_WORDS as u32 - 1;
const LOG2_TRANSFERS_PER_FRAME: u32 = 1;
const BUFFER_FRAMES: u32 = DMA_BUFFER_FRAMES as u32;
const BLOCKS_AT_SIX_FRAMES_RECIPROCAL: u32 = 43;
const BLOCKS_AT_SIX_FRAMES_SHIFT: u32 = 8;
const FRAME_INDEX_BITS: u32 = 8;
const FRAME_INDEX_MASK: u32 = (1 << FRAME_INDEX_BITS) - 1;
const TICKS_SERVED_SHIFT: u32 = 2 * FRAME_INDEX_BITS;
const ENDS_MASK: u32 = (1 << TICKS_SERVED_SHIFT) - 1;
const ONE_TICK_SERVED: u32 = 1 << TICKS_SERVED_SHIFT;

const _: () = assert!(
    TRANSMIT_LEAD_BLOCKS + 2 <= (DMA_BUFFER_FRAMES / MAX_FRAMES) as u32,
    "at 24 frames per tick the buffer holds the block in progress, the lead and the block just played"
);

pub const fn tick_counter_start(frames: FramesPerTick) -> u32 {
    frames.count() as u32 - TICK_FRAMES_AFTER_THE_BLOCK_BOUNDARY
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XferCount(u16);

impl XferCount {
    const FIELD_SHIFT: u32 = 16;
    const FIELD_MASK: u32 = 0x3ff;

    pub const fn from_xfercfg(register: u32) -> Self {
        Self(((register >> Self::FIELD_SHIFT) & Self::FIELD_MASK) as u16)
    }

    pub const fn frame_in_progress(self) -> DmaFrame {
        let moved = LAST_TRANSFER.wrapping_sub(self.0 as u32) & Self::FIELD_MASK;
        let moved = if moved > LAST_TRANSFER {
            LAST_TRANSFER
        } else {
            moved
        };
        DmaFrame((moved >> LOG2_TRANSFERS_PER_FRAME) as u8)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaFrame(u8);

impl DmaFrame {
    pub const fn index(self) -> u32 {
        self.0 as u32
    }
}

const fn frames_from_to(from: u32, to: u32) -> u32 {
    wrapped(to + BUFFER_FRAMES - from)
}

const fn wrapped(frame: u32) -> u32 {
    if frame >= BUFFER_FRAMES {
        frame - BUFFER_FRAMES
    } else {
        frame
    }
}

const fn log2_blocks_of_six(frames: FramesPerTick) -> u32 {
    match frames {
        FramesPerTick::Six => 0,
        FramesPerTick::Twelve => 1,
        FramesPerTick::TwentyFour => 2,
    }
}

const fn last_block(frames: FramesPerTick) -> u32 {
    match frames {
        FramesPerTick::Six => 15,
        FramesPerTick::Twelve => 7,
        FramesPerTick::TwentyFour => 3,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickBlocks {
    frames: FramesPerTick,
    receive: u8,
    transmit: u8,
}

impl TickBlocks {
    pub const fn around(position: DmaFrame, frames: FramesPerTick) -> Self {
        let blocks_of_six =
            (position.index() * BLOCKS_AT_SIX_FRAMES_RECIPROCAL) >> BLOCKS_AT_SIX_FRAMES_SHIFT;
        let in_progress = blocks_of_six >> log2_blocks_of_six(frames);
        let last_block = last_block(frames);
        Self {
            frames,
            receive: (in_progress.wrapping_sub(1) & last_block) as u8,
            transmit: ((in_progress + TRANSMIT_LEAD_BLOCKS) & last_block) as u8,
        }
    }

    const fn receive_end(self) -> u32 {
        wrapped((self.receive as u32 + 1) * self.frames.count() as u32)
    }

    const fn transmit_end(self) -> u32 {
        wrapped((self.transmit as u32 + 1) * self.frames.count() as u32)
    }

    #[inline(never)]
    pub fn receive(
        reading: XferCount,
        frames: FramesPerTick,
        transport: &DmaTransport,
        into: &mut [Frame<I2sWord>; MAX_FRAMES],
    ) -> Self {
        let blocks = Self::around(reading.frame_in_progress(), frames);
        let block = blocks.receive as usize;
        let buffer = &transport.received;
        match frames {
            FramesPerTick::Six => buffer.load_block::<6, 12>(block, into),
            FramesPerTick::Twelve => buffer.load_block::<12, 24>(block, into),
            FramesPerTick::TwentyFour => buffer.load_block::<24, 48>(block, into),
        }
        blocks
    }

    #[inline(never)]
    pub fn transmit(self, from: &[Frame<I2sWord>; MAX_FRAMES], transport: &DmaTransport) {
        let block = self.transmit as usize;
        let (first, second) = (&transport.played, &transport.mirrored);
        match self.frames {
            FramesPerTick::Six => store_block::<6, 12>(from, block, first, second),
            FramesPerTick::Twelve => store_block::<12, 24>(from, block, first, second),
            FramesPerTick::TwentyFour => store_block::<24, 48>(from, block, first, second),
        }
        transport.cursor.served(self);
    }
}

pub struct DmaTransport {
    pub received: DmaBuffer,
    pub played: DmaBuffer,
    pub mirrored: DmaBuffer,
    pub cursor: DmaCursor,
}

impl DmaTransport {
    pub const fn silent() -> Self {
        Self {
            received: DmaBuffer::silent(),
            played: DmaBuffer::silent(),
            mirrored: DmaBuffer::silent(),
            cursor: DmaCursor::new(),
        }
    }

    pub fn zero_transmit_buffers(&self) {
        self.played.zero();
        self.mirrored.zero();
    }
}

#[repr(C, align(4))]
pub struct DmaBuffer {
    words: [AtomicU32; DMA_BUFFER_WORDS],
}

impl DmaBuffer {
    pub const fn silent() -> Self {
        Self {
            words: [const { AtomicU32::new(0) }; DMA_BUFFER_WORDS],
        }
    }

    pub fn first_word_address(&self) -> usize {
        self.words.as_ptr() as usize
    }

    pub fn last_word_address(&self) -> usize {
        self.words.as_ptr_range().end as usize - size_of::<AtomicU32>()
    }

    pub fn zero(&self) {
        for word in &self.words {
            word.store(0, Ordering::Relaxed);
        }
    }

    pub fn load_word(&self, word: usize) -> u32 {
        self.words
            .get(word)
            .map_or(0, |cell| cell.load(Ordering::Relaxed))
    }

    pub fn store_word(&self, word: usize, value: u32) {
        if let Some(cell) = self.words.get(word) {
            cell.store(value, Ordering::Relaxed);
        }
    }

    #[inline(always)]
    fn block<const WORDS: usize>(&self, block: usize) -> Option<&[AtomicU32; WORDS]> {
        self.words
            .get(block * WORDS..)
            .and_then(|from| from.first_chunk::<WORDS>())
    }

    #[inline(always)]
    fn load_block<const FRAMES: usize, const WORDS: usize>(
        &self,
        block: usize,
        into: &mut [Frame<I2sWord>; MAX_FRAMES],
    ) {
        let (Some(words), Some(frames)) =
            (self.block::<WORDS>(block), into.first_chunk_mut::<FRAMES>())
        else {
            return;
        };
        for (frame, pair) in frames.iter_mut().zip(words.chunks_exact(WORDS_PER_FRAME)) {
            if let [left, right] = pair {
                *frame = [
                    I2sWord::from_bits(left.load(Ordering::Relaxed) as i32),
                    I2sWord::from_bits(right.load(Ordering::Relaxed) as i32),
                ];
            }
        }
    }
}

#[inline(always)]
fn store_block<const FRAMES: usize, const WORDS: usize>(
    from: &[Frame<I2sWord>; MAX_FRAMES],
    block: usize,
    first: &DmaBuffer,
    second: &DmaBuffer,
) {
    let (Some(frames), Some(one), Some(two)) = (
        from.first_chunk::<FRAMES>(),
        first.block::<WORDS>(block),
        second.block::<WORDS>(block),
    ) else {
        return;
    };
    for ((frame, one), two) in frames
        .iter()
        .zip(one.chunks_exact(WORDS_PER_FRAME))
        .zip(two.chunks_exact(WORDS_PER_FRAME))
    {
        if let ([one_left, one_right], [two_left, two_right]) = (one, two) {
            let left = frame[0].to_bits() as u32;
            let right = frame[1].to_bits() as u32;
            one_left.store(left, Ordering::Relaxed);
            one_right.store(right, Ordering::Relaxed);
            two_left.store(left, Ordering::Relaxed);
            two_right.store(right, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxDmaPosition {
    pub frames_queued: Frames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaFill {
    pub playback: TxDmaPosition,
    pub capture: Frames,
}

#[derive(Debug)]
pub struct DmaCursor {
    served: AtomicU32,
}

impl DmaCursor {
    pub const fn new() -> Self {
        Self {
            served: AtomicU32::new(0),
        }
    }

    pub fn served(&self, blocks: TickBlocks) {
        let ticks = self.served.load(Ordering::Relaxed) & !ENDS_MASK;
        self.served.store(
            ticks.wrapping_add(ONE_TICK_SERVED)
                | (blocks.transmit_end() << FRAME_INDEX_BITS)
                | blocks.receive_end(),
            Ordering::Relaxed,
        );
    }

    pub fn ticks_served(&self) -> u16 {
        (self.served.load(Ordering::Relaxed) >> TICKS_SERVED_SHIFT) as u16
    }

    pub fn fill_at(&self, position: DmaFrame) -> DmaFill {
        let served = self.served.load(Ordering::Relaxed);
        let transmit_end = (served >> FRAME_INDEX_BITS) & FRAME_INDEX_MASK;
        let receive_end = served & FRAME_INDEX_MASK;
        DmaFill {
            playback: TxDmaPosition {
                frames_queued: Frames::new(frames_from_to(position.index(), transmit_end)),
            },
            capture: Frames::new(frames_from_to(receive_end, position.index())),
        }
    }
}

impl Default for DmaCursor {
    fn default() -> Self {
        Self::new()
    }
}
