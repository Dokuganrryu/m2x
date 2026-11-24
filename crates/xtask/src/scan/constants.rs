use super::address::{
    ForbiddenBlock, forbidden_block, high_half_may_reach_a_guarded_range, is_factory_page,
    is_factory_read,
};
use super::input::{Input, InputError, InputErrorKind};
use super::instruction::{
    general_register, is_op, is_op_setting_flags_or_not, operand_list, register_list,
};
use super::instruction_listing::{Function, Instruction, Line};
use super::register::{Formed, RegisterEvent, RegisterValue};
use super::rules::{FactoryAccessor, Rules};
use super::violation::Violation;
use std::collections::HashMap;

const TRACKED_REGISTERS: usize = 13;
const CALLER_SAVED: [usize; 5] = [0, 1, 2, 3, 12];

type RegisterFile = [RegisterValue; TRACKED_REGISTERS];

struct Site<'a> {
    rules: &'a Rules,
    function: &'a str,
    line: usize,
}

pub fn check(rules: &Rules, functions: &[Function]) -> Result<Vec<Violation>, InputError> {
    let literal_pool: HashMap<u32, u32> = functions
        .iter()
        .flat_map(|function| &function.lines)
        .filter_map(|line| match line {
            Line::Data(data) => Some((data.address, data.value)),
            Line::Instruction(_) => None,
        })
        .collect();
    let mut violations = Vec::new();
    for function in functions {
        let mut registers: RegisterFile = [RegisterValue::Unknown; TRACKED_REGISTERS];
        for line in &function.lines {
            match line {
                Line::Data(data) => {
                    let site = Site {
                        rules,
                        function: &function.name,
                        line: data.line,
                    };
                    site.judge_value(data.value, &mut violations);
                }
                Line::Instruction(instruction) => {
                    let site = Site {
                        rules,
                        function: &function.name,
                        line: instruction.line,
                    };
                    for value in immediates(instruction)? {
                        site.judge_value(value, &mut violations);
                    }
                    site.judge_store(instruction, &registers, &mut violations);
                    for formed in track(instruction, &mut registers, &literal_pool) {
                        site.judge_formed(formed, &mut violations);
                    }
                }
            }
        }
    }
    violations.dedup();
    Ok(violations)
}

impl Site<'_> {
    fn judge_formed(&self, formed: Formed, violations: &mut Vec<Violation>) {
        match formed {
            Formed::Value(value) => self.judge_value(value, violations),
            Formed::HighHalfOnly(high) if high_half_may_reach_a_guarded_range(high) => violations
                .push(Violation::UnresolvedHighHalf {
                    line: self.line,
                    function: self.function.to_string(),
                    high,
                }),
            Formed::HighHalfOnly(_) => {}
        }
    }

    fn judge_value(&self, value: u32, violations: &mut Vec<Violation>) {
        let function = self.function.to_string();
        let line = self.line;
        if let Some(block) = forbidden_block(value) {
            violations.push(match block {
                ForbiddenBlock::FlashController => Violation::FlashControllerConstant {
                    line,
                    function,
                    value,
                },
                ForbiddenBlock::BootRequestRegister => Violation::BootRequestRegisterConstant {
                    line,
                    function,
                    value,
                },
                ForbiddenBlock::DebugMailbox => Violation::DebugMailboxConstant {
                    line,
                    function,
                    value,
                },
                ForbiddenBlock::ResetRequestRegister => Violation::ResetRequestRegisterConstant {
                    line,
                    function,
                    value,
                },
            });
        } else if self.rules.rom_flash_api_entries.contains(&value) {
            violations.push(Violation::RomFlashApiEntryConstant {
                line,
                function,
                value,
            });
        } else if is_factory_page(value) && !is_factory_read(value) {
            violations.push(Violation::FactoryPageConstant {
                line,
                function,
                value,
            });
        } else if is_factory_read(value) && !self.is_inside_accessor() {
            violations.push(Violation::FactoryAddressOutsideAccessor {
                line,
                function,
                value,
            });
        }
    }

    fn is_inside_accessor(&self) -> bool {
        match &self.rules.factory_accessor {
            FactoryAccessor::Named(name) => name == self.function,
            FactoryAccessor::NoneInThisImage => false,
        }
    }

    fn judge_store(
        &self,
        instruction: &Instruction,
        registers: &RegisterFile,
        violations: &mut Vec<Violation>,
    ) {
        let Some(base) = store_base(instruction) else {
            return;
        };
        if let RegisterValue::Known(address) = registers[base]
            && is_factory_page(address)
        {
            violations.push(Violation::FactoryWrite {
                line: self.line,
                function: self.function.to_string(),
                address,
            });
        }
    }
}

fn store_base(instruction: &Instruction) -> Option<usize> {
    let mnemonic = instruction.mnemonic.as_str();
    let operands = operand_list(&instruction.operands);
    if mnemonic.starts_with("stm") {
        return operands
            .first()
            .and_then(|operand| general_register(operand));
    }
    if !mnemonic.starts_with("str") {
        return None;
    }
    operands.iter().find_map(|operand| {
        let inner = operand.strip_prefix('[')?;
        general_register(inner.split([',', ']']).next()?)
    })
}

fn immediates(instruction: &Instruction) -> Result<Vec<u32>, InputError> {
    instruction
        .operands
        .split('#')
        .skip(1)
        .filter_map(|tail| {
            let text = tail
                .split([',', ']', '}', '!', ' '])
                .next()
                .unwrap_or("")
                .trim();
            read_integer(text).transpose()
        })
        .map(|read| {
            read.map_err(|text| {
                InputError::at(
                    Input::InstructionListing,
                    instruction.line,
                    InputErrorKind::UnreadableImmediate(text),
                )
            })
        })
        .collect()
}

fn read_integer(text: &str) -> Result<Option<u32>, String> {
    let (negative, magnitude) = match text.strip_prefix('-') {
        Some(magnitude) => (true, magnitude),
        None => (false, text),
    };
    let value = match magnitude.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None if magnitude.contains('.') => return Ok(None),
        None => magnitude.parse::<u32>().ok(),
    }
    .ok_or_else(|| text.to_string())?;
    Ok(Some(if negative {
        value.wrapping_neg()
    } else {
        value
    }))
}

fn signed_immediate(operand: &str) -> Option<i64> {
    let value = read_integer(operand.strip_prefix('#')?).ok()??;
    Some(i64::from(value as i32))
}

fn track(
    instruction: &Instruction,
    registers: &mut RegisterFile,
    literal_pool: &HashMap<u32, u32>,
) -> Vec<Formed> {
    let mnemonic = instruction.mnemonic.as_str();
    let operands = operand_list(&instruction.operands);
    let destination = operands
        .first()
        .and_then(|operand| general_register(operand));
    let mut formed = Vec::new();
    let mut apply = |register: usize, source: usize, event: RegisterEvent| {
        let (next, output) = registers[source].next(event);
        registers[register] = next;
        formed.extend(output);
    };
    let immediate = operands
        .get(1)
        .and_then(|operand| signed_immediate(operand));
    match (destination, immediate) {
        (Some(rd), Some(value)) if is_op(mnemonic, "movw") => {
            apply(rd, rd, RegisterEvent::MoveWide(value as u16))
        }
        (Some(rd), Some(value)) if is_op(mnemonic, "movt") => {
            apply(rd, rd, RegisterEvent::MoveTop(value as u16))
        }
        (Some(rd), Some(value)) if is_op_setting_flags_or_not(mnemonic, "mov") => {
            apply(rd, rd, RegisterEvent::Load(value as u32))
        }
        (Some(rd), Some(value)) if is_op_setting_flags_or_not(mnemonic, "mvn") => {
            apply(rd, rd, RegisterEvent::Load(!(value as u32)))
        }
        (Some(rd), _) if is_op(mnemonic, "ldr") && is_pc_relative(&operands) => {
            let event = instruction
                .literal_address
                .and_then(|address| literal_pool.get(&address))
                .map_or(RegisterEvent::Clobber, |value| RegisterEvent::Load(*value));
            apply(rd, rd, event)
        }
        (Some(rd), _) if is_add_or_subtract(mnemonic) => {
            let (source, offset) = match operands.as_slice() {
                [_, offset] => (Some(rd), signed_immediate(offset)),
                [_, source, offset] => (general_register(source), signed_immediate(offset)),
                _ => (None, None),
            };
            match (source, offset) {
                (Some(source), Some(offset)) => {
                    let offset = if is_subtract(mnemonic) {
                        -offset
                    } else {
                        offset
                    };
                    apply(rd, source, RegisterEvent::AddImmediate(offset))
                }
                _ => apply(rd, rd, RegisterEvent::Clobber),
            }
        }
        _ if is_op(mnemonic, "bl") || is_op(mnemonic, "blx") => {
            for register in CALLER_SAVED {
                apply(register, register, RegisterEvent::Clobber);
            }
        }
        _ if mnemonic.starts_with("ldm") || mnemonic.starts_with("pop") => {
            let written = operands
                .iter()
                .flat_map(|operand| register_list(operand))
                .chain(destination.filter(|_| mnemonic.starts_with("ldm")));
            for register in written.collect::<Vec<_>>() {
                apply(register, register, RegisterEvent::Clobber);
            }
        }
        (Some(rd), _) if writes_first_operand(mnemonic) => apply(rd, rd, RegisterEvent::Clobber),
        _ => {}
    }
    formed
}

fn is_pc_relative(operands: &[&str]) -> bool {
    operands
        .get(1)
        .is_some_and(|operand| operand.starts_with("[pc"))
}

fn is_add_or_subtract(mnemonic: &str) -> bool {
    ["add", "addw", "sub", "subw"]
        .into_iter()
        .any(|base| is_op_setting_flags_or_not(mnemonic, base))
}

fn is_subtract(mnemonic: &str) -> bool {
    mnemonic.starts_with("sub")
}

fn writes_first_operand(mnemonic: &str) -> bool {
    const LEAVES_FIRST_OPERAND: [&str; 12] = [
        "str", "stm", "push", "cmp", "cmn", "tst", "teq", "cb", "tb", "it", "msr", "pld",
    ];
    !LEAVES_FIRST_OPERAND
        .into_iter()
        .any(|prefix| mnemonic.starts_with(prefix))
        && !is_op(mnemonic, "b")
        && !is_op(mnemonic, "bx")
}
