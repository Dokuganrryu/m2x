use super::instruction::is_floating_point;
use super::instruction_listing::{Function, Line};
use super::violation::Violation;

pub fn check(functions: &[Function]) -> Vec<Violation> {
    functions
        .iter()
        .flat_map(|function| {
            function.lines.iter().filter_map(|line| match line {
                Line::Instruction(instruction) if is_floating_point(&instruction.mnemonic) => {
                    Some(Violation::FloatingPointInstruction {
                        line: instruction.line,
                        function: function.name.clone(),
                        mnemonic: instruction.mnemonic.clone(),
                    })
                }
                Line::Instruction(_) | Line::Data(_) => None,
            })
        })
        .collect()
}
