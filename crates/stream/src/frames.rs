use dsp::FramesPerTick;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Frames(u32);

impl Frames {
    pub const ZERO: Self = Self(0);

    pub const fn new(count: u32) -> Self {
        Self(count)
    }

    pub const fn count(self) -> u32 {
        self.0
    }

    pub const fn in_one_tick(frames: FramesPerTick) -> Self {
        Self(frames.count() as u32)
    }

    pub const fn playback_target(frames: FramesPerTick) -> Self {
        Self(Self::in_one_tick(frames).0 * TARGET_FILL_IN_TICK_BLOCKS)
    }

    pub const fn capture_target(frames: FramesPerTick) -> Self {
        Self(Self::in_one_tick(frames).0 * TARGET_FILL_IN_TICK_BLOCKS)
    }

    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

const TARGET_FILL_IN_TICK_BLOCKS: u32 = 4;
