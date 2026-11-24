use super::constants;
use super::input::{Input, InputError, InputErrorKind};
use super::instruction_listing::{Data, Function, Line};
use super::rules;
use super::violation::Violation;

const ARM_FORMAT: &str = "elf32-littlearm";
const HEX_COLUMNS: usize = 35;
const WORD_BYTES: usize = 4;
const LOADED_DATA_SECTIONS: [&str; 4] = [".vector_table", ".rodata", ".data", ".gnu.sgstubs"];
const CODE_SECTIONS: [&str; 1] = [".text"];
const UNLOADED_SECTIONS: [&str; 10] = [
    ".running_vectors",
    ".bss",
    ".uninit",
    ".panel_buffers",
    ".comment",
    ".ARM.attributes",
    ".symtab",
    ".strtab",
    ".shstrtab",
    ".debug",
];

enum Policy {
    ReadWords,
    LeaveToTheInstructionListing,
    NotLoaded,
}

struct Byte {
    line: usize,
    value: u8,
}

struct Section {
    name: String,
    start: u32,
    bytes: Vec<Byte>,
}

enum Reading {
    BeforeHeader,
    BeforeSection,
    InSection(Policy),
}

pub fn scan_data_sections(rules_text: &str, dump_text: &str) -> Result<Vec<Violation>, InputError> {
    let rules = rules::read(rules_text)?;
    let words: Vec<Function> = read(dump_text)?
        .into_iter()
        .map(Section::into_words)
        .collect();
    constants::check(&rules, &words)
}

fn read(text: &str) -> Result<Vec<Section>, InputError> {
    let mut reading = Reading::BeforeHeader;
    let mut sections: Vec<Section> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |kind| InputError::at(Input::DataSections, line, kind);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Reading::BeforeHeader = reading {
            match trimmed.split_once("file format ") {
                Some((_, ARM_FORMAT)) => reading = Reading::BeforeSection,
                _ => return Err(fail(InputErrorKind::NotAnArmListing)),
            }
            continue;
        }
        if let Some(name) = trimmed
            .strip_prefix("Contents of section ")
            .and_then(|rest| rest.strip_suffix(':'))
        {
            let policy = policy_of(name)
                .ok_or_else(|| fail(InputErrorKind::UnrecognisedDataSection(name.to_string())))?;
            if let Policy::ReadWords = policy {
                sections.push(Section {
                    name: name.to_string(),
                    start: 0,
                    bytes: Vec::new(),
                });
            }
            reading = Reading::InSection(policy);
            continue;
        }
        match &reading {
            Reading::BeforeHeader | Reading::BeforeSection => {
                return Err(fail(InputErrorKind::LineOutsideAnyFunction));
            }
            Reading::InSection(Policy::LeaveToTheInstructionListing | Policy::NotLoaded) => {}
            Reading::InSection(Policy::ReadWords) => {
                let section = sections
                    .last_mut()
                    .ok_or_else(|| fail(InputErrorKind::LineOutsideAnyFunction))?;
                section
                    .append(line, raw)
                    .ok_or_else(|| fail(InputErrorKind::UnreadableLine))?;
            }
        }
    }
    if let Reading::BeforeHeader = reading {
        return Err(InputError::whole(
            Input::DataSections,
            InputErrorKind::NotAnArmListing,
        ));
    }
    if sections.iter().all(|section| section.bytes.is_empty()) {
        return Err(InputError::whole(
            Input::DataSections,
            InputErrorKind::Empty,
        ));
    }
    Ok(sections)
}

fn policy_of(name: &str) -> Option<Policy> {
    let belongs_to = |family: &str| name == family || name.starts_with(&format!("{family}."));
    if LOADED_DATA_SECTIONS.into_iter().any(belongs_to) {
        Some(Policy::ReadWords)
    } else if CODE_SECTIONS.into_iter().any(belongs_to) {
        Some(Policy::LeaveToTheInstructionListing)
    } else if UNLOADED_SECTIONS.into_iter().any(belongs_to) || name.starts_with(".debug_") {
        Some(Policy::NotLoaded)
    } else {
        None
    }
}

impl Section {
    fn append(&mut self, line: usize, raw: &str) -> Option<()> {
        let text = raw.trim_start();
        let (address, rest) = text.split_once(' ')?;
        let address = u32::from_str_radix(address, 16).ok()?;
        if self.bytes.is_empty() {
            self.start = address;
        }
        let expected = self
            .start
            .checked_add(u32::try_from(self.bytes.len()).ok()?)?;
        if address != expected {
            return None;
        }
        let columns = rest.get(..HEX_COLUMNS).unwrap_or(rest);
        for group in columns.split_whitespace() {
            if group.len() % 2 != 0 || group.len() > 2 * WORD_BYTES {
                return None;
            }
            for pair in group.as_bytes().chunks(2) {
                let pair = std::str::from_utf8(pair).ok()?;
                let value = u8::from_str_radix(pair, 16).ok()?;
                self.bytes.push(Byte { line, value });
            }
        }
        Some(())
    }

    fn into_words(self) -> Function {
        let misalignment = (WORD_BYTES - self.start as usize % WORD_BYTES) % WORD_BYTES;
        let lines = self
            .bytes
            .get(misalignment..)
            .unwrap_or_default()
            .chunks_exact(WORD_BYTES)
            .enumerate()
            .map(|(index, word)| {
                Line::Data(Data {
                    line: word[0].line,
                    address: self.start + (misalignment + index * WORD_BYTES) as u32,
                    value: u32::from_le_bytes([
                        word[0].value,
                        word[1].value,
                        word[2].value,
                        word[3].value,
                    ]),
                })
            })
            .collect();
        Function {
            line: self.bytes.first().map_or(0, |byte| byte.line),
            name: format!("section {}", self.name),
            lines,
        }
    }
}
