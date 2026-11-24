use super::scan;
use crate::exit::Outcome;
use std::fs;

const SCAN_LIMIT: &str = "a constant scan cannot prove that no computed address reaches a guarded range; copying the factory page to RAM once at boot and routing every register write through the single board executor narrow that gap.";

pub fn command(rules: &str, symbols: &str, instruction_listing: &str) -> Outcome {
    let mut texts = Vec::new();
    for path in [rules, symbols, instruction_listing] {
        match fs::read_to_string(path) {
            Ok(text) => texts.push(text),
            Err(error) => {
                eprintln!("{path}: {error}");
                return Outcome::UsageOrInputError;
            }
        }
    }
    let violations = match scan(&texts[0], &texts[1], &texts[2]) {
        Ok(violations) => violations,
        Err(error) => {
            eprintln!("{error}");
            return Outcome::UsageOrInputError;
        }
    };
    for violation in &violations {
        println!("{violation}");
    }
    println!("{} violations. {SCAN_LIMIT}", violations.len());
    if violations.is_empty() {
        Outcome::GatePassed
    } else {
        Outcome::GateFailed
    }
}
