use super::input::{Input, InputError, InputErrorKind};

pub struct Symbol {
    pub line: usize,
    pub name: String,
}

pub fn read(text: &str) -> Result<Vec<Symbol>, InputError> {
    let mut symbols = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let name = symbol_name(raw)
            .ok_or_else(|| InputError::at(Input::Symbols, line, InputErrorKind::UnreadableLine))?;
        refuse_mangled(Input::Symbols, line, name)?;
        symbols.push(Symbol {
            line,
            name: name.to_string(),
        });
    }
    if symbols.is_empty() {
        return Err(InputError::whole(Input::Symbols, InputErrorKind::Empty));
    }
    Ok(symbols)
}

fn symbol_name(raw: &str) -> Option<&str> {
    let rest = match raw.strip_prefix(' ') {
        Some(_) => raw.trim_start(),
        None => {
            let (address, rest) = raw.split_once(' ')?;
            if address.is_empty() || !address.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            rest
        }
    };
    let mut kind = rest.chars();
    let type_letter = kind.next()?;
    let name = kind.as_str().strip_prefix(' ')?;
    (type_letter.is_ascii_alphabetic() && !name.trim().is_empty()).then_some(name)
}

pub fn refuse_mangled(input: Input, line: usize, name: &str) -> Result<(), InputError> {
    let mut after_r = name.strip_prefix("_R").unwrap_or("").chars();
    let v0 = after_r
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if v0 || name.starts_with("_ZN") {
        Err(InputError::at(
            input,
            line,
            InputErrorKind::MangledName(name.to_string()),
        ))
    } else {
        Ok(())
    }
}
