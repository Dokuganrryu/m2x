use super::input::{Input, InputError, InputErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKey {
    InterruptRoot,
    FactoryAccessor,
    RomFlashApiEntry,
    DacRegisterZeroWriter,
    DeadStripped,
}

const EVERY_KEY: [RuleKey; 5] = [
    RuleKey::InterruptRoot,
    RuleKey::FactoryAccessor,
    RuleKey::RomFlashApiEntry,
    RuleKey::DacRegisterZeroWriter,
    RuleKey::DeadStripped,
];

impl RuleKey {
    pub fn word(self) -> &'static str {
        match self {
            RuleKey::InterruptRoot => "interrupt-root",
            RuleKey::FactoryAccessor => "factory-accessor",
            RuleKey::RomFlashApiEntry => "rom-flash-api-entry",
            RuleKey::DacRegisterZeroWriter => "dac-register-zero-writer",
            RuleKey::DeadStripped => "dead-stripped",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        EVERY_KEY.into_iter().find(|key| key.word() == word)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FactoryAccessor {
    Named(String),
    NoneInThisImage,
}

pub struct Rules {
    pub interrupt_roots: Vec<String>,
    pub factory_accessor: FactoryAccessor,
    pub rom_flash_api_entries: Vec<u32>,
    pub dac_register_zero_writers: Vec<String>,
    pub dead_stripped: Vec<String>,
}

pub fn read(text: &str) -> Result<Rules, InputError> {
    let mut interrupt_roots = Vec::new();
    let mut factory_accessor = None;
    let mut rom_flash_api_entries = Vec::new();
    let mut dac_register_zero_writers = Vec::new();
    let mut no_dac_register_zero_writer = false;
    let mut dead_stripped = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |kind| InputError::at(Input::Rules, line, kind);
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let (word, value) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let key = RuleKey::from_word(word)
            .ok_or_else(|| fail(InputErrorKind::UnknownRuleKey(word.to_string())))?;
        let value = value.trim();
        if value.is_empty() {
            return Err(fail(InputErrorKind::MissingValue(key)));
        }
        match key {
            RuleKey::InterruptRoot => interrupt_roots.push(value.to_string()),
            RuleKey::FactoryAccessor => {
                let accessor = match value {
                    "none" => FactoryAccessor::NoneInThisImage,
                    name => FactoryAccessor::Named(name.to_string()),
                };
                if factory_accessor.replace(accessor).is_some() {
                    return Err(fail(InputErrorKind::SecondFactoryAccessor));
                }
            }
            RuleKey::RomFlashApiEntry => rom_flash_api_entries.push(
                read_address(value)
                    .ok_or_else(|| fail(InputErrorKind::UnreadableAddress(value.to_string())))?,
            ),
            RuleKey::DacRegisterZeroWriter => {
                match value {
                    "none" => no_dac_register_zero_writer = true,
                    path => dac_register_zero_writers.push(path.to_string()),
                }
                if no_dac_register_zero_writer && !dac_register_zero_writers.is_empty() {
                    return Err(fail(InputErrorKind::NoneBesideNamed(key)));
                }
            }
            RuleKey::DeadStripped => dead_stripped.push(value.to_string()),
        }
    }
    let without_data = |key| InputError::whole(Input::Rules, InputErrorKind::RuleWithoutData(key));
    if interrupt_roots.is_empty() {
        return Err(without_data(RuleKey::InterruptRoot));
    }
    let factory_accessor =
        factory_accessor.ok_or_else(|| without_data(RuleKey::FactoryAccessor))?;
    if rom_flash_api_entries.is_empty() {
        return Err(without_data(RuleKey::RomFlashApiEntry));
    }
    if dac_register_zero_writers.is_empty() && !no_dac_register_zero_writer {
        return Err(without_data(RuleKey::DacRegisterZeroWriter));
    }
    if dead_stripped.is_empty() {
        return Err(without_data(RuleKey::DeadStripped));
    }
    Ok(Rules {
        interrupt_roots,
        factory_accessor,
        rom_flash_api_entries,
        dac_register_zero_writers,
        dead_stripped,
    })
}

fn read_address(text: &str) -> Option<u32> {
    u32::from_str_radix(text.strip_prefix("0x")?, 16).ok()
}
