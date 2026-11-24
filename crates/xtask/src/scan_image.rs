mod listings;

use crate::exit::Outcome;
use crate::scan::{self, GraphFinding, InputError, Rule, Violation};
use listings::{Listings, Workspace};
use std::fmt;

const PLANTED_CODE: &str = "board::firmware::image_plants::";
const RULES_FILE: &str = "m2x-image.rules";
const WHAT_THE_PASS_PROVES: &str = "In the clean release ELF: no symbol contains `flash`, `romapi` or the token `iap`; no immediate, literal-pool word, tracked register value or aligned word of .vector_table, .rodata or .data lies in 0x40034000..0x40034FFF, 0x50034000..0x50034FFF, AOREG1 at 0x40020084 or 0x50020084, the debug mailbox 0x4009C000..0x4009C0FF or 0x5009C000..0x5009C0FF, SCB AIRCR at 0xE000ED0C..0xE000ED0F, or 0x00038000..0x00038FFF, or equals one of the listed ROM flash API entries; no `movt` of 0x4003, 0x5003, 0x4002, 0x5002, 0x4009, 0x5009 or 0x0003 is left with an unknown low half; no instruction anywhere in the image is a VFP instruction, which Non-secure state faults as NOCP; and no function reached by direct calls from `audio_tick_interrupt` or `usb_interrupt` divides, uses the FPU, masks interrupts, calls indirectly, panics, formats, allocates or calls libm. So the image holds no code that names the flash controller or a ROM flash entry, which is the only way this part erases or programs flash, nor AOREG1 or the debug mailbox, which can end image boot at the next reset, nor the reset request register, so the bring-up image never resets itself by name, and the factory page cannot be written by any address the image spells out.";
const WHAT_IT_CANNOT_PROVE: &str = "Not proven: an address computed at run time (a base plus a register offset, a word built by arithmetic the scan does not track, a table filled at run time), a value crossing a branch the linear register tracking merges, and a forged ES9026 register number from an unsafe transmute. Two things narrow those gaps: flash is read only at or below 100 MHz (and in the `sound3` build, main copies the factory page and NMPA words to RAM once at boot before raising the clock to 150 MHz, so no later code reads flash at all); and every register write goes through the single board executor or the fatal path.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Speed,
    BoardForSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImagePlant {
    LiteralPoolFlashController,
    MovwMovtFlashController,
    FlashDriverSymbol,
    FactoryReadOutsideAccessor,
    DivisionInTick,
    BootRequestRegister,
    DebugMailbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageBuild {
    Clean(Level),
    Planted(ImagePlant),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    LiteralPool,
    MoveWideAndTop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Holds,
    CleanImageFailed { violations: usize },
    Refused(String),
    PlantPassed,
    FailedForAnotherReason(Violation),
    WrongEncoding(Encoding),
}

impl ImagePlant {
    pub const ALL: [ImagePlant; 7] = [
        ImagePlant::LiteralPoolFlashController,
        ImagePlant::MovwMovtFlashController,
        ImagePlant::FlashDriverSymbol,
        ImagePlant::FactoryReadOutsideAccessor,
        ImagePlant::DivisionInTick,
        ImagePlant::BootRequestRegister,
        ImagePlant::DebugMailbox,
    ];

    pub const fn cfg_value(self) -> &'static str {
        match self {
            ImagePlant::LiteralPoolFlashController => "literal_pool_flash_controller",
            ImagePlant::MovwMovtFlashController => "movw_movt_flash_controller",
            ImagePlant::FlashDriverSymbol => "flash_driver_symbol",
            ImagePlant::FactoryReadOutsideAccessor => "factory_read_outside_accessor",
            ImagePlant::DivisionInTick => "division_in_tick",
            ImagePlant::BootRequestRegister => "boot_request_register",
            ImagePlant::DebugMailbox => "debug_mailbox",
        }
    }

    pub const fn level(self) -> Level {
        match self {
            ImagePlant::LiteralPoolFlashController => Level::BoardForSize,
            ImagePlant::MovwMovtFlashController
            | ImagePlant::FlashDriverSymbol
            | ImagePlant::FactoryReadOutsideAccessor
            | ImagePlant::DivisionInTick
            | ImagePlant::BootRequestRegister
            | ImagePlant::DebugMailbox => Level::Speed,
        }
    }

    const fn rule(self) -> Rule {
        match self {
            ImagePlant::LiteralPoolFlashController | ImagePlant::MovwMovtFlashController => {
                Rule::FlashControllerConstant
            }
            ImagePlant::FlashDriverSymbol => Rule::FlashDriverSymbol,
            ImagePlant::FactoryReadOutsideAccessor => Rule::FactoryAddressOutsideAccessor,
            ImagePlant::DivisionInTick => Rule::InterruptGraph,
            ImagePlant::BootRequestRegister => Rule::BootRequestRegisterConstant,
            ImagePlant::DebugMailbox => Rule::DebugMailboxConstant,
        }
    }

    const fn encoding(self) -> Option<Encoding> {
        match self {
            ImagePlant::LiteralPoolFlashController => Some(Encoding::LiteralPool),
            ImagePlant::MovwMovtFlashController => Some(Encoding::MoveWideAndTop),
            ImagePlant::FlashDriverSymbol
            | ImagePlant::FactoryReadOutsideAccessor
            | ImagePlant::DivisionInTick
            | ImagePlant::BootRequestRegister
            | ImagePlant::DebugMailbox => None,
        }
    }

    fn is_its_own(self, violation: &Violation) -> bool {
        let right_finding = match violation {
            Violation::InInterruptGraph { finding, .. } => {
                matches!(finding, GraphFinding::Division(_))
            }
            _ => true,
        };
        violation.rule() == self.rule() && right_finding && site(violation).contains(PLANTED_CODE)
    }
}

fn site(violation: &Violation) -> &str {
    match violation {
        Violation::FlashDriverSymbol { symbol, .. }
        | Violation::DeadStrippedSymbol { symbol, .. }
        | Violation::DacRegisterZeroWriter { symbol, .. } => symbol,
        Violation::FlashControllerConstant { function, .. }
        | Violation::BootRequestRegisterConstant { function, .. }
        | Violation::DebugMailboxConstant { function, .. }
        | Violation::ResetRequestRegisterConstant { function, .. }
        | Violation::RomFlashApiEntryConstant { function, .. }
        | Violation::FactoryPageConstant { function, .. }
        | Violation::FactoryAddressOutsideAccessor { function, .. }
        | Violation::UnresolvedHighHalf { function, .. }
        | Violation::FactoryWrite { function, .. }
        | Violation::InInterruptGraph { function, .. }
        | Violation::FloatingPointInstruction { function, .. } => function,
    }
}

fn line_of(violation: &Violation) -> usize {
    match violation {
        Violation::FlashDriverSymbol { line, .. }
        | Violation::DeadStrippedSymbol { line, .. }
        | Violation::DacRegisterZeroWriter { line, .. }
        | Violation::FlashControllerConstant { line, .. }
        | Violation::BootRequestRegisterConstant { line, .. }
        | Violation::DebugMailboxConstant { line, .. }
        | Violation::ResetRequestRegisterConstant { line, .. }
        | Violation::RomFlashApiEntryConstant { line, .. }
        | Violation::FactoryPageConstant { line, .. }
        | Violation::FactoryAddressOutsideAccessor { line, .. }
        | Violation::UnresolvedHighHalf { line, .. }
        | Violation::FactoryWrite { line, .. }
        | Violation::InInterruptGraph { line, .. }
        | Violation::FloatingPointInstruction { line, .. } => *line,
    }
}

fn encoding_at(instruction_listing: &str, line: usize) -> Option<Encoding> {
    let text = instruction_listing.lines().nth(line.checked_sub(1)?)?;
    text.split_whitespace().find_map(|token| match token {
        ".word" => Some(Encoding::LiteralPool),
        "movt" => Some(Encoding::MoveWideAndTop),
        _ => None,
    })
}

fn built_as(expected: Encoding, violations: &[Violation], instruction_listing: &str) -> bool {
    let encodings: Vec<Option<Encoding>> = violations
        .iter()
        .map(|violation| encoding_at(instruction_listing, line_of(violation)))
        .collect();
    encodings.contains(&Some(expected))
        && encodings
            .iter()
            .all(|encoding| encoding.is_none_or(|encoding| encoding == expected))
}

pub fn judge(
    build: ImageBuild,
    scanned: &Result<Vec<Violation>, InputError>,
    instruction_listing: &str,
) -> Verdict {
    let violations = match scanned {
        Ok(violations) => violations,
        Err(error) => return Verdict::Refused(error.to_string()),
    };
    match build {
        ImageBuild::Clean(_) if violations.is_empty() => Verdict::Holds,
        ImageBuild::Clean(_) => Verdict::CleanImageFailed {
            violations: violations.len(),
        },
        ImageBuild::Planted(_) if violations.is_empty() => Verdict::PlantPassed,
        ImageBuild::Planted(plant) => {
            if let Some(stray) = violations.iter().find(|v| !plant.is_its_own(v)) {
                return Verdict::FailedForAnotherReason(stray.clone());
            }
            match plant.encoding() {
                Some(expected) if !built_as(expected, violations, instruction_listing) => {
                    Verdict::WrongEncoding(expected)
                }
                Some(_) | None => Verdict::Holds,
            }
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Speed => "release profile, opt-level 3",
            Level::BoardForSize => "release profile, board crate at opt-level z",
        })
    }
}

impl fmt::Display for ImageBuild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageBuild::Clean(level) => write!(f, "clean image ({level})"),
            ImageBuild::Planted(plant) => {
                write!(f, "planted {} ({})", plant.cfg_value(), plant.level())
            }
        }
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Encoding::LiteralPool => "a literal-pool `.word`",
            Encoding::MoveWideAndTop => "a `movw`/`movt` pair",
        })
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Holds => f.write_str("holds"),
            Verdict::CleanImageFailed { violations } => {
                write!(f, "the clean image has {violations} violations")
            }
            Verdict::Refused(reason) => write!(f, "the scan refused its input: {reason}"),
            Verdict::PlantPassed => f.write_str("passed the scan, so the plant was not caught"),
            Verdict::FailedForAnotherReason(violation) => {
                write!(f, "failed, but not on the planted rule alone: {violation}")
            }
            Verdict::WrongEncoding(expected) => {
                write!(
                    f,
                    "failed on its rule, but the address was not built as {expected}"
                )
            }
        }
    }
}

fn scan_build(rules: &str, listings: &Listings) -> Result<Vec<Violation>, InputError> {
    let mut violations = scan::scan(rules, &listings.symbols, &listings.instruction_listing)?;
    violations.extend(scan::scan_data_sections(rules, &listings.sections)?);
    Ok(violations)
}

pub fn command() -> Outcome {
    let workspace = Workspace::locate();
    let rules = match workspace.rules(RULES_FILE) {
        Ok(rules) => rules,
        Err(error) => {
            eprintln!("{RULES_FILE}: {error}");
            return Outcome::UsageOrInputError;
        }
    };
    let builds = [
        ImageBuild::Clean(Level::Speed),
        ImageBuild::Clean(Level::BoardForSize),
    ]
    .into_iter()
    .chain(ImagePlant::ALL.map(ImageBuild::Planted));
    let mut every_build_holds = true;
    for build in builds {
        let listings = match workspace.build_and_list(build) {
            Ok(listings) => listings,
            Err(error) => {
                eprintln!("{build}: {error}");
                return Outcome::UsageOrInputError;
            }
        };
        let scanned = scan_build(&rules, &listings);
        let verdict = judge(build, &scanned, &listings.instruction_listing);
        println!("{build}: {verdict}");
        println!("  listings in {}", listings.folder.display());
        for violation in scanned.iter().flatten() {
            println!("  {violation}");
        }
        every_build_holds &= verdict == Verdict::Holds;
    }
    if every_build_holds {
        println!(
            "every build holds, two clean and {} planted. {WHAT_THE_PASS_PROVES}",
            ImagePlant::ALL.len()
        );
        println!("{WHAT_IT_CANNOT_PROVE}");
        Outcome::GatePassed
    } else {
        println!("the image scan check fails. {WHAT_IT_CANNOT_PROVE}");
        Outcome::GateFailed
    }
}
