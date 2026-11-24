use super::program::Program;
use super::run::{FRAME_COUNTS, Monitoring, Phase, Preset, Run, SCENARIOS, TickRun};
use crate::price::UsbPath;
use dsp::FramesPerTick;
use std::fmt::Write;
use std::path::Path;

pub const WARM_UP_TICKS: usize = 40;
pub const TRACED_TICKS: usize = 8;
const FADE_TICKS: usize = dsp::CROSSFADE_TICKS;
const TICKS_INTO_THE_FADE_BEFORE_TRACING: usize = 12;

const BENCH_MEMORY: [(u32, u32); 5] = [
    (0x0000_0000, 0x0010_0000),
    (0x2000_0000, 0x0004_0000),
    (
        PERIPHERALS_BELOW_THE_FLASH_CONTROLLER,
        FLASH_CONTROLLER - PERIPHERALS_BELOW_THE_FLASH_CONTROLLER,
    ),
    (FLASH_CONTROLLER_END, PERIPHERALS_END - FLASH_CONTROLLER_END),
    (0xE000_0000, 0x0010_0000),
];
const BENCH_STACK_TOP: u32 = 0x2003_FF00;
const IMAGE_MEMORY: [(u32, u32); 4] = [
    (0x2000_0000, 0x0004_0000),
    (
        PERIPHERALS_BELOW_THE_FLASH_CONTROLLER,
        FLASH_CONTROLLER - PERIPHERALS_BELOW_THE_FLASH_CONTROLLER,
    ),
    (FLASH_CONTROLLER_END, PERIPHERALS_END - FLASH_CONTROLLER_END),
    (0xE000_0000, 0x0010_0000),
];
// WHY: Peripherals start at 0x4000_0000, the flash controller block is 0x4003_4000 to 0x4003_4FFF, left unmapped so any access to it stops the run, and USB RAM ends the mapped range at 0x4010_3FFF, UM11126 section 44.1
const PERIPHERALS_BELOW_THE_FLASH_CONTROLLER: u32 = 0x4000_0000;
const FLASH_CONTROLLER: u32 = 0x4003_4000;
const FLASH_CONTROLLER_END: u32 = 0x4003_5000;
const PERIPHERALS_END: u32 = 0x4010_4000;
// WHY: The factory page at 0x38200 and its records to 0x38A00 sit in this flash page, mapped read-only so a write to it stops the boot
const IMAGE_READ_ONLY_MEMORY: [(u32, u32); 1] = [(0x0003_8000, 0x0000_1000)];
// WHY: CMSIS MPU_TYPE at 0xE000_ED90 with DREGION in bits 15:8, and the fence needs five regions; the part has eight per security state, UM11126 rev 1.8 7.5.3.2.1
const MPU_TYPE_DREGION_BYTE: u32 = 0xE000_ED91;
const MPU_REGIONS_THE_FENCE_NEEDS: u32 = 5;
// WHY: SVD28, status bits the hardware raises on its own that the board polls at boot: XO32M_STATUS.XO_READY once the crystal runs, SPI1 FIFOSTAT TXEMPTY and TXNOTFULL and SPI1 STAT MSTIDLE at their reset values 0x30 and 0x100
const STATUS_BYTES_THE_HARDWARE_RAISES: [(u32, u32); 3] = [
    (0x4001_3024, 0x01),
    (0x4008_7E04, 0x30),
    (0x4008_7409, 0x01),
];
pub const IMAGE_MAIN_LOOP: &str = "board::firmware::__cortex_m_rt_main";
const IMAGE_STACK_TOP: u32 = 0x2003_0000;
const IMAGE_BOOT_INSTRUCTIONS: u32 = 2_000_000;

#[derive(Debug, PartialEq, Eq)]
pub struct MissingSymbol(pub String);

struct Entries {
    init: u32,
    request: u32,
    tick: u32,
    usb: UsbEntries,
}

struct UsbEntries {
    event: u32,
    console_event: u32,
    start_of_frame: u32,
    start_of_frame_with_console: u32,
    playback: u32,
    feedback: u32,
    capture: u32,
    console_packet_read: u32,
    console_bytes: u32,
    configuration_descriptor: u32,
    sampling_frequency: u32,
    capture_reselected: u32,
    control_continues: u32,
    check: u32,
}

pub const FEEDBACK_POLL_MICROFRAMES: usize = 8;
const CONTROL_EVENTS_AFTER_A_SETUP: usize = 8;

struct ImageEntries {
    reset: u32,
    main_loop: (u32, u32),
    tick: u32,
    usb: u32,
    dsp_mode: u32,
}

pub fn bench_plan(elf: &Path, bench: &Program) -> Result<String, MissingSymbol> {
    let find = |name: &str| {
        bench
            .address_of(name)
            .ok_or_else(|| MissingSymbol(name.to_string()))
    };
    let entries = Entries {
        init: find("budget_init")?,
        request: find("budget_request")?,
        tick: find("budget_tick")?,
        usb: UsbEntries {
            event: find("budget_usb_event")?,
            console_event: find("budget_usb_console_event")?,
            start_of_frame: find("budget_usb_host_starts_a_microframe")?,
            start_of_frame_with_console: find(
                "budget_usb_host_starts_a_microframe_with_console_bytes_waiting",
            )?,
            playback: find("budget_usb_host_sends_a_playback_packet")?,
            feedback: find("budget_usb_host_reads_feedback")?,
            capture: find("budget_usb_host_reads_a_capture_packet")?,
            console_packet_read: find("budget_usb_host_reads_the_console_packet")?,
            console_bytes: find("budget_usb_host_sends_console_bytes")?,
            configuration_descriptor: find("budget_usb_host_reads_the_configuration_descriptor")?,
            sampling_frequency: find("budget_usb_host_sets_the_sampling_frequency")?,
            capture_reselected: find("budget_usb_host_reselects_capture")?,
            control_continues: find("budget_usb_host_continues_the_control_transfer")?,
            check: find("budget_usb_check")?,
        },
    };
    let mut plan = String::new();
    for (base, size) in BENCH_MEMORY {
        line(&mut plan, format_args!("map {base:#x} {size:#x}"));
    }
    line(&mut plan, format_args!("load {}", elf.display()));
    line(&mut plan, format_args!("stack {BENCH_STACK_TOP:#x}"));
    for frames in FRAME_COUNTS {
        for (monitoring, preset) in SCENARIOS {
            let traces_usb = (monitoring, preset) == SCENARIOS[0];
            bench_scenario(&mut plan, &entries, frames, monitoring, preset, traces_usb);
        }
    }
    Ok(plan)
}

struct UsbHost<'a> {
    entries: &'a UsbEntries,
    frames: FramesPerTick,
    microframe: usize,
}

impl UsbHost<'_> {
    fn event(&self, plan: &mut String, path: UsbPath, traced: bool) {
        let entry = match path {
            UsbPath::ConsoleReceived | UsbPath::StartOfFrameWithConsole => {
                self.entries.console_event
            }
            UsbPath::Microframe | UsbPath::ControlTransfer => self.entries.event,
        };
        match traced {
            true => trace(plan, Run::BenchUsb(self.frames, path), entry),
            false => call(plan, entry),
        }
    }

    fn microframe(&mut self, plan: &mut String, traced: bool) {
        call(plan, self.entries.start_of_frame);
        self.event(plan, UsbPath::Microframe, traced);
        self.rest_of_the_microframe(plan);
    }

    fn rest_of_the_microframe(&mut self, plan: &mut String) {
        call(plan, self.entries.playback);
        if self.microframe % FEEDBACK_POLL_MICROFRAMES == 0 {
            call(plan, self.entries.feedback);
        }
        call(plan, self.entries.capture);
        self.microframe += 1;
    }

    fn occasional_events(&mut self, plan: &mut String) {
        call(plan, self.entries.start_of_frame_with_console);
        self.event(plan, UsbPath::StartOfFrameWithConsole, true);
        self.rest_of_the_microframe(plan);
        call(plan, self.entries.console_packet_read);
        call(plan, self.entries.console_bytes);
        self.event(plan, UsbPath::ConsoleReceived, true);
        for setup in [
            self.entries.configuration_descriptor,
            self.entries.sampling_frequency,
            self.entries.capture_reselected,
        ] {
            call(plan, setup);
            self.event(plan, UsbPath::ControlTransfer, true);
            for _ in 0..CONTROL_EVENTS_AFTER_A_SETUP {
                call(plan, self.entries.control_continues);
                self.event(plan, UsbPath::ControlTransfer, true);
            }
        }
        call(plan, self.entries.check);
    }
}

fn bench_scenario(
    plan: &mut String,
    entries: &Entries,
    frames: FramesPerTick,
    monitoring: Monitoring,
    preset: Preset,
    traces_usb: bool,
) {
    let flag = |set: bool| u32::from(set);
    line(
        plan,
        format_args!(
            "call {:#x} {} {} {}",
            entries.init,
            frames.count(),
            flag(monitoring == Monitoring::BothInputs),
            flag(preset == Preset::FiveBands)
        ),
    );
    let tick_run = |phase| TickRun {
        frames,
        monitoring,
        preset,
        phase,
    };
    let mut host = UsbHost {
        entries: &entries.usb,
        frames,
        microframe: 0,
    };
    let microframes = |plan: &mut String,
                       host: &mut UsbHost<'_>,
                       count: usize,
                       traced: Option<Phase>,
                       usb: bool| {
        for _ in 0..count {
            host.microframe(plan, usb);
            match traced {
                Some(phase) => trace(plan, Run::BenchTick(tick_run(phase)), entries.tick),
                None => call(plan, entries.tick),
            }
        }
    };
    let request = |plan: &mut String, on: bool| {
        line(
            plan,
            format_args!("call {:#x} {}", entries.request, flag(on)),
        );
    };
    let host = &mut host;
    microframes(plan, host, WARM_UP_TICKS, None, false);
    microframes(plan, host, TRACED_TICKS, Some(Phase::Bypass), traces_usb);
    if traces_usb {
        host.occasional_events(plan);
    }
    request(plan, true);
    microframes(plan, host, 1, Some(Phase::FadeToOnFirstTick), false);
    microframes(
        plan,
        host,
        TICKS_INTO_THE_FADE_BEFORE_TRACING - 1,
        None,
        false,
    );
    microframes(plan, host, TRACED_TICKS, Some(Phase::FadeToOn), false);
    microframes(plan, host, settle_after_fade(), None, false);
    microframes(plan, host, TRACED_TICKS, Some(Phase::On), false);
    request(plan, false);
    microframes(plan, host, 1, Some(Phase::FadeToBypassFirstTick), false);
    microframes(
        plan,
        host,
        TICKS_INTO_THE_FADE_BEFORE_TRACING - 1,
        None,
        false,
    );
    microframes(plan, host, TRACED_TICKS, Some(Phase::FadeToBypass), false);
}

const fn settle_after_fade() -> usize {
    FADE_TICKS - TICKS_INTO_THE_FADE_BEFORE_TRACING - TRACED_TICKS + WARM_UP_TICKS
}

pub fn image_plan(elf: &Path, image: &Program, symbols: &str) -> Result<String, MissingSymbol> {
    let find = |name: &str| {
        image
            .address_of(name)
            .ok_or_else(|| MissingSymbol(name.to_string()))
    };
    let entries = ImageEntries {
        reset: find("Reset")?,
        main_loop: image
            .range_of(IMAGE_MAIN_LOOP)
            .ok_or_else(|| MissingSymbol(IMAGE_MAIN_LOOP.to_string()))?,
        tick: find("audio_tick_interrupt")?,
        usb: find("usb_interrupt")?,
        dsp_mode: data_address(symbols, "board::firmware::DSP_MODE")
            .ok_or_else(|| MissingSymbol("board::firmware::DSP_MODE".to_string()))?,
    };
    let mut plan = String::new();
    for (base, size) in IMAGE_MEMORY {
        line(&mut plan, format_args!("map {base:#x} {size:#x}"));
    }
    for (base, size) in IMAGE_READ_ONLY_MEMORY {
        line(&mut plan, format_args!("rom {base:#x} {size:#x}"));
    }
    line(&mut plan, format_args!("load {}", elf.display()));
    line(&mut plan, format_args!("stack {IMAGE_STACK_TOP:#x}"));
    line(
        &mut plan,
        format_args!("poke8 {MPU_TYPE_DREGION_BYTE:#x} {MPU_REGIONS_THE_FENCE_NEEDS}"),
    );
    for (address, byte) in STATUS_BYTES_THE_HARDWARE_RAISES {
        line(&mut plan, format_args!("poke8 {address:#x} {byte:#x}"));
    }
    line(
        &mut plan,
        format_args!(
            "boot {:#x} {IMAGE_BOOT_INSTRUCTIONS} {:#x} {:#x}",
            entries.reset, entries.main_loop.0, entries.main_loop.1
        ),
    );
    let microframes = |plan: &mut String, count: usize, traced: Option<Phase>, usb: bool| {
        for _ in 0..count {
            match usb {
                true => trace(plan, Run::ImageUsbIdle, entries.usb),
                false => call(plan, entries.usb),
            }
            match traced {
                Some(phase) => trace(plan, Run::ImageTick(phase), entries.tick),
                None => call(plan, entries.tick),
            }
        }
    };
    let request = |plan: &mut String, on: bool| {
        line(
            plan,
            format_args!("poke8 {:#x} {}", entries.dsp_mode, u32::from(on)),
        );
    };
    microframes(&mut plan, WARM_UP_TICKS, None, false);
    microframes(&mut plan, TRACED_TICKS, Some(Phase::Bypass), true);
    request(&mut plan, true);
    microframes(&mut plan, 1, Some(Phase::FadeToOnFirstTick), false);
    microframes(
        &mut plan,
        TICKS_INTO_THE_FADE_BEFORE_TRACING - 1,
        None,
        false,
    );
    microframes(&mut plan, TRACED_TICKS, Some(Phase::FadeToOn), false);
    microframes(&mut plan, settle_after_fade(), None, false);
    microframes(&mut plan, TRACED_TICKS, Some(Phase::On), false);
    request(&mut plan, false);
    microframes(&mut plan, 1, Some(Phase::FadeToBypassFirstTick), false);
    microframes(
        &mut plan,
        TICKS_INTO_THE_FADE_BEFORE_TRACING - 1,
        None,
        false,
    );
    microframes(&mut plan, TRACED_TICKS, Some(Phase::FadeToBypass), false);
    Ok(plan)
}

fn data_address(symbols: &str, name: &str) -> Option<u32> {
    symbols.lines().find_map(|line| {
        let mut words = line.trim().splitn(4, ' ');
        let address = words.next()?;
        let rest: Vec<&str> = words.collect();
        let symbol = rest.last()?;
        (*symbol == name).then(|| u32::from_str_radix(address, 16).ok())?
    })
}

fn call(plan: &mut String, entry: u32) {
    line(plan, format_args!("call {entry:#x}"));
}

fn trace(plan: &mut String, run: Run, entry: u32) {
    line(plan, format_args!("trace {} {entry:#x}", run.label()));
}

fn line(plan: &mut String, text: std::fmt::Arguments<'_>) {
    let _ = plan.write_fmt(text);
    plan.push('\n');
}
