use crate::price::{TickMode, UsbPath};
use dsp::FramesPerTick;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Monitoring {
    Off,
    BothInputs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Preset {
    FiveBands,
    Flat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Bypass,
    FadeToOnFirstTick,
    FadeToOn,
    On,
    FadeToBypassFirstTick,
    FadeToBypass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickRun {
    pub frames: FramesPerTick,
    pub monitoring: Monitoring,
    pub preset: Preset,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    BenchTick(TickRun),
    BenchUsb(FramesPerTick, UsbPath),
    ImageTick(Phase),
    ImageUsbIdle,
}

pub const FRAME_COUNTS: [FramesPerTick; 3] = [
    FramesPerTick::Six,
    FramesPerTick::Twelve,
    FramesPerTick::TwentyFour,
];

pub const SCENARIOS: [(Monitoring, Preset); 3] = [
    (Monitoring::Off, Preset::FiveBands),
    (Monitoring::BothInputs, Preset::FiveBands),
    (Monitoring::Off, Preset::Flat),
];

pub const PHASES: [Phase; 6] = [
    Phase::Bypass,
    Phase::FadeToOnFirstTick,
    Phase::FadeToOn,
    Phase::On,
    Phase::FadeToBypassFirstTick,
    Phase::FadeToBypass,
];

pub const USB_PATHS: [UsbPath; 4] = [
    UsbPath::Microframe,
    UsbPath::ControlTransfer,
    UsbPath::ConsoleReceived,
    UsbPath::StartOfFrameWithConsole,
];

impl Phase {
    pub const fn mode(self) -> TickMode {
        match self {
            Phase::Bypass => TickMode::Bypass,
            Phase::On => TickMode::On,
            Phase::FadeToOnFirstTick
            | Phase::FadeToOn
            | Phase::FadeToBypassFirstTick
            | Phase::FadeToBypass => TickMode::Fade,
        }
    }

    const fn word(self) -> &'static str {
        match self {
            Phase::Bypass => "bypass",
            Phase::FadeToOnFirstTick => "fade-to-on-first-tick",
            Phase::FadeToOn => "fade-to-on",
            Phase::On => "on",
            Phase::FadeToBypassFirstTick => "fade-to-bypass-first-tick",
            Phase::FadeToBypass => "fade-to-bypass",
        }
    }

    pub const fn describe(self) -> &'static str {
        match self {
            Phase::Bypass => "Bypass",
            Phase::FadeToOnFirstTick => "Fade to On, first tick",
            Phase::FadeToOn => "Fade to On",
            Phase::On => "On",
            Phase::FadeToBypassFirstTick => "Fade to Bypass, first tick",
            Phase::FadeToBypass => "Fade to Bypass",
        }
    }
}

impl Monitoring {
    const fn word(self) -> &'static str {
        match self {
            Monitoring::Off => "unmonitored",
            Monitoring::BothInputs => "monitored",
        }
    }
}

impl Preset {
    const fn word(self) -> &'static str {
        match self {
            Preset::FiveBands => "five-bands",
            Preset::Flat => "flat",
        }
    }
}

pub const fn usb_word(path: UsbPath) -> &'static str {
    match path {
        UsbPath::Microframe => "microframe",
        UsbPath::ControlTransfer => "control-transfer",
        UsbPath::ConsoleReceived => "console-received",
        UsbPath::StartOfFrameWithConsole => "start-of-frame-with-console",
    }
}

fn frames_word(frames: FramesPerTick) -> String {
    frames.count().to_string()
}

impl Run {
    pub fn label(self) -> String {
        match self {
            Run::BenchTick(run) => format!(
                "tick/{}/{}/{}/{}",
                frames_word(run.frames),
                run.monitoring.word(),
                run.preset.word(),
                run.phase.word()
            ),
            Run::BenchUsb(frames, path) => {
                format!("usb/{}/{}", frames_word(frames), usb_word(path))
            }
            Run::ImageTick(phase) => format!("image-tick/{}", phase.word()),
            Run::ImageUsbIdle => "image-usb/idle".to_string(),
        }
    }

    pub fn every() -> Vec<Run> {
        let mut runs = Vec::new();
        for frames in FRAME_COUNTS {
            for (monitoring, preset) in SCENARIOS {
                for phase in PHASES {
                    runs.push(Run::BenchTick(TickRun {
                        frames,
                        monitoring,
                        preset,
                        phase,
                    }));
                }
            }
            for path in USB_PATHS {
                runs.push(Run::BenchUsb(frames, path));
            }
        }
        runs.extend(PHASES.map(Run::ImageTick));
        runs.push(Run::ImageUsbIdle);
        runs
    }

    pub fn from_label(label: &str) -> Option<Run> {
        Run::every().into_iter().find(|run| run.label() == label)
    }
}
