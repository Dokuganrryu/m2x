use std::collections::BTreeSet;

use device::adc::AdcAddress;
use device::analog_word::{AnalogWord, SHIFT_BYTE};
use device::boot::{ADC_WAKE, BOOT_RATE, PROBE_SPACING, RAIL_SETTLE, RAIL_STAGGER, VolumeTargets};
use device::breadcrumb::{Breadcrumb, MAGIC, Place, Weight};
use device::dac::{Attenuation, DAC_ADDRESS, MasterTrim, MasterTrimByte, VolumeRegister};
use device::expander::EXPANDER_ADDRESS;
use device::fault::FaultCode;
use device::knob::{KnobReading, KnobReadings, Knobs};
use device::knob_law::volume_targets;
use device::output_pin::OutputPin;
use device::phantom::Milliseconds;
use device::pin::{PHANTOM_ACTIVE_PIN, Pin};
use device::pll::{CORE_PLL_STABILISATION, audio_pll_stabilisation};
use device::rate_change::DAC_CLOCK_SETTLE;
use device::supervisor::WATCHDOG_TIMEOUT;
use dsp::SampleRate;

use super::trace::{
    Access, Ending, Entry, Event, I2cStatus, I2cTransaction, Level, Nanoseconds, Trace,
};

// WHY: Phantom 1 is bit 14 and phantom 2 bit 6 of the 16-bit analog word, as device::analog_word builds it
const PHANTOM_BITS: u32 = (1 << 14) | (1 << 6);
// WHY: The I2S transmit DMA channels are 5 (FC0 TX) and 11 (FC2 TX)
const TRANSMIT_DMA_CHANNELS: u32 = (1 << 5) | (1 << 11);
// WHY: Stopping the audio, B1 clears channels 5, 11, 16 and 18
const EVERY_AUDIO_DMA_CHANNEL: u32 = 0x5_0820;
// WHY: The watchdog clock is FRO 1 MHz +-15%, so a timeout can come at 85% of nominal
const WATCHDOG_CLOCK_FAST_EDGE_PERCENT: u64 = 85;
// WHY: SVD28, I2S CFG1 bit 0 MAINENABLE, CTIMER TCR bit 0 CEN
const ENABLE_BIT: u32 = 1;
// WHY: The factory page magic at 0x38200 and ten serial bytes at 0x38204 are the only flash the image may read
const FACTORY_READ: (u32, u32) = (0x0003_8200, 0x0003_820E);
// WHY: ES9026 registers 0 and 62 are never written
const FORBIDDEN_DAC_REGISTERS: [u8; 2] = [0, 62];
const I2S_FLEXCOMMS: [&str; 4] = ["FC7.CFG1", "FC6.CFG1", "FC2.CFG1", "FC0.CFG1"];
// WHY: SVD28 AHBCLKCTRL1 FC7 bit 18, FC6 bit 17, FC2 bit 13, FC0 bit 11; the quiesce leaves a Flexcomm whose clock is off alone, since it cannot run, and the model starts with them off
const I2S_FLEXCOMM_CLOCKS: [u32; 4] = [1 << 18, 1 << 17, 1 << 13, 1 << 11];

fn flexcomm_clocked_by(entries: &[Entry], register: &str, until: usize) -> bool {
    let Some(at) = I2S_FLEXCOMMS.iter().position(|name| *name == register) else {
        return true;
    };
    let bit = I2S_FLEXCOMM_CLOCKS[at];
    entries[..=until.min(entries.len().saturating_sub(1))]
        .iter()
        .any(|entry| {
            is_write(&entry.event, "SYSCON.AHBCLKCTRLSET1", |value| {
                value & bit != 0
            })
        })
}
const LATCHED_WORD_BITS: u32 = 24;
const WWDT_MOD_WDEN: u32 = 1;
const WWDT_LONGEST_TIMEOUT: u32 = 0xFF_FFFF;
const MODEL_KNOB_READING: KnobReading = KnobReading(0x8000);
const USB1_RAM_FIRST_BYTE: u32 = 0x4010_0000;
const CRYSTAL_AND_USB_PHY_POWER: u32 = 0x0014_11C0;
// WHY: SVD28 XO32M_CTRL ENABLE_PLL_USB_OUT
const XO32M_CTRL_ENABLE_PLL_USB_OUT: u32 = 1 << 23;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    AllOffWordFirst,
    PhantomNeverRaised,
    PreampWordsWhole,
    DacAttenuatedBeforeOutput,
    MasterTrimAtMostMinusOneDb,
    DacRegistersZeroAndSixtyTwoUntouched,
    FlashUntouched,
    FenceNeverTripped,
    FenceProgrammed,
    PllLockReadBeforeClockUse,
    WatchdogNeverEnabled,
    WatchdogNeverStarved,
    WatchdogLongestTimeoutBeforeFirstFeed,
    UsbConnectsBeforeTheRails,
    BootOrderD9,
    PanelAfterFault,
    EndState,
    QuietAtRest,
    ConverterBusQuietAfterFault,
    SilencedBeforeTheFault,
    RampAtMostKnobTarget,
    HostRateChangeInOrder,
    MainClockStaysPut,
    ConverterBusConfiguredBeforeItsFirstTransfer,
    UsbListStartSetBeforeConnect,
    UsbPhyPoweredBeforeTheClockWaits,
    UsbPhyPllLockReadBeforeConnect,
    UsbDeviceRegistersAfterThePhy,
    LcdStartsItsOwnClockSource,
}

impl Check {
    pub const ALL: [Check; 29] = [
        Check::AllOffWordFirst,
        Check::PhantomNeverRaised,
        Check::PreampWordsWhole,
        Check::DacAttenuatedBeforeOutput,
        Check::MasterTrimAtMostMinusOneDb,
        Check::DacRegistersZeroAndSixtyTwoUntouched,
        Check::FlashUntouched,
        Check::FenceNeverTripped,
        Check::FenceProgrammed,
        Check::PllLockReadBeforeClockUse,
        Check::WatchdogNeverEnabled,
        Check::WatchdogNeverStarved,
        Check::WatchdogLongestTimeoutBeforeFirstFeed,
        Check::UsbConnectsBeforeTheRails,
        Check::BootOrderD9,
        Check::PanelAfterFault,
        Check::EndState,
        Check::QuietAtRest,
        Check::ConverterBusQuietAfterFault,
        Check::SilencedBeforeTheFault,
        Check::RampAtMostKnobTarget,
        Check::HostRateChangeInOrder,
        Check::MainClockStaysPut,
        Check::ConverterBusConfiguredBeforeItsFirstTransfer,
        Check::UsbListStartSetBeforeConnect,
        Check::UsbPhyPoweredBeforeTheClockWaits,
        Check::UsbPhyPllLockReadBeforeConnect,
        Check::UsbDeviceRegistersAfterThePhy,
        Check::LcdStartsItsOwnClockSource,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Check::AllOffWordFirst => "all-off word first",
            Check::PhantomNeverRaised => "phantom never raised",
            Check::PreampWordsWhole => "preamp words whole",
            Check::DacAttenuatedBeforeOutput => "P0_28 only after B8 attenuation with MCLK",
            Check::MasterTrimAtMostMinusOneDb => "master trim at most -1.00 dB",
            Check::DacRegistersZeroAndSixtyTwoUntouched => "DAC registers 0 and 62 untouched",
            Check::FlashUntouched => "flash untouched",
            Check::FenceNeverTripped => "fence never tripped",
            Check::FenceProgrammed => "fence programmed",
            Check::PllLockReadBeforeClockUse => "PLL lock read before the clock that uses it",
            Check::WatchdogNeverEnabled => "watchdog never enabled",
            Check::WatchdogNeverStarved => "watchdog never starved",
            Check::WatchdogLongestTimeoutBeforeFirstFeed => {
                "longest watchdog timeout before the first feed"
            }
            Check::UsbConnectsBeforeTheRails => "USB connects before the rails rise",
            Check::BootOrderD9 => "boot order D9",
            Check::PanelAfterFault => "panel and USB after the fault",
            Check::EndState => "end state",
            Check::QuietAtRest => "quiet at rest",
            Check::ConverterBusQuietAfterFault => "converter bus quiet after the fault",
            Check::SilencedBeforeTheFault => "tick, DMA and I2S stopped before the fault code",
            Check::RampAtMostKnobTarget => "B12 ramp never louder than the knob law target",
            Check::HostRateChangeInOrder => {
                "host rate change holds, stops, waits for the PLL and settles in order"
            }
            Check::MainClockStaysPut => "the main clock stays put on a failed core PLL",
            Check::ConverterBusConfiguredBeforeItsFirstTransfer => {
                "the converter bus is configured before its first transfer"
            }
            Check::UsbListStartSetBeforeConnect => {
                "the attach writes USB1.EPLISTSTART with 0x40100000 before the DEVCMDSTAT connect write"
            }
            Check::UsbPhyPoweredBeforeTheClockWaits => {
                "the USB PHY is powered before the clock waits"
            }
            Check::UsbPhyPllLockReadBeforeConnect => {
                "the attach reads the USB PHY PLL lock after USBPHY.CTRL_SET and before the connect"
            }
            Check::UsbDeviceRegistersAfterThePhy => {
                "the attach touches no USB1 device register before PORTMODE selects the device and the PHY writes end"
            }
            Check::LcdStartsItsOwnClockSource => {
                "the LCD set-up writes FRO192M_CTRL and FROHFDIV after its FC4 clock-gate write and before FCCLKSEL4"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterFault {
    None,
    UsbOnly,
    PanelOnly,
    PanelAndUsb,
    AudioPlaying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Silence {
    NotRequired,
    TransmitDmaOnly,
    WordAndTransmitDma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalogWords {
    Latched,
    NeverLatched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateChange {
    Ordered,
    NeverRestarts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainClock {
    MaySwitch,
    MustStayPut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedEnding {
    LoopRunsOn,
    Parked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceProbe {
    pub address: u32,
    pub access: Access,
    pub allowed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub code: FaultCode,
    pub weight: Weight,
    pub place: Place,
}

impl Recorded {
    pub const fn raised(code: FaultCode, stage: u8, step: u8) -> Self {
        Self {
            code,
            weight: Weight::Raised,
            place: Place { stage, step },
        }
    }

    pub const fn noted(code: FaultCode, stage: u8, step: u8) -> Self {
        Self {
            code,
            weight: Weight::Noted,
            place: Place { stage, step },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expectation {
    pub ending: ExpectedEnding,
    pub record: Option<Recorded>,
    pub quiet_after: Option<Nanoseconds>,
    pub fence: &'static [FenceProbe],
    pub planted_store: Option<u32>,
    pub after_fault: AfterFault,
    pub silence: Silence,
    pub analog_words: AnalogWords,
    pub rate_change: RateChange,
    pub main_clock: MainClock,
}

impl Expectation {
    pub const fn fault(&self) -> Option<FaultCode> {
        match self.record {
            Some(Recorded {
                code,
                weight: Weight::Raised,
                ..
            }) => Some(code),
            Some(Recorded {
                weight: Weight::Noted,
                ..
            })
            | None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParkFunction {
    pub name: &'static str,
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub check: Check,
    pub failure: Option<String>,
}

type Outcome = Result<(), String>;
type EventTest = Box<dyn Fn(&Event) -> bool>;

pub fn judge(trace: &Trace, expectation: &Expectation, park: &[ParkFunction]) -> Vec<Verdict> {
    let entries = trace.entries.as_slice();
    Check::ALL
        .into_iter()
        .map(|check| Verdict {
            check,
            failure: run(check, entries, expectation, park).err(),
        })
        .collect()
}

fn run(
    check: Check,
    entries: &[Entry],
    expectation: &Expectation,
    park: &[ParkFunction],
) -> Outcome {
    match check {
        Check::AllOffWordFirst => all_off_word_first(entries, expectation.analog_words),
        Check::PhantomNeverRaised => phantom_never_raised(entries),
        Check::PreampWordsWhole => preamp_words_whole(entries),
        Check::DacAttenuatedBeforeOutput => dac_attenuated_before_output(entries),
        Check::MasterTrimAtMostMinusOneDb => master_trim_at_most_minus_one_db(entries),
        Check::DacRegistersZeroAndSixtyTwoUntouched => dac_registers_zero_and_sixty_two(entries),
        Check::FlashUntouched => flash_untouched(entries, expectation.planted_store),
        Check::FenceNeverTripped => fence_never_tripped(entries, expectation.planted_store),
        Check::FenceProgrammed => fence_programmed(entries, expectation.fence),
        Check::PllLockReadBeforeClockUse => pll_lock_read_before_clock_use(entries),
        Check::WatchdogNeverEnabled => watchdog_never_enabled(entries),
        Check::WatchdogNeverStarved => watchdog_never_starved(entries),
        Check::WatchdogLongestTimeoutBeforeFirstFeed => {
            longest_timeout_before_the_first_feed(entries)
        }
        Check::UsbConnectsBeforeTheRails => usb_connects_before_the_rails(entries, expectation),
        Check::UsbListStartSetBeforeConnect => usb_list_start_set_before_each_connect(entries),
        Check::UsbPhyPoweredBeforeTheClockWaits => usb_phy_powered_before_the_clock_waits(entries),
        Check::UsbPhyPllLockReadBeforeConnect => usb_phy_pll_lock_read_before_each_connect(entries),
        Check::UsbDeviceRegistersAfterThePhy => usb_device_registers_after_the_phy(entries),
        Check::LcdStartsItsOwnClockSource => lcd_starts_its_own_clock_source(entries),
        Check::BootOrderD9 => boot_order(entries),
        Check::PanelAfterFault => panel_after_fault(entries, expectation),
        Check::EndState => end_state(entries, expectation, park),
        Check::QuietAtRest => quiet_at_rest(entries, expectation.quiet_after),
        Check::ConverterBusQuietAfterFault => converter_bus_quiet_after_fault(entries, expectation),
        Check::SilencedBeforeTheFault => silenced_before_the_fault(entries, expectation),
        Check::RampAtMostKnobTarget => ramp_at_most_knob_target(entries, expectation),
        Check::HostRateChangeInOrder => host_rate_changes_in_order(entries, expectation),
        Check::MainClockStaysPut => main_clock_stays_put(entries, expectation),
        Check::ConverterBusConfiguredBeforeItsFirstTransfer => {
            converter_bus_configured_before_its_first_transfer(entries)
        }
    }
}

fn all_off_word() -> u32 {
    (u32::from(AnalogWord::ALL_OFF.bits()) << 8) | u32::from(SHIFT_BYTE)
}

fn first(entries: &[Entry], wanted: impl Fn(&Event) -> bool) -> Option<&Entry> {
    entries.iter().find(|entry| wanted(&entry.event))
}

fn position(entries: &[Entry], wanted: impl Fn(&Event) -> bool) -> Option<usize> {
    entries.iter().position(|entry| wanted(&entry.event))
}

fn last(entries: &[Entry], wanted: impl Fn(&Event) -> bool) -> Option<&Entry> {
    entries.iter().rev().find(|entry| wanted(&entry.event))
}

fn is_pin(event: &Event, pin: Pin, level: Level) -> bool {
    matches!(event, Event::Pin { port, number, level: seen }
        if *port == pin.port && *number == pin.number && *seen == level)
}

fn is_output(event: &Event, output: OutputPin, level: Level) -> bool {
    is_pin(event, output.pin(), level)
}

fn is_write(event: &Event, register: &str, accept: impl Fn(u32) -> bool) -> bool {
    matches!(event, Event::Write { register: name, value } if name == register && accept(*value))
}

fn is_preamp_word(event: &Event) -> bool {
    matches!(event, Event::PreampWord { .. })
}

fn is_all_off(event: &Event) -> bool {
    matches!(event, Event::PreampWord { bits, value } if *bits == LATCHED_WORD_BITS && *value == all_off_word())
}

fn is_i2c_to(event: &Event, peer: u8) -> bool {
    matches!(event, Event::I2c(transaction) if transaction.address() == peer)
}

fn dac_write(event: &Event) -> Option<(u8, u8, bool)> {
    match event {
        Event::I2c(I2cTransaction::Write {
            address,
            bytes,
            status,
        }) if *address == DAC_ADDRESS && bytes.len() >= 2 => {
            Some((bytes[0], bytes[1], *status == I2cStatus::Acked))
        }
        _ => None,
    }
}

const RAIL_OUTPUTS: [OutputPin; 5] = [
    OutputPin::FirstBringUp,
    OutputPin::SecondBringUp,
    OutputPin::ThirdBringUp,
    OutputPin::ConverterEnable,
    OutputPin::AdcEnable,
];

fn reaches_the_analog_chain(event: &Event) -> bool {
    RAIL_OUTPUTS
        .iter()
        .any(|output| is_output(event, *output, Level::High))
        || matches!(event, Event::I2c(_))
        || matches!(event, Event::Write { register, .. }
            if register.starts_with("SYSCON.PLL0") || register.starts_with("SYSCON.MCLK"))
}

fn all_off_word_first(entries: &[Entry], analog_words: AnalogWords) -> Outcome {
    let step = first(entries, reaches_the_analog_chain);
    let Some(word) = first(entries, is_preamp_word) else {
        return match (analog_words, step) {
            (AnalogWords::Latched, _) => Err("no preamp word was ever latched".into()),
            (AnalogWords::NeverLatched, None) => Ok(()),
            (AnalogWords::NeverLatched, Some(step)) => Err(format!(
                "{:?} at {} reaches the analog chain and no all-off word was ever latched",
                step.event, step.at
            )),
        };
    };
    if !is_all_off(&word.event) {
        return Err(format!(
            "the first latched preamp word, at {}, is {:?}, not the all-off word",
            word.at, word.event
        ));
    }
    match step {
        Some(step) if step.at < word.at => Err(format!(
            "{:?} at {} comes before the first all-off word at {}",
            step.event, step.at, word.at
        )),
        _ => Ok(()),
    }
}

fn phantom_never_raised(entries: &[Entry]) -> Outcome {
    for entry in entries {
        match &entry.event {
            Event::PreampWord { value, .. } if (value >> 8) & PHANTOM_BITS != 0 => {
                return Err(format!(
                    "the word latched at {} has a phantom bit set ({value:#08x}) and the model simulated no button press",
                    entry.at
                ));
            }
            event if is_pin(event, PHANTOM_ACTIVE_PIN, Level::High) => {
                return Err(format!(
                    "P1_4 went high at {} and the model simulated no button press",
                    entry.at
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn preamp_words_whole(entries: &[Entry]) -> Outcome {
    for entry in entries {
        match entry.event {
            Event::PreampWord { bits, value } if bits != LATCHED_WORD_BITS => {
                return Err(format!(
                    "the chain latched {bits} bits ({value:#08x}) at {}, not a whole 24-bit word",
                    entry.at
                ));
            }
            Event::SpiCut => {
                return Err(format!(
                    "a preamp frame was cut by a Flexcomm 1 reset or flush at {}",
                    entry.at
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_output_enabling(event: &Event) -> bool {
    I2S_FLEXCOMMS
        .iter()
        .any(|register| is_write(event, register, |value| value & ENABLE_BIT != 0))
        || is_write(event, "DMA0.ENABLESET0", |value| {
            value & TRANSMIT_DMA_CHANNELS != 0
        })
        || is_output(event, OutputPin::TransportActive, Level::High)
        || matches!(dac_write(event), Some((register, value, _))
            if VolumeRegister::ALL.iter().any(|volume| *volume as u8 == register) && value != 0xFF)
}

fn is_master_clock_start(event: &Event) -> bool {
    is_write(event, "SYSCON.MCLKIO", |value| value & ENABLE_BIT != 0)
}

fn is_host_rate_request(entry: &Entry) -> bool {
    matches!(entry.event, Event::HandoverWrite { .. })
}

fn is_audio_clock_write(event: &Event) -> bool {
    matches!(event, Event::Write { register, .. }
        if register.starts_with("SYSCON.PLL0") || register.starts_with("SYSCON.MCLK"))
}

fn is_held_by_rate_change(register: u8) -> bool {
    (VolumeRegister::MainA as u8..=VolumeRegister::HeadphoneB as u8).contains(&register)
}

fn host_rate_changes_in_order(entries: &[Entry], expectation: &Expectation) -> Outcome {
    let requests: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_host_rate_request(entry))
        .map(|(index, _)| index)
        .collect();
    for (number, &start) in requests.iter().enumerate() {
        let end = requests.get(number + 1).copied().unwrap_or(entries.len());
        let Event::HandoverWrite { value, .. } = entries[start].event else {
            continue;
        };
        let rate = usize::try_from(value & 0b111)
            .ok()
            .and_then(|code| SampleRate::ALL.get(code).copied())
            .ok_or_else(|| format!("the handover word {value:#x} names no rate"))?;
        let judged = match expectation.rate_change {
            RateChange::Ordered => rate_change_in_order(&entries[start..end], rate),
            RateChange::NeverRestarts => rate_change_never_restarts(&entries[start..end], rate),
        };
        judged.map_err(|failure| {
            format!(
                "the request for {} Hz at {}: {failure}",
                rate.hz(),
                entries[start].at
            )
        })?;
    }
    Ok(())
}

fn held_registers_before(window: &[Entry], before: &[Entry]) -> Outcome {
    let touched = window[0].at;
    let held: BTreeSet<u8> = before
        .iter()
        .filter_map(|entry| dac_write(&entry.event))
        .filter(|(register, value, acked)| {
            *acked && *value == 0xFF && is_held_by_rate_change(*register)
        })
        .map(|(register, _, _)| register)
        .collect();
    if held.len() != 4 {
        return Err(format!(
            "the audio clock was touched at {touched} with only DAC registers {held:?} acknowledged at 0xFF"
        ));
    }
    let stops: [(&str, EventTest); 3] = [
        (
            "the tick timer stopped",
            Box::new(|event| is_write(event, "CTIMER0.TCR", |value| value == 0)),
        ),
        (
            "the DMA stopped",
            Box::new(|event| is_write(event, "DMA0.ENABLECLR0", |value| value != 0)),
        ),
        (
            "P0_28 fell",
            Box::new(|event| is_output(event, OutputPin::TransportActive, Level::Low)),
        ),
    ];
    for (what, stopped) in &stops {
        if !before.iter().any(|entry| stopped(&entry.event)) {
            return Err(format!(
                "the audio clock was touched at {touched} before {what}"
            ));
        }
    }
    Ok(())
}

fn rate_change_in_order(window: &[Entry], rate: SampleRate) -> Outcome {
    let clock = window
        .iter()
        .position(|entry| is_audio_clock_write(&entry.event))
        .ok_or("no audio clock register was written")?;
    let (before, after) = window.split_at(clock);
    held_registers_before(window, before)?;
    let touched = window[clock].at;
    let restart = after
        .iter()
        .position(|entry| is_output(&entry.event, OutputPin::TransportActive, Level::High))
        .ok_or("P0_28 never rose again")?;
    let restarted = after[restart].at;
    let programmed = after[..restart]
        .iter()
        .rev()
        .find(|entry| is_audio_clock_write(&entry.event))
        .map_or(touched, |entry| entry.at);
    let wait = Nanoseconds::from_milliseconds(u64::from(audio_pll_stabilisation(rate.family()).0));
    if restarted.since(programmed) < wait {
        return Err(format!(
            "the transport restarted at {restarted}, {} after the PLL was programmed at {programmed}, short of the {wait} wait",
            restarted.since(programmed)
        ));
    }
    let released = after[restart..]
        .iter()
        .find(|entry| {
            matches!(dac_write(&entry.event), Some((register, value, _))
                if is_held_by_rate_change(register) && value != 0xFF)
        })
        .ok_or("the outputs were never released")?;
    let settle = Nanoseconds::from_milliseconds(u64::from(DAC_CLOCK_SETTLE.0));
    if released.at.since(restarted) < settle {
        return Err(format!(
            "the outputs were released at {}, {} after the restart at {restarted}, short of the {settle} settle",
            released.at,
            released.at.since(restarted)
        ));
    }
    Ok(())
}

fn rate_change_never_restarts(window: &[Entry], _rate: SampleRate) -> Outcome {
    let clock = window
        .iter()
        .position(|entry| is_audio_clock_write(&entry.event))
        .ok_or("no audio clock register was written")?;
    let (before, after) = window.split_at(clock);
    held_registers_before(window, before)?;
    if let Some(enabled) = after.iter().find(|entry| is_output_enabling(&entry.event)) {
        return Err(format!(
            "{:?} at {} restarts an output after the failed clock change at {}",
            enabled.event, enabled.at, window[clock].at
        ));
    }
    Ok(())
}

const FULL_ATTENUATION: u8 = 0xFF;

fn quiet_enough(acknowledged: Option<u8>, ceiling: Option<u8>) -> bool {
    match (acknowledged, ceiling) {
        (Some(FULL_ATTENUATION), _) => true,
        (Some(value), Some(ceiling)) => value >= ceiling,
        (Some(_) | None, _) => false,
    }
}

fn dac_attenuated_before_output(entries: &[Entry]) -> Outcome {
    let mut acknowledged: [Option<u8>; VolumeRegister::ALL.len()] =
        [None; VolumeRegister::ALL.len()];
    let mut ceilings: [Option<u8>; VolumeRegister::ALL.len()] = [None; VolumeRegister::ALL.len()];
    let mut clocked = false;
    let mut connected_once = false;
    for entry in entries {
        if is_master_clock_start(&entry.event) && !connected_once {
            clocked = true;
            acknowledged = [None; VolumeRegister::ALL.len()];
        }
        if is_output(&entry.event, OutputPin::TransportActive, Level::Low) && connected_once {
            ceilings = acknowledged;
        }
        if is_output(&entry.event, OutputPin::TransportActive, Level::High) {
            if !clocked {
                return Err(format!(
                    "P0_28 rose at {} before the master clock started",
                    entry.at
                ));
            }
            let loud: Vec<(u8, Option<u8>, Option<u8>)> = VolumeRegister::ALL
                .iter()
                .zip(acknowledged.iter().zip(ceilings))
                .filter(|(_, (value, ceiling))| !quiet_enough(**value, *ceiling))
                .map(|(register, (value, ceiling))| (*register as u8, *value, ceiling))
                .collect();
            if !loud.is_empty() {
                return Err(format!(
                    "P0_28 rose at {} while DAC registers (register, last acknowledged, last before P0_28 fell) {loud:?} were neither acknowledged at 0xFF since the master clock started nor at most as loud as before P0_28 fell",
                    entry.at
                ));
            }
            connected_once = true;
        }
        if let Some((register, value, acked)) = dac_write(&entry.event) {
            if let Some(index) = VolumeRegister::ALL
                .iter()
                .position(|volume| *volume as u8 == register)
            {
                acknowledged[index] = acked.then_some(value);
            }
        }
    }
    Ok(())
}

fn master_trim_at_most_minus_one_db(entries: &[Entry]) -> Outcome {
    let bytes = [
        MasterTrimByte::Lowest,
        MasterTrimByte::Low,
        MasterTrimByte::High,
        MasterTrimByte::Highest,
    ];
    let mut trim: [Option<u8>; 4] = [None; 4];
    for entry in entries {
        let Some((register, value, true)) = dac_write(&entry.event) else {
            continue;
        };
        let Some(index) = bytes.iter().position(|byte| *byte as u8 == register) else {
            continue;
        };
        trim[index] = Some(value);
        if let [Some(a), Some(b), Some(c), Some(d)] = trim {
            let gain = u32::from_le_bytes([a, b, c, d]);
            if gain > MasterTrim::MINUS_ONE_DB.raw() {
                return Err(format!(
                    "master trim reads {gain:#010x} after the write at {}, louder than {:#010x} (-1.00 dB)",
                    entry.at,
                    MasterTrim::MINUS_ONE_DB.raw()
                ));
            }
        }
    }
    Ok(())
}

fn dac_registers_zero_and_sixty_two(entries: &[Entry]) -> Outcome {
    for entry in entries {
        if let Event::I2c(I2cTransaction::Write { address, bytes, .. }) = &entry.event
            && *address == DAC_ADDRESS
            && let Some(register) = bytes.first()
            && FORBIDDEN_DAC_REGISTERS.contains(register)
        {
            return Err(format!(
                "ES9026 register {register} was addressed for a write at {}",
                entry.at
            ));
        }
    }
    Ok(())
}

fn flash_untouched(entries: &[Entry], planted_store: Option<u32>) -> Outcome {
    let breaches: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let what = match entry.event {
                Event::FlashControllerAccess { address } if Some(address) == planted_store => {
                    return None;
                }
                Event::FlashControllerAccess { address } => {
                    format!("flash controller block access at {address:#x}")
                }
                Event::SecureAliasAccess { address } => {
                    format!("secure alias access at {address:#x}")
                }
                Event::FlashWrite { address } => format!("flash write at {address:#x}"),
                Event::FlashRead { address }
                    if !(FACTORY_READ.0..FACTORY_READ.1).contains(&address) =>
                {
                    format!("flash read at {address:#x} outside the factory accessor")
                }
                _ => return None,
            };
            Some(format!("{what} at {}", entry.at))
        })
        .collect();
    match breaches.first() {
        None => Ok(()),
        Some(first) => Err(format!("{} breach(es), first {first}", breaches.len())),
    }
}

fn fence_never_tripped(entries: &[Entry], planted_store: Option<u32>) -> Outcome {
    let tripped = |wanted: &Event| match (wanted, planted_store) {
        (Event::MpuFault { address, access }, Some(planted)) => {
            !(*address == planted && *access == Access::Write)
        }
        (Event::MpuFault { .. }, None) => true,
        _ => false,
    };
    if let Some(entry) = first(entries, tripped) {
        return Err(format!(
            "the image tripped its own fence with {:?} at {}",
            entry.event, entry.at
        ));
    }
    let Some(planted) = planted_store else {
        return Ok(());
    };
    match first(
        entries,
        |event| matches!(event, Event::MpuFault { address, .. } if *address == planted),
    ) {
        Some(_) => Ok(()),
        None => Err(format!(
            "the planted store to {planted:#x} did not fault, so the fence the image raised does not cover it"
        )),
    }
}

fn fence_programmed(entries: &[Entry], probes: &[FenceProbe]) -> Outcome {
    for probe in probes {
        let answer = entries.iter().find_map(|entry| match entry.event {
            Event::MpuProbe {
                address,
                access,
                allowed,
            } if address == probe.address && access == probe.access => Some(allowed),
            _ => None,
        });
        let verdict = |allowed: bool| if allowed { "allows" } else { "faults" };
        match answer {
            None => {
                return Err(format!(
                    "the model printed no fence probe for {:?} at {:#x}",
                    probe.access, probe.address
                ));
            }
            Some(allowed) if allowed != probe.allowed => {
                return Err(format!(
                    "the programmed MPU {} a {:?} at {:#x}, the fence should {} it",
                    verdict(allowed),
                    probe.access,
                    probe.address,
                    if probe.allowed { "allow" } else { "fault" }
                ));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn pll_lock_read_before_clock_use(entries: &[Entry]) -> Outcome {
    if let Some(switched) = position(entries, |event| matches!(event, Event::MainClock { .. })) {
        require_read_before(
            entries,
            "SYSCON.PLL1STAT",
            switched,
            "the main clock switched to PLL1",
        )?;
    }
    if let Some(selected) = position(entries, |event| {
        is_write(event, "SYSCON.MCLKCLKSEL", |_| true)
    }) {
        require_read_before(entries, "SYSCON.PLL0STAT", selected, "MCLK selected PLL0")?;
    }
    Ok(())
}

fn require_read_before(entries: &[Entry], status: &str, used_at: usize, what: &str) -> Outcome {
    let used = entries[used_at].at;
    match position(
        entries,
        |event| matches!(event, Event::Read { register } if register.as_str() == status),
    ) {
        Some(read) if read < used_at => Ok(()),
        Some(read) => Err(format!(
            "{what} at {used} before {status}.LOCK was read, the first read is at {}",
            entries[read].at
        )),
        None => Err(format!("{what} at {used} and {status}.LOCK was never read")),
    }
}

fn quiesce_steps() -> Vec<(&'static str, EventTest)> {
    let mut steps: Vec<(&'static str, EventTest)> = vec![
        (
            "every NVIC interrupt disabled",
            Box::new(
                |event| matches!(event, Event::Write { register, value } if register == "NVIC+0x80" && *value == u32::MAX),
            ),
        ),
        (
            "the four audio DMA channels stopped",
            Box::new(|event| {
                is_write(event, "DMA0.ENABLECLR0", |value| {
                    value & EVERY_AUDIO_DMA_CHANNEL == EVERY_AUDIO_DMA_CHANNEL
                })
            }),
        ),
        (
            "CTIMER0 stopped",
            Box::new(|event| is_write(event, "CTIMER0.TCR", |value| value & ENABLE_BIT == 0)),
        ),
        (
            "USB soft-detached",
            Box::new(|event| is_write(event, "USB1.DEVCMDSTAT", |_| true)),
        ),
        (
            "the MPU fence raised",
            Box::new(|event| matches!(event, Event::MpuEnabled)),
        ),
    ];
    for register in I2S_FLEXCOMMS {
        steps.push((
            register,
            Box::new(move |event| is_write(event, register, |value| value & ENABLE_BIT == 0)),
        ));
    }
    steps
}

fn watchdog_never_enabled(entries: &[Entry]) -> Outcome {
    match first(entries, |event| {
        matches!(event, Event::WatchdogArmed)
            || is_write(event, "WWDT.MOD", |value| value & WWDT_MOD_WDEN != 0)
    }) {
        Some(entry) => Err(format!(
            "{:?} at {} enables the watchdog, and the bring-up policy never does",
            entry.event, entry.at
        )),
        None => Ok(()),
    }
}

fn quiesced_by_the_fence(boot: &[Entry]) -> Outcome {
    let Some(fence) = first(boot, |event| Milestone::Quiesce.opens_with(event)) else {
        return Ok(());
    };
    let fence_index = boot
        .iter()
        .position(|entry| Milestone::Quiesce.opens_with(&entry.event))
        .unwrap_or(0);
    for (step, matches) in quiesce_steps() {
        if !flexcomm_clocked_by(boot, step, fence_index) {
            continue;
        }
        match first(boot, |event| matches(event)) {
            Some(entry) if entry.at <= fence.at => {}
            Some(entry) => {
                return Err(format!(
                    "quiesce step `{step}` at {} comes after the fence at {}, and the fence is the last quiesce step the trace shows",
                    entry.at, fence.at
                ));
            }
            None => return Err(format!("quiesce step `{step}` never happened")),
        }
    }
    rail_pins_low_by_the_fence(boot, fence)
}

const QUIESCE_RAIL_LOWS: [OutputPin; 3] = [
    OutputPin::SecondBringUp,
    OutputPin::FirstBringUp,
    OutputPin::ThirdBringUp,
];
const QUIESCE_RAIL_ORDER: &str = "the quiesce drives P1_26, P1_22 and P1_10 low, in that order, after the P0_28 pad write and by the fence";

fn rail_pins_low_by_the_fence(boot: &[Entry], fence: &Entry) -> Outcome {
    let Some(pad) = position(boot, |event| is_write(event, "IOCON.P0_28", |_| true)) else {
        return Err(format!(
            "the P0_28 pad write never happened, and {QUIESCE_RAIL_ORDER}"
        ));
    };
    let mut earlier = ("the P0_28 pad write".to_string(), pad);
    for output in QUIESCE_RAIL_LOWS {
        let pin = output.pin();
        let name = format!("P{}_{}", pin.port, pin.number);
        let Some(at) = position(
            boot,
            |event| matches!(event, Event::Pin { port, number, .. } if *port == pin.port && *number == pin.number),
        ) else {
            return Err(format!("{name} is never driven, and {QUIESCE_RAIL_ORDER}"));
        };
        let entry = &boot[at];
        if !is_output(&entry.event, output, Level::Low) {
            return Err(format!(
                "the first {name} level is {} at {}, and {QUIESCE_RAIL_ORDER}",
                if is_output(&entry.event, output, Level::High) {
                    "high"
                } else {
                    "released"
                },
                entry.at
            ));
        }
        if at < earlier.1 {
            return Err(format!(
                "{name} low at {} comes before {}, and {QUIESCE_RAIL_ORDER}",
                entry.at, earlier.0
            ));
        }
        if entry.at > fence.at {
            return Err(format!(
                "{name} low at {} comes after the fence at {}, and {QUIESCE_RAIL_ORDER}",
                entry.at, fence.at
            ));
        }
        earlier = (format!("{name} low"), at);
    }
    Ok(())
}

fn watchdog_never_starved(entries: &[Entry]) -> Outcome {
    let starved = first(entries, |event| {
        matches!(event, Event::WatchdogStarved | Event::WatchdogBadFeed)
    });
    if let Some(entry) = starved {
        return Err(format!("{:?} at {}", entry.event, entry.at));
    }
    let limit = Nanoseconds(
        Nanoseconds::from_milliseconds(u64::from(WATCHDOG_TIMEOUT.0)).0
            * WATCHDOG_CLOCK_FAST_EDGE_PERCENT
            / 100,
    );
    let mut feeds: Vec<Nanoseconds> = entries
        .iter()
        .filter(|entry| matches!(entry.event, Event::WatchdogArmed | Event::WatchdogFed))
        .map(|entry| entry.at)
        .collect();
    if feeds.is_empty() {
        return Ok(());
    }
    if let Some(end) = entries
        .iter()
        .find(|entry| matches!(entry.event, Event::End(Ending::TimeLimit)))
    {
        feeds.push(end.at);
    }
    for pair in feeds.windows(2) {
        let gap = pair[1].since(pair[0]);
        if gap >= limit {
            return Err(format!(
                "no feed for {gap} from {}, and a watchdog clock at the fast edge of its tolerance resets after {limit}",
                pair[0]
            ));
        }
    }
    Ok(())
}

fn longest_timeout_before_the_first_feed(entries: &[Entry]) -> Outcome {
    if let Some(other) = first(entries, |event| {
        is_write(event, "WWDT.TC", |value| value != WWDT_LONGEST_TIMEOUT)
    }) {
        return Err(format!(
            "WWDT.TC written at {} with a value other than {WWDT_LONGEST_TIMEOUT:#x}, and the bring-up policy writes no other timeout",
            other.at
        ));
    }
    let Some(feed) = position(entries, |event| matches!(event, Event::WatchdogFed)) else {
        return Ok(());
    };
    match position(entries, |event| {
        is_write(event, "WWDT.TC", |value| value == WWDT_LONGEST_TIMEOUT)
    }) {
        Some(timeout) if timeout < feed => Ok(()),
        _ => Err(format!(
            "the first watchdog feed at {} comes before any write of WWDT.TC {WWDT_LONGEST_TIMEOUT:#x}, and the claim sets the longest timeout before it feeds an inherited watchdog",
            entries[feed].at
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Milestone {
    Quiesce,
    Clocks,
    UsbAttach,
    AnalogOff,
    Rails,
    StartLcd,
    AudioClocks,
    DacAttenuate,
    AdcProbe,
    Transport,
    DacInit,
    AdcInit,
    Panel,
}

const MILESTONES: [Milestone; 13] = [
    Milestone::Quiesce,
    Milestone::Clocks,
    Milestone::UsbAttach,
    Milestone::AnalogOff,
    Milestone::Rails,
    Milestone::StartLcd,
    Milestone::AudioClocks,
    Milestone::DacAttenuate,
    Milestone::AdcProbe,
    Milestone::Transport,
    Milestone::DacInit,
    Milestone::AdcInit,
    Milestone::Panel,
];

fn is_adc_address(address: u8) -> bool {
    [AdcAddress::At13, AdcAddress::At4F]
        .iter()
        .any(|adc| adc.address() == address)
}

impl Milestone {
    fn opens_with(self, event: &Event) -> bool {
        match self {
            Milestone::Quiesce => matches!(event, Event::MpuEnabled),
            Milestone::Clocks => is_write(event, "SYSCON.PLL1CTRL", |_| true),
            Milestone::UsbAttach => matches!(event, Event::UsbConnected),
            Milestone::AnalogOff => is_preamp_word(event),
            Milestone::Rails => is_output(event, OutputPin::FirstBringUp, Level::High),
            Milestone::StartLcd => is_lcd_start(event),
            Milestone::AudioClocks => is_write(event, "SYSCON.PLL0SSCG1", |_| true),
            Milestone::DacAttenuate => is_i2c_to(event, DAC_ADDRESS),
            Milestone::AdcProbe => is_output(event, OutputPin::ConverterEnable, Level::High),
            Milestone::Transport => {
                (is_output_enabling(event) && dac_write(event).is_none())
                    || is_write(event, "CTIMER0.TCR", |value| value & ENABLE_BIT != 0)
            }
            Milestone::DacInit => match event {
                Event::I2c(I2cTransaction::Read { address, .. }) => *address == DAC_ADDRESS,
                _ => matches!(dac_write(event), Some((register, _, _))
                    if !VolumeRegister::ALL.iter().any(|volume| *volume as u8 == register)),
            },
            Milestone::AdcInit => matches!(event,
                Event::I2c(I2cTransaction::Write { address, .. }) if is_adc_address(*address)),
            Milestone::Panel => {
                is_i2c_to(event, EXPANDER_ADDRESS)
                    || matches!(event, Event::Write { register, .. } if register.starts_with("ADC0"))
            }
        }
    }

    const fn optional(self) -> bool {
        matches!(self, Milestone::AdcInit)
    }
}

fn breadcrumb(event: &Event) -> Option<Breadcrumb> {
    match event {
        Event::Breadcrumb(word) => Some(Breadcrumb::unpack(*word)),
        _ => None,
    }
}

fn raised_records(entries: &[Entry]) -> Vec<usize> {
    let mut kept = (0, Weight::Noted);
    let mut records = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(crumb) = breadcrumb(&entry.event) else {
            continue;
        };
        let now = (crumb.code, crumb.weight);
        if now != kept && crumb.code != 0 && crumb.weight == Weight::Raised {
            records.push(index);
        }
        kept = now;
    }
    records
}

fn fault_start(entries: &[Entry]) -> Option<Nanoseconds> {
    let quiesced = entries
        .iter()
        .position(|entry| {
            is_write(&entry.event, "DMA0.ENABLECLR0", |value| {
                value == EVERY_AUDIO_DMA_CHANNEL
            })
        })
        .unwrap_or(entries.len());
    let stopped = entries
        .iter()
        .enumerate()
        .skip(quiesced + 1)
        .find(|(_, entry)| {
            is_write(&entry.event, "DMA0.ENABLECLR0", |value| {
                value == TRANSMIT_DMA_CHANNELS
            })
        })
        .map(|(_, entry)| entry.at);
    let recorded = raised_records(entries).first().copied();
    let raised = entries
        .iter()
        .enumerate()
        .find(|(index, entry)| {
            recorded == Some(*index)
                || matches!(
                    entry.event,
                    Event::Exception { .. }
                        | Event::WatchdogStarved
                        | Event::MpuFault { .. }
                        | Event::BusFault { ignored: false, .. }
                )
        })
        .map(|(_, entry)| entry.at);
    stopped.or(raised)
}

fn before_the_fault(entries: &[Entry]) -> &[Entry] {
    match fault_start(entries) {
        Some(start) => {
            let end = entries
                .iter()
                .position(|entry| entry.at >= start)
                .unwrap_or(entries.len());
            &entries[..end]
        }
        None => entries,
    }
}

struct Hold {
    what: &'static str,
    from: EventTest,
    to: EventTest,
    at_least: Milliseconds,
}

fn after_milliseconds(duration: Milliseconds) -> Nanoseconds {
    Nanoseconds::from_milliseconds(u64::from(duration.0))
}

fn holds() -> Vec<Hold> {
    let pin = |output: OutputPin, level: Level| -> EventTest {
        Box::new(move |event| is_output(event, output, level))
    };
    let write = |register: &'static str| -> EventTest {
        Box::new(move |event| is_write(event, register, |_| true))
    };
    let probe =
        |adc: AdcAddress| -> EventTest { Box::new(move |event| is_i2c_to(event, adc.address())) };
    vec![
        Hold {
            what: "rails, P1_22 high to P1_26 high",
            from: pin(OutputPin::FirstBringUp, Level::High),
            to: pin(OutputPin::SecondBringUp, Level::High),
            at_least: RAIL_STAGGER,
        },
        Hold {
            what: "rails, P1_26 high to P1_10 high",
            from: pin(OutputPin::SecondBringUp, Level::High),
            to: pin(OutputPin::ThirdBringUp, Level::High),
            at_least: RAIL_STAGGER,
        },
        Hold {
            what: "rails, P1_10 high to the all-off resend",
            from: pin(OutputPin::ThirdBringUp, Level::High),
            to: Box::new(is_preamp_word),
            at_least: RAIL_SETTLE,
        },
        Hold {
            what: "clocks, PLL1 start to the main clock switch",
            from: write("SYSCON.PLL1CTRL"),
            to: Box::new(|event| matches!(event, Event::MainClock { .. })),
            at_least: CORE_PLL_STABILISATION,
        },
        Hold {
            what: "audio clocks, PLL0 start to MCLK out",
            from: write("SYSCON.PLL0SSCG1"),
            to: write("SYSCON.MCLKIO"),
            at_least: audio_pll_stabilisation(BOOT_RATE.family()),
        },
        Hold {
            what: "ADC probe, P1_29 high to the 0x13 probe",
            from: pin(OutputPin::AdcEnable, Level::High),
            to: probe(AdcAddress::At13),
            at_least: ADC_WAKE,
        },
        Hold {
            what: "ADC probe, 0x13 probe to the 0x4F probe",
            from: probe(AdcAddress::At13),
            to: probe(AdcAddress::At4F),
            at_least: PROBE_SPACING,
        },
    ]
}

fn boot_order(entries: &[Entry]) -> Outcome {
    let boot = before_the_fault(entries);
    let opened: Vec<(Milestone, Option<Nanoseconds>)> = MILESTONES
        .iter()
        .map(|milestone| {
            (
                *milestone,
                first(boot, |event| milestone.opens_with(event)).map(|entry| entry.at),
            )
        })
        .collect();
    let mut previous: Option<(Milestone, Nanoseconds)> = None;
    for (index, (milestone, at)) in opened.iter().enumerate() {
        let Some(at) = at else { continue };
        if let Some((earlier, earlier_at)) = previous
            && *at < earlier_at
        {
            return Err(format!(
                "{milestone:?} opens at {at}, before {earlier:?} at {earlier_at}"
            ));
        }
        if let Some((skipped, _)) = opened[..index]
            .iter()
            .find(|(skipped, seen)| seen.is_none() && !skipped.optional())
        {
            return Err(format!(
                "{milestone:?} opens at {at} but {skipped:?} never happened"
            ));
        }
        previous = Some((*milestone, *at));
    }
    quiesced_by_the_fence(boot)?;
    for (pin, step) in [
        (OutputPin::ConverterEnable, "P1_3 low"),
        (OutputPin::AdcEnable, "P1_29 low"),
    ] {
        if let Some(rail) = first(boot, |event| {
            is_output(event, OutputPin::FirstBringUp, Level::High)
        }) && first(boot, |event| is_output(event, pin, Level::Low))
            .is_none_or(|low| low.at > rail.at)
        {
            return Err(format!(
                "the rails raised P1_22 at {} before {step}",
                rail.at
            ));
        }
    }
    for hold in holds() {
        let Some(from) = first(boot, |event| (hold.from)(event)) else {
            continue;
        };
        let Some(to) = boot.iter().find(|entry| {
            entry.at >= from.at && (hold.to)(&entry.event) && !std::ptr::eq(*entry, from)
        }) else {
            continue;
        };
        let held = to.at.since(from.at);
        if held < after_milliseconds(hold.at_least) {
            return Err(format!(
                "{} held {held}, the boot machine holds at least {} ms",
                hold.what, hold.at_least.0
            ));
        }
    }
    Ok(())
}

fn usb_connects_before_the_rails(entries: &[Entry], expectation: &Expectation) -> Outcome {
    let Some(rail) = first(entries, |event| Milestone::Rails.opens_with(event)) else {
        return Ok(());
    };
    let connect = first(entries, |event| matches!(event, Event::UsbConnected));
    match (connect, expectation.after_fault) {
        (Some(connect), _) if connect.at < rail.at => Ok(()),
        (Some(connect), _) => Err(format!(
            "USB connected at {}, after the first rail rose at {}",
            connect.at, rail.at
        )),
        (None, AfterFault::PanelOnly) => Ok(()),
        (
            None,
            AfterFault::None
            | AfterFault::UsbOnly
            | AfterFault::PanelAndUsb
            | AfterFault::AudioPlaying,
        ) => Err(format!(
            "the first rail rose at {} and USB never connected",
            rail.at
        )),
    }
}

fn usb_list_start_set_before_each_connect(entries: &[Entry]) -> Outcome {
    let mut written = None;
    for entry in entries {
        match &entry.event {
            Event::Write { register, value } if register == "USB1.EPLISTSTART" => {
                if *value != USB1_RAM_FIRST_BYTE {
                    return Err(format!(
                        "USB1.EPLISTSTART written {value:#x} at {}, not the list at {USB1_RAM_FIRST_BYTE:#x}",
                        entry.at
                    ));
                }
                written = Some(*value);
            }
            Event::UsbConnected => match written.take() {
                Some(USB1_RAM_FIRST_BYTE) => {}
                Some(value) => {
                    return Err(format!(
                        "USB connected at {} with USB1.EPLISTSTART last written {value:#x}, not the list at {USB1_RAM_FIRST_BYTE:#x}",
                        entry.at
                    ));
                }
                None => {
                    return Err(format!(
                        "USB connected at {} with no USB1.EPLISTSTART write since the previous connect",
                        entry.at
                    ));
                }
            },
            _ => {}
        }
    }
    Ok(())
}

fn is_read(event: &Event, status: &str) -> bool {
    matches!(event, Event::Read { register } if register == status)
}

fn is_usb_phy_write(event: &Event) -> bool {
    matches!(event, Event::Write { register, .. } if register.starts_with("USBPHY"))
}

fn usb_phy_powered_before_the_clock_waits(entries: &[Entry]) -> Outcome {
    let Some(phy) = position(entries, is_usb_phy_write) else {
        return Ok(());
    };
    let steps: [(&str, EventTest); 5] = [
        (
            "the PMC.RTCOSC32K write of 0",
            Box::new(|event| is_write(event, "PMC.RTCOSC32K", |value| value == 0)),
        ),
        (
            "the PMC.PDRUNCFGCLR0 write that holds 0x1411c0",
            Box::new(|event| {
                is_write(event, "PMC.PDRUNCFGCLR0", |value| {
                    value & CRYSTAL_AND_USB_PHY_POWER == CRYSTAL_AND_USB_PHY_POWER
                })
            }),
        ),
        (
            "the ANACTRL.XO32M_CTRL write with bit 23 set",
            Box::new(|event| {
                is_write(event, "ANACTRL.XO32M_CTRL", |value| {
                    value & XO32M_CTRL_ENABLE_PLL_USB_OUT != 0
                })
            }),
        ),
        (
            "the crystal-ready read",
            Box::new(|event| is_read(event, "ANACTRL.XO32M_STATUS")),
        ),
        (
            "the core PLL lock read",
            Box::new(|event| is_read(event, "SYSCON.PLL1STAT")),
        ),
    ];
    let mut earlier: Option<(&str, usize)> = None;
    for (what, matches) in &steps {
        let Some(at) = position(entries, |event| matches(event)) else {
            return Err(format!(
                "the first USB PHY write is at {} and {what} never happened",
                entries[phy].at
            ));
        };
        if let Some((before, before_at)) = earlier
            && at < before_at
        {
            return Err(format!(
                "{what} at {} comes before {before} at {}",
                entries[at].at, entries[before_at].at
            ));
        }
        earlier = Some((what, at));
    }
    match earlier {
        Some((what, at)) if phy < at => Err(format!(
            "the first USB PHY write at {} comes before {what} at {}",
            entries[phy].at, entries[at].at
        )),
        _ => Ok(()),
    }
}

#[derive(Clone, Copy)]
enum PhyLock {
    NotYetUngated,
    Ungated,
    Read,
}

fn usb_phy_pll_lock_read_before_each_connect(entries: &[Entry]) -> Outcome {
    let mut lock = PhyLock::NotYetUngated;
    for entry in entries {
        match (&entry.event, lock) {
            (event, _) if is_write(event, "USBPHY.CTRL_SET", |_| true) => lock = PhyLock::Ungated,
            (event, PhyLock::NotYetUngated) if is_read(event, "USBPHY.PLL_SIC") => {
                return Err(format!(
                    "USBPHY.PLL_SIC was read at {} before the USBPHY.CTRL_SET write, with the PHY still gated",
                    entry.at
                ));
            }
            (event, PhyLock::Ungated) if is_read(event, "USBPHY.PLL_SIC") => lock = PhyLock::Read,
            (Event::UsbConnected, PhyLock::Read) => lock = PhyLock::NotYetUngated,
            (Event::UsbConnected, PhyLock::NotYetUngated | PhyLock::Ungated) => {
                return Err(format!(
                    "USB connected at {} with no USBPHY.PLL_SIC lock read after a USBPHY.CTRL_SET write since the previous connect",
                    entry.at
                ));
            }
            _ => {}
        }
    }
    match lock {
        PhyLock::Read => Err(
            "USBPHY.PLL_SIC lock was read and USB never connected after it, and a lock timeout must not cost the connect"
                .into(),
        ),
        PhyLock::NotYetUngated | PhyLock::Ungated => Ok(()),
    }
}

// WHY: SVD28 AHBCLKCTRL2 USB1_DEV bit 5; PORTMODE and the PHY go before the first USBHSD access, or the device never arrives on the bus
const AHBCLKCTRL2_USB1_DEV: u32 = 1 << 5;

fn is_usb_clock_on(event: &Event) -> bool {
    is_write(event, "SYSCON.AHBCLKCTRLSET2", |value| {
        value & AHBCLKCTRL2_USB1_DEV != 0
    })
}

fn is_usb_device_access(event: &Event) -> bool {
    match event {
        Event::Write { register, .. } | Event::Read { register } => register.starts_with("USB1"),
        _ => false,
    }
}

fn is_port_mode_write(event: &Event) -> bool {
    matches!(event, Event::Write { register, .. } if register.starts_with("USBHSH"))
}

fn usb_device_registers_after_the_phy(entries: &[Entry]) -> Outcome {
    let mut attach_start = None;
    for (index, entry) in entries.iter().enumerate() {
        if is_usb_clock_on(&entry.event) {
            attach_start = Some(index);
        }
        if !matches!(entry.event, Event::UsbConnected) {
            continue;
        }
        let Some(start) = attach_start.take() else {
            return Err(format!(
                "USB connected at {} with no USB clock write since the previous connect",
                entry.at
            ));
        };
        let window = &entries[start..index];
        let first_device = position(window, is_usb_device_access);
        let port_mode = position(window, is_port_mode_write);
        let last_phy = window
            .iter()
            .rposition(|entry| is_usb_phy_write(&entry.event));
        match (first_device, port_mode, last_phy) {
            (Some(device), Some(port), Some(phy)) if port < device && phy < device => {}
            (device, port, phy) => {
                return Err(format!(
                    "the attach that connected at {} has its first USB1 device access at {}, PORTMODE at {} and its last USB PHY write at {}",
                    entry.at,
                    describe_at(window, device),
                    describe_at(window, port),
                    describe_at(window, phy)
                ));
            }
        }
    }
    Ok(())
}

fn describe_at(window: &[Entry], index: Option<usize>) -> String {
    index.map_or("none".to_string(), |index| window[index].at.to_string())
}

// WHY: SVD28 AHBCLKCTRL1 FC4 bit 15, FRO192M_CTRL ENA_96MHZCLK bit 30, FROHFDIV DIV in bits 7:0 with HALT bit 30; FCCLKSEL4 3 is FRO_HF_DIV, which is halted with its 96 MHz source off out of reset
const AHBCLKCTRL1_FC4: u32 = 1 << 15;
const FRO192M_CTRL_ENA_96MHZCLK: u32 = 1 << 30;
const FROHFDIV_FACTORY_DIVIDE_BY_FOUR: u32 = 3;

#[derive(Clone, Copy)]
enum LcdClock {
    NotGated,
    Gated(Nanoseconds),
    SourceOn(Nanoseconds),
    Divided,
}

fn lcd_starts_its_own_clock_source(entries: &[Entry]) -> Outcome {
    let mut clock = LcdClock::NotGated;
    for entry in entries {
        let event = &entry.event;
        if is_write(event, "SYSCON.AHBCLKCTRLSET1", |value| {
            value & AHBCLKCTRL1_FC4 != 0
        }) {
            clock = LcdClock::Gated(entry.at);
        } else if is_write(event, "ANACTRL.FRO192M_CTRL", |value| {
            value & FRO192M_CTRL_ENA_96MHZCLK != 0
        }) {
            if let LcdClock::Gated(gate) = clock {
                clock = LcdClock::SourceOn(gate);
            }
        } else if is_write(event, "SYSCON.FROHFDIV", |value| {
            value == FROHFDIV_FACTORY_DIVIDE_BY_FOUR
        }) {
            if let LcdClock::SourceOn(_) = clock {
                clock = LcdClock::Divided;
            }
        } else if is_write(event, "SYSCON.FCCLKSEL4", |_| true) {
            let missing = match clock {
                LcdClock::Divided => {
                    clock = LcdClock::NotGated;
                    continue;
                }
                LcdClock::NotGated => "no FC4 clock-gate write before it".to_string(),
                LcdClock::Gated(gate) => format!(
                    "no ANACTRL.FRO192M_CTRL write that sets bit 30 since the FC4 clock-gate write at {gate}"
                ),
                LcdClock::SourceOn(gate) => format!(
                    "no SYSCON.FROHFDIV write of 3 after the ANACTRL.FRO192M_CTRL write since the FC4 clock-gate write at {gate}"
                ),
            };
            return Err(format!(
                "SYSCON.FCCLKSEL4 is written at {} with {missing}",
                entry.at
            ));
        }
    }
    Ok(())
}

fn ending_matches(ending: Option<&Ending>, expected: ExpectedEnding) -> bool {
    matches!(
        (ending, expected),
        (
            Some(Ending::TimeLimit),
            ExpectedEnding::LoopRunsOn | ExpectedEnding::Parked
        )
    )
}

fn is_inert(event: &Event) -> bool {
    matches!(
        event,
        Event::WatchdogFed
            | Event::End(_)
            | Event::StoppedAt(_)
            | Event::MpuProbe { .. }
            | Event::InterruptsDelivered { .. }
            | Event::InterruptsDeliveredAtBreadcrumb { .. }
    )
}

// WHY: The screen sends on DMA0 channel 13, the FC4 transmit channel. The LCD set-up ends with its CFG at 0x4008_24D0, and the fatal path writes its XFERCFG at 0x4008_24D8, ENABLESET0 and SETTRIG0 at 0x4008_2070, each with only bit 13
const FAULT_SCREEN_CHANNEL: u32 = 1 << 13;

fn is_fault_screen_channel_setup(event: &Event) -> bool {
    is_write(event, "DMA0+0x4d0", |_| true)
}

fn is_fault_screen_send(event: &Event) -> bool {
    is_write(event, "DMA0+0x4d8", |_| true)
        || is_write(event, "DMA0.ENABLESET0", |value| {
            value == FAULT_SCREEN_CHANNEL
        })
        || is_write(event, "DMA0+0x70", |value| value == FAULT_SCREEN_CHANNEL)
}

fn parked_after_the_code(entries: &[Entry], park: &[ParkFunction]) -> Outcome {
    let stored = raised_records(entries)
        .pop()
        .ok_or("the scenario expects a park and no fault code was recorded")?;
    let code = &entries[stored];
    let after_the_code = &entries[stored + 1..];
    if let Some(entry) = after_the_code
        .iter()
        .find(|entry| !is_inert(&entry.event) && !is_fault_screen_send(&entry.event))
    {
        return Err(format!(
            "{:?} at {} follows the fault code at {}, and a parked image does nothing but send the fault screen and feed the watchdog",
            entry.event, entry.at, code.at
        ));
    }
    if let Some(setup) = entries[..stored]
        .iter()
        .find(|entry| is_fault_screen_channel_setup(&entry.event))
        && !after_the_code
            .iter()
            .any(|entry| is_fault_screen_send(&entry.event))
    {
        return Err(format!(
            "the screen's DMA channel was set up at {}, before the fault code at {}, and no fault screen send follows the code",
            setup.at, code.at
        ));
    }
    let delivered_at_the_code = |wanted: u32| {
        after_the_code
            .iter()
            .find_map(|entry| match entry.event {
                Event::InterruptsDeliveredAtBreadcrumb { irq, count } if irq == wanted => {
                    Some(count)
                }
                _ => None,
            })
            .unwrap_or(0)
    };
    for entry in after_the_code {
        if let Event::InterruptsDelivered { irq, count } = entry.event {
            let before = delivered_at_the_code(irq);
            if count != before {
                return Err(format!(
                    "IRQ {irq} was delivered {} times after the fault code at {}, and FAULTMASK keeps every configurable interrupt out of a parked image",
                    count.saturating_sub(before),
                    code.at
                ));
            }
        }
    }
    let ranges = park
        .iter()
        .map(|function| {
            format!(
                "{} ({:#x}..{:#x})",
                function.name, function.start, function.end
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let stop = after_the_code
        .iter()
        .rev()
        .find_map(|entry| match entry.event {
            Event::StoppedAt(pc) => Some(pc),
            _ => None,
        });
    match stop {
        Some(pc)
            if park
                .iter()
                .any(|function| (function.start..function.end).contains(&pc)) =>
        {
            Ok(())
        }
        Some(pc) => Err(format!(
            "the run stopped at {pc:#x}, outside {ranges} where the park loop runs"
        )),
        None => Err(format!(
            "the run left no stop address after the fault code, so nothing shows it parked in {ranges}"
        )),
    }
}

fn is_lcd_start(event: &Event) -> bool {
    matches!(event, Event::Write { register, .. } if register.starts_with("FC4"))
}

fn panel_after_fault(entries: &[Entry], expectation: &Expectation) -> Outcome {
    let (screen, usb) = match expectation.after_fault {
        AfterFault::None => return Ok(()),
        AfterFault::UsbOnly => (false, true),
        AfterFault::PanelOnly => (true, false),
        AfterFault::PanelAndUsb | AfterFault::AudioPlaying => (true, true),
    };
    match (first(entries, is_lcd_start), screen) {
        (Some(_), true) | (None, false) => {}
        (None, true) => {
            return Err("the LCD never started on a path that completed the rails".into());
        }
        (Some(lit), false) => {
            return Err(format!(
                "the LCD started at {} on a path that may have skipped the panel's power",
                lit.at
            ));
        }
    }
    let connect = first(entries, |event| matches!(event, Event::UsbConnected));
    let detached = last(entries, |event| {
        matches!(event, Event::UsbConnected | Event::UsbDisconnected)
    })
    .filter(|entry| matches!(entry.event, Event::UsbDisconnected));
    match (connect, usb, detached) {
        (None, false, _) | (Some(_), true, None) => Ok(()),
        (None, true, _) => Err("USB never attached on a path whose core PLL locked".into()),
        (Some(connect), false, _) => Err(format!(
            "USB connected at {} on a path whose core PLL never locked",
            connect.at
        )),
        (Some(_), true, Some(detached)) => Err(format!(
            "USB was detached at {} and the fault should have left it attached",
            detached.at
        )),
    }
}

fn breadcrumb_matches(entries: &[Entry], expected: Option<Recorded>) -> Outcome {
    let first_store = entries.iter().position(|entry| {
        matches!(
            entry.event,
            Event::Breadcrumb(_) | Event::BreadcrumbMagic(_)
        )
    });
    let started = first_store.is_some_and(|first| {
        entries[first].event == Event::Breadcrumb(Breadcrumb::START.pack())
            && entries.get(first + 1).map(|entry| &entry.event)
                == Some(&Event::BreadcrumbMagic(MAGIC))
    });
    if !started {
        return Err(format!(
            "the image did not start its breadcrumb with the fresh record and then the magic word: its first breadcrumb store is {:?}",
            first_store.map(|first| &entries[first].event)
        ));
    }
    if let Some(entry) = entries
        .iter()
        .skip(first_store.map_or(0, |first| first + 2))
        .find(|entry| matches!(entry.event, Event::BreadcrumbMagic(_)))
    {
        return Err(format!(
            "the magic word was stored again at {}, after the breadcrumb started",
            entry.at
        ));
    }
    let crumb = entries
        .iter()
        .rev()
        .find_map(|entry| breadcrumb(&entry.event))
        .unwrap_or(Breadcrumb::START);
    let kept = (crumb.code != 0).then_some((crumb.code, crumb.weight, crumb.place));
    let wanted = expected.map(|record| (record.code.number(), record.weight, record.place));
    if kept != wanted {
        return Err(format!(
            "the breadcrumb ends as {crumb:?}, the scenario expects the record {expected:?}"
        ));
    }
    Ok(())
}

fn end_state(entries: &[Entry], expectation: &Expectation, park: &[ParkFunction]) -> Outcome {
    let ending = entries.iter().find_map(|entry| match &entry.event {
        Event::End(ending) => Some(ending),
        _ => None,
    });
    if !ending_matches(ending, expectation.ending) {
        return Err(format!(
            "the run ended with {ending:?}, the scenario expects {:?}",
            expectation.ending
        ));
    }
    let stored_index = raised_records(entries).pop();
    let stored = stored_index.map(|index| &entries[index]);
    breadcrumb_matches(entries, expectation.record)?;
    if expectation.ending == ExpectedEnding::Parked {
        parked_after_the_code(entries, park)?;
    }
    let silenced = expectation.silence != Silence::NotRequired;
    let faulted = expectation.fault().is_some() || expectation.ending != ExpectedEnding::LoopRunsOn;
    if !faulted || !silenced {
        return Ok(());
    }
    if expectation.silence == Silence::WordAndTransmitDma {
        let last_word = last(entries, is_preamp_word);
        if !last_word.is_some_and(|entry| is_all_off(&entry.event)) {
            return Err(format!(
                "the last latched preamp word is {:?}, not the all-off word",
                last_word.map(|entry| &entry.event)
            ));
        }
    }
    let start = fault_start(entries).ok_or("the scenario expects a fault and none started")?;
    if let Some(connected) = last(entries, |event| {
        is_output(event, OutputPin::TransportActive, Level::High)
            || is_output(event, OutputPin::TransportActive, Level::Low)
    })
    .filter(|entry| is_output(&entry.event, OutputPin::TransportActive, Level::High))
    {
        return Err(format!(
            "P0_28 rose at {} and is still high when the run ends in a fault",
            connected.at
        ));
    }
    if let Some(enabled) = entries
        .iter()
        .find(|entry| entry.at >= start && is_output_enabling(&entry.event))
    {
        return Err(format!(
            "{:?} at {} unmutes an output after the fault started at {start}",
            enabled.event, enabled.at
        ));
    }
    let silenced = entries
        .iter()
        .find(|entry| {
            entry.at >= start
                && is_write(&entry.event, "DMA0.ENABLECLR0", |value| {
                    value & TRANSMIT_DMA_CHANNELS == TRANSMIT_DMA_CHANNELS
                })
        })
        .ok_or_else(|| {
            format!("after the fault started at {start} the trace lacks the TX DMA stop")
        })?;
    let word_after = if expectation.silence == Silence::WordAndTransmitDma {
        Some(
            entries
                .iter()
                .find(|entry| entry.at >= start && is_all_off(&entry.event))
                .ok_or_else(|| {
                    format!("after the fault started at {start} the trace lacks the all-off word")
                })?,
        )
    } else {
        None
    };
    let stored = stored.ok_or("the fault path stored no fault code")?;
    let last_enable = entries.iter().rposition(|entry| {
        is_output_enabling(&entry.event)
            || is_output(&entry.event, OutputPin::TransportActive, Level::High)
    });
    let recorded_after_the_audio_stopped = stored_index
        .zip(last_enable)
        .is_none_or(|(stored_index, last_enable)| stored_index > last_enable);
    if recorded_after_the_audio_stopped && silenced.at > stored.at {
        return Err(format!(
            "the fault code was stored at {} before the TX DMA stopped at {}",
            stored.at, silenced.at
        ));
    }
    if expectation.ending == ExpectedEnding::Parked
        && let Some(word_after) = word_after
        && !(silenced.at <= word_after.at && word_after.at <= stored.at)
    {
        return Err(format!(
            "D15 orders TX DMA stop, all-off word, fault code, park; the trace has them at {}, {}, {}",
            silenced.at, word_after.at, stored.at
        ));
    }
    Ok(())
}

fn is_converter_transaction(event: &Event) -> bool {
    let Event::I2c(transaction) = event else {
        return false;
    };
    let converter = transaction.address() == DAC_ADDRESS || is_adc_address(transaction.address());
    converter && transaction.status() != I2cStatus::Abandoned
}

fn converter_bus_quiet_after_fault(entries: &[Entry], expectation: &Expectation) -> Outcome {
    if expectation.silence == Silence::NotRequired {
        return Ok(());
    }
    let Some(start) = fault_start(entries) else {
        return Ok(());
    };
    match entries
        .iter()
        .find(|entry| entry.at >= start && is_converter_transaction(&entry.event))
    {
        None => Ok(()),
        Some(entry) => Err(format!(
            "{:?} at {} talks to a converter after the fault started at {start}",
            entry.event, entry.at
        )),
    }
}

fn converter_bus_configured_before_its_first_transfer(entries: &[Entry]) -> Outcome {
    let configured = entries
        .iter()
        .position(|entry| is_write(&entry.event, "FC3.CFG", |value| value & ENABLE_BIT != 0));
    let transfer = entries
        .iter()
        .position(|entry| matches!(entry.event, Event::I2c(_)));
    match (configured, transfer) {
        (None, None) => Ok(()),
        (None, Some(index)) => Err(format!(
            "FC3 carried a transfer at {} and FC3.CFG is never set with MSTEN",
            entries[index].at
        )),
        (Some(configured), Some(transfer)) if transfer < configured => Err(format!(
            "FC3 carried a transfer at {} before FC3.CFG was set at {}",
            entries[transfer].at, entries[configured].at
        )),
        (Some(_), _) => Ok(()),
    }
}

fn silenced_before_the_fault(entries: &[Entry], expectation: &Expectation) -> Outcome {
    if expectation.silence == Silence::NotRequired
        || expectation.ending != ExpectedEnding::LoopRunsOn
    {
        return Ok(());
    }
    let quiesced = entries
        .iter()
        .position(|entry| {
            is_write(&entry.event, "DMA0.ENABLECLR0", |value| {
                value == EVERY_AUDIO_DMA_CHANNEL
            })
        })
        .unwrap_or(0);
    let Some(start) = fault_start(entries) else {
        return Ok(());
    };
    let until = entries
        .iter()
        .rposition(|entry| entry.at <= start)
        .unwrap_or(0);
    let stops: Vec<(&str, EventTest)> = {
        let mut stops: Vec<(&str, EventTest)> = vec![
            (
                "the tick timer stopped",
                Box::new(|event| is_write(event, "CTIMER0.TCR", |value| value == 0)),
            ),
            (
                "every audio DMA channel stopped",
                Box::new(|event| {
                    is_write(event, "DMA0.ENABLECLR0", |value| {
                        value & EVERY_AUDIO_DMA_CHANNEL == EVERY_AUDIO_DMA_CHANNEL
                    })
                }),
            ),
        ];
        for register in I2S_FLEXCOMMS {
            stops.push((
                register,
                Box::new(move |event| is_write(event, register, |value| value & ENABLE_BIT == 0)),
            ));
        }
        stops
    };
    for (what, stopped) in &stops {
        if !flexcomm_clocked_by(entries, what, quiesced) {
            continue;
        }
        let seen = (quiesced + 1..=until).any(|index| stopped(&entries[index].event));
        if !seen {
            return Err(format!(
                "{what} did not happen between the quiesce at {} and the fault at {start}",
                entries[quiesced].at
            ));
        }
    }
    Ok(())
}

fn quiet_at_rest(entries: &[Entry], quiet_after: Option<Nanoseconds>) -> Outcome {
    let Some(from) = quiet_after else {
        return Ok(());
    };
    let noisy: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.at >= from)
        .filter(|entry| !is_inert(&entry.event))
        .collect();
    match noisy.first() {
        None => Ok(()),
        Some(entry) => Err(format!(
            "{} event(s) after {from} with the panel idle, first {:?} at {}",
            noisy.len(),
            entry.event,
            entry.at
        )),
    }
}

pub fn furthest_boot_step(trace: &Trace) -> Option<&'static str> {
    let boot = before_the_fault(&trace.entries);
    MILESTONES
        .iter()
        .rev()
        .find(|milestone| first(boot, |event| milestone.opens_with(event)).is_some())
        .map(|milestone| milestone.label())
}

impl Milestone {
    const fn label(self) -> &'static str {
        match self {
            Milestone::Quiesce => "quiesce",
            Milestone::Clocks => "clocks",
            Milestone::UsbAttach => "USB attach",
            Milestone::AnalogOff => "analog off",
            Milestone::Rails => "rails",
            Milestone::StartLcd => "LCD start",
            Milestone::AudioClocks => "audio clocks",
            Milestone::DacAttenuate => "DAC attenuate",
            Milestone::AdcProbe => "ADC probe",
            Milestone::Transport => "transport",
            Milestone::DacInit => "DAC init",
            Milestone::AdcInit => "ADC init",
            Milestone::Panel => "panel",
        }
    }
}

fn ramp_targets() -> [(VolumeRegister, Attenuation); 6] {
    let VolumeTargets { main, headphone } =
        volume_targets(Knobs::first_scan(KnobReadings([MODEL_KNOB_READING; 5])));
    [
        (VolumeRegister::FactoryZeroA, Attenuation::ZERO_DB),
        (VolumeRegister::FactoryZeroB, Attenuation::ZERO_DB),
        (VolumeRegister::MainA, main),
        (VolumeRegister::MainB, main),
        (VolumeRegister::HeadphoneA, headphone),
        (VolumeRegister::HeadphoneB, headphone),
    ]
}

fn ramp_at_most_knob_target(entries: &[Entry], expectation: &Expectation) -> Outcome {
    let targets = ramp_targets();
    let mut last: [Option<u8>; 6] = [None; 6];
    for entry in entries {
        let Some((register, value, _)) = dac_write(&entry.event) else {
            continue;
        };
        let Some(index) = targets.iter().position(|(each, _)| *each as u8 == register) else {
            continue;
        };
        let target = targets[index].1.half_decibel_steps();
        if value < target {
            return Err(format!(
                "DAC register {register} written {value:#04x} at {}, louder than the knob law target {target:#04x}",
                entry.at
            ));
        }
        last[index] = Some(value);
    }
    let reaches_running =
        expectation.fault().is_none() && expectation.ending == ExpectedEnding::LoopRunsOn;
    if !reaches_running {
        return Ok(());
    }
    for ((register, target), written) in targets.iter().zip(last) {
        if written != Some(target.half_decibel_steps()) {
            return Err(format!(
                "Running not reached: DAC register {} last written {written:?}, the knob law target is {:#04x}",
                *register as u8,
                target.half_decibel_steps()
            ));
        }
    }
    Ok(())
}

fn main_clock_stays_put(entries: &[Entry], expectation: &Expectation) -> Outcome {
    if expectation.main_clock == MainClock::MaySwitch {
        return Ok(());
    }
    match first(entries, |event| matches!(event, Event::MainClock { .. })) {
        None => Ok(()),
        Some(entry) => Err(format!(
            "the main clock switched at {} after the core PLL lock timed out",
            entry.at
        )),
    }
}
