use super::input::{Input, InputError, InputErrorKind};
use super::symbols::refuse_mangled;

const ARM_FORMAT: &str = "elf32-littlearm";
const LINKER_TRAP_PADDING: &str = "d4d4";

pub struct Function {
    pub name: String,
    pub line: usize,
    pub lines: Vec<Line>,
}

pub enum Line {
    Instruction(Instruction),
    Data(Data),
}

pub struct Instruction {
    pub line: usize,
    pub mnemonic: String,
    pub operands: String,
    pub literal_address: Option<u32>,
}

pub struct Data {
    pub line: usize,
    pub address: u32,
    pub value: u32,
}

enum Reading {
    BeforeHeader,
    BeforeSection,
    InSection,
}

pub fn read(text: &str) -> Result<Vec<Function>, InputError> {
    let mut reading = Reading::BeforeHeader;
    let mut functions: Vec<Function> = Vec::new();
    let mut trailing_padding = 0;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |kind| InputError::at(Input::InstructionListing, line, kind);
        let text = raw.trim();
        if text.is_empty() || text == "..." {
            continue;
        }
        match reading {
            Reading::BeforeHeader => {
                match text.split_once("file format ") {
                    Some((_, ARM_FORMAT)) => reading = Reading::BeforeSection,
                    _ => return Err(fail(InputErrorKind::NotAnArmListing)),
                }
                continue;
            }
            Reading::BeforeSection | Reading::InSection => {}
        }
        if let Some(section) = text
            .strip_prefix("Disassembly of section ")
            .and_then(|rest| rest.strip_suffix(':'))
        {
            if section != ".text" && !section.starts_with(".text.") {
                return Err(fail(InputErrorKind::UnrecognisedSection(
                    section.to_string(),
                )));
            }
            reading = Reading::InSection;
            continue;
        }
        if let Some(name) = function_header(text) {
            if !matches!(reading, Reading::InSection) {
                return Err(fail(InputErrorKind::LineOutsideAnyFunction));
            }
            refuse_mangled(Input::InstructionListing, line, name)?;
            drop_trailing_padding(functions.last_mut(), &mut trailing_padding);
            functions.push(Function {
                name: name.to_string(),
                line,
                lines: Vec::new(),
            });
            continue;
        }
        let (read, is_padding) = read_line(line, text)?;
        let function = functions
            .last_mut()
            .ok_or_else(|| fail(InputErrorKind::LineOutsideAnyFunction))?;
        function.lines.push(read);
        trailing_padding = if is_padding { trailing_padding + 1 } else { 0 };
    }
    drop_trailing_padding(functions.last_mut(), &mut trailing_padding);
    if matches!(reading, Reading::BeforeHeader) {
        return Err(InputError::whole(
            Input::InstructionListing,
            InputErrorKind::NotAnArmListing,
        ));
    }
    if functions.is_empty() {
        return Err(InputError::whole(
            Input::InstructionListing,
            InputErrorKind::Empty,
        ));
    }
    Ok(functions)
}

fn drop_trailing_padding(function: Option<&mut Function>, trailing_padding: &mut usize) {
    if let Some(function) = function {
        function
            .lines
            .truncate(function.lines.len() - *trailing_padding);
    }
    *trailing_padding = 0;
}

fn function_header(text: &str) -> Option<&str> {
    let (address, rest) = text.split_once(' ')?;
    if !is_hex(address) {
        return None;
    }
    rest.strip_prefix('<')?.strip_suffix(">:")
}

fn read_line(line: usize, text: &str) -> Result<(Line, bool), InputError> {
    let fail = |kind| InputError::at(Input::InstructionListing, line, kind);
    let (address, mut rest) = text
        .split_once(": ")
        .filter(|(address, _)| is_hex(address))
        .ok_or_else(|| fail(InputErrorKind::UnreadableLine))?;
    let address =
        u32::from_str_radix(address, 16).map_err(|_| fail(InputErrorKind::UnreadableLine))?;
    let mut encoding = String::new();
    loop {
        let (word, tail) = split_first_word(rest);
        if !is_encoding_word(word) {
            break;
        }
        encoding.push_str(word);
        rest = tail;
    }
    let (mnemonic, operands) = split_first_word(rest);
    if encoding.is_empty() || mnemonic.is_empty() {
        return Err(fail(InputErrorKind::UnreadableLine));
    }
    if mnemonic == "<unknown>" {
        return Err(fail(InputErrorKind::UndecodedInstruction));
    }
    let (operands, annotation) = match operands.split_once('@') {
        Some((operands, annotation)) => (operands.trim(), Some(annotation.trim())),
        None => (operands.trim(), None),
    };
    if let Some(directive) = mnemonic.strip_prefix('.') {
        let value = match directive {
            "word" | "short" | "byte" | "long" => read_hex(operands),
            _ => None,
        }
        .ok_or_else(|| fail(InputErrorKind::UnreadableLine))?;
        return Ok((
            Line::Data(Data {
                line,
                address,
                value,
            }),
            false,
        ));
    }
    let literal_address = annotation
        .and_then(|annotation| annotation.split_whitespace().next())
        .and_then(read_hex);
    Ok((
        Line::Instruction(Instruction {
            line,
            mnemonic: mnemonic.to_string(),
            operands: operands.to_string(),
            literal_address,
        }),
        encoding == LINKER_TRAP_PADDING,
    ))
}

fn read_hex(text: &str) -> Option<u32> {
    u32::from_str_radix(text.strip_prefix("0x")?, 16).ok()
}

fn split_first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    text.split_once(char::is_whitespace).unwrap_or((text, ""))
}

fn is_hex(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_encoding_word(word: &str) -> bool {
    matches!(word.len(), 2 | 4) && is_hex(word)
}
