use crate::price::{
    CONDITION_FAILED_INSTRUCTION, DecodedInstruction, decode_instruction_listing_line,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

pub struct Program {
    instructions: HashMap<u32, DecodedInstruction>,
    functions: Vec<Function>,
}

struct Function {
    start: u32,
    end: u32,
    name: String,
}

pub struct Trace {
    pub label: String,
    pub executed: Vec<Executed>,
}

#[derive(Clone, Copy)]
pub struct Executed {
    pub address: u32,
    pub size: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FunctionCost {
    pub instructions: u32,
    pub cycles: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PricedTrace {
    pub per_function: BTreeMap<String, FunctionCost>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProgramError {
    UnreadableSymbolLine(String),
    UnreadableTraceLine(String),
    TraceNotClosed(String),
    NotDecoded(u32),
    WidthDiffers {
        address: u32,
        executed: u32,
        decoded: u32,
    },
    NoProxyPrice {
        address: u32,
        mnemonic: String,
    },
}

impl Program {
    pub fn read(instruction_listing: &str, symbols: &str) -> Result<Self, ProgramError> {
        let instructions = instruction_listing
            .lines()
            .filter_map(decode_instruction_listing_line)
            .map(|instruction| (instruction.address, instruction))
            .collect();
        let mut functions = symbols
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(read_symbol_line)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        functions.sort_by_key(|function| function.start);
        Ok(Self {
            instructions,
            functions,
        })
    }

    pub fn price(&self, trace: &Trace) -> Result<PricedTrace, ProgramError> {
        let mut priced = PricedTrace::default();
        let mut caller = "?";
        let successors = trace
            .executed
            .iter()
            .skip(1)
            .map(|next| Some(next.address))
            .chain([None]);
        for (index, (executed, next)) in trace.executed.iter().zip(successors).enumerate() {
            let decoded = self
                .instructions
                .get(&executed.address)
                .ok_or(ProgramError::NotDecoded(executed.address))?;
            if decoded.size != executed.size {
                return Err(ProgramError::WidthDiffers {
                    address: executed.address,
                    executed: executed.size,
                    decoded: decoded.size,
                });
            }
            let price = if next == Some(decoded.address + decoded.size) {
                decoded.falls_through
            } else {
                decoded.jumps
            };
            let cycles = price.ok_or_else(|| ProgramError::NoProxyPrice {
                address: decoded.address,
                mnemonic: decoded.mnemonic.clone(),
            })?;
            let holder = self.function_holding(executed.address);
            if !is_runtime_helper(holder) {
                caller = holder;
            }
            let condition_failed =
                self.condition_failed_after(decoded, &trace.executed[index + 1..]);
            let cost = priced.per_function.entry(caller.to_string()).or_default();
            cost.instructions += 1 + condition_failed;
            cost.cycles += cycles.get() + condition_failed * CONDITION_FAILED_INSTRUCTION.get();
        }
        Ok(priced)
    }

    fn condition_failed_after(&self, decoded: &DecodedInstruction, following: &[Executed]) -> u32 {
        let Some(block_length) = it_block_length(&decoded.mnemonic) else {
            return 0;
        };
        let mut address = decoded.address + decoded.size;
        let mut following = following.iter().peekable();
        let mut failed = 0;
        for _ in 0..block_length {
            let Some(covered) = self.instructions.get(&address) else {
                break;
            };
            if following
                .next_if(|executed| executed.address == address)
                .is_none()
            {
                failed += 1;
            }
            address += covered.size;
        }
        failed
    }

    pub fn function_names(&self) -> impl Iterator<Item = &str> {
        self.functions.iter().map(|function| function.name.as_str())
    }

    pub fn range_of(&self, name: &str) -> Option<(u32, u32)> {
        self.functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| (function.start, function.end))
    }

    pub fn address_of(&self, name: &str) -> Option<u32> {
        self.functions
            .iter()
            .find(|function| function.name == name)
            .map(|function| function.start)
    }

    pub fn executed_shape_of(&self, name: &str) -> Option<Vec<String>> {
        let function = self
            .functions
            .iter()
            .find(|function| function.name == name)?;
        let mut addresses: Vec<&u32> = self
            .instructions
            .keys()
            .filter(|address| (function.start..function.end).contains(*address))
            .collect();
        addresses.sort();
        Some(
            addresses
                .into_iter()
                .filter_map(|address| self.instructions.get(address))
                .map(shape_of)
                .collect(),
        )
    }

    pub fn function_holding(&self, address: u32) -> &str {
        let index = self
            .functions
            .partition_point(|function| function.start <= address);
        index
            .checked_sub(1)
            .and_then(|index| self.functions.get(index))
            .filter(|function| address < function.end)
            .map_or("?", |function| function.name.as_str())
    }
}

fn it_block_length(mnemonic: &str) -> Option<u32> {
    let then_else = mnemonic.strip_prefix("it")?;
    (then_else.len() <= 3 && then_else.chars().all(|letter| matches!(letter, 't' | 'e')))
        .then_some(1 + then_else.len() as u32)
}

const RUNTIME_HELPERS: [&str; 2] = ["__aeabi_", "compiler_builtins::"];

fn is_runtime_helper(function: &str) -> bool {
    RUNTIME_HELPERS
        .iter()
        .any(|prefix| function.starts_with(prefix))
}

const ADDRESS_HALF_LOADS: [&str; 2] = ["movw", "movt"];

fn shape_of(instruction: &DecodedInstruction) -> String {
    let loads_an_address_half = ADDRESS_HALF_LOADS
        .iter()
        .any(|root| instruction.mnemonic.starts_with(root));
    let operands = match instruction.operands.split_once('<') {
        Some((_, target)) => target.trim_end_matches('>').to_string(),
        None if loads_an_address_half => instruction
            .operands
            .split(',')
            .next()
            .unwrap_or_default()
            .to_string(),
        None => instruction.operands.clone(),
    };
    format!("{} {}", instruction.mnemonic, operands)
}

fn read_symbol_line(line: &str) -> Result<Option<Function>, ProgramError> {
    let unreadable = || ProgramError::UnreadableSymbolLine(line.to_string());
    let mut words = line.trim().splitn(4, ' ');
    let (Some(start), Some(size), Some(kind), Some(name)) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return Ok(None);
    };
    if !matches!(kind, "T" | "t" | "W" | "w") {
        return Ok(None);
    }
    let start = u32::from_str_radix(start, 16).map_err(|_| unreadable())? & !1;
    let size = u32::from_str_radix(size, 16).map_err(|_| unreadable())?;
    Ok(Some(Function {
        start,
        end: start + size,
        name: name.to_string(),
    }))
}

pub fn read_traces(text: &str) -> Result<Vec<Trace>, ProgramError> {
    let mut traces = Vec::new();
    let mut open: Option<Trace> = None;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with(BOOTED) && open.is_none() {
            continue;
        }
        if let Some(label) = line.strip_prefix("trace ") {
            if let Some(unclosed) = open.take() {
                return Err(ProgramError::TraceNotClosed(unclosed.label));
            }
            open = Some(Trace {
                label: label.to_string(),
                executed: Vec::new(),
            });
        } else if line == "end" {
            let closed = open
                .take()
                .ok_or_else(|| ProgramError::UnreadableTraceLine(line.to_string()))?;
            traces.push(closed);
        } else {
            let trace = open
                .as_mut()
                .ok_or_else(|| ProgramError::UnreadableTraceLine(line.to_string()))?;
            trace.executed.push(read_executed(line)?);
        }
    }
    match open {
        Some(unclosed) => Err(ProgramError::TraceNotClosed(unclosed.label)),
        None => Ok(traces),
    }
}

const BOOTED: &str = "booted ";

pub fn read_boot_end(text: &str) -> Result<Option<u32>, ProgramError> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(BOOTED))
        .map(|address| {
            u32::from_str_radix(address, 16)
                .map_err(|_| ProgramError::UnreadableTraceLine(address.to_string()))
        })
        .transpose()
}

fn read_executed(line: &str) -> Result<Executed, ProgramError> {
    let unreadable = || ProgramError::UnreadableTraceLine(line.to_string());
    let (address, size) = line.split_once(' ').ok_or_else(unreadable)?;
    Ok(Executed {
        address: u32::from_str_radix(address, 16).map_err(|_| unreadable())?,
        size: size.parse().map_err(|_| unreadable())?,
    })
}

impl PricedTrace {
    pub fn function(&self, name: &str) -> Option<&FunctionCost> {
        self.per_function.get(name)
    }

    pub fn total_cycles(&self) -> u32 {
        self.per_function.values().map(|cost| cost.cycles).sum()
    }

    pub fn total_instructions(&self) -> u32 {
        self.per_function
            .values()
            .map(|cost| cost.instructions)
            .sum()
    }
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::UnreadableSymbolLine(line) => {
                write!(f, "llvm-nm line not understood: `{line}`")
            }
            ProgramError::UnreadableTraceLine(line) => {
                write!(f, "emulator line not understood: `{line}`")
            }
            ProgramError::TraceNotClosed(label) => {
                write!(f, "the emulator output ends inside trace `{label}`")
            }
            ProgramError::NotDecoded(address) => write!(
                f,
                "the emulator executed 0x{address:x}, which the instruction_listing holds no instruction at"
            ),
            ProgramError::WidthDiffers {
                address,
                executed,
                decoded,
            } => write!(
                f,
                "the emulator executed {executed} bytes at 0x{address:x}, where the instruction_listing decodes {decoded}"
            ),
            ProgramError::NoProxyPrice { address, mnemonic } => write!(
                f,
                "`{mnemonic}` at 0x{address:x} has no price in the proxy table"
            ),
        }
    }
}

impl std::error::Error for ProgramError {}
