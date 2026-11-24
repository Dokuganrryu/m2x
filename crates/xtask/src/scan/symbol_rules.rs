use super::rules::Rules;
use super::symbols::Symbol;
use super::violation::Violation;

pub fn check(rules: &Rules, symbols: &[Symbol]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for symbol in symbols {
        let line = symbol.line;
        let name = &symbol.name;
        if is_flash_driver(name) {
            violations.push(Violation::FlashDriverSymbol {
                line,
                symbol: name.clone(),
            });
        }
        if rules
            .dead_stripped
            .iter()
            .any(|path| names_path(name, path))
        {
            violations.push(Violation::DeadStrippedSymbol {
                line,
                symbol: name.clone(),
            });
        }
        if rules
            .dac_register_zero_writers
            .iter()
            .any(|path| names_path(name, path))
        {
            violations.push(Violation::DacRegisterZeroWriter {
                line,
                symbol: name.clone(),
            });
        }
    }
    violations
}

fn is_flash_driver(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("flash")
        || lower.replace('_', "").contains("romapi")
        || lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|token| token == "iap")
}

fn names_path(symbol: &str, path: &str) -> bool {
    let plain: String = symbol.chars().filter(|c| !matches!(c, '<' | '>')).collect();
    let plain = without_legacy_hash(&plain);
    plain == path || plain.ends_with(&format!("::{path}"))
}

fn without_legacy_hash(name: &str) -> &str {
    match name.rsplit_once("::h") {
        Some((path, hash)) if hash.len() == 16 && hash.chars().all(|c| c.is_ascii_hexdigit()) => {
            path
        }
        _ => name,
    }
}
