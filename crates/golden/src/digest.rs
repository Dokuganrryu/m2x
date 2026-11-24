use dsp::{Frame, I2sWord, StreamSample};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    PlaybackLeft,
    PlaybackRight,
    CaptureLeft,
    CaptureRight,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickDigest([u32; 4]);

#[derive(Clone, Copy)]
pub(crate) struct Fnv1a(u32);

const FNV_OFFSET_BASIS: u32 = 0x811C_9DC5;
const FNV_PRIME: u32 = 0x0100_0193;

impl Channel {
    pub const ALL: [Self; 4] = [
        Self::PlaybackLeft,
        Self::PlaybackRight,
        Self::CaptureLeft,
        Self::CaptureRight,
    ];

    const fn column(self) -> usize {
        match self {
            Self::PlaybackLeft => 0,
            Self::PlaybackRight => 1,
            Self::CaptureLeft => 2,
            Self::CaptureRight => 3,
        }
    }
}

impl Fnv1a {
    pub(crate) const fn new() -> Self {
        Self(FNV_OFFSET_BASIS)
    }

    pub(crate) fn absorb(&mut self, word: i32) {
        for byte in word.to_le_bytes() {
            self.0 = (self.0 ^ u32::from(byte)).wrapping_mul(FNV_PRIME);
        }
    }

    pub(crate) const fn value(self) -> u32 {
        self.0
    }
}

impl TickDigest {
    pub(crate) fn of_tick(playback: &[Frame<I2sWord>], capture: &[Frame<StreamSample>]) -> Self {
        let mut columns = [Fnv1a::new(); 4];
        for [left, right] in playback {
            columns[Channel::PlaybackLeft.column()].absorb(left.to_bits());
            columns[Channel::PlaybackRight.column()].absorb(right.to_bits());
        }
        for [left, right] in capture {
            columns[Channel::CaptureLeft.column()].absorb(left.to_bits());
            columns[Channel::CaptureRight.column()].absorb(right.to_bits());
        }
        Self(columns.map(Fnv1a::value))
    }

    pub const fn of(self, channel: Channel) -> u32 {
        self.0[channel.column()]
    }

    pub const fn columns(self) -> [u32; 4] {
        self.0
    }
}
