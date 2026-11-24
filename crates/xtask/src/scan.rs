mod address;
mod command;
mod constants;
mod data_sections;
mod floating_point;
mod input;
mod instruction;
mod instruction_listing;
mod interrupt_graph;
mod register;
mod rules;
mod symbol_rules;
mod symbols;
mod violation;

pub use command::command;
pub use data_sections::scan_data_sections;
pub use input::{Input, InputError, InputErrorKind};
pub use register::{Formed, RegisterEvent, RegisterValue};
pub use rules::RuleKey;
pub use violation::{GraphFinding, Rule, SymbolFamily, Violation};

pub fn scan(
    rules_text: &str,
    symbols_text: &str,
    instruction_listing_text: &str,
) -> Result<Vec<Violation>, InputError> {
    let rules = rules::read(rules_text)?;
    let symbols = symbols::read(symbols_text)?;
    let functions = instruction_listing::read(instruction_listing_text)?;
    let mut violations = symbol_rules::check(&rules, &symbols);
    violations.extend(constants::check(&rules, &functions)?);
    violations.extend(interrupt_graph::check(&rules, &functions)?);
    violations.extend(floating_point::check(&functions));
    Ok(violations)
}
