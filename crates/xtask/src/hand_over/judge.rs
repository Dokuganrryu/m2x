use super::inherited::{
    INHERITED_ENABLED_CHANNELS, LIVE_RANGES, LiveRange, TRANSMIT_CHANNELS, channel_bits,
};
use super::trace::{Context, Event, HandOverState, Write};

const DMA0_ENABLESET0: u32 = 0x4008_2020;
const DMA0_ENABLECLR0: u32 = 0x4008_2028;
// WHY: CMSIS core_cm33.h SysTick CTRL TICKINT bit 1, ICSR PENDSTSET bit 26 and PENDSVSET bit 28
const SYST_CSR_TICKINT: u32 = 1 << 1;
const ICSR_PENDSTSET_OR_PENDSVSET: u32 = (1 << 26) | (1 << 28);
// WHY: SVD28 CTIMER0 TCR CEN bit 0, USBHSD DEVCMDSTAT DCON bit 16 and SYSCON AHBCLKCTRL2 USB1_DEV bit 5
const CTIMER0_TCR_CEN: u32 = 1;
const USBHSD_DEVCMDSTAT_DCON: u32 = 1 << 16;
const SYSCON_AHBCLKCTRL2_USB1_DEV: u32 = 1 << 5;
// WHY: SVD28 GPIO CLR0 at 0x4008E280 and SET0 at 0x4008E200 act on the bits written as ones, and W0_28 at 0x4008D070 drives P0_28 from any nonzero word
const GPIO_SET0: u32 = 0x4008_E200;
const GPIO_CLR0: u32 = 0x4008_E280;
const GPIO_W0_28: u32 = 0x4008_D070;
const P0_28: u32 = 1 << 28;
const PRIMASK_SET: u32 = 1;
const SRAM: core::ops::Range<u32> = 0x2000_0000..0x2004_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hazard {
    InterruptsUnmasked,
    DmaChannelsEnabled(u32),
    InterruptsEnabled([u32; 2]),
    InterruptsPending([u32; 2]),
    SysTickOrPendSvPending,
    SysTickInterruptEnabled,
    TickTimerCounting,
    UsbConnectedWithNoClock,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    LiveMemoryWritten {
        write: Write,
        range: LiveRange,
    },
    HandlerLeftHandOverLive {
        number: u8,
        handler: u32,
        hazards: Vec<Hazard>,
    },
    HandlerDidNotReturn {
        number: u8,
        handler: u32,
        pc: u32,
    },
    StartupStopped {
        pc: u32,
        reason: String,
    },
    EntryReachedWithHandOverLive {
        hazards: Vec<Hazard>,
    },
    RamWrittenWithOutputsConnected {
        write: Write,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub first_transmit_stop: Option<u64>,
    pub sram_writes_before_transmit_stop: usize,
    pub exceptions: Vec<(u8, u32)>,
    pub reached: Option<u64>,
    pub peripheral_writes: Vec<u32>,
}

pub fn hazards(state: &HandOverState) -> Vec<Hazard> {
    let mut found = Vec::new();
    if state.primask & PRIMASK_SET == 0 {
        found.push(Hazard::InterruptsUnmasked);
    }
    if state.dma0_enabled != 0 {
        found.push(Hazard::DmaChannelsEnabled(state.dma0_enabled));
    }
    if state.iser != [0, 0] {
        found.push(Hazard::InterruptsEnabled(state.iser));
    }
    if state.ispr != [0, 0] {
        found.push(Hazard::InterruptsPending(state.ispr));
    }
    if state.icsr & ICSR_PENDSTSET_OR_PENDSVSET != 0 {
        found.push(Hazard::SysTickOrPendSvPending);
    }
    if state.syst_csr & SYST_CSR_TICKINT != 0 {
        found.push(Hazard::SysTickInterruptEnabled);
    }
    if state.ctimer0_tcr & CTIMER0_TCR_CEN != 0 {
        found.push(Hazard::TickTimerCounting);
    }
    if state.usb_devcmdstat & USBHSD_DEVCMDSTAT_DCON != 0
        && state.usb1_dev_clock & SYSCON_AHBCLKCTRL2_USB1_DEV == 0
    {
        found.push(Hazard::UsbConnectedWithNoClock);
    }
    found
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputConnect {
    InheritedHigh,
    Low,
    High,
    Reported,
}

impl OutputConnect {
    fn follow(self, write: &Write) -> Self {
        let level = match write.address {
            GPIO_CLR0 if write.value & P0_28 != 0 => Some(OutputConnect::Low),
            GPIO_SET0 if write.value & P0_28 != 0 => Some(OutputConnect::High),
            GPIO_W0_28 if write.value == 0 => Some(OutputConnect::Low),
            GPIO_W0_28 => Some(OutputConnect::High),
            _ => None,
        };
        match (self, level) {
            (OutputConnect::Reported, _) => OutputConnect::Reported,
            (_, Some(level)) => level,
            (unchanged, None) => unchanged,
        }
    }

    const fn connected(self) -> bool {
        match self {
            OutputConnect::InheritedHigh | OutputConnect::High => true,
            OutputConnect::Low | OutputConnect::Reported => false,
        }
    }
}

struct Channels(u32);

impl Channels {
    fn follow(&mut self, write: &Write) {
        match write.address {
            DMA0_ENABLECLR0 => self.0 &= !write.value,
            DMA0_ENABLESET0 => self.0 |= write.value,
            _ => {}
        }
    }

    fn live_range_hit(&self, write: &Write) -> Option<LiveRange> {
        LIVE_RANGES.into_iter().find(|range| {
            self.0 & (1 << range.channel.0) != 0 && range.holds(write.address, write.size)
        })
    }
}

pub fn judge(events: &[Event]) -> (Vec<Finding>, Summary) {
    let mut findings = Vec::new();
    let mut summary = Summary::default();
    let mut channels = Channels(INHERITED_ENABLED_CHANNELS);
    let mut output_connect = OutputConnect::InheritedHigh;
    let mut open_exception: Option<(u8, u32)> = None;
    let mut awaiting_state: Option<Option<(u8, u32)>> = None;
    let transmit = channel_bits(&TRANSMIT_CHANNELS);
    for event in events {
        match event {
            Event::Write(write) => {
                if write.context != Context::After
                    && let Some(range) = channels.live_range_hit(write)
                {
                    findings.push(Finding::LiveMemoryWritten {
                        write: *write,
                        range,
                    });
                }
                if matches!(write.context, Context::Startup | Context::Exception(_))
                    && SRAM.contains(&write.address)
                    && output_connect.connected()
                {
                    findings.push(Finding::RamWrittenWithOutputsConnected { write: *write });
                    output_connect = OutputConnect::Reported;
                }
                output_connect = output_connect.follow(write);
                if summary.first_transmit_stop.is_none() {
                    if write.address == DMA0_ENABLECLR0 && write.value & transmit == transmit {
                        summary.first_transmit_stop = Some(write.index);
                    } else if SRAM.contains(&write.address) {
                        summary.sram_writes_before_transmit_stop += 1;
                    }
                }
                if !SRAM.contains(&write.address)
                    && !summary.peripheral_writes.contains(&write.address)
                {
                    summary.peripheral_writes.push(write.address);
                }
                channels.follow(write);
            }
            Event::Exception {
                number, handler, ..
            } => {
                open_exception = Some((*number, *handler));
                summary.exceptions.push((*number, *handler));
            }
            Event::Returned { .. } => awaiting_state = Some(open_exception.take()),
            Event::NoReturn { number, pc } => {
                let handler = open_exception.take().map_or(0, |(_, handler)| handler);
                findings.push(Finding::HandlerDidNotReturn {
                    number: *number,
                    handler,
                    pc: *pc,
                });
            }
            Event::Reached { index, .. } => {
                summary.reached = Some(*index);
                awaiting_state = Some(None);
            }
            Event::Stopped { pc, reason, .. } => findings.push(Finding::StartupStopped {
                pc: *pc,
                reason: reason.clone(),
            }),
            Event::State(state) => {
                let left = hazards(state);
                match awaiting_state.take() {
                    Some(Some((number, handler))) if !left.is_empty() => {
                        findings.push(Finding::HandlerLeftHandOverLive {
                            number,
                            handler,
                            hazards: left,
                        });
                    }
                    Some(None) if !left.is_empty() => {
                        findings.push(Finding::EntryReachedWithHandOverLive { hazards: left });
                    }
                    Some(_) | None => {}
                }
            }
        }
    }
    (findings, summary)
}
