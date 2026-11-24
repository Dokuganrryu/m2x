mod build;
mod held_low;
mod inherited;
mod judge;
mod rails;
mod registers;
mod trace;

pub use build::Image;
pub use held_low::{
    ADC_ENABLE, CONVERTER_ENABLE, HELD_LOW, HeldPin, PHANTOM, PinAtEntry, pin_at_entry,
};
pub use inherited::{INHERITED, INHERITED_ENABLED_CHANNELS, LIVE_RANGES, LiveRange, Poke};
pub use judge::{Finding, Hazard, Summary, hazards, judge};
pub use rails::quiesce_lows;
pub use registers::{RegisterNames, starts_something};
pub use trace::{Context, Event, HandOverState, TraceError, Write, read};

use crate::exit::Outcome;
use crate::link_check::Layout;
use crate::scan;
use build::{Built, Workspace};
use std::fmt::Write as _;
use std::ops::Range;

// WHY: Peripherals from 0x40000000 with the flash controller block 0x40034000 to 0x40034FFF left unmapped so any access stops the run, USB RAM ends at 0x40103FFF, UM11126 section 44.1; the factory page at 0x38200 sits in the read-only page at 0x38000
const MEMORY: [(u32, u32); 4] = [
    (0x2000_0000, 0x0004_0000),
    (0x4000_0000, 0x0003_4000),
    (0x4003_5000, 0x000C_F000),
    (0xE000_0000, 0x0010_0000),
];
const READ_ONLY_MEMORY: [(u32, u32); 1] = [(0x0003_8000, 0x0000_1000)];
// WHY: CMSIS MPU_TYPE DREGION byte at 0xE000ED91, five regions for the fence; SVD28 SPI1 FIFOSTAT TXEMPTY and TXNOTFULL and STAT MSTIDLE, the bits the SPI raises on its own
const HARDWARE_RAISED_BYTES: [(u32, u8); 3] =
    [(0xE000_ED91, 5), (0x4008_7E04, 0x30), (0x4008_7409, 0x01)];
const STARTUP_INSTRUCTIONS: u32 = 200_000;
const QUIESCE_ONLY_IDLE_INSTRUCTIONS: u32 = 3_000_000;
const WWDT_FEED: u32 = 0x4000_C008;
const WWDT_FEED_FIRST: u32 = 0xAA;
const FEEDS_THE_IDLE_RUN_MUST_SHOW: usize = 2;
const HAND_OVER_ROUTINE: &str = "__pre_init";
const QUIESCE_LOWS_HOLD: &str =
    "past `main` the first four pin drives are P0_28, P1_26, P1_22 and P1_10 low, in that order";

const WHAT_THE_PASS_PROVES: &str = "Started at Reset from the modelled state of a previous image, with PRIMASK clear, the tick, USB and SysTick exceptions pending and five DMA channels live, the image wrote no byte of a live inherited DMA buffer, descriptor or head entry while its channel was enabled, every exception it took returned with the hand-over quiesced, and it reached `main` with interrupts masked, every DMA channel disabled, the NVIC clear, SysTick's interrupt off and CTIMER0 stopped, and with the USB controller either disconnected or still connected only while the USB1 device clock the bootloader left on was there for the quiesce to detach it with.";
const WHAT_IT_CANNOT_PROVE: &str = "Not proven: the analog side (whether the DAC is silent when its DMA stops mid-buffer, and what the I2S block sends once its FIFO runs dry), whether a DMA beat already in flight lands after ENABLECLR0, an interrupt that becomes pending between resume and the first instruction other than those modelled as pending, peripheral timing, and every register the model treats as plain memory.";

fn symbol(symbols: &str, name: &str) -> Option<u32> {
    symbols.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let address = words.next()?;
        let _kind = words.next()?;
        (words.next()? == name && words.next().is_none())
            .then(|| u32::from_str_radix(address, 16).ok())
            .flatten()
    })
}

pub fn extent(symbols: &str, name: &str) -> Option<Range<u32>> {
    const THUMB_BIT: u32 = 1;
    let start = symbol(symbols, name)? & !THUMB_BIT;
    let end = symbols
        .lines()
        .filter_map(|line| u32::from_str_radix(line.split_whitespace().next()?, 16).ok())
        .map(|address| address & !THUMB_BIT)
        .filter(|address| *address > start)
        .min()?;
    Some(start..end)
}

fn line(plan: &mut String, text: std::fmt::Arguments<'_>) {
    let _ = plan.write_fmt(text);
    plan.push('\n');
}

pub fn plan(image: Image, elf: &str, reset: u32, main: u32) -> String {
    let mut plan = String::new();
    for (base, size) in MEMORY {
        line(&mut plan, format_args!("map {base:#x} {size:#x}"));
    }
    for (base, size) in READ_ONLY_MEMORY {
        line(&mut plan, format_args!("rom {base:#x} {size:#x}"));
    }
    line(&mut plan, format_args!("load {elf}"));
    for item in INHERITED {
        for poke in item.pokes {
            match *poke {
                Poke::Word(address, word) => {
                    line(&mut plan, format_args!("poke32 {address:#x} {word:#x}"));
                }
                Poke::Byte(address, byte) => {
                    line(&mut plan, format_args!("poke8 {address:#x} {byte:#x}"));
                }
            }
        }
    }
    for (address, byte) in HARDWARE_RAISED_BYTES {
        line(&mut plan, format_args!("poke8 {address:#x} {byte:#x}"));
    }
    line(
        &mut plan,
        format_args!("sp {:#x}", inherited::INHERITED_STACK_POINTER),
    );
    line(&mut plan, format_args!("primask 0"));
    if image == Image::QuiesceOnly {
        line(&mut plan, format_args!("countflag"));
    }
    line(
        &mut plan,
        format_args!("run {reset:#x} {main:#x} {STARTUP_INSTRUCTIONS}"),
    );
    if image == Image::QuiesceOnly {
        line(
            &mut plan,
            format_args!("continue {QUIESCE_ONLY_IDLE_INSTRUCTIONS}"),
        );
    }
    plan
}

pub fn describe(finding: &Finding) -> String {
    match finding {
        Finding::LiveMemoryWritten { write, range } => format!(
            "write of {} bytes at {:#010x} by the instruction at {:#010x} (step {}, {:?}) lands in the inherited {} while DMA channel {} is enabled",
            write.size,
            write.address,
            write.pc,
            write.index,
            write.context,
            range.what,
            range.channel.0
        ),
        Finding::HandlerLeftHandOverLive {
            number,
            handler,
            hazards,
        } => format!(
            "exception {number} entered the handler at {handler:#010x} before the quiesce and returned with {hazards:?}"
        ),
        Finding::HandlerDidNotReturn {
            number,
            handler,
            pc,
        } => format!(
            "exception {number} entered the handler at {handler:#010x} before the quiesce and did not return, last at {pc:#010x}"
        ),
        Finding::StartupStopped { pc, reason } => {
            format!("the startup stopped at {pc:#010x}: {reason}")
        }
        Finding::EntryReachedWithHandOverLive { hazards } => {
            format!("`main` was reached with {hazards:?}")
        }
        Finding::RamWrittenWithOutputsConnected { write } => format!(
            "RAM at {:#010x} was written by the instruction at {:#010x} (step {}, {:?}) while P0_28, inherited high from the previous image, still connected the outputs",
            write.address, write.pc, write.index, write.context
        ),
    }
}

struct Checked {
    passed: bool,
    report: String,
}

fn overlap_report(symbols: &str) -> String {
    let (Some(start), Some(end)) = (symbol(symbols, "__sdata"), symbol(symbols, "__euninit"))
    else {
        return "  .data and .bss bounds not found".to_string();
    };
    let hit: Vec<&str> = LIVE_RANGES
        .iter()
        .filter(|range| range.overlaps(start, end))
        .map(|range| range.what)
        .collect();
    format!(
        "  .data, .bss and .uninit span {start:#010x}..{end:#010x} and overlap {} live inherited ranges: {}",
        hit.len(),
        hit.join(", ")
    )
}

fn register_report(names: &RegisterNames, summary: &Summary) -> String {
    let mut report = String::new();
    for address in &summary.peripheral_writes {
        let _ = writeln!(report, "    {address:#010x} {}", names.name(*address));
    }
    report
}

fn idle_checks(names: &RegisterNames, events: &[Event]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut feeds = 0;
    for event in events {
        let Event::Write(write) = event else { continue };
        if write.context != Context::After {
            continue;
        }
        let name = names.name(write.address);
        if starts_something(name, write.value) {
            problems.push(format!(
                "the idle run wrote {:#x} to {name} at {:#010x}, which starts a clock, the transport, USB, the analog word bus, a rail or the panel",
                write.value, write.address
            ));
        }
        if write.address == WWDT_FEED && write.value == WWDT_FEED_FIRST {
            feeds += 1;
        }
    }
    if feeds < FEEDS_THE_IDLE_RUN_MUST_SHOW {
        problems.push(format!(
            "the idle run fed the watchdog {feeds} times, fewer than {FEEDS_THE_IDLE_RUN_MUST_SHOW}"
        ));
    }
    problems
}

fn check(workspace: &Workspace, names: &RegisterNames, image: Image) -> Result<Checked, String> {
    let built: Built = workspace.build(image)?;
    let mut report = String::new();
    let mut passed = true;
    let layout = Layout::from_symbols(&built.symbols).map_err(|error| format!("{error:?}"))?;
    let _ = writeln!(
        report,
        "{} image: links under the memory.x assertions, {} bytes loaded, {} bytes of stack",
        image.slug(),
        layout.image.0,
        layout.stack.0
    );
    if image != Image::Full {
        let rules = workspace.scan_rules()?;
        let mut violations = scan::scan(&rules, &built.symbols, &built.instruction_listing)
            .map_err(|error| error.to_string())?;
        violations.extend(
            scan::scan_data_sections(&rules, &built.sections).map_err(|error| error.to_string())?,
        );
        let _ = writeln!(
            report,
            "  image scan with m2x-image.rules: {} violations",
            violations.len()
        );
        for violation in &violations {
            let _ = writeln!(report, "    {violation}");
        }
        passed &= violations.is_empty();
    }
    let _ = writeln!(report, "{}", overlap_report(&built.symbols));
    let reset = symbol(&built.symbols, "Reset").ok_or("the image has no `Reset`")?;
    let main = symbol(&built.symbols, "main").ok_or("the image has no `main`")?;
    let elf = built.elf.display().to_string();
    let output = workspace.emulate(&plan(image, &elf, reset, main), &built.folder)?;
    let events = read(&output).map_err(|error| error.to_string())?;
    let (findings, summary) = judge(&events);
    let _ = writeln!(
        report,
        "  first write that stops both transmit channels: step {}; SRAM writes before it: {}; exceptions taken before `main`: {:x?}; `main` reached at step {}",
        summary
            .first_transmit_stop
            .map_or("never".to_string(), |step| step.to_string()),
        summary.sram_writes_before_transmit_stop,
        summary.exceptions,
        summary
            .reached
            .map_or("never".to_string(), |step| step.to_string()),
    );
    for finding in &findings {
        if !matches!(finding, Finding::LiveMemoryWritten { .. }) {
            let _ = writeln!(report, "  {}", describe(finding));
        }
    }
    for range in LIVE_RANGES {
        let writes: Vec<&Write> = findings
            .iter()
            .filter_map(|finding| match finding {
                Finding::LiveMemoryWritten { write, range: hit } if *hit == range => Some(write),
                _ => None,
            })
            .collect();
        if let Some(first) = writes.first() {
            let _ = writeln!(
                report,
                "  {} writes land in the inherited {} while DMA channel {} is enabled, the first at {:#010x} by the instruction at {:#010x}, step {}, {:?}",
                writes.len(),
                range.what,
                range.channel.0,
                first.address,
                first.pc,
                first.index,
                first.context,
            );
        }
    }
    passed &= findings.is_empty();
    let table = extent(&built.symbols, HAND_OVER_ROUTINE)
        .ok_or("the image has no `__pre_init` with a symbol after it")?;
    for pin in HELD_LOW {
        let state = pin_at_entry(&events, &table, pin);
        let _ = writeln!(report, "  {}", state.describe(pin));
        passed &= state == PinAtEntry::DrivenLow;
    }
    if image == Image::QuiesceOnly {
        for problem in idle_checks(names, &events) {
            let _ = writeln!(report, "  {problem}");
            passed = false;
        }
        match quiesce_lows(&events) {
            Ok(()) => {
                let _ = writeln!(report, "  {QUIESCE_LOWS_HOLD}");
            }
            Err(problem) => {
                let _ = writeln!(report, "  {problem}");
                passed = false;
            }
        }
    }
    let _ = writeln!(report, "  registers written:");
    report.push_str(&register_report(names, &summary));
    let _ = writeln!(
        report,
        "  trace in {}",
        built.folder.join("trace.txt").display()
    );
    Ok(Checked { passed, report })
}

pub fn command() -> Outcome {
    let workspace = Workspace::locate();
    let names = match workspace.rows() {
        Ok(rows) => RegisterNames::from_rows(&rows),
        Err(error) => {
            eprintln!("{error}");
            return Outcome::UsageOrInputError;
        }
    };
    let mut every_image_holds = true;
    for image in Image::ALL {
        match check(&workspace, &names, image) {
            Ok(checked) => {
                print!("{}", checked.report);
                println!(
                    "{} image: {}",
                    image.slug(),
                    if checked.passed { "holds" } else { "fails" }
                );
                every_image_holds &= checked.passed;
            }
            Err(error) => {
                eprintln!("{} image: {error}", image.slug());
                return Outcome::UsageOrInputError;
            }
        }
    }
    if every_image_holds {
        println!("the hand-over holds for every image. {WHAT_THE_PASS_PROVES}");
        println!("{WHAT_IT_CANNOT_PROVE}");
        Outcome::GatePassed
    } else {
        println!("the hand-over check fails. {WHAT_IT_CANNOT_PROVE}");
        Outcome::GateFailed
    }
}
