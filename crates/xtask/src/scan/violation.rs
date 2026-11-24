use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    FlashDriverSymbol,
    FlashControllerConstant,
    BootRequestRegisterConstant,
    DebugMailboxConstant,
    ResetRequestRegisterConstant,
    RomFlashApiEntryConstant,
    FactoryPageConstant,
    FactoryAddressOutsideAccessor,
    UnresolvedHighHalf,
    FactoryWrite,
    InterruptGraph,
    FloatingPointInstruction,
    DeadStrippedSymbol,
    DacRegisterZeroWriter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolFamily {
    DivisionHelper,
    Formatting,
    Panic,
    CriticalSection,
    Libm,
    Allocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphFinding {
    Division(String),
    FloatingPoint(String),
    InterruptMasking(String),
    IndirectCall,
    Symbol(SymbolFamily),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Violation {
    FlashDriverSymbol {
        line: usize,
        symbol: String,
    },
    FlashControllerConstant {
        line: usize,
        function: String,
        value: u32,
    },
    BootRequestRegisterConstant {
        line: usize,
        function: String,
        value: u32,
    },
    DebugMailboxConstant {
        line: usize,
        function: String,
        value: u32,
    },
    ResetRequestRegisterConstant {
        line: usize,
        function: String,
        value: u32,
    },
    RomFlashApiEntryConstant {
        line: usize,
        function: String,
        value: u32,
    },
    FactoryPageConstant {
        line: usize,
        function: String,
        value: u32,
    },
    FactoryAddressOutsideAccessor {
        line: usize,
        function: String,
        value: u32,
    },
    UnresolvedHighHalf {
        line: usize,
        function: String,
        high: u16,
    },
    FactoryWrite {
        line: usize,
        function: String,
        address: u32,
    },
    InInterruptGraph {
        root: String,
        function: String,
        line: usize,
        finding: GraphFinding,
    },
    FloatingPointInstruction {
        line: usize,
        function: String,
        mnemonic: String,
    },
    DeadStrippedSymbol {
        line: usize,
        symbol: String,
    },
    DacRegisterZeroWriter {
        line: usize,
        symbol: String,
    },
}

impl SymbolFamily {
    pub fn of(name: &str) -> Option<SymbolFamily> {
        let has_path = |path: &str| {
            name.starts_with(path)
                || name.contains(&format!("<{path}"))
                || name.contains(&format!(" {path}"))
                || name.contains(&format!("::{path}"))
        };
        if (name.starts_with("__") && name.contains("div")) || name.contains("specialized_div_rem")
        {
            Some(SymbolFamily::DivisionHelper)
        } else if has_path("core::fmt::") {
            Some(SymbolFamily::Formatting)
        } else if has_path("core::panicking::") || name.ends_with("rust_begin_unwind") {
            Some(SymbolFamily::Panic)
        } else if name.contains("critical_section") || has_path("cortex_m::interrupt::") {
            Some(SymbolFamily::CriticalSection)
        } else if has_path("libm::") {
            Some(SymbolFamily::Libm)
        } else if [
            "__rust_alloc",
            "__rust_dealloc",
            "__rust_realloc",
            "__rdl_",
            "__rg_",
        ]
        .into_iter()
        .any(|prefix| name.starts_with(prefix))
            || has_path("alloc::")
        {
            Some(SymbolFamily::Allocation)
        } else {
            None
        }
    }

    fn description(self) -> &'static str {
        match self {
            SymbolFamily::DivisionHelper => "a division helper",
            SymbolFamily::Formatting => "core::fmt",
            SymbolFamily::Panic => "a panic path",
            SymbolFamily::CriticalSection => "a critical section",
            SymbolFamily::Libm => "libm",
            SymbolFamily::Allocation => "an allocator",
        }
    }
}

impl Rule {
    pub fn id(self) -> &'static str {
        match self {
            Rule::FlashDriverSymbol => "flash-driver-symbol",
            Rule::FlashControllerConstant => "flash-controller-constant",
            Rule::BootRequestRegisterConstant => "boot-request-register-constant",
            Rule::DebugMailboxConstant => "debug-mailbox-constant",
            Rule::ResetRequestRegisterConstant => "reset-request-register-constant",
            Rule::RomFlashApiEntryConstant => "rom-flash-api-entry-constant",
            Rule::FactoryPageConstant => "factory-page-constant",
            Rule::FactoryAddressOutsideAccessor => "factory-address-outside-accessor",
            Rule::UnresolvedHighHalf => "unresolved-high-half",
            Rule::FactoryWrite => "factory-write",
            Rule::InterruptGraph => "interrupt-graph",
            Rule::FloatingPointInstruction => "floating-point-instruction",
            Rule::DeadStrippedSymbol => "dead-stripped-symbol",
            Rule::DacRegisterZeroWriter => "dac-register-zero-writer",
        }
    }
}

impl Violation {
    pub fn rule(&self) -> Rule {
        match self {
            Violation::FlashDriverSymbol { .. } => Rule::FlashDriverSymbol,
            Violation::FlashControllerConstant { .. } => Rule::FlashControllerConstant,
            Violation::BootRequestRegisterConstant { .. } => Rule::BootRequestRegisterConstant,
            Violation::DebugMailboxConstant { .. } => Rule::DebugMailboxConstant,
            Violation::ResetRequestRegisterConstant { .. } => Rule::ResetRequestRegisterConstant,
            Violation::RomFlashApiEntryConstant { .. } => Rule::RomFlashApiEntryConstant,
            Violation::FactoryPageConstant { .. } => Rule::FactoryPageConstant,
            Violation::FactoryAddressOutsideAccessor { .. } => Rule::FactoryAddressOutsideAccessor,
            Violation::UnresolvedHighHalf { .. } => Rule::UnresolvedHighHalf,
            Violation::FactoryWrite { .. } => Rule::FactoryWrite,
            Violation::InInterruptGraph { .. } => Rule::InterruptGraph,
            Violation::FloatingPointInstruction { .. } => Rule::FloatingPointInstruction,
            Violation::DeadStrippedSymbol { .. } => Rule::DeadStrippedSymbol,
            Violation::DacRegisterZeroWriter { .. } => Rule::DacRegisterZeroWriter,
        }
    }
}

impl fmt::Display for GraphFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphFinding::Division(mnemonic) => write!(f, "`{mnemonic}` divides"),
            GraphFinding::FloatingPoint(mnemonic) => {
                write!(f, "`{mnemonic}` uses the FPU")
            }
            GraphFinding::InterruptMasking(mnemonic) => {
                write!(f, "`{mnemonic}` masks interrupts")
            }
            GraphFinding::IndirectCall => f.write_str(
                "an indirect call or jump, whose callee the scan cannot check (fail closed)",
            ),
            GraphFinding::Symbol(family) => write!(f, "the function is {}", family.description()),
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FAIL {}: ", self.rule().id())?;
        match self {
            Violation::FlashDriverSymbol { line, symbol } => write!(
                f,
                "llvm-nm line {line}: `{symbol}` names a flash driver, the FLASH accessor or a ROM API wrapper"
            ),
            Violation::FlashControllerConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} lies in the flash controller block or its secure alias"
            ),
            Violation::BootRequestRegisterConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} is AOREG1, the boot-request register, or its secure alias; a store there and a reset park the chip in ROM ISP"
            ),
            Violation::DebugMailboxConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} lies in the debug mailbox block or its secure alias"
            ),
            Violation::ResetRequestRegisterConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} is SCB AIRCR, whose SYSRESETREQ bit resets the chip, and the bring-up image never resets itself"
            ),
            Violation::RomFlashApiEntryConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} is the ROM flash API entry named by the rules"
            ),
            Violation::FactoryPageConstant {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} lies in the factory page and is neither the magic nor the serial"
            ),
            Violation::FactoryAddressOutsideAccessor {
                line,
                function,
                value,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: {value:#010x} is a factory read address outside the accessor"
            ),
            Violation::UnresolvedHighHalf {
                line,
                function,
                high,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: `movt` of {high:#06x} with an unknown low half may form a guarded address (fail closed)"
            ),
            Violation::FactoryWrite {
                line,
                function,
                address,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: a store goes through a register holding the factory address {address:#010x}"
            ),
            Violation::InInterruptGraph {
                root,
                function,
                line,
                finding,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`, reached from `{root}`: {finding}"
            ),
            Violation::FloatingPointInstruction {
                line,
                function,
                mnemonic,
            } => write!(
                f,
                "llvm-objdump line {line} in `{function}`: `{mnemonic}` is a VFP instruction, and the image runs Non-secure with no proof that NSACR grants CP10 and CP11, where any VFP instruction is a NOCP UsageFault"
            ),
            Violation::DeadStrippedSymbol { line, symbol } => write!(
                f,
                "llvm-nm line {line}: `{symbol}` should have been dropped by the link"
            ),
            Violation::DacRegisterZeroWriter { line, symbol } => write!(
                f,
                "llvm-nm line {line}: `{symbol}` writes ES9026 register 0 and belongs only in the protocol build"
            ),
        }
    }
}
