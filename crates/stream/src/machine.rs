use dsp::FramesPerTick;

use crate::Frames;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Priming,
    Running,
    Fading,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackEvent {
    Started,
    Stopped,
    Tick {
        buffered: Frames,
        block: FramesPerTick,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackCommand {
    Hold,
    DiscardRing,
    OutputSilence,
    OutputBlock,
    DropOldestThenOutputBlock { excess: Frames },
    OutputWhatIsLeftThenSilenceAfterUnderrun,
    FadeOutOneBlockThenDiscardRing,
}

impl PlaybackState {
    pub const fn step(self, event: PlaybackEvent) -> (Self, PlaybackCommand) {
        use PlaybackCommand::*;
        use PlaybackState::*;
        match (self, event) {
            (Idle | Priming | Running | Fading, PlaybackEvent::Started) => (Priming, DiscardRing),
            (Idle | Priming, PlaybackEvent::Stopped) => (Idle, DiscardRing),
            (Running | Fading, PlaybackEvent::Stopped) => (Fading, Hold),
            (Idle, PlaybackEvent::Tick { .. }) => (Idle, OutputSilence),
            (Priming, PlaybackEvent::Tick { buffered, block }) => {
                let target = Frames::playback_target(block).count();
                if buffered.count() >= target {
                    (
                        Running,
                        DropOldestThenOutputBlock {
                            excess: Frames::new(buffered.count() - target),
                        },
                    )
                } else {
                    (Priming, OutputSilence)
                }
            }
            (Running, PlaybackEvent::Tick { buffered, block }) => {
                if buffered.count() >= Frames::in_one_tick(block).count() {
                    (Running, OutputBlock)
                } else {
                    (Priming, OutputWhatIsLeftThenSilenceAfterUnderrun)
                }
            }
            (Fading, PlaybackEvent::Tick { .. }) => (Idle, FadeOutOneBlockThenDiscardRing),
        }
    }
}
