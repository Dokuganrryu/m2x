use super::share::Block;
use dsp::FramesPerTick;
use std::fmt;

const HALFWORD_BYTES: u32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub struct ListingError {
    pub line: usize,
    pub kind: ListingErrorKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ListingErrorKind {
    MalformedBlockHeader,
    UnknownBlock(String),
    UnknownFrameCount(String),
    InstructionBeforeAnyBlock,
    MissingEncoding,
    DataInPath(String),
    UnreadableLine,
    EmptyBlock(&'static str),
    NoProxyPrice(String),
}

pub struct Instruction<'a> {
    pub line: usize,
    pub address: u32,
    pub size: u32,
    pub mnemonic: &'a str,
    pub operands: &'a str,
}

pub struct BlockPath<'a> {
    pub block: Block,
    pub frames: FramesPerTick,
    pub instructions: Vec<Instruction<'a>>,
}

enum Line<'a> {
    Ignored,
    BlockHeader(Block, FramesPerTick),
    Instruction(Instruction<'a>),
}

pub fn block_paths(text: &str) -> Result<Vec<BlockPath<'_>>, ListingError> {
    let mut paths: Vec<(usize, BlockPath<'_>)> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |kind| ListingError { line, kind };
        match read_line(line, raw).map_err(fail)? {
            Line::Ignored => {}
            Line::BlockHeader(block, frames) => paths.push((
                line,
                BlockPath {
                    block,
                    frames,
                    instructions: Vec::new(),
                },
            )),
            Line::Instruction(instruction) => match paths.last_mut() {
                Some((_, path)) => path.instructions.push(instruction),
                None => return Err(fail(ListingErrorKind::InstructionBeforeAnyBlock)),
            },
        }
    }
    paths
        .into_iter()
        .map(|(line, path)| {
            if path.instructions.is_empty() {
                Err(ListingError {
                    line,
                    kind: ListingErrorKind::EmptyBlock(path.block.id()),
                })
            } else {
                Ok(path)
            }
        })
        .collect()
}

fn read_line(line: usize, raw: &str) -> Result<Line<'_>, ListingErrorKind> {
    let text = raw.trim();
    if text.is_empty() || is_objdump_framing(text) {
        return Ok(Line::Ignored);
    }
    if let Some(header) = text.strip_prefix("block ") {
        return read_block_header(header);
    }
    read_instruction(line, text).map(Line::Instruction)
}

fn is_objdump_framing(text: &str) -> bool {
    text.starts_with("Disassembly of section")
        || text.contains("file format")
        || (text.ends_with(">:") && text.contains(" <"))
}

fn read_block_header(header: &str) -> Result<Line<'_>, ListingErrorKind> {
    let words: Vec<&str> = header.split_whitespace().collect();
    let [id, "frames", count] = words.as_slice() else {
        return Err(ListingErrorKind::MalformedBlockHeader);
    };
    let block =
        Block::from_id(id).ok_or_else(|| ListingErrorKind::UnknownBlock((*id).to_string()))?;
    let frames = match *count {
        "6" => FramesPerTick::Six,
        "12" => FramesPerTick::Twelve,
        "24" => FramesPerTick::TwentyFour,
        other => return Err(ListingErrorKind::UnknownFrameCount(other.to_string())),
    };
    Ok(Line::BlockHeader(block, frames))
}

pub(super) fn read_instruction(
    line: usize,
    text: &str,
) -> Result<Instruction<'_>, ListingErrorKind> {
    let (address, mut rest) = text
        .split_once(':')
        .and_then(|(address, rest)| Some((u32::from_str_radix(address, 16).ok()?, rest)))
        .ok_or(ListingErrorKind::UnreadableLine)?;
    let mut halfwords = 0;
    loop {
        let (word, tail) = split_first_word(rest);
        if !is_encoding_halfword(word) {
            break;
        }
        halfwords += 1;
        rest = tail;
    }
    let (mnemonic, operands) = split_first_word(rest);
    if mnemonic.is_empty() || mnemonic.starts_with('<') {
        return Err(ListingErrorKind::UnreadableLine);
    }
    if mnemonic.starts_with('.') {
        return Err(ListingErrorKind::DataInPath(mnemonic.to_string()));
    }
    if halfwords == 0 {
        return Err(ListingErrorKind::MissingEncoding);
    }
    let operands = operands.split('@').next().unwrap_or("").trim();
    Ok(Instruction {
        line,
        address,
        size: halfwords * HALFWORD_BYTES,
        mnemonic,
        operands,
    })
}

fn split_first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    text.split_once(char::is_whitespace).unwrap_or((text, ""))
}

fn is_encoding_halfword(word: &str) -> bool {
    word.len() == 4 && word.chars().all(|c| c.is_ascii_hexdigit())
}

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "listing line {}: ", self.line)?;
        match &self.kind {
            ListingErrorKind::MalformedBlockHeader => {
                f.write_str("a block header reads `block <block-id> frames <6|12|24>`")
            }
            ListingErrorKind::UnknownBlock(id) => write!(f, "`{id}` is not a budgeted block"),
            ListingErrorKind::UnknownFrameCount(count) => {
                write!(f, "{count} frames per tick is not 6, 12 or 24")
            }
            ListingErrorKind::InstructionBeforeAnyBlock => {
                f.write_str("an instruction comes before any block header")
            }
            ListingErrorKind::MissingEncoding => f.write_str(
                "the instruction has no encoding halfwords, so its size and the branch outcome are unknown",
            ),
            ListingErrorKind::DataInPath(directive) => {
                write!(f, "`{directive}` is data, not an executed instruction")
            }
            ListingErrorKind::UnreadableLine => f.write_str(
                "the line is neither a block header, an instruction nor llvm-objdump framing",
            ),
            ListingErrorKind::EmptyBlock(id) => write!(f, "block `{id}` holds no instruction"),
            ListingErrorKind::NoProxyPrice(mnemonic) => write!(
                f,
                "`{mnemonic}` has no price in the proxy table (crates/xtask/src/price)"
            ),
        }
    }
}

impl std::error::Error for ListingError {}
