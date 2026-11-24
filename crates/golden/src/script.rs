use dsp::{DspMode, Gain, MonitorMix};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Dsp(DspMode),
    CaptureGains([Gain; 2]),
    Monitor(MonitorMix),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptCue {
    pub tick: u16,
    pub cue: Cue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Script {
    pub ticks: u16,
    pub cues: &'static [ScriptCue],
}

const HALF: Gain = Gain::from_q5_27(0x0400_0000);
const QUARTER: Gain = Gain::from_q5_27(0x0200_0000);
const MINUS_THREE_DECIBELS: Gain = Gain::from_q5_27(0x05A8_2799);

pub const STANDARD_SCRIPT: Script = Script {
    ticks: 128,
    cues: &[
        ScriptCue {
            tick: 16,
            cue: Cue::Dsp(DspMode::On),
        },
        ScriptCue {
            tick: 56,
            cue: Cue::CaptureGains([HALF; 2]),
        },
        ScriptCue {
            tick: 64,
            cue: Cue::Monitor(MonitorMix {
                playback: Gain::UNITY,
                inputs: [MINUS_THREE_DECIBELS; 2],
            }),
        },
        ScriptCue {
            tick: 80,
            cue: Cue::Dsp(DspMode::Bypass),
        },
        ScriptCue {
            tick: 96,
            cue: Cue::CaptureGains([HALF, QUARTER]),
        },
    ],
};
