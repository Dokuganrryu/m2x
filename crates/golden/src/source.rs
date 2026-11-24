use dsp::{Frame, I2sWord, StreamSample};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFrame {
    pub playback: Frame<StreamSample>,
    pub adc: Frame<I2sWord>,
}

pub trait FrameSource {
    fn next_frame(&mut self) -> SourceFrame;
}

#[derive(Clone, Copy, Debug)]
pub struct SeededNoise {
    state: u32,
    frame: u32,
}

const STREAM_PADDING_MASK: i32 = !0xFF;
const BURST_FRAMES_LOG2: u32 = 7;
const QUIET_BURST_ATTENUATION_SHIFT: u32 = 4;

impl SeededNoise {
    pub const fn seeded(seed: u32) -> Self {
        Self {
            state: seed | 1,
            frame: 0,
        }
    }

    fn next_word(&mut self) -> i32 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 17;
        self.state ^= self.state << 5;
        self.state as i32
    }

    const fn in_quiet_burst(&self) -> bool {
        (self.frame >> BURST_FRAMES_LOG2) & 1 == 1
    }

    fn shaped_word(&mut self) -> i32 {
        let word = self.next_word();
        if self.in_quiet_burst() {
            word >> QUIET_BURST_ATTENUATION_SHIFT
        } else {
            word
        }
    }
}

impl FrameSource for SeededNoise {
    fn next_frame(&mut self) -> SourceFrame {
        let playback =
            [(); 2].map(|_| StreamSample::from_bits(self.shaped_word() & STREAM_PADDING_MASK));
        let adc = [(); 2].map(|_| I2sWord::from_bits(self.shaped_word()));
        self.frame = self.frame.wrapping_add(1);
        SourceFrame { playback, adc }
    }
}
