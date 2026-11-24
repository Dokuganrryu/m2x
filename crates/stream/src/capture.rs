use core::sync::atomic::{AtomicU32, Ordering};

use dsp::{Frame, Sample, SampleRate, StreamSample};

use crate::ring::RingFrames;
use crate::{
    AlternateSetting, CaptureCommand, CaptureEvent, CaptureState, Frames,
    MAX_CAPTURE_PACKET_FRAMES, RingReader,
};

pub type CapturePacket = [Frame<StreamSample>; MAX_CAPTURE_PACKET_FRAMES];

pub struct PacketFrames<'a> {
    ring: RingFrames<'a>,
    size: usize,
}

const SILENT_WIRE_WORD: u32 = StreamSample::SILENCE.to_bits() as u32;
const WORDS_PER_FRAME: usize = 2;

impl PacketFrames<'_> {
    pub fn store_as_wire_words(&self, words: &[AtomicU32]) -> Frames {
        let words = words.get(..self.size * WORDS_PER_FRAME).unwrap_or(words);
        let stored = self.ring.store_as_wire_words(words);
        for word in words.iter().skip(stored * WORDS_PER_FRAME) {
            word.store(SILENT_WIRE_WORD, Ordering::Relaxed);
        }
        Frames::new((words.len() / WORDS_PER_FRAME) as u32)
    }

    fn copy_into(&self, packet: &mut [Frame<StreamSample>]) -> Frames {
        let packet = packet.get_mut(..self.size).unwrap_or_default();
        let copied = self.ring.copy_into(packet);
        for frame in packet.iter_mut().skip(copied) {
            *frame = SILENT_FRAME;
        }
        Frames::new(packet.len() as u32)
    }
}

const SILENT_FRAME: Frame<StreamSample> = [StreamSample::SILENCE; 2];

pub struct CaptureUsb<'a> {
    reader: RingReader<'a>,
    state: CaptureState,
}

impl<'a> CaptureUsb<'a> {
    pub const fn new(reader: RingReader<'a>, rate: SampleRate) -> Self {
        Self {
            reader,
            state: CaptureState::Idle(rate),
        }
    }

    pub fn select_alternate(&mut self, alternate: AlternateSetting) {
        self.apply_control(CaptureEvent::AltSettingSelected(alternate));
    }

    pub fn reset_for_new_rate(&mut self, rate: SampleRate) {
        self.apply_control(CaptureEvent::ResetStream(rate));
    }

    pub fn fill_packet(
        &mut self,
        frames_received_by_dma: Frames,
        packet: &mut CapturePacket,
    ) -> Option<Frames> {
        self.send_packet(frames_received_by_dma, |frames| frames.copy_into(packet))
    }

    #[inline(always)]
    pub fn send_packet<R>(
        &mut self,
        frames_received_by_dma: Frames,
        write: impl FnOnce(PacketFrames<'_>) -> R,
    ) -> Option<R> {
        let fill = self.reader.level().saturating_add(frames_received_by_dma);
        let (next, command) = self.state.step(CaptureEvent::Microframe { fill });
        self.state = next;
        match command {
            CaptureCommand::SendNothing => None,
            CaptureCommand::DiscardRing => {
                self.reader.discard_all();
                None
            }
            CaptureCommand::SendSilence(size) => Some(self.reader.pop_through(0, |ring| {
                write(PacketFrames {
                    ring,
                    size: size.count() as usize,
                })
            })),
            CaptureCommand::SendRingFrames(size) => {
                let size = size.count() as usize;
                Some(
                    self.reader
                        .pop_through(size, |ring| write(PacketFrames { ring, size })),
                )
            }
        }
    }

    fn apply_control(&mut self, event: CaptureEvent) {
        let (next, command) = self.state.step(event);
        self.state = next;
        match command {
            CaptureCommand::DiscardRing => self.reader.discard_all(),
            CaptureCommand::SendNothing
            | CaptureCommand::SendSilence(_)
            | CaptureCommand::SendRingFrames(_) => {}
        }
    }
}
