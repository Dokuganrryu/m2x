use super::rules::RuleKey;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Rules,
    Symbols,
    InstructionListing,
    DataSections,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InputError {
    pub input: Input,
    pub line: Option<usize>,
    pub kind: InputErrorKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InputErrorKind {
    Empty,
    UnreadableLine,
    UnknownRuleKey(String),
    MissingValue(RuleKey),
    UnreadableAddress(String),
    SecondFactoryAccessor,
    NoneBesideNamed(RuleKey),
    RuleWithoutData(RuleKey),
    NotInListing(String),
    NotAnArmListing,
    UnrecognisedSection(String),
    UnrecognisedDataSection(String),
    LineOutsideAnyFunction,
    UndecodedInstruction,
    UnreadableImmediate(String),
    MangledName(String),
    UnresolvedCallTarget,
}

impl InputError {
    pub fn at(input: Input, line: usize, kind: InputErrorKind) -> Self {
        InputError {
            input,
            line: Some(line),
            kind,
        }
    }

    pub fn whole(input: Input, kind: InputErrorKind) -> Self {
        InputError {
            input,
            line: None,
            kind,
        }
    }
}

impl fmt::Display for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Input::Rules => "rules",
            Input::Symbols => "llvm-nm listing",
            Input::InstructionListing => "llvm-objdump listing",
            Input::DataSections => "llvm-objdump section dump",
        })
    }
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{} line {line}: ", self.input)?,
            None => write!(f, "{}: ", self.input)?,
        }
        match &self.kind {
            InputErrorKind::Empty => f.write_str("the input holds nothing to scan"),
            InputErrorKind::UnreadableLine => f.write_str("the line has no form the scan reads"),
            InputErrorKind::UnknownRuleKey(key) => write!(f, "`{key}` is not a rule key"),
            InputErrorKind::MissingValue(key) => write!(f, "`{}` needs a value", key.word()),
            InputErrorKind::UnreadableAddress(text) => {
                write!(f, "`{text}` is not a 0x-prefixed 32-bit address")
            }
            InputErrorKind::SecondFactoryAccessor => {
                f.write_str("only one factory accessor may be named")
            }
            InputErrorKind::NoneBesideNamed(key) => write!(
                f,
                "`{} none` and a named `{}` contradict each other",
                key.word(),
                key.word()
            ),
            InputErrorKind::RuleWithoutData(key) => write!(
                f,
                "no `{}` line, and the scan refuses to run a rule without its data",
                key.word()
            ),
            InputErrorKind::NotInListing(name) => {
                write!(
                    f,
                    "`{name}` has no body in the instruction_listing, so it cannot be checked"
                )
            }
            InputErrorKind::NotAnArmListing => f.write_str(
                "the first line is not the `file format elf32-littlearm` header of llvm-objdump -d",
            ),
            InputErrorKind::UnrecognisedSection(section) => {
                write!(
                    f,
                    "section `{section}` is not .text, so the scan does not know its rules"
                )
            }
            InputErrorKind::UnrecognisedDataSection(section) => write!(
                f,
                "section `{section}` has no policy: the scan neither reads it as loaded data nor knows it holds none"
            ),
            InputErrorKind::LineOutsideAnyFunction => {
                f.write_str("the line comes before any section and function header")
            }
            InputErrorKind::UndecodedInstruction => f.write_str(
                "llvm-objdump could not decode this word; decode with --mcpu=cortex-m33",
            ),
            InputErrorKind::UnreadableImmediate(text) => {
                write!(f, "`{text}` is not an integer immediate")
            }
            InputErrorKind::MangledName(name) => write!(
                f,
                "`{name}` is mangled; produce the listing with --demangle"
            ),
            InputErrorKind::UnresolvedCallTarget => f.write_str("the call names no target symbol"),
        }
    }
}

impl std::error::Error for InputError {}
