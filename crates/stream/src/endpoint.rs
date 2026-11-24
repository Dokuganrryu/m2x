use core::sync::atomic::AtomicU32;

use dsp::{Frame, SampleRate, StreamSample};

use crate::{
    AlternateSetting, FeedbackEvent, FeedbackState, FeedbackValue, Frames, PlaybackUsb,
    TxDmaPosition,
};

pub struct PlaybackEndpoint<'a> {
    usb: PlaybackUsb<'a>,
    feedback: FeedbackState,
    tick_was_playing: bool,
}

impl<'a> PlaybackEndpoint<'a> {
    pub const fn new(usb: PlaybackUsb<'a>, rate: SampleRate) -> Self {
        Self {
            usb,
            feedback: FeedbackState::Stopped { rate },
            tick_was_playing: false,
        }
    }

    pub fn select_alternate(&mut self, alternate: AlternateSetting) {
        self.usb.select_alternate(alternate);
        self.feed(match alternate {
            AlternateSetting::Zero => FeedbackEvent::Stopped,
            AlternateSetting::One => FeedbackEvent::Started,
        });
    }

    pub fn reset_for_new_rate(&mut self, rate: SampleRate) {
        self.usb.reset_for_new_rate();
        self.feed(FeedbackEvent::ResetStream { rate });
    }

    pub fn push_packet(&mut self, packet: &[Frame<StreamSample>]) -> Frames {
        self.usb.push_packet(packet)
    }

    pub fn push_wire_words(&mut self, words: &[AtomicU32]) -> Frames {
        self.usb.push_wire_words(words)
    }

    #[inline(always)]
    pub fn on_sof(&mut self, dma: TxDmaPosition) -> FeedbackValue {
        let effective_fill = self.usb.observe_sof(dma);
        let playing = self.usb.tick_is_playing();
        match (self.tick_was_playing, playing) {
            (false, true) => self.feed(FeedbackEvent::PrimingDone),
            (true, false) => self.feed(FeedbackEvent::Underran),
            (false, false) | (true, true) => {}
        }
        self.tick_was_playing = playing;
        let (next, value) = self.feedback.step(FeedbackEvent::Sof { effective_fill });
        self.feedback = next;
        value
    }

    fn feed(&mut self, event: FeedbackEvent) {
        self.feedback = self.feedback.step(event).0;
    }
}
