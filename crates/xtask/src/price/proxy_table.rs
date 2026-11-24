use super::cycles::ProxyCycles;

const CONDITION_CODES: [&str; 16] = [
    "eq", "ne", "cs", "hs", "cc", "lo", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le",
];
const ALWAYS_TAKEN_BRANCHES: [&str; 6] = ["b", "bl", "blx", "bx", "tbb", "tbh"];
const CONDITIONAL_BRANCH_ROOTS: [&str; 4] = ["b", "bl", "blx", "bx"];
const COMPARE_AND_BRANCHES: [&str; 2] = ["cbz", "cbnz"];
const REGISTER_LIST_ROOTS: [&str; 8] =
    ["ldm", "stm", "push", "pop", "vldm", "vstm", "vpush", "vpop"];
const DOUBLE_WORD_ROOTS: [&str; 2] = ["ldrd", "strd"];
const SINGLE_TRANSFER_ROOTS: [&str; 6] = ["ldr", "lda", "str", "stl", "vldr", "vstr"];
const FLOAT_CHAINED_MULTIPLY_ACCUMULATES: [&str; 6] =
    ["vfma", "vfms", "vfnma", "vfnms", "vmla", "vmls"];
const ABSENT_FROM_THE_PROXY_TABLE: [&str; 3] = ["sdiv", "udiv", "vsqrt"];

const SINGLE_CYCLE: u32 = 1;
const SINGLE_TRANSFER: u32 = 2;
const DOUBLE_WORD_TRANSFER: u32 = 3;
const FLOAT_CHAINED_MULTIPLY_ACCUMULATE: u32 = 3;
const FLOAT_DIVIDE: u32 = 14;
const BRANCH_TAKEN: u32 = 2;
const BRANCH_NOT_TAKEN: u32 = 1;
const PIPELINE_REFILL_AFTER_LOADING_PC: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriceClass {
    SingleCycle,
    SingleTransfer,
    DoubleWordTransfer,
    RegisterList,
    AlwaysTakenBranch,
    ConditionalBranch,
    FloatChainedMultiplyAccumulate,
    FloatDivide,
    AbsentFromTheProxyTable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    FallsThrough,
    Jumps,
}

pub fn classify(mnemonic: &str) -> PriceClass {
    let root = mnemonic.split('.').next().unwrap_or(mnemonic);
    if starts_with_any(root, &REGISTER_LIST_ROOTS) {
        PriceClass::RegisterList
    } else if starts_with_any(root, &DOUBLE_WORD_ROOTS) {
        PriceClass::DoubleWordTransfer
    } else if starts_with_any(root, &SINGLE_TRANSFER_ROOTS) {
        PriceClass::SingleTransfer
    } else if ALWAYS_TAKEN_BRANCHES.contains(&root) {
        PriceClass::AlwaysTakenBranch
    } else if CONDITIONAL_BRANCH_ROOTS
        .iter()
        .any(|branch| carries_a_condition(root, branch))
        || COMPARE_AND_BRANCHES.contains(&root)
    {
        PriceClass::ConditionalBranch
    } else if any_conditional_form(root, &ABSENT_FROM_THE_PROXY_TABLE) {
        PriceClass::AbsentFromTheProxyTable
    } else if is_conditional_form_of(root, "vdiv") {
        PriceClass::FloatDivide
    } else if any_conditional_form(root, &FLOAT_CHAINED_MULTIPLY_ACCUMULATES) {
        PriceClass::FloatChainedMultiplyAccumulate
    } else {
        PriceClass::SingleCycle
    }
}

pub fn proxy_cycles(class: PriceClass, operands: &str, flow: Flow) -> Option<ProxyCycles> {
    let cycles = match class {
        PriceClass::SingleCycle => SINGLE_CYCLE,
        PriceClass::SingleTransfer => SINGLE_TRANSFER,
        PriceClass::DoubleWordTransfer => DOUBLE_WORD_TRANSFER,
        PriceClass::RegisterList => register_list_cycles(operands),
        PriceClass::AlwaysTakenBranch => BRANCH_TAKEN,
        PriceClass::ConditionalBranch => match flow {
            Flow::Jumps => BRANCH_TAKEN,
            Flow::FallsThrough => BRANCH_NOT_TAKEN,
        },
        PriceClass::FloatChainedMultiplyAccumulate => FLOAT_CHAINED_MULTIPLY_ACCUMULATE,
        PriceClass::FloatDivide => FLOAT_DIVIDE,
        PriceClass::AbsentFromTheProxyTable => return None,
    };
    Some(ProxyCycles::new(cycles))
}

fn register_list_cycles(operands: &str) -> u32 {
    let list = operands
        .split_once('{')
        .and_then(|(_, rest)| rest.split_once('}'))
        .map_or("", |(inside, _)| inside);
    let registers: u32 = list.split(',').map(registers_in_entry).sum();
    let refill = if list.split(',').any(|entry| entry.trim() == "pc") {
        PIPELINE_REFILL_AFTER_LOADING_PC
    } else {
        0
    };
    SINGLE_CYCLE + registers + refill
}

fn registers_in_entry(entry: &str) -> u32 {
    match entry.trim().split_once('-') {
        Some((first, last)) => register_number(last) - register_number(first) + 1,
        None if entry.trim().is_empty() => 0,
        None => 1,
    }
}

fn register_number(register: &str) -> u32 {
    register
        .trim()
        .trim_start_matches(char::is_alphabetic)
        .parse()
        .unwrap_or(0)
}

fn starts_with_any(root: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| root.starts_with(prefix))
}

fn any_conditional_form(root: &str, bases: &[&str]) -> bool {
    bases.iter().any(|base| is_conditional_form_of(root, base))
}

fn carries_a_condition(root: &str, base: &str) -> bool {
    root.strip_prefix(base)
        .is_some_and(|suffix| CONDITION_CODES.contains(&suffix))
}

fn is_conditional_form_of(root: &str, base: &str) -> bool {
    root.strip_prefix(base)
        .is_some_and(|suffix| suffix.is_empty() || CONDITION_CODES.contains(&suffix))
}
