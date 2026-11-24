use super::run::Phase;
use crate::price::Block;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribution {
    Dsp(Block),
    MonitoredKernel,
    TickContext,
    DmaMoves,
    Unattributed,
}

const MONITORED: &str = "WithMonitoredInputs";

const KERNELS: [(&str, Block); 10] = [
    ("dsp::capture::Capture", Block::CaptureRamp),
    ("dsp::biquad::filter_band", Block::OnPeq),
    ("dsp::biquad::FilterCascade", Block::OnPeq),
    ("dsp::playback::convert_into_intake", Block::OnIntake),
    (
        "dsp::playback::mix_into_intake",
        Block::OnPeqOffIntakeWithPeak,
    ),
    ("dsp::playback::mix_in_place", Block::OnLimiterPeak),
    ("dsp::playback::bypass_out", Block::BypassOut),
    (
        "dsp::playback::limited_out_refilling_line",
        Block::FadeOutOfOn,
    ),
    ("dsp::playback::limited_out::", Block::OnLimiterRamp),
    ("dsp::playback::blended_out", Block::FadeOut),
];

const SEED: &str = "dsp::playback::PlaybackPath>::start_processing";

const DISPATCH: [&str; 2] = [
    "<dsp::playback::PlaybackTick<",
    "<dsp::pipeline::Pipeline>::tick",
];

const DMA_MOVES: &str = "stream::transport::TickBlocks";

const TICK_CONTEXT: [&str; 8] = [
    "budget_tick",
    "audio_tick_interrupt",
    "stream::",
    "dsp::swap::CoefficientTick",
    "dsp::crossfade::",
    "dsp::handover::",
    "dsp::capture_targets::",
    "board::firmware::tick::",
];

pub fn attribute(function: &str, phase: Phase) -> Attribution {
    if let Some((_, block)) = KERNELS.iter().find(|(name, _)| function.contains(name)) {
        return if function.contains(MONITORED) {
            Attribution::MonitoredKernel
        } else {
            Attribution::Dsp(*block)
        };
    }
    if function.contains(SEED) {
        return Attribution::Dsp(Block::SeedOnEntry);
    }
    if DISPATCH.iter().any(|name| function.contains(name)) {
        return Attribution::Dsp(Block::Dispatch(phase.mode()));
    }
    if function.contains(DMA_MOVES) {
        return Attribution::DmaMoves;
    }
    if TICK_CONTEXT.iter().any(|name| function.contains(name)) {
        return Attribution::TickContext;
    }
    Attribution::Unattributed
}
