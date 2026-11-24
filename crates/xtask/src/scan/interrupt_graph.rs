use super::input::{Input, InputError, InputErrorKind};
use super::instruction::{
    branch_target, general_register, is_floating_point, is_op, operand_list, without_qualifier,
};
use super::instruction_listing::{Function, Instruction, Line};
use super::rules::Rules;
use super::violation::{GraphFinding, SymbolFamily, Violation};
use std::collections::{HashMap, HashSet, VecDeque};

const MASK_REGISTERS: [&str; 4] = ["primask", "basepri", "basepri_max", "faultmask"];

pub fn check(rules: &Rules, functions: &[Function]) -> Result<Vec<Violation>, InputError> {
    let by_name: HashMap<&str, &Function> = functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut violations = Vec::new();
    for root in &rules.interrupt_roots {
        let mut queue = VecDeque::from([root.as_str()]);
        while let Some(name) = queue.pop_front() {
            if !visited.insert(name) {
                continue;
            }
            let function = by_name.get(name).ok_or_else(|| {
                InputError::whole(
                    Input::InstructionListing,
                    InputErrorKind::NotInListing(name.to_string()),
                )
            })?;
            let mut report = |line, finding| {
                violations.push(Violation::InInterruptGraph {
                    root: root.clone(),
                    function: function.name.clone(),
                    line,
                    finding,
                })
            };
            if let Some(family) = SymbolFamily::of(&function.name) {
                report(function.line, GraphFinding::Symbol(family));
            }
            for instruction in function.lines.iter().filter_map(|line| match line {
                Line::Instruction(instruction) => Some(instruction),
                Line::Data(_) => None,
            }) {
                if let Some(finding) = forbidden_instruction(instruction) {
                    report(instruction.line, finding);
                }
                if let Some(callee) = callee(instruction)?
                    && callee != function.name
                {
                    queue.push_back(callee);
                }
            }
        }
    }
    Ok(violations)
}

fn forbidden_instruction(instruction: &Instruction) -> Option<GraphFinding> {
    let mnemonic = instruction.mnemonic.as_str();
    let plain = without_qualifier(mnemonic).to_string();
    let operands = operand_list(&instruction.operands);
    let first = operands.first().copied().unwrap_or("");
    if is_op(mnemonic, "sdiv") || is_op(mnemonic, "udiv") {
        Some(GraphFinding::Division(base_of(&plain, ["sdiv", "udiv"])))
    } else if is_floating_point(mnemonic) {
        Some(GraphFinding::FloatingPoint(mnemonic.to_string()))
    } else if mnemonic.starts_with("cpsid") || mnemonic.starts_with("cpsie") {
        Some(GraphFinding::InterruptMasking(base_of(
            &plain,
            ["cpsid", "cpsie"],
        )))
    } else if is_op(mnemonic, "msr")
        && MASK_REGISTERS.contains(&first.to_ascii_lowercase().as_str())
    {
        Some(GraphFinding::InterruptMasking("msr".to_string()))
    } else if is_indirect_transfer(mnemonic, &operands) {
        Some(GraphFinding::IndirectCall)
    } else {
        None
    }
}

fn base_of<const N: usize>(plain: &str, bases: [&str; N]) -> String {
    bases
        .into_iter()
        .find(|base| plain.starts_with(base))
        .unwrap_or(plain)
        .to_string()
}

fn is_indirect_transfer(mnemonic: &str, operands: &[&str]) -> bool {
    let first = operands.first().copied().unwrap_or("");
    let through_register = general_register(first).is_some();
    ((is_op(mnemonic, "blx") || is_op(mnemonic, "bl")) && through_register)
        || (is_op(mnemonic, "bx") && first != "lr")
        || (is_op(mnemonic, "mov") && first == "pc")
        || (mnemonic.starts_with("ldr")
            && first == "pc"
            && !operands
                .get(1)
                .is_some_and(|address| address.starts_with("[sp")))
}

fn callee(instruction: &Instruction) -> Result<Option<&str>, InputError> {
    let mnemonic = instruction.mnemonic.as_str();
    let operands = operand_list(&instruction.operands);
    let is_transfer = is_op(mnemonic, "b") || is_op(mnemonic, "bl") || is_op(mnemonic, "blx");
    if !is_transfer
        || operands
            .first()
            .is_some_and(|first| general_register(first).is_some())
    {
        return Ok(None);
    }
    branch_target(&instruction.operands)
        .map(Some)
        .ok_or_else(|| {
            InputError::at(
                Input::InstructionListing,
                instruction.line,
                InputErrorKind::UnresolvedCallTarget,
            )
        })
}
