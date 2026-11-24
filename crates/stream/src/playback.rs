use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use dsp::{Frame, FramesPerTick, MAX_FRAMES, Sample, StreamSample};

use crate::fade::fade_out;
use crate::{
    FrameRing, Frames, PlaybackCommand, PlaybackEvent, PlaybackState, RingReader, RingWriter,
    TxDmaPosition,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlternateSetting {
    Zero,
    One,
}

const WORDS_PER_FRAME: usize = 2;

const SILENT_FRAME: Frame<StreamSample> = [StreamSample::SILENCE; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillRange {
    pub lowest: Frames,
    pub highest: Frames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackCounters {
    pub underruns: u32,
    pub overruns: u32,
    pub effective_fill_while_running: Option<FillRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamRequest(u32);

impl StreamRequest {
    const ALTERNATE_ONE: u32 = 1;
    const GENERATION_STEP: u32 = 2;
    const INITIAL: Self = Self(0);

    const fn next(self, alternate: AlternateSetting) -> Self {
        let generation = (self.0 & !Self::ALTERNATE_ONE).wrapping_add(Self::GENERATION_STEP);
        match alternate {
            AlternateSetting::Zero => Self(generation),
            AlternateSetting::One => Self(generation | Self::ALTERNATE_ONE),
        }
    }

    const fn alternate(self) -> AlternateSetting {
        if self.0 & Self::ALTERNATE_ONE == 0 {
            AlternateSetting::Zero
        } else {
            AlternateSetting::One
        }
    }

    const fn event(self) -> PlaybackEvent {
        match self.alternate() {
            AlternateSetting::Zero => PlaybackEvent::Stopped,
            AlternateSetting::One => PlaybackEvent::Started,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PackedFillRange(u32);

impl PackedFillRange {
    const EMPTY: Self = Self(0);
    const HALF: u32 = 16;
    const HALF_MASK: u32 = 0xffff;

    fn widened_by(self, fill: Frames) -> Self {
        let fill = fill.count().min(Self::HALF_MASK);
        let lowest = ((self.0 >> Self::HALF) ^ Self::HALF_MASK).min(fill);
        let highest = (self.0 & Self::HALF_MASK).max(fill);
        Self(((lowest ^ Self::HALF_MASK) << Self::HALF) | highest)
    }

    const fn unpack(self) -> Option<FillRange> {
        let lowest = (self.0 >> Self::HALF) ^ Self::HALF_MASK;
        let highest = self.0 & Self::HALF_MASK;
        if lowest > highest {
            None
        } else {
            Some(FillRange {
                lowest: Frames::new(lowest),
                highest: Frames::new(highest),
            })
        }
    }
}

struct Shared {
    requested: AtomicU32,
    applied: AtomicU32,
    tick_is_running: AtomicBool,
    underruns: AtomicU32,
    overruns: AtomicU32,
    fill_range: AtomicU32,
    gate_is_open: AtomicBool,
}

pub struct Playback {
    ring: FrameRing,
    shared: Shared,
}

pub struct PlaybackUsb<'a> {
    writer: RingWriter<'a>,
    shared: &'a Shared,
    request: StreamRequest,
}

pub struct PlaybackTick<'a> {
    reader: RingReader<'a>,
    shared: &'a Shared,
    state: PlaybackState,
    applied: StreamRequest,
}

#[derive(Clone, Copy)]
pub struct PlaybackCounterReader<'a> {
    shared: &'a Shared,
}

pub struct PlaybackGate<'a> {
    shared: &'a Shared,
}

impl Playback {
    pub const fn new() -> Self {
        Self {
            ring: FrameRing::new(),
            shared: Shared {
                requested: AtomicU32::new(StreamRequest::INITIAL.0),
                applied: AtomicU32::new(StreamRequest::INITIAL.0),
                tick_is_running: AtomicBool::new(false),
                underruns: AtomicU32::new(0),
                overruns: AtomicU32::new(0),
                fill_range: AtomicU32::new(PackedFillRange::EMPTY.0),
                gate_is_open: AtomicBool::new(false),
            },
        }
    }

    pub fn split(
        &mut self,
    ) -> (
        PlaybackUsb<'_>,
        PlaybackTick<'_>,
        PlaybackCounterReader<'_>,
        PlaybackGate<'_>,
    ) {
        let (writer, reader) = self.ring.split();
        let shared = &self.shared;
        (
            PlaybackUsb {
                writer,
                shared,
                request: StreamRequest::INITIAL,
            },
            PlaybackTick {
                reader,
                shared,
                state: PlaybackState::Idle,
                applied: StreamRequest::INITIAL,
            },
            PlaybackCounterReader { shared },
            PlaybackGate { shared },
        )
    }
}

impl Default for Playback {
    fn default() -> Self {
        Self::new()
    }
}

impl PlaybackUsb<'_> {
    pub fn select_alternate(&mut self, alternate: AlternateSetting) {
        self.publish(self.request.next(alternate));
    }

    pub fn reset_for_new_rate(&mut self) {
        self.publish(self.request.next(self.request.alternate()));
    }

    pub fn push_packet(&mut self, packet: &[Frame<StreamSample>]) -> Frames {
        if !self.accepts_packets() {
            return Frames::ZERO;
        }
        let stored = self.writer.push(packet);
        self.count_an_overrun_unless(stored, packet.len());
        stored
    }

    pub fn push_wire_words(&mut self, words: &[AtomicU32]) -> Frames {
        if !self.accepts_packets() {
            return Frames::ZERO;
        }
        let stored = self.writer.push_wire_words(words);
        self.count_an_overrun_unless(stored, words.len() / WORDS_PER_FRAME);
        stored
    }

    fn count_an_overrun_unless(&self, stored: Frames, offered: usize) {
        if (stored.count() as usize) < offered {
            self.shared.overruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn observe_sof(&mut self, dma: TxDmaPosition) -> Frames {
        let effective = self.writer.level().saturating_add(dma.frames_queued);
        if self.shared.tick_is_running.load(Ordering::Relaxed) {
            self.widen_fill_range(effective);
        }
        effective
    }

    pub fn tick_is_playing(&self) -> bool {
        self.shared.tick_is_running.load(Ordering::Relaxed)
    }

    fn widen_fill_range(&self, fill: Frames) {
        let cell = &self.shared.fill_range;
        let mut seen = cell.load(Ordering::Relaxed);
        while let Err(changed) = cell.compare_exchange_weak(
            seen,
            PackedFillRange(seen).widened_by(fill).0,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            seen = changed;
        }
    }

    fn publish(&mut self, request: StreamRequest) {
        self.request = request;
        self.shared.requested.store(request.0, Ordering::Release);
    }

    // WHY: `&` folds the gate byte into the compare the receive path already makes: 3 instructions and 5 to 6 cycles a packet in `cargo xtask budget`; `&&` costs 8 to 9 instructions and 12 to 15 cycles
    fn accepts_packets(&self) -> bool {
        self.request.alternate() == AlternateSetting::One
            && (self.shared.applied.load(Ordering::Acquire) == self.request.0)
                & self.shared.gate_is_open.load(Ordering::Relaxed)
    }
}

impl PlaybackTick<'_> {
    pub fn render(&mut self, frames: FramesPerTick, out: &mut [Frame<StreamSample>; MAX_FRAMES]) {
        match frames {
            FramesPerTick::Six => self.render_block::<6>(frames, out),
            FramesPerTick::Twelve => self.render_block::<12>(frames, out),
            FramesPerTick::TwentyFour => self.render_block::<24>(frames, out),
        }
    }

    fn render_block<const N: usize>(
        &mut self,
        frames: FramesPerTick,
        out: &mut [Frame<StreamSample>; MAX_FRAMES],
    ) {
        let Some(block) = out.first_chunk_mut::<N>() else {
            return;
        };
        let requested = StreamRequest(self.shared.requested.load(Ordering::Acquire));
        if requested != self.applied {
            self.apply(requested.event(), block);
            self.applied = requested;
            self.shared.applied.store(requested.0, Ordering::Release);
        }
        let buffered = self.reader.level();
        self.apply(
            PlaybackEvent::Tick {
                buffered,
                block: frames,
            },
            block,
        );
        self.shared
            .tick_is_running
            .store(self.state == PlaybackState::Running, Ordering::Relaxed);
    }

    fn apply<const N: usize>(
        &mut self,
        event: PlaybackEvent,
        block: &mut [Frame<StreamSample>; N],
    ) {
        let (next, command) = self.state.step(event);
        self.state = next;
        match command {
            PlaybackCommand::Hold => {}
            PlaybackCommand::DiscardRing => self.reader.discard_all(),
            PlaybackCommand::OutputSilence => *block = [SILENT_FRAME; N],
            PlaybackCommand::OutputBlock => {
                self.reader.pop_block(block);
            }
            PlaybackCommand::DropOldestThenOutputBlock { excess } => {
                self.reader.discard_oldest(excess);
                self.reader.pop_block(block);
            }
            PlaybackCommand::OutputWhatIsLeftThenSilenceAfterUnderrun => {
                self.pop_block_padded(block);
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
            PlaybackCommand::FadeOutOneBlockThenDiscardRing => {
                self.pop_block_padded(block);
                fade_out(block);
                self.reader.discard_all();
            }
        }
    }

    fn pop_block_padded<const N: usize>(&mut self, block: &mut [Frame<StreamSample>; N]) {
        let taken = self.reader.pop_block(block).count() as usize;
        for frame in block.iter_mut().skip(taken) {
            *frame = SILENT_FRAME;
        }
    }
}

impl PlaybackGate<'_> {
    pub fn open(&self) {
        self.shared.gate_is_open.store(true, Ordering::Relaxed);
    }
}

impl PlaybackCounterReader<'_> {
    pub fn take(&self) -> PlaybackCounters {
        let range = self
            .shared
            .fill_range
            .swap(PackedFillRange::EMPTY.0, Ordering::Relaxed);
        PlaybackCounters {
            underruns: self.shared.underruns.swap(0, Ordering::Relaxed),
            overruns: self.shared.overruns.swap(0, Ordering::Relaxed),
            effective_fill_while_running: PackedFillRange(range).unpack(),
        }
    }
}
