use super::cycles::ProxyCycles;
use dsp::FramesPerTick;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickMode {
    Bypass,
    On,
    Fade,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UsbPath {
    Microframe,
    ControlTransfer,
    ConsoleReceived,
    StartOfFrameWithConsole,
}

impl UsbPath {
    pub const EVERY_MICROFRAME: [UsbPath; 1] = [UsbPath::Microframe];

    pub const OCCASIONAL: [UsbPath; 3] = [
        UsbPath::ControlTransfer,
        UsbPath::ConsoleReceived,
        UsbPath::StartOfFrameWithConsole,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Block {
    TickEntryAndExit,
    DmaMoves,
    TickContext,
    CaptureRamp,
    BypassOut,
    OnIntake,
    OnPeq,
    OnLimiterPeak,
    OnPeqOffIntakeWithPeak,
    OnLimiterRamp,
    FadeOut,
    FadeOutOfOn,
    SeedOnEntry,
    Dispatch(TickMode),
    MonitorMix(TickMode),
    DspWorstTick,
    TickAndUsb,
    Usb(UsbPath),
    UsbInterrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Share {
    Cap(ProxyCycles),
    OffAtThisFrameCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Basis {
    Published,
    Measured,
    MeasuredDifference,
    Cap,
}

const fn caps(six: u32, twelve: u32, twenty_four: u32) -> [Share; 3] {
    [
        Share::Cap(ProxyCycles::new(six)),
        Share::Cap(ProxyCycles::new(twelve)),
        Share::Cap(ProxyCycles::new(twenty_four)),
    ]
}

const fn off_at_twenty_four(six: u32, twelve: u32) -> [Share; 3] {
    [
        Share::Cap(ProxyCycles::new(six)),
        Share::Cap(ProxyCycles::new(twelve)),
        Share::OffAtThisFrameCount,
    ]
}

pub const DSP_SHARE_CAP: u32 = 6_250;
pub const TICK_AND_USB_CAP: u32 = 8_750;
pub const USB_INTERRUPT_CAPS: [u32; 3] = [1_500, 1_600, 1_800];

impl Block {
    pub const ALL: [Block; 26] = [
        Block::TickEntryAndExit,
        Block::DmaMoves,
        Block::TickContext,
        Block::CaptureRamp,
        Block::BypassOut,
        Block::OnIntake,
        Block::OnPeq,
        Block::OnLimiterPeak,
        Block::OnPeqOffIntakeWithPeak,
        Block::OnLimiterRamp,
        Block::FadeOut,
        Block::FadeOutOfOn,
        Block::SeedOnEntry,
        Block::Dispatch(TickMode::Bypass),
        Block::Dispatch(TickMode::On),
        Block::Dispatch(TickMode::Fade),
        Block::MonitorMix(TickMode::Bypass),
        Block::MonitorMix(TickMode::On),
        Block::MonitorMix(TickMode::Fade),
        Block::DspWorstTick,
        Block::TickAndUsb,
        Block::Usb(UsbPath::Microframe),
        Block::UsbInterrupt,
        Block::Usb(UsbPath::ControlTransfer),
        Block::Usb(UsbPath::ConsoleReceived),
        Block::Usb(UsbPath::StartOfFrameWithConsole),
    ];

    pub const fn share(self, frames: FramesPerTick) -> Share {
        let column = match frames {
            FramesPerTick::Six => 0,
            FramesPerTick::Twelve => 1,
            FramesPerTick::TwentyFour => 2,
        };
        self.d3_row()[column]
    }

    const fn d3_row(self) -> [Share; 3] {
        match self {
            Block::TickEntryAndExit => caps(22, 22, 22),
            Block::DmaMoves => caps(241, 369, 606),
            Block::TickContext => caps(499, 640, 889),
            Block::CaptureRamp => caps(278, 494, 936),
            Block::BypassOut => caps(186, 302, 758),
            Block::OnIntake => off_at_twenty_four(67, 118),
            Block::OnPeq => off_at_twenty_four(1_999, 2_879),
            Block::OnLimiterPeak => off_at_twenty_four(82, 152),
            Block::OnPeqOffIntakeWithPeak => caps(114, 210, 466),
            Block::OnLimiterRamp => caps(193, 411, 735),
            Block::FadeOut => caps(541, 985, 1_873),
            Block::FadeOutOfOn => caps(242, 624, 1_146),
            Block::SeedOnEntry => caps(325, 391, 523),
            Block::Dispatch(TickMode::Bypass) => caps(51, 51, 51),
            Block::Dispatch(TickMode::On) => caps(135, 135, 121),
            Block::Dispatch(TickMode::Fade) => caps(159, 159, 145),
            Block::MonitorMix(TickMode::Bypass) => caps(301, 551, 827),
            Block::MonitorMix(TickMode::On) => caps(248, 451, 936),
            Block::MonitorMix(TickMode::Fade) => caps(586, 998, 1_019),
            Block::DspWorstTick => caps(DSP_SHARE_CAP, DSP_SHARE_CAP, DSP_SHARE_CAP),
            Block::TickAndUsb => caps(TICK_AND_USB_CAP, TICK_AND_USB_CAP, TICK_AND_USB_CAP),
            Block::Usb(UsbPath::Microframe) => caps(1_414, 1_510, 1_738),
            Block::Usb(UsbPath::ControlTransfer) => caps(3_437, 3_437, 3_437),
            Block::Usb(UsbPath::ConsoleReceived) => caps(9_226, 9_226, 9_226),
            Block::Usb(UsbPath::StartOfFrameWithConsole) => caps(6_420, 6_524, 6_762),
            Block::UsbInterrupt => caps(
                USB_INTERRUPT_CAPS[0],
                USB_INTERRUPT_CAPS[1],
                USB_INTERRUPT_CAPS[2],
            ),
        }
    }

    pub const fn basis(self) -> Basis {
        match self {
            Block::TickEntryAndExit => Basis::Published,
            Block::MonitorMix(_) => Basis::MeasuredDifference,
            Block::DspWorstTick | Block::TickAndUsb | Block::UsbInterrupt => Basis::Cap,
            Block::DmaMoves
            | Block::TickContext
            | Block::CaptureRamp
            | Block::BypassOut
            | Block::OnIntake
            | Block::OnPeq
            | Block::OnLimiterPeak
            | Block::OnPeqOffIntakeWithPeak
            | Block::OnLimiterRamp
            | Block::FadeOut
            | Block::FadeOutOfOn
            | Block::SeedOnEntry
            | Block::Dispatch(_)
            | Block::Usb(_) => Basis::Measured,
        }
    }

    pub const fn id(self) -> &'static str {
        match self {
            Block::TickEntryAndExit => "tick-entry-exit",
            Block::DmaMoves => "dma-moves",
            Block::TickContext => "tick-context",
            Block::CaptureRamp => "capture-ramp",
            Block::BypassOut => "bypass-out",
            Block::OnIntake => "on-intake",
            Block::OnPeq => "on-peq",
            Block::OnLimiterPeak => "on-limiter-peak",
            Block::OnPeqOffIntakeWithPeak => "on-peq-off-intake",
            Block::OnLimiterRamp => "on-limiter-ramp",
            Block::FadeOut => "fade-out",
            Block::FadeOutOfOn => "fade-out-of-on",
            Block::SeedOnEntry => "seed-on-entry",
            Block::Dispatch(TickMode::Bypass) => "dispatch-bypass",
            Block::Dispatch(TickMode::On) => "dispatch-on",
            Block::Dispatch(TickMode::Fade) => "dispatch-fade",
            Block::MonitorMix(TickMode::Bypass) => "monitor-mix-bypass",
            Block::MonitorMix(TickMode::On) => "monitor-mix-on",
            Block::MonitorMix(TickMode::Fade) => "monitor-mix-fade",
            Block::DspWorstTick => "dsp-worst-tick",
            Block::TickAndUsb => "tick-and-usb",
            Block::Usb(UsbPath::Microframe) => "usb-microframe",
            Block::Usb(UsbPath::ControlTransfer) => "usb-control-transfer",
            Block::Usb(UsbPath::ConsoleReceived) => "usb-console-received",
            Block::Usb(UsbPath::StartOfFrameWithConsole) => "usb-start-of-frame-with-console",
            Block::UsbInterrupt => "usb-interrupt",
        }
    }

    pub const fn d3_label(self) -> &'static str {
        match self {
            Block::TickEntryAndExit => "Tick entry and exit, exception stacking",
            Block::DmaMoves => {
                "DMA buffer moves. The received block out of the FC6 buffer, the playback block into the FC2 and FC0 buffers"
            }
            Block::TickContext => {
                "Tick context. Crossfade step, coefficient adopt, playback ring render, capture ring push, the call into the pipeline"
            }
            Block::CaptureRamp => {
                "Capture digital gain ramp per input, input mute and input meter, one pass per input"
            }
            Block::BypassOut => "Bypass. One-tick bypass line and output meter, one fused pass",
            Block::OnIntake => "On. Stream to Q2.29 into the limiter intake",
            Block::OnPeq => "On. PEQ, 5 bands x 2 channels",
            Block::OnLimiterPeak => "On. Limiter peak scan after the PEQ",
            Block::OnPeqOffIntakeWithPeak => {
                "On, PEQ off. Stream to Q2.29 and limiter peak, one fused pass"
            }
            Block::OnLimiterRamp => {
                "On. Limiter ramp writing the requantised output word, and the output meter, one fused pass"
            }
            Block::FadeOut => {
                "Fade. Limiter ramp, crossfade blend, bypass line and output meter, one fused pass, in place of the row above"
            }
            Block::FadeOutOfOn => {
                "First fade tick out of On. Limiter ramp refilling the bypass line, in place of the row above"
            }
            Block::SeedOnEntry => {
                "First fade tick into On. Seed of the lookahead pair and PEQ history from the bypass line"
            }
            Block::Dispatch(TickMode::Bypass) => "Dispatch, limiter gain, meter cells. Bypass",
            Block::Dispatch(TickMode::On) => "Dispatch, limiter gain, meter cells. On",
            Block::Dispatch(TickMode::Fade) => "Dispatch, limiter gain, meter cells. Fade",
            Block::MonitorMix(TickMode::Bypass) => "Monitor mix, both inputs monitored. Bypass",
            Block::MonitorMix(TickMode::On) => "Monitor mix, both inputs monitored. On",
            Block::MonitorMix(TickMode::Fade) => "Monitor mix, both inputs monitored. Fade",
            Block::DspWorstTick => "**DSP share, worst tick, cap**",
            Block::TickAndUsb => "**Tick total plus the USB interrupt, cap 70%**",
            Block::Usb(UsbPath::Microframe) => {
                "USB. The one interrupt per microframe, at start of frame, decoded by the board and run through `Service::handle`: the playback packet into the ring and re-armed, feedback and capture completions, feedback update and re-arm, capture fill and arm"
            }
            Block::UsbInterrupt => {
                "**USB interrupt, cap per microframe**. Entry and exit plus the USB row above"
            }
            Block::Usb(UsbPath::ControlTransfer) => {
                "USB, not every microframe. Control transfer, worst single event"
            }
            Block::Usb(UsbPath::ConsoleReceived) => {
                "USB, not every microframe. Console bulk OUT packet of 512 bytes into the console pipe"
            }
            Block::Usb(UsbPath::StartOfFrameWithConsole) => {
                "USB, not every microframe. Start of frame that also fills a 511-byte console IN packet"
            }
        }
    }

    pub fn from_id(id: &str) -> Option<Block> {
        Block::ALL.into_iter().find(|block| block.id() == id)
    }
}
